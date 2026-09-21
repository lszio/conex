use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use serde_json::{Value, json};
use tokio::time::interval;
use tokio_tungstenite::Connector;
use tokio_tungstenite::tungstenite::Message;

use conex_core::transport_ws::{BootstrapFrame, BootstrapState, ClientHandshake, ProfileId};
use conex_proto::v1;
use conex_proto::wire::{decode_wire, encode_wire};
use conex_provider_fs::{FsRoot, list, read::MAX_DOC_BYTES};
use conex_source::contracts::{DEFAULT_PAGE, MAX_ITEMS};
use conex_source::resource::normalize_resource;

use crate::config::{AgentConfig, EndpointConfig};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const RECONNECT_MAX_SECS: u64 = 30;
const REGISTER_REQUEST_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const HEARTBEAT_REQUEST_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAW";

pub async fn run(config: AgentConfig) -> Result<(), String> {
    let token = read_secret(&config.token_backend)?;
    let mut backoff_secs = 1;
    loop {
        match connect_once(&config, &token).await {
            Ok(()) => backoff_secs = 1,
            Err(error) => {
                eprintln!("agent link closed: {error}");
                let jitter_ms = now_millis() % 250;
                tokio::time::sleep(Duration::from_secs(backoff_secs) + Duration::from_millis(jitter_ms)).await;
                backoff_secs = (backoff_secs.saturating_mul(2)).min(RECONNECT_MAX_SECS);
            }
        }
    }
}

async fn connect_once(config: &AgentConfig, token: &str) -> Result<(), String> {
    let mut request = config
        .host_url
        .as_str()
        .into_client_request()
        .map_err(|error| format!("build websocket request: {error}"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {token}")
            .parse()
            .map_err(|error| format!("build authorization header: {error}"))?,
    );
    request.headers_mut().insert(
        "User-Agent",
        "conex-agent/0"
            .parse()
            .expect("static user-agent header is valid"),
    );
    if let Some(origin) = &config.host_origin {
        request.headers_mut().insert(
            "Origin",
            origin
                .parse()
                .map_err(|error| format!("build origin header: {error}"))?,
        );
    }
    let connector = Some(Connector::Rustls(Arc::new(tls_config(config)?)));
    if let Some(expected) = &config.expected_server_name {
        let host = request
            .uri()
            .host()
            .ok_or_else(|| "host_url must include a server name".to_string())?;
        if expected != host {
            return Err("expected_server_name does not match host_url".into());
        }
    }
    let (mut socket, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, connector),
    )
    .await
    .map_err(|_| "websocket connect timed out".to_string())?
    .map_err(|error| format!("connect websocket: {error}"))?;

    let methods = config
        .endpoints
        .iter()
        .flat_map(|endpoint| endpoint.methods.iter().cloned())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut handshake = ClientHandshake::new();
    let hello = handshake
        .build_hello(ProfileId::JsonRpc2WssV1, v1::Plane::Broker, methods.clone(), vec![])
        .map_err(|error| error.to_string())?;
    socket
        .send(Message::Text(hello.json.into()))
        .await
        .map_err(|error| format!("send hello: {error}"))?;
    let ready = recv_text(&mut socket).await?;
    let ready_request = handshake
        .ingest(BootstrapFrame { json: ready })
        .map_err(|error| error.to_string())?;
    let ready_frame = handshake
        .build_ready_frame(&ready_request)
        .map_err(|error| error.to_string())?;
    socket
        .send(Message::Text(ready_frame.json.into()))
        .await
        .map_err(|error| format!("send ready: {error}"))?;
    let ready_result = recv_text(&mut socket).await?;
    handshake
        .ingest(BootstrapFrame { json: ready_result })
        .map_err(|error| error.to_string())?;
    if !matches!(handshake.state(), BootstrapState::Ready { .. }) {
        return Err("websocket handshake did not become ready".into());
    }

    let host_origin = config.host_origin.clone().unwrap_or_else(|| {
        let authority = config
            .host_url
            .split("//")
            .nth(1)
            .and_then(|value| value.split(['/', '?', '#']).next())
            .unwrap_or("host");
        format!("conex://{authority}")
    });
    let registration = json!({
        "agentId": config.agent_id,
        "providerIds": config.endpoints.iter().map(|endpoint| endpoint.endpoint_id.clone()).collect::<Vec<_>>(),
        "methods": methods,
        "resources": config.endpoints.iter().flat_map(|endpoint| endpoint.resources.iter().cloned()).collect::<Vec<_>>(),
        "hostOrigin": host_origin,
    });
    send_call(&mut socket, "agent/register", registration, REGISTER_REQUEST_ID).await?;
    await_response(&mut socket, REGISTER_REQUEST_ID).await?;

    let providers = config
        .endpoints
        .iter()
        .map(LocalProvider::new)
        .collect::<Result<Vec<_>, _>>()?;
    let mut heartbeat = interval(HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                send_call(&mut socket, "agent/heartbeat", json!({"agentId": config.agent_id}), HEARTBEAT_REQUEST_ID).await?;
            }
            frame = tokio::time::timeout(Duration::from_secs(45), socket.next()) => {
                let frame = frame
                    .map_err(|_| "websocket read timed out".to_string())?;
                let Some(frame) = frame else { return Err("websocket closed".into()); };
                let frame = frame.map_err(|error| format!("receive websocket frame: {error}"))?;
                match frame {
                    Message::Text(text) => {
                        if let Ok(message) = decode_wire(text.as_bytes()) {
                            match message.body {
                                Some(v1::message::Body::Success(success))
                                    if success.request_id == HEARTBEAT_REQUEST_ID => {}
                                Some(v1::message::Body::Failure(failure))
                                    if failure.request_id.as_deref() == Some(HEARTBEAT_REQUEST_ID) =>
                                {
                                    let message = failure.error.map(|error| error.message).unwrap_or_else(|| "heartbeat rejected".into());
                                    return Err(format!("heartbeat rejected: {message}"));
                                }
                                Some(v1::message::Body::Request(_)) => {
                                    if let Some(reply) = handle_request(&providers, text.as_bytes()).await {
                                        socket.send(Message::Text(reply.into())).await.map_err(|error| format!("send response: {error}"))?;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    Message::Ping(payload) => socket.send(Message::Pong(payload)).await.map_err(|error| format!("send pong: {error}"))?,
                    Message::Close(_) => return Err("host closed websocket".into()),
                    Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
                }
            }
        }
    }
}

async fn recv_text<S>(socket: &mut S) -> Result<String, String>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .map_err(|_| "websocket handshake read timed out".to_string())?;
        let Some(frame) = frame else { return Err("websocket closed during handshake".into()); };
        match frame.map_err(|error| format!("receive handshake frame: {error}"))? {
            Message::Text(text) => return Ok(text.to_string()),
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => return Err("websocket closed during handshake".into()),
            Message::Binary(_) | Message::Frame(_) => return Err("binary frame during handshake".into()),
        }
    }
}

async fn send_call<S>(socket: &mut S, method: &str, input: Value, request_id: &str) -> Result<(), String>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let message = v1::Message {
        body: Some(v1::message::Body::Request(v1::Request {
            request_id: request_id.into(),
            method: method.into(),
            params: Some(v1::CallParams {
                context: Some(v1::RequestContext {
                    provider_endpoint_id: "agent-mgr".into(),
                    plane: v1::Plane::Broker as i32,
                    binding_id: None,
                }),
                timeout_budget_ms: 8_000,
                input: Some(serde_json::from_value(input).unwrap_or_default()),
            }),
        })),
    };
    let bytes = encode_wire(&message).map_err(|error| error.message)?;
    socket
        .send(Message::Text(String::from_utf8(bytes).map_err(|error| error.to_string())?.into()))
        .await
        .map_err(|error| format!("send {method}: {error}"))
}
async fn await_response<S>(socket: &mut S, request_id: &str) -> Result<(), String>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures::Sink<Message>
        + Unpin,
    S::Error: std::fmt::Display,
{
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .map_err(|_| "response read timed out".to_string())?;
        let Some(frame) = frame else {
            return Err("websocket closed waiting for response".into());
        };
        match frame.map_err(|error| format!("receive response: {error}"))? {
            Message::Text(text) => {
                let message = decode_wire(text.as_bytes())
                    .map_err(|error| format!("decode response: {}", error.message))?;
                match message.body {
                    Some(v1::message::Body::Success(success))
                        if success.request_id == request_id =>
                    {
                        return Ok(());
                    }
                    Some(v1::message::Body::Failure(failure))
                        if failure.request_id.as_deref() == Some(request_id) =>
                    {
                        let message = failure
                            .error
                            .map(|error| error.message)
                            .unwrap_or_else(|| "request rejected".into());
                        return Err(format!("{request_id} rejected: {message}"));
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => socket
                .send(Message::Pong(payload))
                .await
                .map_err(|error| format!("send pong: {error}"))?,
            Message::Close(_) => return Err("host closed websocket".into()),
            Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}


async fn handle_request(providers: &[LocalProvider], bytes: &[u8]) -> Option<String> {
    let message = decode_wire(bytes).ok()?;
    let request = match message.body? {
        v1::message::Body::Request(request) => request,
        _ => return None,
    };
    let result = dispatch_local(providers, &request.method, request.params.as_ref().and_then(|params| params.input.as_ref()).map(|value| serde_json::to_value(value).unwrap_or(Value::Null)).unwrap_or(Value::Null), request.params.as_ref().and_then(|params| params.context.as_ref()).map(|context| context.provider_endpoint_id.as_str()).unwrap_or("")).await;
    let body = match result {
        Ok(value) => v1::message::Body::Success(v1::Success { request_id: request.request_id, result: Some(serde_json::from_value(value).unwrap_or_default()) }),
        Err(error) => v1::message::Body::Failure(v1::Failure { request_id: Some(request.request_id), error: Some(v1::Error { code: error.code(), message: error.message().into(), ..Default::default() }) }),
    };
    let bytes = encode_wire(&v1::Message { body: Some(body) }).ok()?;
    String::from_utf8(bytes).ok()
}

async fn dispatch_local(providers: &[LocalProvider], method: &str, input: Value, endpoint_id: &str) -> Result<Value, conex_core::CallError> {
    let provider = providers.iter().find(|provider| provider.endpoint_id == endpoint_id).ok_or_else(|| conex_core::CallError::new(v1::ErrorCode::UnknownProvider, "unknown configured endpoint"))?;
    if !provider.methods.contains(method) {
        return Err(conex_core::CallError::new(v1::ErrorCode::UnsupportedCapability, "method is not enabled for endpoint"));
    }
    provider.dispatch(method, input)
}

struct LocalProvider {
    endpoint_id: String,
    root: Arc<FsRoot>,
    methods: HashSet<String>,
    resources: Vec<String>,
}

impl LocalProvider {
    fn new(config: &EndpointConfig) -> Result<Self, String> {
        Ok(Self {
            endpoint_id: config.endpoint_id.clone(),
            root: Arc::new(FsRoot::open(Path::new(&config.root)).map_err(|error| error.message().to_string())?),
            methods: config.methods.iter().cloned().collect(),
            resources: config.resources.clone(),
        })
    }

    fn allowed(&self, resource: &str) -> bool {
        self.resources.iter().any(|root| root == "*" || root.is_empty() || resource == root || resource.strip_prefix(root).is_some_and(|rest| rest.starts_with('/')))
    }

    fn dispatch(&self, method: &str, input: Value) -> Result<Value, conex_core::CallError> {
        match method {
            "source/read" => {
                let resource = input.get("resourceId").and_then(Value::as_str).ok_or_else(|| conex_core::CallError::new(v1::ErrorCode::BadRequest, "resourceId is required"))?;
                let resource = normalize_resource(resource)?;
                if !self.allowed(&resource) { return Err(conex_core::CallError::new(v1::ErrorCode::Forbidden, "resource is not configured")); }
                let snapshot = self.root.read(&resource, MAX_DOC_BYTES)?;
                let text = String::from_utf8(snapshot.bytes.to_vec()).map_err(|_| conex_core::CallError::new(v1::ErrorCode::BadRequest, "document is not valid UTF-8"))?;
                let summary = list::build_summary(&resource, snapshot.bytes.len() as u64)?;
                serde_json::to_value(v1::SourceReadResponse { resource: Some(summary), text, cid: snapshot.cid }).map_err(|error| conex_core::CallError::new(v1::ErrorCode::Internal, error.to_string()))
            }
            "source/list" | "source/search" => {
                let root = input.get("root").and_then(Value::as_str).unwrap_or("");
                let root = normalize_resource(root)?;
                if !self.allowed(&root) { return Err(conex_core::CallError::new(v1::ErrorCode::Forbidden, "root is not configured")); }
                let query = (method == "source/search").then(|| input.get("query").and_then(Value::as_str).unwrap_or(""));
                if method == "source/search" && query == Some("") { return Err(conex_core::CallError::new(v1::ErrorCode::BadRequest, "query is required")); }
                let items = list::scan(&self.root, &root, query, MAX_ITEMS)?;
                let limit = input.get("limit").and_then(Value::as_u64).unwrap_or(DEFAULT_PAGE as u64).min(100) as usize;
                if method == "source/list" {
                    let items = items.into_iter().take(limit).map(|item| list::build_summary(&item.resource_id, item.size_bytes)).collect::<Result<Vec<_>, _>>()?;
                    serde_json::to_value(v1::SourceListResponse { items, next_cursor: None }).map_err(|error| conex_core::CallError::new(v1::ErrorCode::Internal, error.to_string()))
                } else {
                    let items = items.into_iter().take(limit).map(|item| Ok(v1::SearchHit { resource: Some(list::build_summary(&item.resource_id, item.size_bytes)?), excerpt: item.excerpt.unwrap_or_default() })).collect::<Result<Vec<_>, conex_core::CallError>>()?;
                    serde_json::to_value(v1::SourceSearchResponse { items, next_cursor: None }).map_err(|error| conex_core::CallError::new(v1::ErrorCode::Internal, error.to_string()))
                }
            }
            _ => Err(conex_core::CallError::new(v1::ErrorCode::UnknownMethod, "unsupported local method")),
        }
    }
}

fn tls_config(config: &AgentConfig) -> Result<rustls::ClientConfig, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    if let Some(path) = &config.ca_pem {
        let bytes = std::fs::read(path).map_err(|error| format!("read ca_pem: {error}"))?;
        let mut reader = std::io::BufReader::new(bytes.as_slice());
        for cert in rustls_pemfile::certs(&mut reader) {
            let cert = cert.map_err(|error| format!("parse ca_pem: {error}"))?;
            roots.add(cert).map_err(|error| format!("add ca certificate: {error}"))?;
        }
    } else {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    Ok(rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth())
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
fn read_secret(source: &str) -> Result<String, String> {
    let value = if let Some(name) = source.strip_prefix("env:") {
        std::env::var(name).map_err(|error| format!("read token env: {error}"))?
    } else if let Some(path) = source.strip_prefix("file:") {
        std::fs::read_to_string(path).map_err(|error| format!("read token file: {error}"))?
    } else {
        return Err("token_backend must be env:NAME or file:PATH".into());
    };
    let value = value.trim().to_string();
    if value.is_empty() { return Err("agent token is empty".into()); }
    Ok(value)
}
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;

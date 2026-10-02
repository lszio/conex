use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::time::interval;
use tokio_tungstenite::Connector;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use conex_core::transport_ws::{BootstrapFrame, BootstrapState, ClientHandshake, ProfileId};
use conex_proto;
use conex_provider_fs::FsRoot;
use prost::Message as _;

use crate::config::{AgentConfig, EndpointConfig};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const RECONNECT_MAX_SECS: u64 = 30;
const MAX_INFLIGHT_REQUESTS: usize = 8;
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
                tokio::time::sleep(
                    Duration::from_secs(backoff_secs) + Duration::from_millis(jitter_ms),
                )
                .await;
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
    // M3: agent links negotiate the protobuf binary profile — content slices
    // ride raw bytes end to end, never base64 strings.
    let hello = handshake
        .build_hello(
            ProfileId::ProtobufWss,
            conex_proto::Plane::Broker,
            methods.clone(),
            vec![],
        )
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
    // Per-endpoint registration (plan M2): each configured endpoint is
    // claimed individually; the host reviews every claim and returns the
    // accepted set. Claims narrower than the host config are honored.
    let registration = json!({
        "agentId": config.agent_id,
        "endpoints": config.endpoints.iter().map(|endpoint| json!({
            "endpointId": endpoint.endpoint_id,
            "root": endpoint.resources.first().cloned().unwrap_or_else(|| "*".into()),
            "methods": endpoint.methods,
        })).collect::<Vec<_>>(),
        "hostOrigin": host_origin,
    });
    send_call(
        &mut socket,
        "agent/register",
        registration,
        REGISTER_REQUEST_ID,
    )
    .await?;
    let registered = await_response(&mut socket, REGISTER_REQUEST_ID).await?;
    let accepted: HashSet<String> = registered
        .get("acceptedEndpointIds")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    if accepted.is_empty() {
        return Err("registration accepted no endpoints".into());
    }
    let rejected: Vec<String> = config
        .endpoints
        .iter()
        .filter(|endpoint| !accepted.contains(&endpoint.endpoint_id))
        .map(|endpoint| endpoint.endpoint_id.clone())
        .collect();
    if !rejected.is_empty() {
        eprintln!("endpoints rejected by host: {}", rejected.join(", "));
    }

    let providers: Vec<EndpointProvider> = config
        .endpoints
        .iter()
        .filter(|endpoint| accepted.contains(&endpoint.endpoint_id))
        .map(EndpointProvider::new)
        .collect::<Result<Vec<_>, _>>()?;
    let providers = Arc::new(providers);
    // Bounded dispatch (plan M2): requests run on their own tasks with a
    // concurrency ceiling and the remaining deadline, so a slow scan never
    // blocks heartbeats, pongs, or other endpoints.
    let inflight = Arc::new(tokio::sync::Semaphore::new(MAX_INFLIGHT_REQUESTS));
    let (replies, mut reply_rx) = tokio::sync::mpsc::channel::<Message>(MAX_INFLIGHT_REQUESTS);
    let mut heartbeat = interval(HEARTBEAT_INTERVAL);
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                send_call(&mut socket, "agent/heartbeat", json!({"agentId": config.agent_id}), HEARTBEAT_REQUEST_ID).await?;
            }
            reply = reply_rx.recv() => {
                match reply {
                    Some(reply) => socket.send(reply).await.map_err(|error| format!("send response: {error}"))?,
                    None => return Err("dispatch queue closed".into()),
                }
            }
            frame = tokio::time::timeout(Duration::from_secs(45), socket.next()) => {
                let frame = frame
                    .map_err(|_| "websocket read timed out".to_string())?;
                let Some(frame) = frame else { return Err("websocket closed".into()); };
                let frame = frame.map_err(|error| format!("receive websocket frame: {error}"))?;
                match frame {
                    Message::Binary(bytes) => {
                        let Ok(message) = conex_proto::Message::decode(&bytes[..]) else {
                            return Err("undecodable binary frame on the protobuf link".into());
                        };
                        match message.body {
                            Some(conex_proto::message::Body::Success(success))
                                if success.request_id == HEARTBEAT_REQUEST_ID => {}
                            Some(conex_proto::message::Body::Failure(failure))
                                if failure.request_id.as_deref() == Some(HEARTBEAT_REQUEST_ID) =>
                            {
                                let message = failure.error.map(|error| error.message).unwrap_or_else(|| "heartbeat rejected".into());
                                return Err(format!("heartbeat rejected: {message}"));
                            }
                            Some(conex_proto::message::Body::Request(request)) => {
                                spawn_dispatch(
                                    providers.clone(),
                                    inflight.clone(),
                                    replies.clone(),
                                    request,
                                );
                            }
                            _ => {}
                        }
                    }
                    Message::Ping(payload) => socket.send(Message::Pong(payload)).await.map_err(|error| format!("send pong: {error}"))?,
                    Message::Close(_) => return Err("host closed websocket".into()),
                    Message::Text(_) | Message::Pong(_) | Message::Frame(_) => {}
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
        let Some(frame) = frame else {
            return Err("websocket closed during handshake".into());
        };
        match frame.map_err(|error| format!("receive handshake frame: {error}"))? {
            Message::Text(text) => return Ok(text.to_string()),
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => return Err("websocket closed during handshake".into()),
            Message::Binary(_) | Message::Frame(_) => {
                return Err("binary frame during handshake".into());
            }
        }
    }
}

async fn send_call<S>(
    socket: &mut S,
    method: &str,
    input: Value,
    request_id: &str,
) -> Result<(), String>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::fmt::Display,
{
    let frame = control_request(method, input, request_id);
    let bytes = frame.encode_to_vec();
    socket
        .send(Message::Binary(bytes.into()))
        .await
        .map_err(|error| format!("send {method}: {error}"))
}

fn control_request(method: &str, input: Value, request_id: &str) -> conex_proto::Message {
    conex_proto::Message {
        body: Some(conex_proto::message::Body::Request(conex_proto::Request {
            request_id: request_id.into(),
            method: method.into(),
            params: Some(conex_proto::CallParams {
                context: Some(conex_proto::RequestContext {
                    provider_endpoint_id: "agent-mgr".into(),
                    plane: conex_proto::Plane::Broker as i32,
                    binding_id: None,
                    principal_id: String::new(),
                    tenant_id: String::new(),
                }),
                timeout_budget_ms: 8_000,
                input: Some(serde_json::from_value(input).unwrap_or_default()),
            }),
        })),
    }
}

async fn await_response<S>(socket: &mut S, request_id: &str) -> Result<Value, String>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>>
        + futures::Sink<Message>
        + Unpin,
    S::Error: std::fmt::Display,
{
    use prost::Message as _;
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .map_err(|_| "response read timed out".to_string())?;
        let Some(frame) = frame else {
            return Err("websocket closed waiting for response".into());
        };
        match frame.map_err(|error| format!("receive response: {error}"))? {
            Message::Binary(bytes) => {
                let message = conex_proto::Message::decode(&bytes[..])
                    .map_err(|error| format!("decode response: {error}"))?;
                match message.body {
                    Some(conex_proto::message::Body::Success(success))
                        if success.request_id == request_id =>
                    {
                        return Ok(success
                            .result
                            .as_ref()
                            .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
                            .unwrap_or(Value::Null));
                    }
                    Some(conex_proto::message::Body::Failure(failure))
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
            _ => {}
        }
    }
}

/// One configured endpoint served through the SAME provider mechanism the
/// host uses: provider-fs Handlers + SnapshotCache paging (plan M2). The old
/// hardcoded FsRoot match, the flat scan and the no-cursor responses are gone.
struct EndpointProvider {
    endpoint_id: String,
    resources: Vec<String>,
    root: Arc<FsRoot>,
    handlers: HashMap<String, Arc<dyn conex_core::Handler>>,
}

impl EndpointProvider {
    fn new(config: &EndpointConfig) -> Result<Self, String> {
        let root = Arc::new(
            FsRoot::open(Path::new(&config.root)).map_err(|error| error.message().to_string())?,
        );
        let clock: Arc<dyn conex_source::pagination::Clock> =
            Arc::new(conex_source::pagination::SystemClock);
        let cache = Arc::new(std::sync::Mutex::new(
            conex_source::pagination::SnapshotCache::new(
                clock,
                conex_source::pagination::SnapshotLimits::default(),
            ),
        ));
        let mut handlers: HashMap<String, Arc<dyn conex_core::Handler>> = HashMap::new();
        for (method, handler) in [
            (
                "source/read",
                Arc::new(conex_provider_fs::read::ReadHandler { root: root.clone() })
                    as Arc<dyn conex_core::Handler>,
            ),
            (
                "source/list",
                Arc::new(conex_provider_fs::list::ListHandler {
                    root: root.clone(),
                    cache: cache.clone(),
                }) as Arc<dyn conex_core::Handler>,
            ),
            (
                "source/search",
                Arc::new(conex_provider_fs::search::SearchHandler {
                    root: root.clone(),
                    cache,
                }) as Arc<dyn conex_core::Handler>,
            ),
        ] {
            if config.methods.contains(&method.to_string()) {
                handlers.insert(method.to_string(), handler);
            }
        }
        Ok(Self {
            endpoint_id: config.endpoint_id.clone(),
            resources: config.resources.clone(),
            root,
            handlers,
        })
    }

    fn allowed(&self, resource: &str) -> bool {
        self.resources.iter().any(|root| {
            root == "*"
                || root.is_empty()
                || resource == root
                || resource
                    .strip_prefix(root)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }
}

/// Bounded dispatch (plan M2): each request runs on its own task behind a
/// concurrency ceiling and the remaining deadline from the host, so a slow
/// scan cannot block heartbeats, pongs, or other endpoints.
fn spawn_dispatch(
    providers: Arc<Vec<EndpointProvider>>,
    inflight: Arc<tokio::sync::Semaphore>,
    replies: tokio::sync::mpsc::Sender<Message>,
    request: conex_proto::Request,
) {
    tokio::spawn(async move {
        let _permit = match inflight.acquire().await {
            Ok(permit) => permit,
            Err(_) => return,
        };
        let params = request.params.clone().unwrap_or_default();
        let context = params.context.clone().unwrap_or_default();
        let input = params
            .input
            .as_ref()
            .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        let budget = params.timeout_budget_ms.clamp(1, 60_000);
        // blob/get remote 请求走字节路径：DataChunk 二进制回复，绝不 base64。
        let is_chunk = request.method == "blob/get" && input.get("remote").is_some();
        let body = if is_chunk {
            match tokio::time::timeout(Duration::from_millis(u64::from(budget)), async {
                dispatch_chunk(
                    &providers,
                    &request.request_id,
                    input,
                    &context.provider_endpoint_id,
                )
                .await
            })
            .await
            {
                Ok(Ok(chunk)) => conex_proto::message::Body::DataChunk(chunk),
                Ok(Err(error)) => failure_body(&request.request_id, &error),
                Err(_) => failure_body(
                    &request.request_id,
                    &conex_core::CallError::new(
                        conex_proto::ErrorCode::Timeout,
                        "agent dispatch exceeded the remaining deadline",
                    ),
                ),
            }
        } else {
            let result = tokio::time::timeout(Duration::from_millis(u64::from(budget)), async {
                dispatch_local(
                    &providers,
                    &request.method,
                    input,
                    &context.provider_endpoint_id,
                    &context.principal_id,
                    &context.tenant_id,
                )
                .await
            })
            .await
            .unwrap_or_else(|_| {
                Err(conex_core::CallError::new(
                    conex_proto::ErrorCode::Timeout,
                    "agent dispatch exceeded the remaining deadline",
                ))
            });
            match result {
                Ok(value) => conex_proto::message::Body::Success(conex_proto::Success {
                    request_id: request.request_id,
                    result: Some(serde_json::from_value(value).unwrap_or_default()),
                }),
                Err(error) => failure_body(&request.request_id, &error),
            }
        };
        let frame = conex_proto::Message { body: Some(body) };
        let _ = replies
            .send(Message::Binary(frame.encode_to_vec().into()))
            .await;
    });
}

fn failure_body(request_id: &str, error: &conex_core::CallError) -> conex_proto::message::Body {
    conex_proto::message::Body::Failure(conex_proto::Failure {
        request_id: Some(request_id.to_string()),
        error: Some(conex_proto::Error {
            code: error.code(),
            message: error.message().into(),
            ..Default::default()
        }),
    })
}

/// blob/get(remote) agent side: verify the endpoint claim, bind the file
/// revision, and return one bounded slice (≤ CHUNK_SIZE) with raw bytes.
async fn dispatch_chunk(
    providers: &[EndpointProvider],
    request_id: &str,
    input: Value,
    endpoint_id: &str,
) -> Result<conex_proto::DataChunk, conex_core::CallError> {
    let provider = providers
        .iter()
        .find(|provider| provider.endpoint_id == endpoint_id)
        .ok_or_else(|| {
            conex_core::CallError::new(
                conex_proto::ErrorCode::UnknownProvider,
                "unknown configured endpoint",
            )
        })?;
    let remote = input.get("remote").ok_or_else(|| {
        conex_core::CallError::new(conex_proto::ErrorCode::BadRequest, "remote target required")
    })?;
    let resource = remote
        .get("resourceId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            conex_core::CallError::new(conex_proto::ErrorCode::BadRequest, "resourceId is required")
        })?;
    if !provider.allowed(resource) {
        return Err(conex_core::CallError::new(
            conex_proto::ErrorCode::Forbidden,
            "resource is not configured",
        ));
    }
    let offset: u64 = remote
        .get("offset")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse().ok())
        .ok_or_else(|| {
            conex_core::CallError::new(conex_proto::ErrorCode::BadRequest, "offset is required")
        })?;
    let length: u64 = remote
        .get("length")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse().ok())
        .ok_or_else(|| {
            conex_core::CallError::new(conex_proto::ErrorCode::BadRequest, "length is required")
        })?;
    // 每次最多一个 CHUNK_SIZE 切片：有界内存，主机逐块拉取形成自然背压。
    if length == 0 || length > conex_proto::cid::CHUNK_SIZE as u64 {
        return Err(conex_core::CallError::new(
            conex_proto::ErrorCode::BadRequest,
            "chunk length must be within the protocol chunk size",
        ));
    }
    let requested_revision = remote.get("revision").and_then(Value::as_str);
    let read_len = length.min(conex_proto::cid::CHUNK_SIZE as u64) as usize;
    let (chunk, total, mtime_ns) = provider.root.read_range(resource, offset, read_len)?;
    let revision = mtime_ns.to_string();
    if let Some(expected) = requested_revision {
        if expected != revision {
            return Err(conex_core::CallError::new(
                conex_proto::ErrorCode::StaleRevision,
                format!("resource revision moved: requested {expected}, current {revision}"),
            ));
        }
    }
    let eof = offset + chunk.len() as u64 >= total;
    Ok(conex_proto::DataChunk {
        request_id: request_id.to_string(),
        chunk: chunk.into(),
        revision,
        eof,
    })
}

/// Route a request through the shared provider-fs handlers with the same
/// claim/context semantics the host-side registry enforces (plan M2).
async fn dispatch_local(
    providers: &[EndpointProvider],
    method: &str,
    input: Value,
    endpoint_id: &str,
    principal_id: &str,
    tenant_id: &str,
) -> Result<Value, conex_core::CallError> {
    let bad = |message: &str| {
        conex_core::CallError::new(conex_proto::ErrorCode::BadRequest, message.to_string())
    };
    let provider = providers
        .iter()
        .find(|provider| provider.endpoint_id == endpoint_id)
        .ok_or_else(|| {
            conex_core::CallError::new(
                conex_proto::ErrorCode::UnknownProvider,
                "unknown configured endpoint",
            )
        })?;
    let handler = provider.handlers.get(method).ok_or_else(|| {
        conex_core::CallError::new(
            conex_proto::ErrorCode::UnsupportedCapability,
            "method is not enabled for endpoint",
        )
    })?;
    // The claim mirrors what the source contract prepare produces; the local
    // resources scope then narrows what this agent is willing to serve.
    let claim = match method {
        "source/read" => {
            let resource = input
                .get("resourceId")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("resourceId is required"))?;
            conex_source::resource::read_claim(resource)?
        }
        "source/list" => {
            let root = input.get("root").and_then(Value::as_str).unwrap_or("");
            conex_source::resource::subtree_claim(root, "list")?
        }
        "source/search" => {
            let root = input.get("root").and_then(Value::as_str).unwrap_or("");
            conex_source::resource::subtree_claim(root, "search")?
        }
        other => {
            return Err(conex_core::CallError::new(
                conex_proto::ErrorCode::UnknownMethod,
                format!("unsupported local method {other}"),
            ));
        }
    };
    if !provider.allowed(&claim.resource_id) {
        return Err(conex_core::CallError::new(
            conex_proto::ErrorCode::Forbidden,
            "resource is not configured",
        ));
    }
    let ctx = conex_core::CallContext {
        caller: conex_core::Caller {
            principal_id: principal_id.to_string(),
            tenant_id: tenant_id.to_string(),
            actor_peer_id: String::new(),
        },
        endpoint_id: endpoint_id.to_string(),
        plane: conex_proto::Plane::Broker,
        method: method.to_string(),
        claim,
        policy_version: 0,
        deadline: tokio::time::Instant::now() + Duration::from_secs(60),
    };
    handler
        .execute(
            &ctx,
            input,
            conex_core::ExecutionIo {
                connection: None,
                secret: None,
            },
        )
        .await
}
fn tls_config(config: &AgentConfig) -> Result<rustls::ClientConfig, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut roots = rustls::RootCertStore::empty();
    if let Some(path) = &config.ca_pem {
        let bytes = std::fs::read(path).map_err(|error| format!("read ca_pem: {error}"))?;
        let mut reader = std::io::BufReader::new(bytes.as_slice());
        for cert in rustls_pemfile::certs(&mut reader) {
            let cert = cert.map_err(|error| format!("parse ca_pem: {error}"))?;
            roots
                .add(cert)
                .map_err(|error| format!("add ca certificate: {error}"))?;
        }
    } else {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    Ok(rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth())
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
    if value.is_empty() {
        return Err("agent token is empty".into());
    }
    Ok(value)
}
#[cfg(test)]
#[path = "runtime_tests.rs"]
mod runtime_tests;

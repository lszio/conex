//! WSS bootstrap transport (design §4.4, §5.2).
//!
//! One `conex/wss` connection runs the JSON-RPC 2.0 handshake from
//! `conex-core::transport_ws` until `Ready`, then accepts business envelopes
//! using the same `conex-jsonrpc2-http` envelope. Business frames are
//! dispatched through the shared `Broker` so wire-side cap rules are
//! identical to HTTP.
//!
//! Why one transport for both profiles: both `conex-jsonrpc2-wss` and
//! `conex-protobuf-wss` agree on envelope shape after handshake (they
//! differ only in the bootstrap body); the protobuf business profile is
//! layered on top of `decode_wire`/`encode_wire` as soon as negotiation
//! completes. For P1 only the JSON profile is required; the protobuf path
//! is recognised by `accepts_profile` and falls through to a `not_implemented`
//! business error.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration as StdDuration;
use tokio::task::JoinSet;
use tokio::time::{Instant, interval_at};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};

use conex_core::stream::{
    AckRequest, FlowRequest, ResetRequest, StreamError, StreamFrame, StreamHub, StreamOutcome,
};
use conex_core::transport_ws::{
    BootstrapError, BootstrapFrame, BootstrapState, ClientHandshake, ProfileId, ServerHandshake,
    plane_name,
};
use conex_core::{CallContext, CallError, CallResult, Limits, MethodContract};
use conex_proto;
use conex_proto::cid;
use conex_proto::wire::{ProtocolError, decode_wire, encode_wire, validate_message};
use prost::Message as _; // conex_proto::Message decode/encode for protobuf frames

use crate::broker::{Broker, BrokerCall, BrokerFrame};
use crate::http::HttpState;

const WSS_PROFILES: &[ProfileId] = &[ProfileId::JsonRpc2Wss, ProfileId::ProtobufWss];

pub struct WssState {
    pub broker: Arc<Broker>,
    pub limits: Limits,
    pub supported_profiles: Vec<ProfileId>,
    pub server_provides: Vec<String>,
    pub caller: conex_core::Caller,
    pub role: String,
    pub capability_caps: Vec<String>,
    pub web_session: Option<Arc<crate::web_auth::WebSession>>,
    pub host_side: Option<crate::agent::HostSide>,
    pub stream: tokio::sync::Mutex<Option<StreamHub>>,
    pub ui_links: Option<Arc<crate::ui_links::UiLinkRegistry>>,
    pub ui_link_id: Option<String>,
}

impl WssState {
    pub fn new(broker: Arc<Broker>, limits: Limits, caller: conex_core::Caller) -> Self {
        Self::new_with_capabilities(broker, limits, caller, Vec::new())
    }

    pub fn new_with_capabilities(
        broker: Arc<Broker>,
        limits: Limits,
        caller: conex_core::Caller,
        server_provides: Vec<String>,
    ) -> Self {
        Self {
            broker,
            limits,
            supported_profiles: WSS_PROFILES.to_vec(),
            server_provides,
            caller,
            role: "service".into(),
            capability_caps: Vec::new(),
            web_session: None,
            host_side: None,
            stream: tokio::sync::Mutex::new(None),
            ui_links: None,
            ui_link_id: None,
        }
    }
}

fn random_identity() -> String {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system random source unavailable");
    hex::encode(bytes)
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Generation-fenced response table used by outbound link calls. Request IDs
/// are only unique within one link generation; stale responses cannot revive a
/// later connection.
/// Reply payloads a link can carry: JSON-plane business values and M3 binary
/// data chunks (protobuf profile only).
#[derive(Debug, Clone, PartialEq)]
pub enum AgentReply {
    Value(Value),
    Chunk(conex_proto::DataChunk),
}

type PendingResult = oneshot::Sender<CallResult<AgentReply>>;

pub struct PendingRequests {
    generation: u64,
    inner: Mutex<HashMap<(u64, String), PendingResult>>,
}

impl PendingRequests {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub async fn register(
        &self,
        request_id: impl Into<String>,
    ) -> CallResult<oneshot::Receiver<CallResult<AgentReply>>> {
        let key = (self.generation, request_id.into());
        let mut inner = self.inner.lock().await;
        if inner.contains_key(&key) {
            return Err(CallError::new(
                conex_proto::ErrorCode::Conflict,
                "duplicate request id in link generation",
            ));
        }
        let (tx, rx) = oneshot::channel();
        inner.insert(key, tx);
        Ok(rx)
    }

    pub async fn route(
        &self,
        generation: u64,
        request_id: &str,
        result: CallResult<AgentReply>,
    ) -> bool {
        let tx = self
            .inner
            .lock()
            .await
            .remove(&(generation, request_id.to_owned()));
        tx.map(|tx| tx.send(result).is_ok()).unwrap_or(false)
    }

    pub async fn fail_all(&self) {
        let mut inner = self.inner.lock().await;
        for (_, tx) in inner.drain() {
            let _ = tx.send(Err(CallError::new(
                conex_proto::ErrorCode::Unavailable,
                "websocket connection closed",
            )));
        }
    }

    pub async fn cancel(&self, generation: u64, request_id: &str) {
        self.inner
            .lock()
            .await
            .remove(&(generation, request_id.to_owned()));
    }
}
async fn wait_for_revocation(state: &WssState) {
    if let Some(session) = &state.web_session {
        session.revoked_signal.notified().await;
    } else {
        std::future::pending::<()>().await;
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<Arc<WssState>>,
) -> impl IntoResponse {
    let max_frame = usize::try_from(state.limits.max_frame_bytes).unwrap_or(usize::MAX);
    ws.max_frame_size(max_frame)
        .max_message_size(max_frame)
        .on_upgrade(move |socket| handle_connection(socket, state))
}

/// Axum handler bound to [`HttpState`] in the HTTP router.
pub async fn ws_handler_with_state(
    ws: WebSocketUpgrade,
    axum::extract::Extension(state): axum::extract::Extension<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
    uri: axum::http::Uri,
) -> Response {
    let Some(broker) = state.broker.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({ "code": conex_proto::ErrorCode::Unavailable as i32, "message": "p1 backend is not configured" })),
        ).into_response();
    };
    let query = uri.query().unwrap_or_default();
    let ticket = query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == "ticket").then_some(value)
    });
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    if ticket.is_some() && authorization.is_some() {
        return (
            StatusCode::UNAUTHORIZED,
            "ticket and bearer authentication are mutually exclusive",
        )
            .into_response();
    }
    let (caller, role, capability_caps, web_session, ui_links_ref) = if let Some(ticket) = ticket {
        let Some(web) = state.web_auth.as_ref() else {
            return (StatusCode::UNAUTHORIZED, "web auth is not configured").into_response();
        };
        if let Err(error) = web.check_origin(&headers) {
            return crate::web_auth::error_response(error);
        }
        let origin = headers
            .get(axum::http::header::ORIGIN)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let target_host = headers
            .get(axum::http::header::HOST)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        let entry = match state
            .host_side
            .as_ref()
            .and_then(|side| side.tickets.consume_for(ticket, origin, target_host).ok())
        {
            Some(entry) => entry,
            None => return (StatusCode::UNAUTHORIZED, "invalid ticket").into_response(),
        };
        if entry.peer_role != "ui" {
            return (
                StatusCode::UNAUTHORIZED,
                "ticket role is not valid for browser links",
            )
                .into_response();
        }
        let Some(session_id) = entry.session_id.as_deref() else {
            return (
                StatusCode::UNAUTHORIZED,
                "ticket is not bound to a web session",
            )
                .into_response();
        };
        let Some(session) = web.get(session_id) else {
            return (
                StatusCode::UNAUTHORIZED,
                "web session is invalid or expired",
            )
                .into_response();
        };
        if session.caller.principal_id != entry.principal_id
            || session.caller.tenant_id != entry.tenant_id
        {
            return (StatusCode::UNAUTHORIZED, "ticket session binding mismatch").into_response();
        }
        let registry = state.ui_links.clone();
        registry.increment_tickets(&session.link_id);
        (
            session.caller.clone(),
            session.role.clone(),
            entry.capability_caps,
            Some(session),
            Some(registry),
        )
    } else {
        let inbound = match state.auth.authenticate(authorization) {
            Ok(inbound) => inbound,
            Err(error) => {
                return (
                    StatusCode::UNAUTHORIZED,
                    axum::Json(json!({ "code": error.code(), "message": error.message() })),
                )
                    .into_response();
            }
        };
        if inbound.role == "ui" {
            return (
                StatusCode::UNAUTHORIZED,
                "UI links require a session ticket",
            )
                .into_response();
        }
        (inbound.caller, inbound.role, Vec::new(), None, None)
    };
    let advertised = if role == "ui" {
        capability_caps.clone()
    } else {
        state.p1_provides.clone()
    };
    let mut wss_state =
        WssState::new_with_capabilities(broker, conex_core::Limits::default(), caller, advertised);
    wss_state.role = role;
    wss_state.capability_caps = capability_caps;
    wss_state.host_side = state.host_side.clone();
    wss_state.web_session = web_session.clone();
    wss_state.ui_links = ui_links_ref.clone();
    if let Some(session) = web_session.as_ref() {
        wss_state.ui_link_id = Some(session.link_id.clone());
    }
    let wss_state = Arc::new(wss_state);
    let max_frame = usize::try_from(wss_state.limits.max_frame_bytes).unwrap_or(usize::MAX);
    ws.max_frame_size(max_frame)
        .max_message_size(max_frame)
        .on_upgrade(move |socket| handle_connection(socket, wss_state))
        .into_response()
}
async fn handle_connection(socket: WebSocket, state: Arc<WssState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut handshake = ServerHandshake::new_with_profiles_and_capabilities(
        &state.supported_profiles,
        conex_proto::Plane::Broker,
        state.server_provides.clone(),
        random_identity(),
    );
    let handshake_result = tokio::time::timeout(
        StdDuration::from_millis(conex_core::transport_ws::DEFAULT_HANDSHAKE_TIMEOUT_MS),
        async {
            loop {
                let Some(message) = receiver.next().await else {
                    return Err(BootstrapError::Timeout);
                };
                let message = message.map_err(|error| {
                    BootstrapError::MalformedEnvelope(format!("websocket receive failed: {error}"))
                })?;
                match message {
                    Message::Text(text) => {
                        if text.len() > conex_core::transport_ws::MAX_BOOTSTRAP_MESSAGE_BYTES {
                            return Err(BootstrapError::BufferExceeded);
                        }
                        let response = handshake.ingest(BootstrapFrame {
                            json: text.to_string(),
                        })?;
                        sender
                            .send(Message::Text(response.json.into()))
                            .await
                            .map_err(|error| {
                                BootstrapError::MalformedEnvelope(format!(
                                    "websocket send failed: {error}"
                                ))
                            })?;
                        if let BootstrapState::Ready { profile, .. } = handshake.state() {
                            return Ok(*profile);
                        }
                    }
                    Message::Binary(_) => {
                        return Err(BootstrapError::MalformedEnvelope(
                            "bootstrap is UTF-8 JSON-RPC text for every profile".into(),
                        ));
                    }
                    Message::Ping(payload) => {
                        sender.send(Message::Pong(payload)).await.map_err(|error| {
                            BootstrapError::MalformedEnvelope(format!(
                                "websocket pong failed: {error}"
                            ))
                        })?;
                    }
                    Message::Close(_) => return Err(BootstrapError::Timeout),
                    Message::Pong(_) => {}
                }
            }
        },
    )
    .await
    .unwrap_or(Err(BootstrapError::Timeout));
    let business_profile = match handshake_result {
        Ok(profile) => profile,
        Err(error) => {
            let _ = sender
                .send(Message::Text(bootstrap_error_frame(error).into()))
                .await;
            let _ = sender.close().await;
            return;
        }
    };
    if let (Some(registry), Some(link_id)) = (state.ui_links.as_ref(), state.ui_link_id.as_ref()) {
        registry.touch(link_id);
    }

    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    let pending = Arc::new(PendingRequests::new(generation));
    let max_queued = usize::try_from(state.limits.max_queued_bytes).unwrap_or(usize::MAX);
    let max_frame = usize::try_from(state.limits.max_frame_bytes).unwrap_or(usize::MAX);
    let queued_bytes = Arc::new(AtomicUsize::new(0));
    let queue_capacity = usize::try_from(state.limits.max_inflight.max(1))
        .unwrap_or(usize::MAX)
        .saturating_mul(2);
    let (out_tx, mut out_rx) = mpsc::channel::<(Message, usize)>(queue_capacity);
    let (close_tx, mut close_rx) = mpsc::unbounded_channel::<()>();
    let remote_link = crate::remote::RemoteLink::with_close(
        generation,
        out_tx.clone(),
        queued_bytes.clone(),
        pending.clone(),
        max_queued,
        Some(close_tx.clone()),
    );
    if state.role == "agent"
        && let Some(side) = &state.host_side
    {
        // Staged only: the link starts serving after `agent/register`
        // succeeds, so a failed registration leaves the healthy previous
        // connection alone (plan M2).
        side.stage_connection(&state.caller.principal_id, remote_link)
            .await;
    }
    let writer_queued = queued_bytes.clone();
    let writer = tokio::spawn(async move {
        while let Some((message, bytes)) = out_rx.recv().await {
            if sender.send(message).await.is_err() {
                break;
            }
            writer_queued.fetch_sub(bytes, Ordering::AcqRel);
        }
        let _ = sender.close().await;
    });
    let inflight = Arc::new(Semaphore::new(
        usize::try_from(state.limits.max_inflight.max(1)).unwrap_or(1),
    ));
    let mut dispatch_tasks = JoinSet::new();
    let last_pong = Arc::new(Mutex::new(Instant::now()));
    let mut heartbeat = interval_at(
        Instant::now() + std::time::Duration::from_secs(20),
        std::time::Duration::from_secs(20),
    );

    loop {
        tokio::select! {
            _ = wait_for_revocation(&state) => { break; }
            maybe = receiver.next() => {
                let Some(message) = maybe else { break; };
                let Ok(message) = message else { break; };
                let frame_len = match &message {
                    Message::Text(text) => text.len(),
                    Message::Binary(bytes) => bytes.len(),
                    Message::Ping(bytes) | Message::Pong(bytes) => bytes.len(),
                    Message::Close(_) => 0,
                };
                if frame_len > usize::try_from(state.limits.max_frame_bytes).unwrap_or(usize::MAX) {
                    let failure = conex_proto::Message {
                        body: Some(conex_proto::message::Body::Failure(conex_proto::Failure {
                            request_id: None,
                            error: Some(to_wire_error(&CallError::new(
                                conex_proto::ErrorCode::QuotaExceeded,
                                "websocket frame exceeds negotiated limit",
                            ))),
                        })),
                    };
                    if let Ok(bytes) = encode_wire(&failure)
                        && !enqueue_message(
                            &out_tx,
                            &queued_bytes,
                            max_queued,
                            Message::Text(String::from_utf8_lossy(&bytes).into_owned().into()),
                        ) {
                            break;
                        }
                    break;
                }
                match message {
                    Message::Text(text) => {
                        if business_profile != ProfileId::JsonRpc2Wss {
                            let failure = wire_failure(
                                None,
                                conex_proto::ErrorCode::BadRequest,
                                "text business frames require the JSON profile",
                            );
                            if let Some(bytes) = encode_proto_message(&failure)
                                && !enqueue_message(
                                    &out_tx,
                                    &queued_bytes,
                                    max_queued,
                                    Message::Binary(bytes.into()),
                                ) {
                                    break;
                                }
                            continue;
                        }
                        let message = match decode_wire(text.as_bytes()) {
                            Ok(message) => message,
                            Err(error) => {
                                let failure = wire_failure(
                                    error.request_id,
                                    conex_proto::ErrorCode::try_from(error.rpc_code)
                                        .unwrap_or(conex_proto::ErrorCode::ParseError),
                                    error.message,
                                );
                                if let Some(reply) = encode_json_message(&failure)
                                    && !enqueue_message(
                                        &out_tx,
                                        &queued_bytes,
                                        max_queued,
                                        reply,
                                    ) {
                                        break;
                                    }
                                continue;
                            }
                        };
                        if let Err(error) = validate_message(&message) {
                            let failure = wire_failure(
                                error.request_id,
                                conex_proto::ErrorCode::try_from(error.rpc_code)
                                    .unwrap_or(conex_proto::ErrorCode::BadRequest),
                                error.message,
                            );
                            if let Some(reply) = encode_json_message(&failure)
                                && !enqueue_message(&out_tx, &queued_bytes, max_queued, reply) {
                                    break;
                                }
                            continue;
                        }
                        match message.body.as_ref() {
                            Some(conex_proto::message::Body::Success(success)) => {
                                let value = success
                                    .result
                                    .as_ref()
                                    .map(|result| serde_json::to_value(result).unwrap_or(Value::Null))
                                    .unwrap_or(Value::Null);
                                let _ = pending.route(generation, &success.request_id, Ok(crate::ws_transport::AgentReply::Value(value))).await;
                                continue;
                            }
                            Some(conex_proto::message::Body::DataChunk(chunk)) => {
                                let _ = pending.route(
                                    generation,
                                    &chunk.request_id,
                                    Ok(crate::ws_transport::AgentReply::Chunk(chunk.clone())),
                                )
                                .await;
                                continue;
                            }
                            Some(conex_proto::message::Body::Failure(failure)) => {
                                if let Some(id) = failure.request_id.as_ref() {
                                    let error = failure
                                        .error
                                        .as_ref()
                                        .map(|error| {
                                            CallError::new(
                                                conex_proto::ErrorCode::try_from(error.code)
                                                    .unwrap_or(conex_proto::ErrorCode::BadRequest),
                                                error.message.clone(),
                                            )
                                        })
                                        .unwrap_or_else(|| {
                                            CallError::new(
                                                conex_proto::ErrorCode::BadRequest,
                                                "remote failure",
                                            )
                                        });
                                    let _ = pending.route(generation, id, Err(error)).await;
                                }
                                continue;
                            }
                            Some(conex_proto::message::Body::Request(_)) => {}
                            Some(conex_proto::message::Body::Notification(_)) | None => continue,
                        }
                        let frame = match broker_frame_from_message(message) {
                            Ok(Some(frame)) => frame,
                            Ok(None) => continue,
                            Err(error) => {
                                if let Ok(bytes) = encode_wire(&conex_proto::Message {
                                    body: Some(conex_proto::message::Body::Failure(conex_proto::Failure {
                                        request_id: None,
                                        error: Some(to_wire_error(&error)),
                                    })),
                                })
                                    && !enqueue_message(
                                        &out_tx,
                                        &queued_bytes,
                                        max_queued,
                                        Message::Text(String::from_utf8_lossy(&bytes).into_owned().into()),
                                    ) {
                                        break;
                                    }
                                continue;
                            }
                        };
                        if let Some(session) = &state.web_session
                            && !session.is_active() {
                                break;
                            }
                        if state.role == "ui"
                            && (!crate::web_auth::allowed_ui_method(&frame.method)
                                || !state.capability_caps.iter().any(|cap| cap == &frame.method))
                        {
                            let failure = wire_failure(
                                Some(frame.request_id.clone()),
                                conex_proto::ErrorCode::Forbidden,
                                "method is not allowed on UI links",
                            );
                            if let Some(reply) = encode_json_message(&failure)
                                && !enqueue_message(&out_tx, &queued_bytes, max_queued, reply) {
                                    break;
                                }
                            continue;
                        }
                        let request_id = frame.request_id.clone();
                        let permit = match inflight.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                let failure = conex_proto::Message {
                                    body: Some(conex_proto::message::Body::Failure(conex_proto::Failure {
                                        request_id: Some(request_id),
                                        error: Some(to_wire_error(&CallError::new(
                                            conex_proto::ErrorCode::QuotaExceeded,
                                            "too many in-flight requests",
                                        ))),
                                    })),
                                };
                                if let Ok(bytes) = encode_wire(&failure)
                                    && !enqueue_message(
                                        &out_tx,
                                        &queued_bytes,
                                        max_queued,
                                        Message::Text(String::from_utf8_lossy(&bytes).into_owned().into()),
                                    ) {
                                        break;
                                    }
                                continue;
                            }
                        };
                        let task_state = state.clone();
                        let task_tx = out_tx.clone();
                        let task_queued = queued_bytes.clone();
                        let close_task = close_tx.clone();
                        dispatch_tasks.spawn(async move {
                            let result = dispatch_business(&task_state, frame, business_profile, generation).await;
                            let response = conex_proto::Message {
                                body: Some(match result {
                                    Ok(value) => conex_proto::message::Body::Success(conex_proto::Success {
                                        request_id,
                                        result: Some(crate::http::json_to_pbjson(value)),
                                    }),
                                    Err(error) => conex_proto::message::Body::Failure(conex_proto::Failure {
                                        request_id: Some(request_id),
                                        error: Some(to_wire_error(&error)),
                                    }),
                                }),
                            };
                            if let Ok(bytes) = encode_wire(&response) {
                                if bytes.len() > max_frame
                                    || !enqueue_message(
                                        &task_tx,
                                        &task_queued,
                                        max_queued,
                                        Message::Text(String::from_utf8_lossy(&bytes).into_owned().into()),
                                    )
                                {
                                    let _ = close_task.send(());
                                }
                            } else {
                                let _ = close_task.send(());
                            }
                            drop(permit);
                        });
                    }
                    Message::Binary(bytes) => {
                        if business_profile != ProfileId::ProtobufWss {
                            let failure = wire_failure(
                                None,
                                conex_proto::ErrorCode::BadRequest,
                                "binary business frames require the protobuf profile",
                            );
                            if let Some(reply) = encode_json_message(&failure)
                                && !enqueue_message(&out_tx, &queued_bytes, max_queued, reply) {
                                    break;
                                }
                            continue;
                        }
                        let message = match conex_proto::Message::decode(bytes.as_ref()) {
                            Ok(message) => message,
                            Err(error) => {
                                let failure = wire_failure(
                                    None,
                                    conex_proto::ErrorCode::ParseError,
                                    format!("cannot decode protobuf frame: {error}"),
                                );
                                if let Some(bytes) = encode_proto_message(&failure)
                                    && !enqueue_message(
                                        &out_tx,
                                        &queued_bytes,
                                        max_queued,
                                        Message::Binary(bytes.into()),
                                    ) {
                                        break;
                                    }
                                continue;
                            }
                        };
                        if let Err(error) = validate_message(&message) {
                            let failure = wire_failure(
                                error.request_id,
                                conex_proto::ErrorCode::try_from(error.rpc_code)
                                    .unwrap_or(conex_proto::ErrorCode::BadRequest),
                                error.message,
                            );
                            if let Some(bytes) = encode_proto_message(&failure)
                                && !enqueue_message(
                                    &out_tx,
                                    &queued_bytes,
                                    max_queued,
                                    Message::Binary(bytes.into()),
                                ) {
                                    break;
                                }
                            continue;
                        }
                        match message.body.as_ref() {
                            Some(conex_proto::message::Body::Success(success)) => {
                                let value = success
                                    .result
                                    .as_ref()
                                    .map(|result| serde_json::to_value(result).unwrap_or(Value::Null))
                                    .unwrap_or(Value::Null);
                                let _ = pending.route(generation, &success.request_id, Ok(crate::ws_transport::AgentReply::Value(value))).await;
                                continue;
                            }
                            Some(conex_proto::message::Body::DataChunk(chunk)) => {
                                let _ = pending.route(
                                    generation,
                                    &chunk.request_id,
                                    Ok(crate::ws_transport::AgentReply::Chunk(chunk.clone())),
                                )
                                .await;
                                continue;
                            }
                            Some(conex_proto::message::Body::Failure(failure)) => {
                                if let Some(id) = failure.request_id.as_ref() {
                                    let error = failure
                                        .error
                                        .as_ref()
                                        .map(|error| {
                                            CallError::new(
                                                conex_proto::ErrorCode::try_from(error.code)
                                                    .unwrap_or(conex_proto::ErrorCode::BadRequest),
                                                error.message.clone(),
                                            )
                                        })
                                        .unwrap_or_else(|| {
                                            CallError::new(
                                                conex_proto::ErrorCode::BadRequest,
                                                "remote failure",
                                            )
                                        });
                                    let _ = pending.route(generation, id, Err(error)).await;
                                }
                                continue;
                            }
                            _ => {}
                        }
                        let frame = match broker_frame_from_message(message) {
                            Ok(Some(frame)) => frame,
                            Ok(None) => continue,
                            Err(error) => {
                                let failure = wire_failure(
                                    None,
                                    conex_proto::ErrorCode::try_from(error.code())
                                        .unwrap_or(conex_proto::ErrorCode::BadRequest),
                                    error.message(),
                                );
                                if let Some(buf) = encode_proto_message(&failure)
                                    && !enqueue_message(
                                        &out_tx,
                                        &queued_bytes,
                                        max_queued,
                                        Message::Binary(buf.into()),
                                    ) {
                                        break;
                                    }
                                continue;
                            }
                        };
                        if let Some(session) = &state.web_session
                            && !session.is_active() {
                                break;
                            }
                        if state.role == "ui"
                            && (!crate::web_auth::allowed_ui_method(&frame.method)
                                || !state.capability_caps.iter().any(|cap| cap == &frame.method))
                        {
                            let failure = wire_failure(
                                Some(frame.request_id.clone()),
                                conex_proto::ErrorCode::Forbidden,
                                "method is not allowed on UI links",
                            );
                            if let Some(buf) = encode_proto_message(&failure)
                                && !enqueue_message(&out_tx, &queued_bytes, max_queued, Message::Binary(buf.into())) {
                                    break;
                                }
                            continue;
                        }
                        let request_id = frame.request_id.clone();
                        let permit = match inflight.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                let failure = conex_proto::Message {
                                    body: Some(conex_proto::message::Body::Failure(conex_proto::Failure {
                                        request_id: Some(request_id),
                                        error: Some(to_wire_error(&CallError::new(
                                            conex_proto::ErrorCode::QuotaExceeded,
                                            "too many in-flight requests",
                                        ))),
                                    })),
                                };
                                let mut buf = Vec::new();
                                if failure.encode(&mut buf).is_ok() {
                                    let _ = enqueue_message(
                                        &out_tx,
                                        &queued_bytes,
                                        max_queued,
                                        Message::Binary(buf.into()),
                                    );
                                }
                                continue;
                            }
                        };
                        let task_state = state.clone();
                        let task_tx = out_tx.clone();
                        let task_queued = queued_bytes.clone();
                        let close_task = close_tx.clone();
                        dispatch_tasks.spawn(async move {
                            let result = dispatch_business(&task_state, frame, business_profile, generation).await;
                            let response = conex_proto::Message {
                                body: Some(match result {
                                    Ok(value) => conex_proto::message::Body::Success(conex_proto::Success {
                                        request_id,
                                        result: Some(crate::http::json_to_pbjson(value)),
                                    }),
                                    Err(error) => conex_proto::message::Body::Failure(conex_proto::Failure {
                                        request_id: Some(request_id),
                                        error: Some(to_wire_error(&error)),
                                    }),
                                }),
                            };
                            let mut buf = Vec::new();
                            if response.encode(&mut buf).is_err()
                                || buf.len() > max_frame
                                || !enqueue_message(
                                    &task_tx,
                                    &task_queued,
                                    max_queued,
                                    Message::Binary(buf.into()),
                                )
                            {
                                let _ = close_task.send(());
                            }
                            drop(permit);
                        });
                    }
                    Message::Ping(payload) => {
                        let _ = enqueue_message(&out_tx, &queued_bytes, max_queued, Message::Pong(payload));
                    }
                    Message::Pong(_) => {
                        *last_pong.lock().await = Instant::now();
                    }
                    Message::Close(_) => break,
                }
            }
            _ = heartbeat.tick() => {
                if last_pong.lock().await.elapsed() > StdDuration::from_secs(40) {
                    break;
                }
                if state.role == "agent"
                    && state.host_side.as_ref().is_some_and(|side| {
                        side.agent_heartbeat_expired(&state.caller.principal_id, generation, 45_000)
                    })
                {
                    break;
                }
                if let Some(session) = &state.web_session
                    && !session.is_active() {
                        break;
                    }
                if !enqueue_message(&out_tx, &queued_bytes, max_queued, Message::Ping(Vec::new().into())) {
                    break;
                }
            }
            _ = close_rx.recv() => break,
            Some(_result) = dispatch_tasks.join_next(), if !dispatch_tasks.is_empty() => {},
        }
    }
    dispatch_tasks.abort_all();
    while dispatch_tasks.join_next().await.is_some() {}
    drop(out_tx);
    writer.abort();
    if state.role == "agent"
        && let Some(side) = &state.host_side
    {
        side.disconnect_agent(&state.caller.principal_id, generation);
        side.connections
            .remove(&state.caller.principal_id, generation)
            .await;
    }
    let _ = writer.await;
    pending.fail_all().await;
}

/// Convert the canonical protobuf Request body into the host's existing HTTP
/// dispatch adapter. JSON business frames use the same adapter shape as HTTP;
/// `BrokerFrame` is internal after decoding and is not a second negotiated
/// wire contract. Responses remain standard JSON/protobuf Success/Failure
/// envelopes at the socket boundary.
///
/// Notifications return `None` because they do not have a response.
fn broker_frame_from_message(message: conex_proto::Message) -> CallResult<Option<BrokerFrame>> {
    let (request_id, method, call_params) = match message.body {
        Some(conex_proto::message::Body::Request(request)) => {
            (Some(request.request_id), request.method, request.params)
        }
        Some(conex_proto::message::Body::Notification(notification)) => {
            (None, notification.method, notification.params)
        }
        Some(conex_proto::message::Body::Success(_))
        | Some(conex_proto::message::Body::Failure(_)) => {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "clients must not send responses",
            ));
        }
        Some(conex_proto::message::Body::DataChunk(_)) => {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "data chunks require the protobuf profile",
            ));
        }
        None => {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "empty message",
            ));
        }
    };
    let context = call_params.as_ref().and_then(|p| p.context.clone());
    let input = call_params
        .as_ref()
        .and_then(|p| p.input.as_ref())
        .map(crate::http::pbjson_to_json)
        .unwrap_or(Value::Null);
    let (caller, plane) = match context.as_ref() {
        Some(ctx) => (ctx.provider_endpoint_id.clone(), ctx.plane),
        None => (String::new(), conex_proto::Plane::Broker as i32),
    };
    let frame = BrokerFrame {
        request_id: request_id.clone().unwrap_or_else(|| "notif".to_string()),
        method,
        context: crate::broker::BrokerContext {
            principal_id: String::new(),
            tenant_id: String::new(),
            provider_endpoint_id: caller,
            plane,
            binding_id: context.as_ref().and_then(|c| c.binding_id.clone()),

            timeout_budget_ms: call_params
                .as_ref()
                .map(|p| p.timeout_budget_ms)
                .unwrap_or(0),
        },
        params: Some(input),
    };
    Ok(if request_id.is_some() {
        Some(frame)
    } else {
        None
    })
}

fn to_wire_error(error: &CallError) -> conex_proto::Error {
    conex_proto::Error {
        code: error.code(),
        message: error.message().to_string(),
        ..Default::default()
    }
}
fn wire_failure(
    request_id: Option<String>,
    code: conex_proto::ErrorCode,
    message: impl Into<String>,
) -> conex_proto::Message {
    conex_proto::Message {
        body: Some(conex_proto::message::Body::Failure(conex_proto::Failure {
            request_id,
            error: Some(conex_proto::Error {
                code: code as i32,
                message: message.into(),
                ..Default::default()
            }),
        })),
    }
}
fn protocol_error_to_call(error: ProtocolError) -> CallError {
    CallError::new(
        conex_proto::ErrorCode::try_from(error.rpc_code)
            .unwrap_or(conex_proto::ErrorCode::BadRequest),
        error.message,
    )
}

fn encode_proto_message(message: &conex_proto::Message) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    message.encode(&mut bytes).ok().map(|()| bytes)
}

fn encode_json_message(message: &conex_proto::Message) -> Option<Message> {
    encode_wire(message)
        .ok()
        .map(|bytes| Message::Text(String::from_utf8_lossy(&bytes).into_owned().into()))
}

fn bootstrap_error_frame(error: BootstrapError) -> String {
    let frame = json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": { "code": -32000, "message": format!("bootstrap error: {error}") },
    })
    .to_string();
    if frame.len() <= conex_core::transport_ws::MAX_BOOTSTRAP_MESSAGE_BYTES {
        frame
    } else {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32000,"message":"bootstrap error"}}"#
            .to_owned()
    }
}

pub(crate) fn message_size(message: &Message) -> usize {
    match message {
        Message::Text(text) => text.len(),
        Message::Binary(bytes) | Message::Ping(bytes) | Message::Pong(bytes) => bytes.len(),
        Message::Close(_) => 0,
    }
}

pub(crate) fn enqueue_message(
    tx: &mpsc::Sender<(Message, usize)>,
    queued_bytes: &AtomicUsize,
    max_queued: usize,
    message: Message,
) -> bool {
    let bytes = message_size(&message);
    let mut current = queued_bytes.load(Ordering::Acquire);
    loop {
        if current.saturating_add(bytes) > max_queued {
            return false;
        }
        match queued_bytes.compare_exchange(
            current,
            current + bytes,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(actual) => current = actual,
        }
    }
    if tx.try_send((message, bytes)).is_err() {
        queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
        return false;
    }
    true
}
async fn dispatch_business(
    state: &WssState,
    frame: BrokerFrame,
    business_profile: ProfileId,
    generation: u64,
) -> CallResult<Value> {
    if state.role == "ui"
        && state
            .web_session
            .as_ref()
            .is_some_and(|session| !session.is_active())
    {
        return Err(CallError::new(
            conex_proto::ErrorCode::Unauthorized,
            "web session is revoked or expired",
        ));
    }
    let plane = frame.context.plane;
    if frame.method.starts_with("agent/") && state.role != "agent" {
        return Err(CallError::new(
            conex_proto::ErrorCode::Forbidden,
            "agent methods require an authenticated agent WSS role",
        ));
    }
    if plane != conex_proto::Plane::Broker as i32 {
        return Err(CallError::new(
            conex_proto::ErrorCode::PlaneMismatch,
            "P1 only supports the broker plane",
        ));
    }
    let tracked_link = if state.role == "ui" {
        if let (Some(registry), Some(link_id)) =
            (state.ui_links.as_ref(), state.ui_link_id.as_ref())
        {
            registry.begin_call(link_id);
            Some((registry.clone(), link_id.clone()))
        } else {
            None
        }
    } else {
        None
    };
    let result = async {
        if state.role == "agent" {
            return dispatch_agent(state, frame, generation).await;
        }
        if frame.method.starts_with("stream/") {
            return handle_stream(state, &frame, business_profile).await;
        }
        dispatch_rpc(state, frame).await
    }
    .await;
    if let Some((registry, link_id)) = tracked_link {
        registry.end_call(&link_id);
    }
    result
}

async fn dispatch_agent(
    state: &WssState,
    frame: BrokerFrame,
    generation: u64,
) -> CallResult<Value> {
    let side = state.host_side.as_ref().ok_or_else(|| {
        CallError::new(
            conex_proto::ErrorCode::Unavailable,
            "agent links are not configured",
        )
    })?;
    let input = frame.params.unwrap_or(Value::Null);
    let map = input.as_object().ok_or_else(|| {
        CallError::new(
            conex_proto::ErrorCode::BadRequest,
            "agent input must be an object",
        )
    })?;
    match frame.method.as_str() {
        "agent/register" => {
            let agent_id = crate::agent::require_string(map, "agentId")?.to_owned();
            let host_origin = crate::agent::require_string(map, "hostOrigin")?.to_owned();
            if state
                .broker
                .deps()
                .host_origin
                .as_deref()
                .is_some_and(|expected| expected != host_origin)
            {
                return Err(CallError::new(
                    conex_proto::ErrorCode::Forbidden,
                    "agent hostOrigin does not match host",
                ));
            }
            if agent_id != state.caller.principal_id {
                return Err(CallError::new(
                    conex_proto::ErrorCode::Forbidden,
                    "agent identity does not match authenticated principal",
                ));
            }
            let registration = crate::agent::AgentRegistration {
                agent_id,
                principal_id: state.caller.principal_id.clone(),
                tenant_id: state.caller.tenant_id.clone(),
                endpoints: crate::agent::parse_endpoints(map)?,
                registered_at_ms: crate::agent::now_ms(),
                last_heartbeat_at_ms: crate::agent::now_ms(),
                host_origin,
            };
            let review = match side.register_agent_link(registration.clone(), generation) {
                Ok(review) => review,
                Err(error) => {
                    // Registration failed: drop the staged link so the
                    // healthy previous connection keeps serving (plan M2).
                    side.discard_connection(&registration.agent_id, generation)
                        .await;
                    return Err(CallError::new(
                        conex_proto::ErrorCode::Forbidden,
                        error.to_string(),
                    ));
                }
            };
            side.activate_connection(
                &registration.agent_id,
                generation,
                &review.accepted_endpoint_ids,
            )
            .await
            .map_err(|error| {
                CallError::new(conex_proto::ErrorCode::Forbidden, error.to_string())
            })?;
            Ok(json!({
                "agentId": registration.agent_id,
                "generation": generation,
                "acceptedEndpointIds": review.accepted_endpoint_ids,
                "rejectedCapabilities": review.rejected_capabilities
                    .into_iter()
                    .map(|(endpoint_id, reason)| json!({
                        "endpointId": endpoint_id,
                        "reason": reason,
                    }))
                    .collect::<Vec<_>>(),
            }))
        }
        "agent/heartbeat" => {
            let agent_id = crate::agent::require_string(map, "agentId")?;
            if agent_id != state.caller.principal_id {
                return Err(CallError::new(
                    conex_proto::ErrorCode::Forbidden,
                    "agent identity does not match authenticated principal",
                ));
            }
            let timestamp = side
                .heartbeat_agent(agent_id, generation)
                .map_err(|error| {
                    CallError::new(conex_proto::ErrorCode::Forbidden, error.to_string())
                })?;
            Ok(json!({"agentId": agent_id, "heartbeatAtMs": timestamp.to_string()}))
        }
        _ => Err(CallError::new(
            conex_proto::ErrorCode::Forbidden,
            "agent links may only register and heartbeat",
        )),
    }
}

/// Broker RPC dispatch for a non-stream business frame. Identity comes from
/// the authenticated upgrade, never from the frame.
async fn dispatch_rpc(state: &WssState, frame: BrokerFrame) -> CallResult<Value> {
    let caller = state.caller.clone();
    let timeout_ms = frame
        .context
        .timeout_budget_ms
        .max(1)
        .min(state.limits.timeout_ms.max(1));
    let timeout = StdDuration::from_millis(u64::from(timeout_ms));
    let deadline = Instant::now() + timeout;
    let input = frame
        .params
        .as_ref()
        .map(|value| serde_json::to_value(value).unwrap_or(Value::Null))
        .unwrap_or(Value::Null);
    let call = BrokerCall {
        caller,
        endpoint_id: frame.context.provider_endpoint_id.clone(),
        method: frame.method.clone(),
        input,
        deadline,
        role: state.role.clone(),
        link_id: state.ui_link_id.clone(),
    };
    state.broker.invoke(call.clone()).await
}

/// P1-04 stream frame dispatch. The hub is created lazily on the first
/// `stream/*` frame for the connection's (session, attachment, epoch); old
/// epochs are fenced (session/resume bumped the generation) and control
/// queue overflow terminates the link.
async fn handle_stream(
    state: &WssState,
    frame: &BrokerFrame,
    business_profile: ProfileId,
) -> CallResult<Value> {
    let input = frame
        .params
        .as_ref()
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let mut hub_guard = state.stream.lock().await;
    let hub = hub_guard.get_or_insert_with(|| {
        // The hub's epoch anchor is taken from the first frame; session/
        // attachment ids scope the connection. If the peer sends a later
        // (resumed) epoch, the hub fences forwards.
        StreamHub::new(
            "wss",
            "wss",
            frame
                .params
                .as_ref()
                .and_then(|p| p.get("epoch").and_then(Value::as_u64))
                .unwrap_or(1),
            conex_core::stream::DEFAULT_WINDOW_BYTES,
            conex_core::stream::DEFAULT_MAX_FRAME_BYTES,
            0,
        )
    });
    let value = match frame.method.as_str() {
        "stream/ack" => {
            let ack: AckRequest = serde_json::from_value(input).map_err(|e| {
                CallError::new(
                    conex_proto::ErrorCode::BadRequest,
                    format!("bad stream/ack: {e}"),
                )
            })?;
            hub.check_epoch(ack.epoch).map_err(stream_error_to_call)?;
            stream_outcome_to_json(
                hub.receiver_ack(&ack.stream_id, ack.last_received_seq)
                    .map_err(stream_error_to_call)?,
            )
        }
        "stream/flow" => {
            let flow: FlowRequest = serde_json::from_value(input).map_err(|e| {
                CallError::new(
                    conex_proto::ErrorCode::BadRequest,
                    format!("bad stream/flow: {e}"),
                )
            })?;
            hub.check_epoch(flow.epoch).map_err(stream_error_to_call)?;
            stream_outcome_to_json(
                hub.apply_flow(
                    &flow.stream_id,
                    flow.consumed_bytes,
                    flow.requested_window_bytes,
                )
                .map_err(stream_error_to_call)?,
            )
        }
        "stream/reset" => {
            let reset: ResetRequest = serde_json::from_value(input).map_err(|e| {
                CallError::new(
                    conex_proto::ErrorCode::BadRequest,
                    format!("bad stream/reset: {e}"),
                )
            })?;
            hub.check_epoch(reset.epoch).map_err(stream_error_to_call)?;
            stream_outcome_to_json(
                hub.apply_reset(
                    &reset.stream_id,
                    reset.after_seq,
                    &reset.reason,
                    reset.resume_handle,
                )
                .map_err(stream_error_to_call)?,
            )
        }
        "stream/frame" => {
            let stream_frame: StreamFrame = serde_json::from_value(input).map_err(|e| {
                CallError::new(
                    conex_proto::ErrorCode::BadRequest,
                    format!("bad stream/frame: {e}"),
                )
            })?;
            hub.check_epoch(stream_frame.epoch)
                .map_err(stream_error_to_call)?;
            let contiguous = hub
                .receive_frame(&stream_frame)
                .map_err(stream_error_to_call)?;
            // A stream data frame carries one business message (design §1:
            // "业务消息先经 conex-proto 解析为 conex_proto::Message，再用 StreamFrame
            // 承载"). Decode per the agreed profile and dispatch through the
            // same broker path — credit/seq/epoch are enforced above.
            if stream_frame.message.is_empty() {
                return Err(stream_error_to_call(StreamError::ZeroByteRejected));
            }
            let inner = match business_profile {
                ProfileId::JsonRpc2Wss => {
                    let message =
                        decode_wire(&stream_frame.message).map_err(protocol_error_to_call)?;
                    validate_message(&message).map_err(protocol_error_to_call)?;
                    broker_frame_from_message(message)?.ok_or_else(|| {
                        CallError::new(
                            conex_proto::ErrorCode::BadRequest,
                            "stream frame message must be a request (not a notification)",
                        )
                    })?
                }
                ProfileId::ProtobufWss => {
                    let message = conex_proto::Message::decode(stream_frame.message.as_slice())
                        .map_err(|e| {
                            CallError::new(
                                conex_proto::ErrorCode::BadRequest,
                                format!("cannot decode stream frame message: {e}"),
                            )
                        })?;
                    validate_message(&message).map_err(protocol_error_to_call)?;
                    broker_frame_from_message(message)?.ok_or_else(|| {
                        CallError::new(
                            conex_proto::ErrorCode::BadRequest,
                            "stream frame message must be a request (not a notification)",
                        )
                    })?
                }
            };
            if inner.method.starts_with("agent/") {
                return Err(CallError::new(
                    conex_proto::ErrorCode::Forbidden,
                    "agent control methods cannot be carried in stream frames",
                ));
            }
            let result = dispatch_rpc(state, inner).await?;
            json!({
                "streamId": stream_frame.stream_id,
                "seq": stream_frame.seq.to_string(),
                "contiguousBytes": contiguous.to_string(),
                "result": result,
            })
        }
        other => {
            return Err(CallError::new(
                conex_proto::ErrorCode::UnknownMethod,
                format!("unknown stream method {other}"),
            ));
        }
    };
    Ok(value)
}

fn stream_error_to_call(error: StreamError) -> CallError {
    match error {
        StreamError::EpochFenced { .. } => {
            CallError::new(conex_proto::ErrorCode::ResumeUnavailable, error.to_string())
        }
        StreamError::ZeroByteRejected => {
            CallError::new(conex_proto::ErrorCode::BadRequest, error.to_string())
        }
        StreamError::StreamFailed(reason) => {
            CallError::new(conex_proto::ErrorCode::BadRequest, reason)
        }
        StreamError::SlowConsumer { .. } => {
            CallError::new(conex_proto::ErrorCode::SlowConsumer, error.to_string())
        }
        StreamError::ControlQueueFull => {
            CallError::new(conex_proto::ErrorCode::QuotaExceeded, error.to_string())
        }
        StreamError::BadRequest(message) => {
            CallError::new(conex_proto::ErrorCode::BadRequest, message)
        }
    }
}

fn stream_outcome_to_json(outcome: StreamOutcome) -> Value {
    use StreamOutcome::*;
    match outcome {
        Accepted { seq, .. } => json!({ "accepted": true, "seq": seq.to_string() }),
        CreditBlocked {
            sent_bytes,
            allowed,
        } => {
            json!({ "accepted": false, "creditBlocked": true, "sentBytes": sent_bytes.to_string(), "allowedBytes": allowed.to_string() })
        }
        Acked { last_received_seq } => {
            json!({ "accepted": true, "lastReceivedSeq": last_received_seq.to_string() })
        }
        GapDetected { last_received_seq } => {
            json!({ "accepted": false, "gap": true, "lastReceivedSeq": last_received_seq.to_string() })
        }
        AckIgnored => json!({ "accepted": false, "ignored": true }),
        StaleFlowIgnored { consumed_bytes } => {
            json!({ "accepted": false, "staleFlow": true, "consumedBytes": consumed_bytes.to_string() })
        }
        StreamFailed { reason } => {
            json!({ "accepted": false, "streamFailed": true, "reason": reason })
        }
        WindowCapRejected { requested, cap } => {
            json!({ "accepted": false, "windowCapRejected": true, "requestedBytes": requested.to_string(), "capBytes": cap.to_string() })
        }
        ZeroByteRejected => json!({ "accepted": false, "zeroByteRejected": true }),
        ResetAccepted {
            after_seq,
            resume_handle,
        } => {
            json!({ "accepted": true, "afterSeq": after_seq.to_string(), "resumeHandle": resume_handle })
        }
        ReconfirmRequired {
            unconfirmed_bytes,
            new_window,
        } => {
            json!({ "accepted": false, "reconfirmRequired": true, "unconfirmedBytes": unconfirmed_bytes.to_string(), "newWindowBytes": new_window.to_string() })
        }
        EpochFenced { expected, got } => {
            json!({ "accepted": false, "epochFenced": true, "expectedEpoch": expected.to_string(), "gotEpoch": got.to_string() })
        }
        SlowConsumer {
            stream_id,
            since_ms,
        } => {
            json!({ "accepted": false, "slowConsumer": true, "streamId": stream_id, "sinceMs": since_ms.to_string() })
        }
        ControlQueueFull => json!({ "accepted": false, "controlQueueFull": true }),
        Cancelled => json!({ "accepted": true, "cancelled": true }),
    }
}

/// Apply JSON-RPC 2.0 envelope and dispatch. Helpers keep the websocket loop
/// readable.
pub fn validate_envelope(value: &Value) -> CallResult<&str> {
    match value.get("jsonrpc").and_then(Value::as_str) {
        Some("2.0") => {}
        _ => {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "jsonrpc must be \"2.0\"",
            ));
        }
    }
    value
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| CallError::new(conex_proto::ErrorCode::BadRequest, "method is required"))
}

pub fn expected_planes() -> Vec<&'static str> {
    vec![plane_name(conex_proto::Plane::Broker)]
}

pub fn accepted_profile(value: ProfileId) -> &'static str {
    value.as_str()
}

pub fn deserialize_bootstrap(text: &str) -> CallResult<BootstrapFrame> {
    Ok(BootstrapFrame { json: text.into() })
}

pub use conex_core::transport_ws::BootstrapError as HandshakeError;

/// Standalone helper used by integration tests: build a `ClientHandshake` from
/// the same profile the server uses.
pub fn build_client(profile: ProfileId, plane: conex_proto::Plane) -> ClientHandshake {
    let mut client = ClientHandshake::new();
    // Ignore the unused result; tests rely on `ClientHandshake::build_hello`
    // directly to drive the state machine.
    let _ = client.build_hello(profile, plane, Vec::new(), Vec::new());
    client
}

#[doc(hidden)]
pub fn _unused_helper() {
    let _ = cid::cid_for_raw(b"x");
    let _: Box<dyn std::error::Error> = Box::new(HandshakeError::RepeatedReady);
    let _ = MethodContract {
        adapter_id: "noop",
        input_schema: "",
        output_schema: "",
        prepare: |_| {
            Ok(conex_core::PreparedInput {
                canonical: Value::Null,
                claim: conex_core::ResourceClaim {
                    resource_id: String::new(),
                    action: String::new(),
                    subtree: false,
                },
                binding: Default::default(),
            })
        },
        validate_output: |_| Ok(()),
    };
}

#[allow(dead_code)]
fn _unused_upgrade_marker(_: CallContext) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pending_routes_generation_and_request_id_without_cross_delivery() {
        let current = PendingRequests::new(7);
        let previous = PendingRequests::new(6);
        let current_rx = current.register("same-id").await.unwrap();
        let previous_rx = previous.register("same-id").await.unwrap();
        assert!(
            current
                .register("same-id")
                .await
                .expect_err("duplicate id must be rejected")
                .message()
                .contains("duplicate")
        );
        assert!(
            !current
                .route(
                    6,
                    "same-id",
                    Ok(AgentReply::Value(json!({"generation": 6})))
                )
                .await
        );
        assert!(
            !current
                .route(7, "late-id", Ok(AgentReply::Value(Value::Null)))
                .await
        );
        assert!(
            current
                .route(
                    7,
                    "same-id",
                    Ok(AgentReply::Value(json!({"generation": 7})))
                )
                .await
        );
        assert_eq!(
            current_rx.await.unwrap().unwrap(),
            AgentReply::Value(json!({"generation": 7}))
        );
        assert!(
            previous
                .route(
                    6,
                    "same-id",
                    Ok(AgentReply::Value(json!({"generation": 6})))
                )
                .await
        );
        assert_eq!(
            previous_rx.await.unwrap().unwrap(),
            AgentReply::Value(json!({"generation": 6}))
        );
        assert!(
            !current
                .route(7, "same-id", Ok(AgentReply::Value(Value::Null)))
                .await
        );
    }

    #[test]
    fn protobuf_business_validation_rejects_incomplete_request() {
        let message = conex_proto::Message {
            body: Some(conex_proto::message::Body::Request(conex_proto::Request {
                request_id: String::new(),
                method: String::new(),
                params: None,
            })),
        };
        let error = validate_message(&message).expect_err("incomplete request must be rejected");
        assert_eq!(error.rpc_code, conex_proto::ErrorCode::BadRequest as i32);
    }

    #[tokio::test]
    async fn enqueue_message_never_exceeds_byte_budget() {
        let (tx, mut rx) = mpsc::channel(4);
        let queued = AtomicUsize::new(0);
        assert!(enqueue_message(
            &tx,
            &queued,
            4,
            Message::Text("123".to_owned().into()),
        ));
        assert!(!enqueue_message(
            &tx,
            &queued,
            4,
            Message::Text("45".to_owned().into()),
        ));
        assert_eq!(queued.load(Ordering::Acquire), 3);
        let (_, bytes) = rx.recv().await.expect("first frame remains queued");
        assert_eq!(bytes, 3);
        queued.fetch_sub(bytes, Ordering::AcqRel);
        assert_eq!(queued.load(Ordering::Acquire), 0);
    }

    #[tokio::test]
    async fn pending_close_finishes_waiters() {
        let pending = PendingRequests::new(9);
        let rx = pending.register("close-me").await.unwrap();
        pending.fail_all().await;
        let error = rx.await.unwrap().unwrap_err();
        assert_eq!(error.code(), conex_proto::ErrorCode::Unavailable as i32);
    }
}

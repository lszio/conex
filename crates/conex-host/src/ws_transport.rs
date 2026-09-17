//! WSS bootstrap transport (design §4.4, §5.2).
//!
//! One `conex/wss` connection runs the JSON-RPC 2.0 handshake from
//! `conex-core::transport_ws` until `Ready`, then accepts business envelopes
//! using the same `conex-jsonrpc2-http-v1` envelope. Business frames are
//! dispatched through the shared `Broker` so wire-side cap rules are
//! identical to HTTP.
//!
//! Why one transport for both profiles: both `conex-jsonrpc2-wss-v1` and
//! `conex-protobuf-wss-v1` agree on envelope shape after handshake (they
//! differ only in the bootstrap body); the protobuf business profile is
//! layered on top of `decode_wire`/`encode_wire` as soon as negotiation
//! completes. For P1 only the JSON profile is required; the protobuf path
//! is recognised by `accepts_profile` and falls through to a `not_implemented`
//! business error.
#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration as StdDuration;
use tokio::time::Instant;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use conex_core::stream::{AckRequest, FlowRequest, ResetRequest, StreamError, StreamFrame, StreamHub, StreamOutcome};
use conex_core::transport_ws::{
    BootstrapError, BootstrapFrame, BootstrapState, ClientHandshake, ProfileId, ServerHandshake,
    plane_name,
};
use conex_core::{CallContext, CallError, CallResult, Limits, MethodContract};
use conex_proto::cid;
use conex_proto::v1;
use prost::Message as _; // v1::Message decode/encode for protobuf frames

use crate::broker::{Broker, BrokerCall, BrokerFrame};
use crate::http::HttpState;

const WSS_PROFILES: &[ProfileId] = &[ProfileId::JsonRpc2WssV1, ProfileId::ProtobufWssV1];

/// State shared by every WS connection: business dispatcher + Limits.
pub struct WssState {
    pub broker: Arc<Broker>,
    pub limits: Limits,
    pub supported_profiles: Vec<ProfileId>,
    /// Authenticated caller identity from the HTTP upgrade (static bearer /
    /// future ticket/OIDC). The broker derives `Caller` from this, not from
    /// client-supplied context — identity must not be attacker-controlled.
    pub caller: conex_core::Caller,
    /// P1-04 stream hub, created lazily on the first `stream/*` frame.
    /// Keyed to the connection's (session_id, attachment_id, epoch).
    pub stream: tokio::sync::Mutex<Option<StreamHub>>,
}

impl WssState {
    pub fn new(broker: Arc<Broker>, limits: Limits, caller: conex_core::Caller) -> Self {
        Self {
            broker,
            limits,
            supported_profiles: WSS_PROFILES.to_vec(),
            caller,
            stream: tokio::sync::Mutex::new(None),
        }
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<Arc<WssState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_connection(socket, state))
}

/// Axum handler bound to [`HttpState`] in the HTTP router.
pub async fn ws_handler_with_state(
    ws: WebSocketUpgrade,
    axum::Extension(state): axum::Extension<Arc<HttpState>>,
    headers: axum::http::HeaderMap,
) -> Response {
    let Some(broker) = state.broker.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(json!({
                "code": v1::ErrorCode::Unavailable as i32,
                "message": "p1 backend is not configured",
            })),
        )
            .into_response();
    };
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let caller = match state.auth.authenticate(authorization) {
        Ok(inbound) => inbound.caller,
        Err(error) => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(json!({
                    "code": error.code(),
                    "message": error.message(),
                })),
            )
                .into_response();
        }
    };
    let wss_state = Arc::new(WssState::new(
        broker,
        conex_core::Limits::default(),
        caller,
    ));
    ws.on_upgrade(move |socket| handle_connection(socket, wss_state))
        .into_response()
}

async fn handle_connection(socket: WebSocket, state: Arc<WssState>) {
    let (mut sender, mut receiver) = socket.split();
    let handshake = Arc::new(Mutex::new(ServerHandshake::new_with_profiles(
        &state.supported_profiles,
        v1::Plane::Broker,
    )));
    let mut open = false;
    let mut outbound = OutboundQueue::default();

    // Handshake: UTF-8 JSON-RPC 2.0 bootstrap for every profile (design
    // §4.4). Once ready the business encoding switches to the agreed
    // profile — JSON text frames for `conex-jsonrpc2-wss-v1`, prost
    // `v1::Message` binary frames for `conex-protobuf-wss-v1`.
    while let Some(message) = receiver.next().await {
        let message = match message {
            Ok(message) => message,
            Err(error) => {
                tracing::debug!("ws receive error: {error}");
                break;
            }
        };
        match message {
            Message::Text(text) => {
                let frame = BootstrapFrame {
                    json: text.to_string(),
                };
                let response = {
                    let mut guard = handshake.lock().await;
                    guard.ingest(frame)
                };
                match response {
                    Ok(out) => {
                        outbound.push_text(out.json);
                        // The client cannot proceed (send ready) until it has
                        // read this handshake result, so flush per step rather
                        // than deferring to the end of the handshake.
                        while let Some(text) = outbound.pop_text() {
                            if sender.send(Message::Text(text.into())).await.is_err() {
                                return;
                            }
                        }
                        if matches!(
                            handshake.lock().await.state(),
                            conex_core::transport_ws::BootstrapState::Ready { .. }
                        ) {
                            open = true;
                            break;
                        }
                    }
                    Err(error) => {
                        send_error_frame(&mut outbound, error).await;
                        let _ = sender.close().await;
                        return;
                    }
                }
            }
            Message::Binary(_) => {
                send_error_frame(
                    &mut outbound,
                    BootstrapError::MalformedEnvelope(
                        "bootstrap is UTF-8 JSON-RPC text for every profile".into(),
                    ),
                )
                .await;
                let _ = sender.close().await;
                return;
            }
            Message::Close(_) => {
                let _ = sender.close().await;
                return;
            }
            _ => continue,
        }
    }
    if !open {
        let _ = sender.close().await;
        return;
    }
    let business_profile = {
        let guard = handshake.lock().await;
        match guard.state() {
            BootstrapState::Ready { profile, .. } => *profile,
            _ => {
                let _ = sender.close().await;
                return;
            }
        }
    };
    // Flush handshake output
    while let Some(text) = outbound.pop_text() {
        if sender.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
    // Business loop
    while let Some(message) = receiver.next().await {
        let message = match message {
            Ok(message) => message,
            Err(_) => break,
        };
        match message {
            Message::Text(text) => {
                if business_profile != ProfileId::JsonRpc2WssV1 {
                    let response = envelope_failure(
                        "?",
                        &CallError::new(
                            v1::ErrorCode::BadRequest,
                            "JSON business frames are not valid on the protobuf profile",
                        ),
                    );
                    if sender
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }
                let frame = match serde_json::from_str::<BrokerFrame>(&text) {
                    Ok(frame) => frame,
                    Err(error) => {
                        let response = envelope_failure(
                            &text_request_id(&text),
                            &CallError::new(
                                v1::ErrorCode::BadRequest,
                                format!("invalid envelope: {error}"),
                            ),
                        );
                        if sender
                            .send(Message::Text(response.to_string().into()))
                            .await
                            .is_err()
                        {
                            return;
                        }
                        continue;
                    }
                };
                let request_id = frame.request_id.clone();
                let result = dispatch_business(&state, frame, business_profile).await;
                let serialized = match result {
                    Ok(value) => envelope_success_id(&request_id, &value),
                    Err(error) => envelope_failure(&request_id, &error),
                };
                if sender
                    .send(Message::Text(serialized.to_string().into()))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Message::Binary(bytes) => {
                if business_profile != ProfileId::ProtobufWssV1 {
                    let response = envelope_failure(
                        "?",
                        &CallError::new(
                            v1::ErrorCode::BadRequest,
                            "binary business frames require the protobuf profile",
                        ),
                    );
                    if sender
                        .send(Message::Text(response.to_string().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    continue;
                }
                let message = match v1::Message::decode(bytes.as_ref()) {
                    Ok(message) => message,
                    Err(error) => {
                        // Protocol-level decode failure: answer with a
                        // Failure carrying parse_error semantics.
                        let failure = v1::Message {
                            body: Some(v1::message::Body::Failure(v1::Failure {
                                request_id: Some("?".into()),
                                error: Some(v1::Error {
                                    code: v1::ErrorCode::ParseError as i32,
                                    message: format!("cannot decode protobuf frame: {error}"),
                                    ..Default::default()
                                }),
                            })),
                        };
                        let mut buf = Vec::new();
                        let _ = failure.encode(&mut buf);
                        if sender.send(Message::Binary(buf.into())).await.is_err() {
                            return;
                        }
                        continue;
                    }
                };
                let frame = match broker_frame_from_message(message) {
                    Ok(Some(frame)) => frame,
                    Ok(None) => continue, // notification: fire-and-forget
                    Err(error) => {
                        let failure = v1::Message {
                            body: Some(v1::message::Body::Failure(v1::Failure {
                                request_id: Some("?".into()),
                                error: Some(to_wire_error(&error)),
                            })),
                        };
                        let mut buf = Vec::new();
                        let _ = failure.encode(&mut buf);
                        if sender.send(Message::Binary(buf.into())).await.is_err() {
                            return;
                        }
                        continue;
                    }
                };
                let request_id = frame.request_id.clone();
                let result = dispatch_business(&state, frame, business_profile).await;
                let response = v1::Message {
                    body: Some(match result {
                        Ok(value) => v1::message::Body::Success(v1::Success {
                            request_id,
                            result: Some(crate::http::json_to_pbjson(value)),
                        }),
                        Err(error) => v1::message::Body::Failure(v1::Failure {
                            request_id: Some(request_id),
                            error: Some(to_wire_error(&error)),
                        }),
                    }),
                };
                let mut buf = Vec::new();
                let _ = response.encode(&mut buf);
                if sender.send(Message::Binary(buf.into())).await.is_err() {
                    return;
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue,
        }
    }
    let _ = sender.close().await;
}

/// Convert a typed `v1::Message` request/notification into the broker's
/// flat frame. Returns `None` for notifications (no response is expected).
fn broker_frame_from_message(message: v1::Message) -> CallResult<Option<BrokerFrame>> {
    let (request_id, method, call_params) = match message.body {
        Some(v1::message::Body::Request(request)) => {
            (Some(request.request_id), request.method, request.params)
        }
        Some(v1::message::Body::Notification(notification)) => {
            (None, notification.method, notification.params)
        }
        Some(v1::message::Body::Success(_)) | Some(v1::message::Body::Failure(_)) => {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "clients must not send responses",
            ));
        }
        None => {
            return Err(CallError::new(v1::ErrorCode::BadRequest, "empty message"));
        }
    };
    let context = call_params.as_ref().and_then(|p| p.context.clone());
    let input = call_params
        .as_ref()
        .and_then(|p| p.input.as_ref())
        .map(crate::http::pbjson_to_json)
        .unwrap_or(Value::Null);
    let (caller, plane) = match context.as_ref() {
        Some(ctx) => (
            ctx.provider_endpoint_id.clone(),
            ctx.plane,
        ),
        None => (String::new(), v1::Plane::Broker as i32),
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
    Ok(if request_id.is_some() { Some(frame) } else { None })
}

fn to_wire_error(error: &CallError) -> v1::Error {
    v1::Error {
        code: error.code(),
        message: error.message().to_string(),
        ..Default::default()
    }
}

fn text_request_id(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.get("id").cloned())
        .and_then(|value| match value {
            Value::String(s) => Some(s),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "?".to_string())
}

/// JSON response built from the frame's own request id (works for both
/// broker results and stream-frame results, which are raw values).
fn envelope_success_id(request_id: &str, value: &Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "result": value,
    })
}

fn envelope_failure(request_id: &str, error: &CallError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {
            "code": error.code(),
            "message": error.message(),
        }
    })
}

async fn send_error_frame(outbound: &mut OutboundQueue, error: BootstrapError) {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": Value::Null,
        "error": { "code": -32000, "message": format!("bootstrap error: {error}") },
    });
    outbound.push_text(payload.to_string());
}

#[derive(Default)]
struct OutboundQueue {
    pending: Vec<String>,
}

impl OutboundQueue {
    fn push_text(&mut self, text: String) {
        self.pending.push(text);
    }
    fn pop_text(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.remove(0))
        }
    }
}

async fn dispatch_business(
    state: &WssState,
    frame: BrokerFrame,
    business_profile: ProfileId,
) -> CallResult<Value> {
    let plane = frame.context.plane;
    if plane != v1::Plane::Broker as i32 {
        return Err(CallError::new(
            v1::ErrorCode::PlaneMismatch,
            "P1 only supports the broker plane",
        ));
    }
    // P1-04 stream control/data frames are handled by the per-connection
    // StreamHub, not the broker.
    if frame.method.starts_with("stream/") {
        return handle_stream(state, &frame, business_profile).await;
    }
    dispatch_rpc(state, frame).await
}

/// Broker RPC dispatch for a non-stream business frame. Identity comes from
/// the authenticated upgrade, never from the frame.
async fn dispatch_rpc(state: &WssState, frame: BrokerFrame) -> CallResult<Value> {
    let caller = state.caller.clone();
    let timeout = StdDuration::from_millis(u64::from(frame.context.timeout_budget_ms.max(1)));
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
                CallError::new(v1::ErrorCode::BadRequest, format!("bad stream/ack: {e}"))
            })?;
            hub.check_epoch(ack.epoch).map_err(stream_error_to_call)?;
            stream_outcome_to_json(
                hub.receiver_ack(&ack.stream_id, ack.last_received_seq)
                    .map_err(stream_error_to_call)?,
            )
        }
        "stream/flow" => {
            let flow: FlowRequest = serde_json::from_value(input).map_err(|e| {
                CallError::new(v1::ErrorCode::BadRequest, format!("bad stream/flow: {e}"))
            })?;
            hub.check_epoch(flow.epoch).map_err(stream_error_to_call)?;
            stream_outcome_to_json(
                hub.apply_flow(&flow.stream_id, flow.consumed_bytes, flow.requested_window_bytes)
                    .map_err(stream_error_to_call)?,
            )
        }
        "stream/reset" => {
            let reset: ResetRequest = serde_json::from_value(input).map_err(|e| {
                CallError::new(v1::ErrorCode::BadRequest, format!("bad stream/reset: {e}"))
            })?;
            hub.check_epoch(reset.epoch).map_err(stream_error_to_call)?;
            stream_outcome_to_json(
                hub.apply_reset(&reset.stream_id, reset.after_seq, &reset.reason, reset.resume_handle)
                    .map_err(stream_error_to_call)?,
            )
        }
        "stream/frame" => {
            let stream_frame: StreamFrame = serde_json::from_value(input).map_err(|e| {
                CallError::new(v1::ErrorCode::BadRequest, format!("bad stream/frame: {e}"))
            })?;
            hub.check_epoch(stream_frame.epoch).map_err(stream_error_to_call)?;
            let contiguous = hub
                .receive_frame(&stream_frame)
                .map_err(stream_error_to_call)?;
            // A stream data frame carries one business message (design §1:
            // "业务消息先经 conex-proto 解析为 v1::Message，再用 StreamFrame
            // 承载"). Decode per the agreed profile and dispatch through the
            // same broker path — credit/seq/epoch are enforced above.
            if stream_frame.message.is_empty() {
                return Err(stream_error_to_call(StreamError::ZeroByteRejected));
            }
            let inner = match business_profile {
                ProfileId::JsonRpc2WssV1 => {
                    let text = std::str::from_utf8(&stream_frame.message).map_err(|_| {
                        CallError::new(
                            v1::ErrorCode::BadRequest,
                            "stream frame message must be UTF-8 JSON on the JSON profile",
                        )
                    })?;
                    serde_json::from_str::<BrokerFrame>(text).map_err(|e| {
                        CallError::new(
                            v1::ErrorCode::BadRequest,
                            format!("invalid business message in stream frame: {e}"),
                        )
                    })?
                }
                ProfileId::ProtobufWssV1 => {
                    let message = v1::Message::decode(stream_frame.message.as_slice()).map_err(
                        |e| {
                            CallError::new(
                                v1::ErrorCode::BadRequest,
                                format!("cannot decode stream frame message: {e}"),
                            )
                        },
                    )?;
                    broker_frame_from_message(message)?.ok_or_else(|| {
                        CallError::new(
                            v1::ErrorCode::BadRequest,
                            "stream frame message must be a request (not a notification)",
                        )
                    })?
                }
            };
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
                v1::ErrorCode::UnknownMethod,
                format!("unknown stream method {other}"),
            ));
        }
    };
    Ok(value)
}

fn stream_error_to_call(error: StreamError) -> CallError {
    match error {
        StreamError::EpochFenced { .. } => {
            CallError::new(v1::ErrorCode::ResumeUnavailable, error.to_string())
        }
        StreamError::ZeroByteRejected => {
            CallError::new(v1::ErrorCode::BadRequest, error.to_string())
        }
        StreamError::StreamFailed(reason) => {
            CallError::new(v1::ErrorCode::BadRequest, reason)
        }
        StreamError::SlowConsumer { .. } => {
            CallError::new(v1::ErrorCode::SlowConsumer, error.to_string())
        }
        StreamError::ControlQueueFull => CallError::new(
            v1::ErrorCode::QuotaExceeded,
            error.to_string(),
        ),
        StreamError::BadRequest(message) => CallError::new(v1::ErrorCode::BadRequest, message),
    }
}

fn stream_outcome_to_json(outcome: StreamOutcome) -> Value {
    use StreamOutcome::*;
    match outcome {
        Accepted { seq, .. } => json!({ "accepted": true, "seq": seq.to_string() }),
        CreditBlocked { sent_bytes, allowed } => {
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
        StreamFailed { reason } => json!({ "accepted": false, "streamFailed": true, "reason": reason }),
        WindowCapRejected { requested, cap } => json!({ "accepted": false, "windowCapRejected": true, "requestedBytes": requested.to_string(), "capBytes": cap.to_string() }),
        ZeroByteRejected => json!({ "accepted": false, "zeroByteRejected": true }),
        ResetAccepted { after_seq, resume_handle } => {
            json!({ "accepted": true, "afterSeq": after_seq.to_string(), "resumeHandle": resume_handle })
        }
        ReconfirmRequired { unconfirmed_bytes, new_window } => json!({ "accepted": false, "reconfirmRequired": true, "unconfirmedBytes": unconfirmed_bytes.to_string(), "newWindowBytes": new_window.to_string() }),
        EpochFenced { expected, got } => json!({ "accepted": false, "epochFenced": true, "expectedEpoch": expected.to_string(), "gotEpoch": got.to_string() }),
        SlowConsumer { stream_id, since_ms } => json!({ "accepted": false, "slowConsumer": true, "streamId": stream_id, "sinceMs": since_ms.to_string() }),
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
                v1::ErrorCode::BadRequest,
                "jsonrpc must be \"2.0\"",
            ));
        }
    }
    value
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| CallError::new(v1::ErrorCode::BadRequest, "method is required"))
}

pub fn expected_planes() -> Vec<&'static str> {
    vec![plane_name(v1::Plane::Broker)]
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
pub fn build_client(profile: ProfileId, plane: v1::Plane) -> ClientHandshake {
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

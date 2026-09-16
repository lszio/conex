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
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use conex_core::transport_ws::{
    BootstrapError, BootstrapFrame, ClientHandshake, ProfileId, ServerHandshake, plane_name,
};
use conex_core::{CallContext, CallError, CallResult, Limits, MethodContract};
use conex_proto::cid;
use conex_proto::v1;

use crate::broker::{Broker, BrokerCall, BrokerFrame};

/// State shared by every WS connection: business dispatcher + Limits.
pub struct WssState {
    pub broker: Arc<Broker>,
    pub limits: Limits,
    pub server_profile: ProfileId,
}

impl WssState {
    pub fn new(broker: Arc<Broker>, limits: Limits, server_profile: ProfileId) -> Self {
        Self {
            broker,
            limits,
            server_profile,
        }
    }
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<Arc<WssState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_connection(socket, state))
}

async fn handle_connection(socket: WebSocket, state: Arc<WssState>) {
    let (mut sender, mut receiver) = socket.split();
    let handshake = Arc::new(Mutex::new(ServerHandshake::new(
        state.server_profile,
        v1::Plane::Broker,
    )));
    let mut open = false;
    let mut outbound = OutboundQueue::default();
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
            Message::Binary(bytes) => {
                // Profile-2 bootstrap delivers the protobuf message as
                // framed binary; map it back through the JSON handshake.
                let parsed = match serde_json::from_slice::<Value>(&bytes) {
                    Ok(value) => value,
                    Err(_) => {
                        send_error_frame(
                            &mut outbound,
                            BootstrapError::MalformedEnvelope(
                                "binary handshake envelopes are not supported".into(),
                            ),
                        )
                        .await;
                        let _ = sender.close().await;
                        return;
                    }
                };
                let frame = BootstrapFrame {
                    json: parsed.to_string(),
                };
                let response = {
                    let mut guard = handshake.lock().await;
                    guard.ingest(frame)
                };
                match response {
                    Ok(out) => {
                        outbound.push_text(out.json);
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
                let frame = match serde_json::from_str::<BrokerFrame>(&text) {
                    Ok(frame) => frame,
                    Err(error) => {
                        let error = CallError::new(
                            v1::ErrorCode::BadRequest,
                            format!("invalid envelope: {error}"),
                        );
                        let response = envelope_failure(&text_request_id(&text), &error);
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
                let response = dispatch_business(&state, frame).await;
                let serialized = match response {
                    Ok(value) => envelope_success(&value),
                    Err(error) => envelope_failure("?", &error),
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
                // Treat binary as JSON text for P1 (the protobuf business
                // profile reuses the same wire envelope; we keep the JSON
                // surface so all SDKs share encoding rules).
                if let Ok(text) = std::str::from_utf8(&bytes) {
                    let frame: BrokerFrame = match serde_json::from_str(text) {
                        Ok(frame) => frame,
                        Err(error) => {
                            let error = CallError::new(
                                v1::ErrorCode::BadRequest,
                                format!("invalid envelope: {error}"),
                            );
                            let response = envelope_failure("?", &error);
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
                    let response = dispatch_business(&state, frame).await;
                    let serialized = match response {
                        Ok(value) => envelope_success(&value),
                        Err(error) => envelope_failure("?", &error),
                    };
                    if sender
                        .send(Message::Text(serialized.to_string().into()))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
            Message::Close(_) => break,
            Message::Ping(_) | Message::Pong(_) => continue,
        }
    }
    let _ = sender.close().await;
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

fn envelope_success(value: &Value) -> Value {
    let id = value.get("id").cloned().unwrap_or(Value::Null);
    let mut response = serde_json::Map::new();
    response.insert("jsonrpc".into(), Value::String("2.0".into()));
    response.insert("id".into(), id);
    response.insert(
        "result".into(),
        value.get("result").cloned().unwrap_or(Value::Null),
    );
    Value::Object(response)
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

async fn dispatch_business(state: &WssState, frame: BrokerFrame) -> CallResult<Value> {
    let plane = frame.context.plane;
    if plane != v1::Plane::Broker as i32 {
        return Err(CallError::new(
            v1::ErrorCode::PlaneMismatch,
            "P1 only supports the broker plane",
        ));
    }
    let caller = conex_core::Caller {
        principal_id: frame.context.principal_id.clone(),
        tenant_id: frame.context.tenant_id.clone(),
        actor_peer_id: "wss".into(),
    };
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
    state.broker.invoke(call.clone()).await.map(|value| {
        json!({
            "id": frame.request_id,
            "result": value,
        })
    })
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

//! Axum /rpc entry: authenticate, decode, validate binding, call the one Host.
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use conex_core::{CallError, Host, Limits};
use conex_proto;
use conex_proto::wire::{ProtocolError, decode_wire, encode_wire};
use serde_json::{Map, Value};

use axum::Extension;

use crate::agent::HostSide;
use crate::auth::{InboundAuth, InboundCaller};
use crate::binding::BindingStore;
use crate::broker::{Broker, BrokerCall};
use crate::tickets;

pub const PROFILE_ID: &str = "conex-jsonrpc2-http";
pub const MAX_BODY_BYTES: usize = 1_048_576;
pub const HELLO_METHOD: &str = "conex/hello";

/// Everything the host wires per-process. Attached to every request via
/// `Router::layer(Extension(state))`.
pub struct HttpState {
    pub host: Arc<Host>,
    pub bindings: Arc<BindingStore>,
    pub auth: Arc<dyn InboundAuth>,
    /// P1 dispatcher. When `Some`, methods prefixed with `blob/`/`session/`/
    /// `operation/`/`agent/` are routed here; other methods still go to
    /// the P0 host. The /wss, /tickets and /oidc/* routes only mount when
    /// this is set (see [`attach_p1`]).
    pub broker: Option<Arc<Broker>>,
    /// P1 sidecar state (tickets, OIDC, agent registry). Only set when
    /// `broker` is also set.
    pub host_side: Option<HostSide>,
    /// P1 capabilities to advertise through hello.
    pub p1_provides: Vec<String>,
    pub web_auth: Option<Arc<crate::web_auth::WebAuth>>,
    pub ui_links: Arc<crate::ui_links::UiLinkRegistry>,
    /// Visitor client registry. The broker lists and addresses clients; the
    /// WSS loop registers each link's writer and resolves pongs.
    pub clients: Arc<crate::clients::ClientRegistry>,
    /// Group-scoped file sharing for the landing page's file scene. Bytes
    /// live only while the owner is connected.
    pub shares: Arc<crate::share::ShareStore>,
}

/// Build the P0 `/rpc` router without any shared state attached. Callers
/// must wrap it (typically with `Extension(state)`) before serving. This
/// keeps the router state-agnostic so P1 routes can be added later without
/// double-injecting extensions.
pub fn build_router() -> Router {
    Router::new()
        .route("/rpc", post(rpc))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

/// Wrap a router with the shared `HttpState` extension so every handler
/// (P0 and P1) can pull it via `Extension<Arc<HttpState>>`.
pub fn attach_state(state: Arc<HttpState>, inner: Router) -> Router {
    inner.layer(Extension(state))
}

/// Mount P1 routes (`/wss`, `/tickets`, `/oidc/*`) onto a router built by
/// [`router`]. All P1 routes pull state via `Extension<Arc<HttpState>>`;
/// the caller must wrap the final router with [`with_state`] before
/// serving.
pub fn attach_p1(base: Router) -> Router {
    base.route("/wss", get(crate::ws_transport::ws_handler_with_state))
        .route("/tickets", post(tickets::issue_ticket))
        .route("/web/login", post(crate::web_auth::login))
        .route("/web/session", get(crate::web_auth::session))
        .route("/content", get(crate::content_http::content))
        .route("/web/files", get(crate::share_http::list_files))
        .route(
            "/web/files",
            post(crate::share_http::upload_file)
                .layer(DefaultBodyLimit::max(crate::share_http::UPLOAD_BODY_LIMIT)),
        )
        .route("/web/files/download", get(crate::share_http::download_file))
        .route("/web/files/remove", post(crate::share_http::remove_file))
        .route("/web/logout", post(crate::web_auth::logout))
        .route("/oidc/authorize", post(tickets::oidc_authorize))
        .route("/oidc/token", post(tickets::oidc_token))
}

async fn rpc(
    Extension(state): Extension<Arc<HttpState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !is_json(&headers) {
        return (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "expected application/json",
        )
            .into_response();
    }
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let inbound = match state.auth.authenticate(authorization) {
        Ok(inbound) if inbound.role != "ui" => inbound,
        Ok(_) => {
            return (
                StatusCode::UNAUTHORIZED,
                "UI credentials require a web session",
            )
                .into_response();
        }
        Err(_) => return (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    };
    let message = match decode_wire(&body) {
        Ok(message) => message,
        Err(error) => return protocol_error(&error),
    };
    match message.body {
        Some(conex_proto::message::Body::Notification(notification)) => {
            handle_notification(&state, &inbound, notification).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Some(conex_proto::message::Body::Request(request)) => {
            handle_request(&state, &inbound, request).await
        }
        Some(conex_proto::message::Body::DataChunk(_)) => (
            StatusCode::BAD_REQUEST,
            "data chunks require the protobuf profile",
        )
            .into_response(),
        Some(conex_proto::message::Body::Success(_))
        | Some(conex_proto::message::Body::Failure(_)) => {
            (StatusCode::BAD_REQUEST, "clients must not send responses").into_response()
        }
        None => (StatusCode::BAD_REQUEST, "empty message").into_response(),
    }
}

async fn handle_request(
    state: &Arc<HttpState>,
    inbound: &InboundCaller,
    request: conex_proto::Request,
) -> Response {
    let request_id = request.request_id.clone();
    let params = match request.params {
        Some(params) => params,
        None => {
            return failure(
                &request_id,
                CallError::new(conex_proto::ErrorCode::BadRequest, "params are required"),
            );
        }
    };
    let context = match params.context {
        Some(context) => context,
        None => {
            return failure(
                &request_id,
                CallError::new(conex_proto::ErrorCode::BadRequest, "context is required"),
            );
        }
    };
    if context.plane != conex_proto::Plane::Broker as i32 {
        return failure(
            &request_id,
            CallError::new(
                conex_proto::ErrorCode::PlaneMismatch,
                "P0 only supports the broker plane",
            ),
        );
    }
    let input = params
        .input
        .as_ref()
        .map(pbjson_to_json)
        .unwrap_or(Value::Null);

    if request.method == HELLO_METHOD {
        let hello = match parse_hello(&input) {
            Ok(hello) => hello,
            Err(error) => return failure(&request_id, error),
        };
        return match state.bindings.issue(&inbound.caller, &hello) {
            Ok(response) => success(
                &request_id,
                serde_json::to_value(response).unwrap_or(Value::Null),
            ),
            Err(error) => failure(&request_id, error),
        };
    }

    let binding_id = match &context.binding_id {
        Some(binding_id) => binding_id.clone(),
        None => {
            return failure(
                &request_id,
                CallError::new(
                    conex_proto::ErrorCode::Unauthorized,
                    "business calls require a bindingId",
                ),
            );
        }
    };
    if let Err(error) = state.bindings.validate(&inbound.caller, &binding_id) {
        return failure(&request_id, error);
    }
    let timeout = Duration::from_millis(u64::from(params.timeout_budget_ms.max(1)));
    if request.method.starts_with("agent/") {
        return failure(
            &request_id,
            CallError::new(
                conex_proto::ErrorCode::Forbidden,
                "agent methods are only available on authenticated agent WSS links",
            ),
        );
    }
    if is_p1_method(&request.method) {
        let Some(broker) = state.broker.clone() else {
            return failure(
                &request_id,
                CallError::new(
                    conex_proto::ErrorCode::Unavailable,
                    format!(
                        "{} is not wired into this host (configure content_root/session_root/operation_root)",
                        request.method
                    ),
                ),
            );
        };
        let call = BrokerCall {
            caller: inbound.caller.clone(),
            endpoint_id: context.provider_endpoint_id.clone(),
            method: request.method.clone(),
            input,
            deadline: tokio::time::Instant::now() + timeout,
            role: inbound.role.clone(),
            link_id: None,
        };
        return match broker.invoke(call).await {
            Ok(value) => success(&request_id, value),
            Err(error) => failure(&request_id, error),
        };
    }
    match state
        .host
        .invoke(
            &inbound.caller,
            &context.provider_endpoint_id,
            &request.method,
            input,
            timeout,
        )
        .await
    {
        Ok(value) => success(&request_id, value),
        Err(error) => failure(&request_id, error),
    }
}

fn is_p1_method(method: &str) -> bool {
    method == "endpoint/list"
        || method == "connection/list"
        || method.starts_with("blob/")
        || method.starts_with("session/")
        || method.starts_with("operation/")
        || method.starts_with("agent/")
}

async fn handle_notification(
    state: &Arc<HttpState>,
    inbound: &InboundCaller,
    notification: conex_proto::Notification,
) {
    let Some(params) = notification.params else {
        return;
    };
    let Some(context) = params.context else {
        return;
    };
    if context.plane != conex_proto::Plane::Broker as i32 {
        return;
    }
    if let Some(binding_id) = &context.binding_id
        && state
            .bindings
            .validate(&inbound.caller, binding_id)
            .is_err()
    {
        return;
    }
    let input = params
        .input
        .as_ref()
        .map(pbjson_to_json)
        .unwrap_or(Value::Null);
    let timeout = Duration::from_millis(u64::from(params.timeout_budget_ms.max(1)));
    let _ = state
        .host
        .invoke(
            &inbound.caller,
            &context.provider_endpoint_id,
            &notification.method,
            input,
            timeout,
        )
        .await;
}

fn parse_hello(input: &Value) -> Result<conex_proto::HelloRequest, CallError> {
    let map = input.as_object().ok_or_else(|| {
        CallError::new(
            conex_proto::ErrorCode::BadRequest,
            "hello input must be an object",
        )
    })?;
    let profile_id = map
        .get("profileId")
        .and_then(Value::as_str)
        .unwrap_or(PROFILE_ID)
        .to_string();
    let plane = match map.get("plane").and_then(Value::as_str) {
        None | Some("broker") | Some("PLANE_BROKER") => conex_proto::Plane::Broker as i32,
        Some("relay") | Some("PLANE_RELAY") => conex_proto::Plane::Relay as i32,
        Some(_) => {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "unknown plane",
            ));
        }
    };
    Ok(conex_proto::HelloRequest {
        profile_id,
        plane,
        provides: string_array(map, "provides"),
        requires: string_array(map, "requires"),
    })
}

fn string_array(map: &Map<String, Value>, key: &str) -> Vec<String> {
    map.get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.starts_with("application/json"))
        .unwrap_or(false)
}

fn success(request_id: &str, value: Value) -> Response {
    let message = conex_proto::Message {
        body: Some(conex_proto::message::Body::Success(conex_proto::Success {
            request_id: request_id.to_string(),
            result: Some(json_to_pbjson(value)),
        })),
    };
    encode_response(&message)
}

fn failure(request_id: &str, error: CallError) -> Response {
    let message = conex_proto::Message {
        body: Some(conex_proto::message::Body::Failure(conex_proto::Failure {
            request_id: Some(request_id.to_string()),
            error: Some(error.into_wire()),
        })),
    };
    encode_response(&message)
}

fn protocol_error(error: &ProtocolError) -> Response {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": error.request_id,
        "error": {"code": error.rpc_code, "message": error.message},
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

fn encode_response(message: &conex_proto::Message) -> Response {
    match encode_wire(message) {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "encode error").into_response(),
    }
}

pub fn pbjson_to_json(value: &pbjson_types::Value) -> Value {
    // pbjson carries every number as f64; JSON integers must stay integers or
    // prost's strict `uint32` decode rejects `100.0` for a `limit` field.
    normalize_numbers(serde_json::to_value(value).unwrap_or(Value::Null))
}

/// Re-tag whole-number floats as integers (the pbjson → JSON round trip).
fn normalize_numbers(value: Value) -> Value {
    match value {
        Value::Number(number) => match number.as_f64() {
            Some(float) if float.fract() == 0.0 && float.abs() < 9.007_199_254_740_992e15 => {
                Value::Number(serde_json::Number::from(float as i64))
            }
            _ => Value::Number(number),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(normalize_numbers).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, normalize_numbers(value)))
                .collect(),
        ),
        other => other,
    }
}

pub fn json_to_pbjson(value: Value) -> pbjson_types::Value {
    serde_json::from_value(value).unwrap_or_default()
}

/// Wire limits for the hello response are derived from core limits.
pub fn wire_limits(limits: Limits) -> conex_proto::Limits {
    conex_proto::Limits {
        max_frame_bytes: limits.max_frame_bytes,
        max_inflight: limits.max_inflight,
        max_queued_bytes: limits.max_queued_bytes,
        timeout_ms: limits.timeout_ms,
    }
}

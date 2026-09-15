//! Axum /rpc entry: authenticate, decode, validate binding, call the one Host.
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use conex_core::{CallError, Host, Limits};
use conex_proto::v1;
use conex_proto::wire::{ProtocolError, decode_wire, encode_wire};
use serde_json::{Map, Value};

use crate::auth::{InboundAuth, InboundCaller};
use crate::binding::BindingStore;

pub const PROFILE_ID: &str = "conex-jsonrpc2-http-v1";
pub const MAX_BODY_BYTES: usize = 1_048_576;
pub const HELLO_METHOD: &str = "conex/hello";

pub struct HttpState {
    pub host: Arc<Host>,
    pub bindings: Arc<BindingStore>,
    pub auth: Arc<dyn InboundAuth>,
}

pub fn router(state: Arc<HttpState>) -> Router {
    Router::new()
        .route("/rpc", post(rpc))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn rpc(State(state): State<Arc<HttpState>>, headers: HeaderMap, body: Bytes) -> Response {
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
        Ok(inbound) => inbound,
        Err(_) => return (StatusCode::UNAUTHORIZED, "unauthorized").into_response(),
    };
    let message = match decode_wire(&body) {
        Ok(message) => message,
        Err(error) => return protocol_error(&error),
    };
    match message.body {
        Some(v1::message::Body::Notification(notification)) => {
            handle_notification(&state, &inbound, notification).await;
            StatusCode::NO_CONTENT.into_response()
        }
        Some(v1::message::Body::Request(request)) => {
            handle_request(&state, &inbound, request).await
        }
        Some(v1::message::Body::Success(_)) | Some(v1::message::Body::Failure(_)) => {
            (StatusCode::BAD_REQUEST, "clients must not send responses").into_response()
        }
        None => (StatusCode::BAD_REQUEST, "empty message").into_response(),
    }
}

async fn handle_request(
    state: &Arc<HttpState>,
    inbound: &InboundCaller,
    request: v1::Request,
) -> Response {
    let request_id = request.request_id.clone();
    let params = match request.params {
        Some(params) => params,
        None => {
            return failure(
                &request_id,
                CallError::new(v1::ErrorCode::BadRequest, "params are required"),
            );
        }
    };
    let context = match params.context {
        Some(context) => context,
        None => {
            return failure(
                &request_id,
                CallError::new(v1::ErrorCode::BadRequest, "context is required"),
            );
        }
    };
    if context.plane != v1::Plane::Broker as i32 {
        return failure(
            &request_id,
            CallError::new(
                v1::ErrorCode::PlaneMismatch,
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
                    v1::ErrorCode::Unauthorized,
                    "business calls require a bindingId",
                ),
            );
        }
    };
    if let Err(error) = state.bindings.validate(&inbound.caller, &binding_id) {
        return failure(&request_id, error);
    }
    let timeout = Duration::from_millis(u64::from(params.timeout_budget_ms.max(1)));
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

async fn handle_notification(
    state: &Arc<HttpState>,
    inbound: &InboundCaller,
    notification: v1::Notification,
) {
    let Some(params) = notification.params else {
        return;
    };
    let Some(context) = params.context else {
        return;
    };
    if context.plane != v1::Plane::Broker as i32 {
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

fn parse_hello(input: &Value) -> Result<v1::HelloRequest, CallError> {
    let map = input.as_object().ok_or_else(|| {
        CallError::new(v1::ErrorCode::BadRequest, "hello input must be an object")
    })?;
    let profile_id = map
        .get("profileId")
        .and_then(Value::as_str)
        .unwrap_or(PROFILE_ID)
        .to_string();
    let plane = match map.get("plane").and_then(Value::as_str) {
        None | Some("broker") | Some("PLANE_BROKER") => v1::Plane::Broker as i32,
        Some("relay") | Some("PLANE_RELAY") => v1::Plane::Relay as i32,
        Some(_) => return Err(CallError::new(v1::ErrorCode::BadRequest, "unknown plane")),
    };
    Ok(v1::HelloRequest {
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
    let message = v1::Message {
        body: Some(v1::message::Body::Success(v1::Success {
            request_id: request_id.to_string(),
            result: Some(json_to_pbjson(value)),
        })),
    };
    encode_response(&message)
}

fn failure(request_id: &str, error: CallError) -> Response {
    let message = v1::Message {
        body: Some(v1::message::Body::Failure(v1::Failure {
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

fn encode_response(message: &v1::Message) -> Response {
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

fn pbjson_to_json(value: &pbjson_types::Value) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

fn json_to_pbjson(value: Value) -> pbjson_types::Value {
    serde_json::from_value(value).unwrap_or_default()
}

/// Wire limits for the hello response are derived from core limits.
pub fn wire_limits(limits: Limits) -> v1::Limits {
    v1::Limits {
        max_frame_bytes: limits.max_frame_bytes,
        max_inflight: limits.max_inflight,
        max_queued_bytes: limits.max_queued_bytes,
        timeout_ms: limits.timeout_ms,
    }
}

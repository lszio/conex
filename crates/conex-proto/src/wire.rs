//! JSON-RPC 2.0 envelope -> conex v1 message mapping (design S5.2).
//!
//! The envelope is hand-mapped (not pbjson) so duplicate keys, unknown envelope
//! fields, ULID ids and the numeric error code table are enforced at the wire
//! boundary. Business payloads stay in google.protobuf.Value and are strictly
//! decoded later by MethodContract.prepare.
//!
//! The nested `if let` form mirrors validation.rs: each clause is an independent
//! rule, so collapsing them hides which rule rejected a value.
#![allow(clippy::collapsible_if)]

use std::collections::HashSet;

use serde::Deserialize;
use serde::de::{self, MapAccess, Visitor};
use serde_json::{Map, Value};

use crate::v1;

const EXECUTION_VALUES: [&str; 3] = ["not_started", "completed", "unknown"];
const RETRY_VALUES: [&str; 3] = ["never", "safe", "with_operation_id"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtocolError {
    pub rpc_code: i32,
    pub request_id: Option<String>,
    pub message: String,
}

impl ProtocolError {
    pub fn new(rpc_code: i32, request_id: Option<String>, message: impl Into<String>) -> Self {
        Self {
            rpc_code,
            request_id,
            message: message.into(),
        }
    }

    pub fn bad_request(request_id: Option<String>, message: impl Into<String>) -> Self {
        Self::new(v1::ErrorCode::BadRequest as i32, request_id, message)
    }
}

/// Crockford base32 ULID: 26 chars, first char at most '7' (128-bit bound).
pub fn is_ulid(value: &str) -> bool {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let bytes = value.as_bytes();
    if bytes.len() != 26 || bytes[0] > b'7' {
        return false;
    }
    bytes.iter().all(|b| ALPHABET.contains(b))
}

struct RawEnvelope {
    jsonrpc: String,
    id: Option<Value>,
    method: Option<String>,
    params: Option<Value>,
    result: Option<Value>,
    error: Option<Value>,
}

impl<'de> Deserialize<'de> for RawEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct EnvelopeVisitor;
        impl<'de> Visitor<'de> for EnvelopeVisitor {
            type Value = RawEnvelope;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON-RPC 2.0 envelope object")
            }
            fn visit_map<A>(self, mut map: A) -> Result<RawEnvelope, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut jsonrpc = None;
                let mut id = None;
                let mut method = None;
                let mut params = None;
                let mut result = None;
                let mut error = None;
                let mut seen: HashSet<String> = HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !seen.insert(key.clone()) {
                        return Err(de::Error::custom(format!("duplicate field {key}")));
                    }
                    match key.as_str() {
                        "jsonrpc" => jsonrpc = Some(map.next_value::<String>()?),
                        "id" => id = Some(map.next_value::<Value>()?),
                        "method" => method = Some(map.next_value::<String>()?),
                        "params" => params = Some(map.next_value::<Value>()?),
                        "result" => result = Some(map.next_value::<Value>()?),
                        "error" => error = Some(map.next_value::<Value>()?),
                        other => {
                            return Err(de::Error::custom(format!(
                                "unknown envelope field {other}"
                            )));
                        }
                    }
                }
                Ok(RawEnvelope {
                    jsonrpc: jsonrpc.ok_or_else(|| de::Error::missing_field("jsonrpc"))?,
                    id,
                    method,
                    params,
                    result,
                    error,
                })
            }
        }
        deserializer.deserialize_map(EnvelopeVisitor)
    }
}

pub fn decode_wire(bytes: &[u8]) -> Result<v1::Message, ProtocolError> {
    let raw: RawEnvelope = serde_json::from_slice(bytes).map_err(|e| {
        ProtocolError::new(
            v1::ErrorCode::ParseError as i32,
            None,
            format!("invalid JSON-RPC envelope: {e}"),
        )
    })?;
    if raw.jsonrpc != "2.0" {
        return Err(ProtocolError::bad_request(
            None,
            "jsonrpc must be the string 2.0",
        ));
    }
    let has_result = raw.result.is_some();
    let has_error = raw.error.is_some();

    if let Some(method) = raw.method.clone() {
        if has_result || has_error {
            return Err(ProtocolError::bad_request(
                None,
                "method cannot appear with result or error",
            ));
        }
        let params_value = raw.params.clone().ok_or_else(|| {
            ProtocolError::bad_request(None, "request or notification requires params")
        })?;
        let params = parse_call_params(&params_value, None)?;
        match raw.id.clone() {
            None => Ok(v1::Message {
                body: Some(v1::message::Body::Notification(v1::Notification {
                    method,
                    params: Some(params),
                })),
            }),
            Some(Value::String(request_id)) => {
                if !is_ulid(&request_id) {
                    return Err(ProtocolError::bad_request(
                        None,
                        "request id must be a ULID",
                    ));
                }
                Ok(v1::Message {
                    body: Some(v1::message::Body::Request(v1::Request {
                        request_id,
                        method,
                        params: Some(params),
                    })),
                })
            }
            Some(_) => Err(ProtocolError::bad_request(
                None,
                "request id must be a ULID string",
            )),
        }
    } else {
        if has_result && has_error {
            return Err(ProtocolError::bad_request(
                None,
                "success and error cannot both be present",
            ));
        }
        let request_id = match raw.id.clone() {
            None => return Err(ProtocolError::bad_request(None, "response requires an id")),
            Some(Value::String(s)) => {
                if !is_ulid(&s) {
                    return Err(ProtocolError::bad_request(
                        None,
                        "response id must be a ULID",
                    ));
                }
                Some(s)
            }
            Some(Value::Null) => None,
            Some(_) => {
                return Err(ProtocolError::bad_request(
                    None,
                    "response id must be a ULID string or null",
                ));
            }
        };
        if has_result {
            let request_id = request_id.ok_or_else(|| {
                ProtocolError::bad_request(None, "success requires a non-null id")
            })?;
            let result = match raw.result.clone() {
                Some(v) => Some(serde_json::from_value(v).map_err(|e| {
                    ProtocolError::bad_request(None, format!("invalid result: {e}"))
                })?),
                None => None,
            };
            Ok(v1::Message {
                body: Some(v1::message::Body::Success(v1::Success {
                    request_id,
                    result,
                })),
            })
        } else if let Some(error) = raw.error.clone() {
            Ok(v1::Message {
                body: Some(v1::message::Body::Failure(v1::Failure {
                    request_id,
                    error: Some(parse_error(&error, None)?),
                })),
            })
        } else {
            Err(ProtocolError::bad_request(
                None,
                "response has neither result nor error",
            ))
        }
    }
}

pub fn encode_wire(message: &v1::Message) -> Result<Vec<u8>, ProtocolError> {
    let body = message
        .body
        .as_ref()
        .ok_or_else(|| ProtocolError::bad_request(None, "message has no body"))?;
    let mut out = Map::new();
    out.insert("jsonrpc".into(), Value::String("2.0".into()));
    match body {
        v1::message::Body::Request(r) => {
            out.insert("id".into(), Value::String(r.request_id.clone()));
            out.insert("method".into(), Value::String(r.method.clone()));
            out.insert("params".into(), call_params_to_json(r.params.as_ref())?);
        }
        v1::message::Body::Success(s) => {
            out.insert("id".into(), Value::String(s.request_id.clone()));
            out.insert(
                "result".into(),
                s.result
                    .as_ref()
                    .map(value_to_json)
                    .transpose()?
                    .unwrap_or(Value::Null),
            );
        }
        v1::message::Body::Failure(f) => {
            out.insert(
                "id".into(),
                f.request_id
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Null),
            );
            out.insert("error".into(), error_to_json(f.error.as_ref())?);
        }
        v1::message::Body::Notification(n) => {
            out.insert("method".into(), Value::String(n.method.clone()));
            out.insert("params".into(), call_params_to_json(n.params.as_ref())?);
        }
    }
    serde_json::to_vec(&Value::Object(out)).map_err(|e| {
        ProtocolError::new(
            v1::ErrorCode::Internal as i32,
            None,
            format!("encode error: {e}"),
        )
    })
}

pub fn validate_message(message: &v1::Message) -> Result<(), ProtocolError> {
    let body = message
        .body
        .as_ref()
        .ok_or_else(|| ProtocolError::bad_request(None, "message has no body"))?;
    match body {
        v1::message::Body::Request(r) => {
            if !is_ulid(&r.request_id) {
                return Err(ProtocolError::bad_request(None, "requestId must be a ULID"));
            }
            if r.method.is_empty() {
                return Err(ProtocolError::bad_request(
                    Some(r.request_id.clone()),
                    "method must not be empty",
                ));
            }
            let params = r.params.as_ref().ok_or_else(|| {
                ProtocolError::bad_request(Some(r.request_id.clone()), "request requires params")
            })?;
            validate_params(params, Some(r.request_id.clone()))?;
        }
        v1::message::Body::Success(s) => {
            if !is_ulid(&s.request_id) {
                return Err(ProtocolError::bad_request(None, "requestId must be a ULID"));
            }
        }
        v1::message::Body::Failure(f) => {
            if let Some(id) = &f.request_id {
                if !is_ulid(id) {
                    return Err(ProtocolError::bad_request(None, "requestId must be a ULID"));
                }
            }
            let error = f.error.as_ref().ok_or_else(|| {
                ProtocolError::bad_request(f.request_id.clone(), "failure requires error")
            })?;
            if error.code == v1::ErrorCode::Unspecified as i32 {
                return Err(ProtocolError::bad_request(
                    f.request_id.clone(),
                    "error code must be specified",
                ));
            }
            if !error.execution.is_empty() && !EXECUTION_VALUES.contains(&error.execution.as_str())
            {
                return Err(ProtocolError::bad_request(
                    f.request_id.clone(),
                    "invalid execution value",
                ));
            }
            if !error.retry.is_empty() && !RETRY_VALUES.contains(&error.retry.as_str()) {
                return Err(ProtocolError::bad_request(
                    f.request_id.clone(),
                    "invalid retry value",
                ));
            }
        }
        v1::message::Body::Notification(n) => {
            if n.method.is_empty() {
                return Err(ProtocolError::bad_request(None, "method must not be empty"));
            }
            let params = n
                .params
                .as_ref()
                .ok_or_else(|| ProtocolError::bad_request(None, "notification requires params"))?;
            validate_params(params, None)?;
        }
    }
    Ok(())
}

fn validate_params(
    params: &v1::CallParams,
    request_id: Option<String>,
) -> Result<(), ProtocolError> {
    let context = params
        .context
        .as_ref()
        .ok_or_else(|| ProtocolError::bad_request(request_id.clone(), "params requires context"))?;
    if context.plane != v1::Plane::Broker as i32 {
        return Err(ProtocolError::new(
            v1::ErrorCode::PlaneMismatch as i32,
            request_id,
            "P0 only supports the broker plane",
        ));
    }
    if params.timeout_budget_ms == 0 {
        return Err(ProtocolError::bad_request(
            request_id,
            "timeoutBudgetMs must be greater than 0",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCallParams {
    context: RawContext,
    #[serde(rename = "timeoutBudgetMs", default)]
    timeout_budget_ms: u32,
    #[serde(default)]
    input: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContext {
    #[serde(rename = "providerEndpointId")]
    provider_endpoint_id: String,
    plane: String,
    #[serde(rename = "bindingId", default)]
    binding_id: Option<String>,
}

fn parse_call_params(
    value: &Value,
    request_id: Option<String>,
) -> Result<v1::CallParams, ProtocolError> {
    let raw: RawCallParams = serde_json::from_value(value.clone()).map_err(|e| {
        ProtocolError::bad_request(request_id.clone(), format!("invalid params: {e}"))
    })?;
    let plane = match raw.context.plane.as_str() {
        "broker" => v1::Plane::Broker as i32,
        "relay" => v1::Plane::Relay as i32,
        "unspecified" => v1::Plane::Unspecified as i32,
        other => {
            return Err(ProtocolError::bad_request(
                request_id,
                format!("unknown plane {other}"),
            ));
        }
    };
    let input = match raw.input {
        Some(v) => Some(serde_json::from_value(v).map_err(|e| {
            ProtocolError::bad_request(request_id.clone(), format!("invalid input: {e}"))
        })?),
        None => None,
    };
    Ok(v1::CallParams {
        context: Some(v1::RequestContext {
            provider_endpoint_id: raw.context.provider_endpoint_id,
            plane,
            binding_id: raw.context.binding_id,
        }),
        timeout_budget_ms: raw.timeout_budget_ms,
        input,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawError {
    code: i64,
    message: String,
    #[serde(default)]
    data: Option<RawErrorData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawErrorData {
    #[serde(default)]
    code: Option<String>,
    #[serde(rename = "diagnosticId", default)]
    diagnostic_id: Option<String>,
    #[serde(default)]
    execution: Option<String>,
    #[serde(default)]
    retry: Option<String>,
    #[serde(default)]
    details: Option<Value>,
}

fn parse_error(value: &Value, request_id: Option<String>) -> Result<v1::Error, ProtocolError> {
    let raw: RawError = serde_json::from_value(value.clone()).map_err(|e| {
        ProtocolError::bad_request(request_id.clone(), format!("invalid error object: {e}"))
    })?;
    let code = i32::try_from(raw.code).map_err(|_| {
        ProtocolError::bad_request(request_id.clone(), "error code out of int32 range")
    })?;
    let data = raw.data.unwrap_or(RawErrorData {
        code: None,
        diagnostic_id: None,
        execution: None,
        retry: None,
        details: None,
    });
    if let Some(semantic) = data.code.as_deref() {
        if semantic != error_code_name(code) {
            return Err(ProtocolError::bad_request(
                request_id.clone(),
                format!("error.data.code {semantic} does not match numeric code {code}"),
            ));
        }
    }
    let details = match data.details {
        Some(v) => Some(serde_json::from_value(v).map_err(|e| {
            ProtocolError::bad_request(request_id.clone(), format!("invalid details: {e}"))
        })?),
        None => None,
    };
    Ok(v1::Error {
        code,
        message: raw.message,
        diagnostic_id: data.diagnostic_id.unwrap_or_default(),
        execution: data.execution.unwrap_or_default(),
        retry: data.retry.unwrap_or_default(),
        details,
    })
}

fn call_params_to_json(params: Option<&v1::CallParams>) -> Result<Value, ProtocolError> {
    let params = params.ok_or_else(|| ProtocolError::bad_request(None, "params is required"))?;
    let context = params
        .context
        .as_ref()
        .ok_or_else(|| ProtocolError::bad_request(None, "context is required"))?;
    let mut ctx = Map::new();
    ctx.insert(
        "providerEndpointId".into(),
        Value::String(context.provider_endpoint_id.clone()),
    );
    ctx.insert(
        "plane".into(),
        Value::String(plane_name(context.plane).into()),
    );
    if let Some(binding) = &context.binding_id {
        ctx.insert("bindingId".into(), Value::String(binding.clone()));
    }
    let mut out = Map::new();
    out.insert("context".into(), Value::Object(ctx));
    out.insert(
        "timeoutBudgetMs".into(),
        Value::Number(params.timeout_budget_ms.into()),
    );
    if let Some(input) = &params.input {
        out.insert("input".into(), value_to_json(input)?);
    }
    Ok(Value::Object(out))
}

fn error_to_json(error: Option<&v1::Error>) -> Result<Value, ProtocolError> {
    let error = error.ok_or_else(|| ProtocolError::bad_request(None, "error is required"))?;
    let mut data = Map::new();
    data.insert(
        "code".into(),
        Value::String(error_code_name(error.code).into()),
    );
    if !error.diagnostic_id.is_empty() {
        data.insert(
            "diagnosticId".into(),
            Value::String(error.diagnostic_id.clone()),
        );
    }
    if !error.execution.is_empty() {
        data.insert("execution".into(), Value::String(error.execution.clone()));
    }
    if !error.retry.is_empty() {
        data.insert("retry".into(), Value::String(error.retry.clone()));
    }
    if let Some(details) = &error.details {
        data.insert("details".into(), value_to_json(details)?);
    }
    let mut out = Map::new();
    out.insert("code".into(), Value::Number(error.code.into()));
    out.insert("message".into(), Value::String(error.message.clone()));
    out.insert("data".into(), Value::Object(data));
    Ok(Value::Object(out))
}

fn value_to_json(value: &pbjson_types::Value) -> Result<Value, ProtocolError> {
    serde_json::to_value(value).map_err(|e| {
        ProtocolError::new(
            v1::ErrorCode::Internal as i32,
            None,
            format!("value encode error: {e}"),
        )
    })
}

fn plane_name(plane: i32) -> &'static str {
    match v1::Plane::try_from(plane) {
        Ok(v1::Plane::Broker) => "broker",
        Ok(v1::Plane::Relay) => "relay",
        Ok(v1::Plane::Unspecified) => "unspecified",
        Err(_) => "unspecified",
    }
}

/// Lowercase semantic name derived from the frozen ErrorCode enum. The match is
/// exhaustive over Ok variants, so adding an enum value without updating this
/// function fails to compile.
pub fn error_code_name(code: i32) -> &'static str {
    use v1::ErrorCode as E;
    match E::try_from(code) {
        Ok(E::Unspecified) => "unspecified",
        Ok(E::ParseError) => "parse_error",
        Ok(E::BadRequest) => "bad_request",
        Ok(E::UnknownMethod) => "unknown_method",
        Ok(E::Internal) => "internal",
        Ok(E::Unauthorized) => "unauthorized",
        Ok(E::Forbidden) => "forbidden",
        Ok(E::UnknownProvider) => "unknown_provider",
        Ok(E::UnsupportedCapability) => "unsupported_capability",
        Ok(E::Unavailable) => "unavailable",
        Ok(E::Timeout) => "timeout",
        Ok(E::PayloadTooLarge) => "payload_too_large",
        Ok(E::QuotaExceeded) => "quota_exceeded",
        Ok(E::PlaneMismatch) => "plane_mismatch",
        Ok(E::PeerUntrusted) => "peer_untrusted",
        Ok(E::Cancelled) => "cancelled",
        Ok(E::OutcomeUnknown) => "outcome_unknown",
        Ok(E::StaleRevision) => "stale_revision",
        Ok(E::Conflict) => "conflict",
        Ok(E::SlowConsumer) => "slow_consumer",
        Ok(E::BadBlob) => "bad_blob",
        Ok(E::SessionLost) => "session_lost",
        Ok(E::ResumeUnavailable) => "resume_unavailable",
        Err(_) => "unknown",
    }
}

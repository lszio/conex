//! S1 能力边界：方法目录、按方向的 context 规则、业务 input/output 严格
//! 校验、规范 uint64 与 state/end Error JSON 编解码（docs/contracts/
//! s1-capabilities.md §4-§9）。
//!
//! 信封级规则（ULID id、任意深度的重复键、plane、S1 通知缺省
//! timeoutBudgetMs、S1 通知必须带 input）留在 [`crate::wire`]；本模块是
//! 方法级规则。形状校验复用生成器产出的单一 JSON Schema 结构源
//! （schema/generated/jsonschema，经 [`crate::validation`] 执行），额外规范
//! 规则（规范 uint64、quality 词表、ULID 标识符、client 身份禁令、state/end
//! error 的枚举名形状）与 sdk/typescript/src/s1-wire.ts 实现同一规则集，
//! 由 conformance/vectors/s1/capabilities.json 共享向量仲裁一致。
#![allow(clippy::collapsible_if)]

use std::sync::LazyLock;

use serde_json::{Map, Value};

use crate::wire::{ProtocolError, is_ulid, value_to_json};

pub const METHOD_MODULE_REGISTER: &str = "module/register";
pub const METHOD_MODULE_UNREGISTER: &str = "module/unregister";
pub const METHOD_MODULE_LIST: &str = "module/list";
pub const METHOD_MODULE_INVOKE: &str = "module/invoke";
pub const METHOD_MODULE_CANCEL: &str = "module/cancel";
pub const METHOD_STATE_SUBSCRIBE: &str = "state/subscribe";
pub const METHOD_STATE_UNSUBSCRIBE: &str = "state/unsubscribe";
pub const METHOD_STATE_UPDATE: &str = "state/update";
pub const METHOD_STATE_END: &str = "state/end";

pub const S1_METHODS: [&str; 9] = [
    METHOD_MODULE_REGISTER,
    METHOD_MODULE_UNREGISTER,
    METHOD_MODULE_LIST,
    METHOD_MODULE_INVOKE,
    METHOD_MODULE_CANCEL,
    METHOD_STATE_SUBSCRIBE,
    METHOD_STATE_UNSUBSCRIBE,
    METHOD_STATE_UPDATE,
    METHOD_STATE_END,
];

pub fn is_s1_method(method: &str) -> bool {
    S1_METHODS.contains(&method)
}

/// S1 方法表中必须以带 JSON-RPC id 的请求形态发送的七个方法。
pub fn is_s1_request(method: &str) -> bool {
    matches!(
        method,
        METHOD_MODULE_REGISTER
            | METHOD_MODULE_UNREGISTER
            | METHOD_MODULE_LIST
            | METHOD_MODULE_INVOKE
            | METHOD_MODULE_CANCEL
            | METHOD_STATE_SUBSCRIBE
            | METHOD_STATE_UNSUBSCRIBE
    )
}

/// S1 通知：state/update、state/end（无 id、省略 timeoutBudgetMs）。
pub fn is_s1_notification(method: &str) -> bool {
    matches!(method, METHOD_STATE_UPDATE | METHOD_STATE_END)
}

/// 控制方法：context 只含 plane/bindingId，目标在 input（契约 §5.2）。
fn is_control(method: &str) -> bool {
    matches!(
        method,
        METHOD_MODULE_REGISTER | METHOD_MODULE_UNREGISTER | METHOD_MODULE_LIST
    )
}

/// 帧来源方向。`Client` 是不受信链路（SDK→中心、提供端→中心）：不得自行
/// 携带 principalId/tenantId。`Host` 是中心盖章后转发的帧：可携带已验证身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    Client,
    Host,
}

/// Complete JSON boundary. Use this for S1 instead of decoding to proto3 first:
/// raw client identity presence must be checked before default values are lost.
pub fn decode_frame(bytes: &[u8], boundary: Boundary) -> Result<crate::Message, ProtocolError> {
    let message = crate::wire::decode_wire_for_boundary(bytes, boundary)?;
    crate::wire::validate_message(&message)?;
    validate_frame(&message, boundary)?;
    Ok(message)
}

/// HTTP admits only directory/invoke/cancel, never provider or subscription state.
/// Authentication and binding ownership remain the Host's responsibility.
pub fn validate_http_frame(message: &crate::Message) -> Result<(), ProtocolError> {
    let (method, params) = match message.body.as_ref() {
        Some(crate::message::Body::Request(r)) => (r.method.as_str(), r.params.as_ref()),
        Some(crate::message::Body::Notification(n)) => (n.method.as_str(), n.params.as_ref()),
        _ => return Ok(()),
    };
    if !is_s1_method(method) {
        return Ok(());
    }
    if !matches!(
        method,
        METHOD_MODULE_LIST | METHOD_MODULE_INVOKE | METHOD_MODULE_CANCEL
    ) {
        return Err(ProtocolError::new(
            crate::ErrorCode::UnsupportedCapability as i32,
            None,
            "S1 method requires WSS",
        ));
    }
    if !matches!(
        message.body.as_ref(),
        Some(crate::message::Body::Request(_))
    ) || params
        .and_then(|p| p.context.as_ref())
        .and_then(|c| c.binding_id.as_deref())
        .is_none_or(str::is_empty)
    {
        return Err(ProtocolError::bad_request(
            None,
            "HTTP S1 requests require bindingId",
        ));
    }
    Ok(())
}

/// S1 帧总闸：方法目录、形态（请求/通知）、预算、context 与 input。
/// 非 S1 方法原样放行（P0/P1 由既有分发处理）；响应帧不在此校验。
pub fn validate_frame(message: &crate::Message, boundary: Boundary) -> Result<(), ProtocolError> {
    let body = message
        .body
        .as_ref()
        .ok_or_else(|| ProtocolError::bad_request(None, "message has no body"))?;
    let (method, params, is_request) = match body {
        crate::message::Body::Request(r) => (r.method.as_str(), r.params.as_ref(), true),
        crate::message::Body::Notification(n) => (n.method.as_str(), n.params.as_ref(), false),
        _ => return Ok(()),
    };
    if !is_s1_method(method) {
        return Ok(());
    }
    if is_s1_request(method) != is_request {
        let what = if is_request {
            "is a notification and must not carry an id"
        } else {
            "requires the request form (with a JSON-RPC id)"
        };
        return Err(ProtocolError::bad_request(None, format!("{method} {what}")));
    }
    let params =
        params.ok_or_else(|| ProtocolError::bad_request(None, "S1 frames require params"))?;
    if is_request && params.timeout_budget_ms == 0 {
        return Err(ProtocolError::bad_request(
            None,
            "timeoutBudgetMs must be greater than 0",
        ));
    }
    if !is_request && params.timeout_budget_ms != 0 {
        return Err(ProtocolError::bad_request(
            None,
            "S1 notifications must omit timeoutBudgetMs",
        ));
    }
    validate_context(method, params.context.as_ref(), boundary)?;
    let input = params
        .input
        .as_ref()
        .ok_or_else(|| ProtocolError::bad_request(None, "S1 frames require input"))?;
    validate_input(method, &value_to_json(input)?)
}

/// context 字段规则（契约 §5.2、§6.3、§7.3）：控制方法只允许 plane/bindingId；
/// 定向方法必须携带非空 providerEndpointId/spaceId 与 ULID registrationId；
/// 客户端帧不得携带 principalId/tenantId，Host 盖章帧可以。
pub fn validate_context(
    method: &str,
    context: Option<&crate::RequestContext>,
    boundary: Boundary,
) -> Result<(), ProtocolError> {
    let context =
        context.ok_or_else(|| ProtocolError::bad_request(None, "S1 frames require context"))?;
    if context.plane != crate::Plane::Broker as i32 {
        return Err(ProtocolError::new(
            crate::ErrorCode::PlaneMismatch as i32,
            None,
            "S1 requires the broker plane",
        ));
    }
    if boundary == Boundary::Client {
        if !context.principal_id.is_empty() || !context.tenant_id.is_empty() {
            return Err(ProtocolError::bad_request(
                None,
                "clients must not supply principalId or tenantId",
            ));
        }
    }
    if is_control(method) {
        if !context.provider_endpoint_id.is_empty()
            || !context.space_id.is_empty()
            || !context.registration_id.is_empty()
        {
            return Err(ProtocolError::bad_request(
                None,
                "control method context must not carry providerEndpointId/spaceId/registrationId",
            ));
        }
        return Ok(());
    }
    if context.provider_endpoint_id.is_empty() {
        return Err(ProtocolError::bad_request(
            None,
            "context requires providerEndpointId",
        ));
    }
    if context.space_id.is_empty() {
        return Err(ProtocolError::bad_request(None, "context requires spaceId"));
    }
    if !is_ulid(&context.registration_id) {
        return Err(ProtocolError::bad_request(
            None,
            "context registrationId must be a ULID",
        ));
    }
    Ok(())
}

/// 规范 uint64 十进制字符串：`"0"` 或不带前导零、不超过 u64::MAX（契约 §2）。
/// 生成 Schema 的 `^[0-9]+$` pattern 允许前导零，因此边界在此收紧。
pub fn is_canonical_uint64(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| b.is_ascii_digit())
        && (s == "0" || !s.starts_with('0'))
        && s.parse::<u64>().is_ok()
}

/// Positive u32 limits with one inflight slot and one frame reserved for control.
pub fn validate_limits(limits: &crate::Limits) -> Result<(), ProtocolError> {
    if [
        limits.max_frame_bytes,
        limits.max_inflight,
        limits.max_queued_bytes,
        limits.timeout_ms,
        limits.max_subscriptions,
        limits.max_modules,
    ]
    .contains(&0)
    {
        return Err(ProtocolError::bad_request(
            None,
            "all S1 limits must be positive",
        ));
    }
    if limits.max_inflight < 2
        || u64::from(limits.max_queued_bytes) < 2 * u64::from(limits.max_frame_bytes)
    {
        return Err(ProtocolError::bad_request(
            None,
            "limits must reserve control capacity",
        ));
    }
    Ok(())
}

/// Strict JSON limits before pbjson's permissive scalar conversion.
pub fn limits_from_json(value: &Value) -> Result<crate::Limits, ProtocolError> {
    validate_domain_schema("conex.Limits", value)?;
    let limits: crate::Limits = serde_json::from_value(value.clone())
        .map_err(|e| ProtocolError::bad_request(None, format!("invalid limits: {e}")))?;
    validate_limits(&limits)?;
    Ok(limits)
}

// ---- 生成 JSON Schema 形状校验 ------------------------------------------

/// 任一生成 Schema 文档都携带完整 $defs 表；`validate_against` 以目标消息名
/// 为根引用，避免为每个消息内嵌整份文档。
const SCHEMA_DOC: &str =
    include_str!("../../../schema/generated/jsonschema/conex.ModuleRegisterInput.schema.json");

static SCHEMA_DOC_PARSED: LazyLock<Value> =
    LazyLock::new(|| serde_json::from_str(SCHEMA_DOC).expect("embedded S1 schema doc is JSON"));

fn schema_defs() -> &'static Value {
    SCHEMA_DOC_PARSED
        .get("$defs")
        .expect("embedded S1 schema doc has $defs")
}

fn validate_schema_message(message: &str, instance: &Value) -> Result<(), String> {
    crate::validation::validate_against(schema_defs(), message, instance)
}

/// 任意生成消息的形状校验（域契约共用：如 module/invoke 的 argument 必须按
/// 具体契约输入、例如 conex.CounterAddInput 校验；T03/T07 消费同一入口）。
pub fn validate_domain_schema(message: &str, value: &Value) -> Result<(), ProtocolError> {
    validate_schema_message(message, value)
        .map_err(|e| ProtocolError::bad_request(None, format!("invalid {message}: {e}")))?;
    if message == "conex.CounterResult" {
        validate_uint64_string(as_object(value, "result")?, "revision")?;
    }
    Ok(())
}

fn input_message(method: &str) -> Option<&'static str> {
    match method {
        METHOD_MODULE_REGISTER => Some("conex.ModuleRegisterInput"),
        METHOD_MODULE_UNREGISTER => Some("conex.ModuleUnregisterInput"),
        METHOD_MODULE_LIST => Some("conex.ModuleListInput"),
        METHOD_MODULE_INVOKE => Some("conex.ModuleInvokeInput"),
        METHOD_MODULE_CANCEL => Some("conex.ModuleCancelInput"),
        METHOD_STATE_SUBSCRIBE => Some("conex.StateSubscribeInput"),
        METHOD_STATE_UNSUBSCRIBE => Some("conex.StateUnsubscribeInput"),
        METHOD_STATE_UPDATE => Some("conex.StateUpdateInput"),
        METHOD_STATE_END => Some("conex.StateEndInput"),
        _ => None,
    }
}

fn result_message(method: &str) -> Option<&'static str> {
    match method {
        METHOD_MODULE_REGISTER => Some("conex.ModuleRegisterResult"),
        METHOD_MODULE_UNREGISTER => Some("conex.ModuleUnregisterResult"),
        METHOD_MODULE_LIST => Some("conex.ModuleListResult"),
        METHOD_MODULE_CANCEL => Some("conex.ModuleCancelResult"),
        METHOD_STATE_SUBSCRIBE => Some("conex.StateSubscribeResult"),
        METHOD_STATE_UNSUBSCRIBE => Some("conex.StateUnsubscribeResult"),
        _ => None,
    }
}

/// S1 方法 input 的完整校验：生成 Schema 形状 + 规范规则。
pub fn validate_input(method: &str, input: &Value) -> Result<(), ProtocolError> {
    let message = input_message(method).ok_or_else(|| {
        ProtocolError::bad_request(None, format!("{method} has no S1 input type"))
    })?;
    validate_schema_message(message, input)
        .map_err(|e| ProtocolError::bad_request(None, format!("invalid {method} input: {e}")))?;
    let map = as_object(input, "input")?;
    match method {
        METHOD_MODULE_REGISTER => {
            for key in [
                "spaceId",
                "integrationId",
                "providerEndpointId",
                "contractId",
            ] {
                require_string(map, key)?;
            }
        }
        METHOD_MODULE_UNREGISTER => {
            require_string(map, "providerEndpointId")?;
            require_ulid(map, "registrationId")?;
        }
        METHOD_MODULE_LIST => {
            require_string(map, "spaceId")?;
        }
        METHOD_MODULE_INVOKE => {
            require_string(map, "method")?;
            if !map.contains_key("argument") {
                return Err(ProtocolError::bad_request(
                    None,
                    "input.argument is required",
                ));
            }
        }
        METHOD_MODULE_CANCEL => {
            require_ulid(map, "requestId")?;
        }
        METHOD_STATE_SUBSCRIBE => {
            require_ulid(map, "subscriptionId")?;
            require_string(map, "state")?;
            if !map.contains_key("argument") {
                return Err(ProtocolError::bad_request(
                    None,
                    "input.argument is required",
                ));
            }
        }
        METHOD_STATE_UNSUBSCRIBE => {
            require_ulid(map, "subscriptionId")?;
        }
        METHOD_STATE_UPDATE => {
            require_ulid(map, "subscriptionId")?;
            validate_uint64_string(map, "seq")?;
            let sample = map
                .get("sample")
                .ok_or_else(|| ProtocolError::bad_request(None, "input.sample is required"))?;
            validate_sample(sample)?;
        }
        METHOD_STATE_END => {
            require_ulid(map, "subscriptionId")?;
            let error = map
                .get("error")
                .ok_or_else(|| ProtocolError::bad_request(None, "input.error is required"))?;
            parse_state_end_error(error).map(|_| ())?;
        }
        _ => {}
    }
    Ok(())
}

/// S1 方法 result 校验：生成 Schema 形状 + 规范规则。
/// module/invoke 的结果是契约 typed-value，由 T03 按具体契约校验，这里放行。
pub fn validate_result(method: &str, result: &Value) -> Result<(), ProtocolError> {
    let Some(message) = result_message(method) else {
        return Ok(());
    };
    validate_schema_message(message, result)
        .map_err(|e| ProtocolError::bad_request(None, format!("invalid {method} result: {e}")))?;
    let map = as_object(result, "result")?;
    match method {
        METHOD_MODULE_REGISTER => {
            require_ulid(map, "registrationId")?;
            for key in [
                "providerEndpointId",
                "spaceId",
                "integrationId",
                "contractId",
            ] {
                require_string(map, key)?;
            }
        }
        METHOD_MODULE_LIST => {
            if let Some(modules) = map.get("modules").and_then(Value::as_array) {
                for (i, module) in modules.iter().enumerate() {
                    let m = as_object(module, &format!("result.modules[{i}]"))?;
                    for key in ["methods", "states"] {
                        if !m.get(key).is_some_and(Value::is_array) {
                            return Err(ProtocolError::bad_request(
                                None,
                                format!("module.{key} is required"),
                            ));
                        }
                    }
                    require_ulid(m, "registrationId")?;
                    for key in [
                        "providerEndpointId",
                        "spaceId",
                        "integrationId",
                        "contractId",
                    ] {
                        require_string(m, key)?;
                    }
                    if let Some(methods) = m.get("methods").and_then(Value::as_array) {
                        if methods.iter().any(|v| !v.is_string()) {
                            return Err(ProtocolError::bad_request(
                                None,
                                "result.modules methods must be strings",
                            ));
                        }
                    }
                    if let Some(states) = m.get("states").and_then(Value::as_array) {
                        if states.iter().any(|v| !v.is_string()) {
                            return Err(ProtocolError::bad_request(
                                None,
                                "result.modules states must be strings",
                            ));
                        }
                    }
                }
            } else {
                return Err(ProtocolError::bad_request(
                    None,
                    "result.modules is required",
                ));
            }
        }
        METHOD_STATE_SUBSCRIBE => {
            require_ulid(map, "subscriptionId")?;
            require_ulid(map, "registrationId")?;
            let snapshot = map
                .get("snapshot")
                .ok_or_else(|| ProtocolError::bad_request(None, "result.snapshot is required"))?;
            validate_sample(snapshot)?;
        }
        _ => {}
    }
    Ok(())
}

// ---- 规范规则 helpers ----------------------------------------------------

fn as_object<'a>(value: &'a Value, path: &str) -> Result<&'a Map<String, Value>, ProtocolError> {
    value
        .as_object()
        .ok_or_else(|| ProtocolError::bad_request(None, format!("{path} must be an object")))
}

fn require_string<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a str, ProtocolError> {
    match map.get(key) {
        Some(Value::String(s)) if !s.is_empty() => Ok(s),
        Some(_) => Err(ProtocolError::bad_request(
            None,
            format!("{key} must be a non-empty string"),
        )),
        None => Err(ProtocolError::bad_request(
            None,
            format!("{key} is required"),
        )),
    }
}

fn require_ulid(map: &Map<String, Value>, key: &str) -> Result<(), ProtocolError> {
    let s = require_string(map, key)?;
    if !is_ulid(s) {
        return Err(ProtocolError::bad_request(
            None,
            format!("{key} must be a ULID"),
        ));
    }
    Ok(())
}

fn validate_uint64_string(map: &Map<String, Value>, key: &str) -> Result<(), ProtocolError> {
    match map.get(key) {
        Some(Value::String(s)) if is_canonical_uint64(s) => Ok(()),
        _ => Err(ProtocolError::bad_request(
            None,
            format!("{key} must be a canonical uint64 decimal string"),
        )),
    }
}

/// StateSample 规范规则：revision/sampledAtMs 规范 uint64、quality 词表。
fn validate_sample(value: &Value) -> Result<(), ProtocolError> {
    validate_schema_message("conex.StateSample", value)
        .map_err(|e| ProtocolError::bad_request(None, e))?;
    let map = as_object(value, "sample")?;
    validate_uint64_string(map, "revision")?;
    validate_uint64_string(map, "sampledAtMs")?;
    if !map.contains_key("value") {
        return Err(ProtocolError::bad_request(None, "sample.value is required"));
    }
    match map.get("quality") {
        Some(Value::String(q)) if q == "current" || q == "stale" => Ok(()),
        _ => Err(ProtocolError::bad_request(
            None,
            "sample.quality must be current or stale",
        )),
    }
}

// ---- state/end Error JSON 编解码（契约 §8.3）-----------------------------

const STATE_END_ERROR_KEYS: [&str; 6] = [
    "code",
    "message",
    "diagnosticId",
    "execution",
    "retry",
    "details",
];

/// 解析 state/end 的 error：生成 Error JSON（code 为枚举名，如
/// ERROR_CODE_SLOW_CONSUMER），不是 JSON-RPC Failure 的数值 code + data 形状。
/// execution=completed and the reason-specific retry are mandatory (§8.3).
pub fn parse_state_end_error(value: &Value) -> Result<crate::Error, ProtocolError> {
    let map = as_object(value, "error")?;
    validate_schema_message("conex.Error", value)
        .map_err(|e| ProtocolError::bad_request(None, e))?;
    for key in map.keys() {
        if !STATE_END_ERROR_KEYS.contains(&key.as_str()) {
            return Err(ProtocolError::bad_request(
                None,
                format!("unknown error field {key}"),
            ));
        }
    }
    let code = match map.get("code") {
        Some(Value::String(name)) => {
            let parsed = crate::ErrorCode::from_str_name(name).ok_or_else(|| {
                ProtocolError::bad_request(None, format!("unknown error code {name}"))
            })?;
            if parsed == crate::ErrorCode::Unspecified {
                return Err(ProtocolError::bad_request(
                    None,
                    "error.code must not be ERROR_CODE_UNSPECIFIED",
                ));
            }
            parsed as i32
        }
        Some(_) => {
            return Err(ProtocolError::bad_request(
                None,
                "error.code must be a string enum name",
            ));
        }
        None => return Err(ProtocolError::bad_request(None, "error.code is required")),
    };
    let string_field = |key: &str| -> Result<String, ProtocolError> {
        match map.get(key) {
            None | Some(Value::String(_)) => Ok(map
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()),
            Some(_) => Err(ProtocolError::bad_request(
                None,
                format!("error.{key} must be a string"),
            )),
        }
    };
    let expected_retry = match crate::ErrorCode::try_from(code) {
        Ok(crate::ErrorCode::Forbidden) => "never",
        Ok(
            crate::ErrorCode::Unavailable
            | crate::ErrorCode::StaleRevision
            | crate::ErrorCode::SlowConsumer
            | crate::ErrorCode::SessionLost,
        ) => "safe",
        _ => return Err(ProtocolError::bad_request(None, "invalid state/end reason")),
    };
    if string_field("execution")? != "completed" || string_field("retry")? != expected_retry {
        return Err(ProtocolError::bad_request(
            None,
            "invalid subscription termination metadata",
        ));
    }
    let details = match map.get("details") {
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| {
            ProtocolError::bad_request(None, format!("invalid error.details: {e}"))
        })?),
        None => None,
    };
    Ok(crate::Error {
        code,
        message: string_field("message")?,
        diagnostic_id: string_field("diagnosticId")?,
        execution: string_field("execution")?,
        retry: string_field("retry")?,
        details,
    })
}

/// 生成 Error JSON：code 用枚举名（ErrorCode::as_str_name），省略默认字段。
pub fn state_end_error_json(error: &crate::Error) -> Result<Value, ProtocolError> {
    let code = crate::ErrorCode::try_from(error.code)
        .map_err(|_| ProtocolError::bad_request(None, "unknown error code"))?;
    if code == crate::ErrorCode::Unspecified {
        return Err(ProtocolError::bad_request(
            None,
            "error code must not be unspecified",
        ));
    }
    let mut out = Map::new();
    out.insert("code".into(), Value::String(code.as_str_name().into()));
    out.insert("message".into(), Value::String(error.message.clone()));
    out.insert(
        "diagnosticId".into(),
        Value::String(error.diagnostic_id.clone()),
    );
    if !error.execution.is_empty() {
        out.insert("execution".into(), Value::String(error.execution.clone()));
    }
    if !error.retry.is_empty() {
        out.insert("retry".into(), Value::String(error.retry.clone()));
    }
    if let Some(details) = &error.details {
        out.insert("details".into(), value_to_json(details)?);
    }
    let out = Value::Object(out);
    parse_state_end_error(&out)?;
    Ok(out)
}

// ---- 状态样本与 S1 通知 input 编解码 --------------------------------------

/// StateSample -> 线上 JSON（revision/sampledAtMs 为十进制字符串）。
pub fn state_sample_json(sample: &crate::StateSample) -> Result<Value, ProtocolError> {
    let mut out = Map::new();
    out.insert(
        "revision".into(),
        Value::String(sample.revision.to_string()),
    );
    out.insert(
        "value".into(),
        sample
            .value
            .as_ref()
            .map(value_to_json)
            .transpose()?
            .unwrap_or(Value::Null),
    );
    out.insert(
        "sampledAtMs".into(),
        Value::String(sample.sampled_at_ms.to_string()),
    );
    out.insert("quality".into(), Value::String(sample.quality.clone()));
    Ok(Value::Object(out))
}

/// Decode generated pbjson after enforcing canonical uint64 and full-sample rules.
pub fn state_sample_from_json(value: &Value) -> Result<crate::StateSample, ProtocolError> {
    validate_sample(value)?;
    serde_json::from_value(value.clone())
        .map_err(|e| ProtocolError::bad_request(None, format!("invalid sample: {e}")))
}

/// 构造 state/update 的 params.input（契约 §11.2），并自检同一套规范规则。
pub fn state_update_input(
    subscription_id: &str,
    seq: u64,
    sample: &crate::StateSample,
) -> Result<Value, ProtocolError> {
    let mut out = Map::new();
    out.insert(
        "subscriptionId".into(),
        Value::String(subscription_id.to_string()),
    );
    out.insert("seq".into(), Value::String(seq.to_string()));
    out.insert("sample".into(), state_sample_json(sample)?);
    let out = Value::Object(out);
    validate_input(METHOD_STATE_UPDATE, &out)?;
    Ok(out)
}

/// 构造 state/end 的 params.input，error 为生成 Error JSON（§8.3），并自检。
pub fn state_end_input(
    subscription_id: &str,
    error: &crate::Error,
) -> Result<Value, ProtocolError> {
    let mut out = Map::new();
    out.insert(
        "subscriptionId".into(),
        Value::String(subscription_id.to_string()),
    );
    out.insert("error".into(), state_end_error_json(error)?);
    let out = Value::Object(out);
    validate_input(METHOD_STATE_END, &out)?;
    Ok(out)
}

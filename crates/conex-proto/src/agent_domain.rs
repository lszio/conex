//! Agent 领域边界：方法目录、执行类别与业务 input/output 严格校验
//! （docs/contracts/agent-domain.md §1-§4、§9；docs/contracts/event-recovery.md §2-§3）。
//!
//! 结构形状复用生成器产出的单一 JSON Schema 源（经
//! [`crate::capability::validate_domain_schema`]）；本模块追加语义规则：
//! ULID 标识符、规范 uint64 十进制字符串、状态词表、UTF-8 字节长度上限、
//! decision 互斥与 cursor 上限（SQLite 首版 0..i64::MAX）。与
//! sdk/typescript/src/agent-domain.ts 实现同一规则集，由
//! conformance/vectors/agent-domain.json 共享向量仲裁一致。
//!
//! 信封级规则（ULID id、任意深度重复键）留在 [`crate::wire`]；本模块不重复
//! 顶层信封解析，也不修改 S1_METHODS 顶层清单。

use serde_json::Value;

use crate::wire::ProtocolError;

pub const AGENT_ENDPOINTS_LIST: &str = "agent/endpoints/list";
pub const AGENT_WORKSPACES_LIST: &str = "agent/workspaces/list";
pub const AGENT_CONVERSATIONS_LIST: &str = "agent/conversations/list";
pub const AGENT_CONVERSATIONS_CREATE: &str = "agent/conversations/create";
pub const AGENT_CONVERSATIONS_GET: &str = "agent/conversations/get";
pub const AGENT_CONVERSATIONS_HISTORY: &str = "agent/conversations/history";
pub const AGENT_MESSAGES_READ: &str = "agent/messages/read";
pub const AGENT_EXECUTIONS_SUBMIT: &str = "agent/executions/submit";
pub const AGENT_EXECUTIONS_GET: &str = "agent/executions/get";
pub const AGENT_EXECUTIONS_CANCEL: &str = "agent/executions/cancel";
pub const AGENT_EXECUTIONS_STEER: &str = "agent/executions/steer";
pub const AGENT_INTERACTIONS_LIST: &str = "agent/interactions/list";
pub const AGENT_INTERACTIONS_RESPOND: &str = "agent/interactions/respond";
pub const AGENT_RELATIONSHIPS_LIST: &str = "agent/relationships/list";
pub const AGENT_SUBAGENTS_LIST: &str = "agent/subagents/list";
pub const AGENT_SUBAGENTS_CANCEL: &str = "agent/subagents/cancel";
pub const EVENTS_SNAPSHOT: &str = "events/snapshot";
pub const EVENTS_READ: &str = "events/read";

/// 完整 agent 领域方法目录（契约 §4 操作表，18 个）。经现有
/// module/invoke 的 input:{method,argument} 分派，不是顶层 JSON-RPC 方法。
pub const AGENT_METHODS: [&str; 18] = [
    AGENT_ENDPOINTS_LIST,
    AGENT_WORKSPACES_LIST,
    AGENT_CONVERSATIONS_LIST,
    AGENT_CONVERSATIONS_CREATE,
    AGENT_CONVERSATIONS_GET,
    AGENT_CONVERSATIONS_HISTORY,
    AGENT_MESSAGES_READ,
    AGENT_EXECUTIONS_SUBMIT,
    AGENT_EXECUTIONS_GET,
    AGENT_EXECUTIONS_CANCEL,
    AGENT_EXECUTIONS_STEER,
    AGENT_INTERACTIONS_LIST,
    AGENT_INTERACTIONS_RESPOND,
    AGENT_RELATIONSHIPS_LIST,
    AGENT_SUBAGENTS_LIST,
    AGENT_SUBAGENTS_CANCEL,
    EVENTS_SNAPSHOT,
    EVENTS_READ,
];

pub fn is_agent_method(method: &str) -> bool {
    AGENT_METHODS.contains(&method)
}

/// 契约 §4：read_only 为列表/get/history/events，其余为 non_replayable。
/// 与 conex-core 的 MethodContract execution 词表一致。
pub fn agent_execution_class(method: &str) -> Option<&'static str> {
    match method {
        AGENT_ENDPOINTS_LIST
        | AGENT_WORKSPACES_LIST
        | AGENT_CONVERSATIONS_LIST
        | AGENT_CONVERSATIONS_GET
        | AGENT_CONVERSATIONS_HISTORY
        | AGENT_MESSAGES_READ
        | AGENT_EXECUTIONS_GET
        | AGENT_INTERACTIONS_LIST
        | AGENT_RELATIONSHIPS_LIST
        | AGENT_SUBAGENTS_LIST
        | EVENTS_SNAPSHOT
        | EVENTS_READ => Some("read_only"),
        AGENT_CONVERSATIONS_CREATE
        | AGENT_EXECUTIONS_SUBMIT
        | AGENT_EXECUTIONS_CANCEL
        | AGENT_EXECUTIONS_STEER
        | AGENT_INTERACTIONS_RESPOND
        | AGENT_SUBAGENTS_CANCEL => Some("non_replayable"),
        _ => None,
    }
}

/// 领域权限词表（契约 §1）；权限还须约束节点、模块、Workspace、Conversation。
pub fn required_permission(method: &str) -> Option<&'static str> {
    match method {
        AGENT_CONVERSATIONS_CREATE => Some("agent.create"),
        AGENT_EXECUTIONS_SUBMIT | AGENT_EXECUTIONS_STEER => Some("agent.execute"),
        AGENT_EXECUTIONS_CANCEL | AGENT_SUBAGENTS_CANCEL => Some("agent.cancel"),
        AGENT_INTERACTIONS_RESPOND => Some("agent.interact"),
        _ if is_agent_method(method) => Some("agent.read"),
        _ => None,
    }
}

fn input_message(method: &str) -> Option<&'static str> {
    match method {
        AGENT_ENDPOINTS_LIST => Some("conex.AgentEndpointListInput"),
        AGENT_WORKSPACES_LIST => Some("conex.AgentWorkspaceListInput"),
        AGENT_CONVERSATIONS_LIST => Some("conex.AgentConversationListInput"),
        AGENT_CONVERSATIONS_CREATE => Some("conex.AgentConversationCreateInput"),
        AGENT_CONVERSATIONS_GET => Some("conex.AgentConversationGetInput"),
        AGENT_CONVERSATIONS_HISTORY => Some("conex.AgentConversationHistoryInput"),
        AGENT_MESSAGES_READ => Some("conex.AgentMessageReadInput"),
        AGENT_EXECUTIONS_SUBMIT => Some("conex.AgentSubmitInput"),
        AGENT_EXECUTIONS_GET => Some("conex.AgentExecutionGetInput"),
        AGENT_EXECUTIONS_CANCEL => Some("conex.AgentExecutionCancelInput"),
        AGENT_EXECUTIONS_STEER => Some("conex.AgentExecutionSteerInput"),
        AGENT_INTERACTIONS_LIST => Some("conex.AgentInteractionListInput"),
        AGENT_INTERACTIONS_RESPOND => Some("conex.AgentInteractionRespondInput"),
        AGENT_RELATIONSHIPS_LIST => Some("conex.AgentRelationshipListInput"),
        AGENT_SUBAGENTS_LIST => Some("conex.AgentSubagentListInput"),
        AGENT_SUBAGENTS_CANCEL => Some("conex.AgentSubagentCancelInput"),
        EVENTS_SNAPSHOT => Some("conex.AgentSnapshotInput"),
        EVENTS_READ => Some("conex.AgentEventReadInput"),
        _ => None,
    }
}

fn result_message(method: &str) -> Option<&'static str> {
    match method {
        AGENT_ENDPOINTS_LIST => Some("conex.AgentEndpointListResult"),
        AGENT_WORKSPACES_LIST => Some("conex.AgentWorkspaceListResult"),
        AGENT_CONVERSATIONS_LIST => Some("conex.AgentConversationListResult"),
        AGENT_CONVERSATIONS_CREATE => Some("conex.AgentConversationCreateResult"),
        AGENT_CONVERSATIONS_GET => Some("conex.AgentConversationGetResult"),
        AGENT_CONVERSATIONS_HISTORY => Some("conex.AgentConversationHistoryResult"),
        AGENT_MESSAGES_READ => Some("conex.AgentMessageReadResult"),
        AGENT_EXECUTIONS_SUBMIT => Some("conex.AgentSubmitResult"),
        AGENT_EXECUTIONS_GET => Some("conex.AgentExecutionGetResult"),
        AGENT_EXECUTIONS_CANCEL => Some("conex.AgentExecutionCancelResult"),
        AGENT_EXECUTIONS_STEER => Some("conex.AgentExecutionSteerResult"),
        AGENT_INTERACTIONS_LIST => Some("conex.AgentInteractionListResult"),
        AGENT_INTERACTIONS_RESPOND => Some("conex.AgentInteractionRespondResult"),
        AGENT_RELATIONSHIPS_LIST => Some("conex.AgentRelationshipListResult"),
        AGENT_SUBAGENTS_LIST => Some("conex.AgentSubagentListResult"),
        AGENT_SUBAGENTS_CANCEL => Some("conex.AgentSubagentCancelResult"),
        EVENTS_SNAPSHOT => Some("conex.AgentSnapshotResult"),
        EVENTS_READ => Some("conex.AgentEventReadResult"),
        _ => None,
    }
}

const MAX_PAGE_BYTES: usize = 128;
const MAX_CONTENT_BYTES: usize = 65536;
const MAX_MESSAGE_READ_BYTES: u64 = 65536;
/// SQLite 首版只使用 0..i64::MAX 的位置（恢复契约 §2）。
const MAX_POSITION: u64 = i64::MAX as u64;

const EXECUTION_STATES: [&str; 7] = [
    "accepted",
    "dispatching",
    "running",
    "unknown",
    "succeeded",
    "failed",
    "cancelled",
];
const INTERACTION_STATES: [&str; 6] = [
    "pending",
    "responding",
    "resolved",
    "expired",
    "invalidated",
    "unknown",
];
const MESSAGE_STATES: [&str; 3] = ["streaming", "complete", "incomplete"];
const ROLES: [&str; 4] = ["user", "assistant", "tool", "system"];
const QUALITIES: [&str; 4] = ["current", "stale", "unknown", "historical"];
const SOURCES: [&str; 3] = ["backend", "config", "user"];
const DECISION_KINDS: [&str; 3] = ["approval", "selection", "input"];
const EFFECTS: [&str; 4] = ["allow_once", "deny", "select", "submit"];
const AVAILABILITY: [&str; 4] = ["ready", "unreachable", "unknown", "stopped"];
const CATALOG_SCOPES: [&str; 2] = ["node-managed", "backend"];
const ADAPTERS: [&str; 2] = ["omp", "codex"];

/// 规范 uint64 字段名：输出侧逐键校验十进制规范形（无前导零、<=u64::MAX）。
const UINT64_KEYS: [&str; 15] = [
    "generation",
    "revision",
    "createdAtMs",
    "updatedAtMs",
    "cancelRequestedAtMs",
    "expiresAtMs",
    "expectedRevision",
    "after",
    "position",
    "head",
    "firstAvailable",
    "offsetBytes",
    "nextOffsetBytes",
    "totalBytes",
    "recordedAtMs",
];

fn as_object<'v>(
    value: &'v Value,
    what: &str,
) -> Result<&'v serde_json::Map<String, Value>, ProtocolError> {
    value
        .as_object()
        .ok_or_else(|| ProtocolError::bad_request(None, format!("{what} must be a JSON object")))
}

fn get<'v>(map: &'v serde_json::Map<String, Value>, key: &str) -> Option<&'v Value> {
    map.get(key).filter(|v| !v.is_null())
}

fn require_ulid(map: &serde_json::Map<String, Value>, key: &str) -> Result<(), ProtocolError> {
    let value = get(map, key)
        .ok_or_else(|| ProtocolError::bad_request(None, format!("{key} is required")))?;
    let s = value
        .as_str()
        .ok_or_else(|| ProtocolError::bad_request(None, format!("{key} must be a string")))?;
    if crate::wire::is_ulid(s) {
        Ok(())
    } else {
        Err(ProtocolError::bad_request(
            None,
            format!("{key} must be a ULID"),
        ))
    }
}

fn require_bounded(
    map: &serde_json::Map<String, Value>,
    key: &str,
    max_bytes: usize,
) -> Result<(), ProtocolError> {
    let value = get(map, key)
        .ok_or_else(|| ProtocolError::bad_request(None, format!("{key} is required")))?;
    let s = value
        .as_str()
        .ok_or_else(|| ProtocolError::bad_request(None, format!("{key} must be a string")))?;
    let len = s.len();
    if s.is_empty() {
        Err(ProtocolError::bad_request(
            None,
            format!("{key} must not be empty"),
        ))
    } else if len > max_bytes {
        Err(ProtocolError::bad_request(
            None,
            format!("{key} exceeds {max_bytes} UTF-8 bytes"),
        ))
    } else {
        Ok(())
    }
}

fn optional_bounded(
    map: &serde_json::Map<String, Value>,
    key: &str,
    max_bytes: usize,
) -> Result<(), ProtocolError> {
    if get(map, key).is_some() {
        require_bounded(map, key, max_bytes)
    } else {
        Ok(())
    }
}

/// 可选页参数：limit 默认 50、范围 1–100；pageToken 上限 128 UTF-8 字节。
fn validate_page(map: &serde_json::Map<String, Value>) -> Result<(), ProtocolError> {
    if let Some(limit) = get(map, "limit") {
        let n = limit.as_u64().ok_or_else(|| {
            ProtocolError::bad_request(None, "limit must be a non-negative integer")
        })?;
        if !(1..=100).contains(&n) {
            return Err(ProtocolError::bad_request(
                None,
                "limit must be within 1..=100",
            ));
        }
    }
    optional_bounded(map, "pageToken", MAX_PAGE_BYTES)
}

fn validate_decision(decision: &Value) -> Result<(), ProtocolError> {
    let map = as_object(decision, "decision")?;
    let kind = get(map, "kind")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::bad_request(None, "decision.kind is required"))?;
    if !DECISION_KINDS.contains(&kind) {
        return Err(ProtocolError::bad_request(
            None,
            "decision.kind must be approval/selection/input",
        ));
    }
    let choice_id = get(map, "choiceId");
    let choice_ids = get(map, "choiceIds");
    let text = get(map, "text");
    match kind {
        "approval" => {
            if choice_id.is_none() || choice_ids.is_some() || text.is_some() {
                return Err(ProtocolError::bad_request(
                    None,
                    "approval decision requires exactly choiceId",
                ));
            }
            require_bounded(map, "choiceId", MAX_PAGE_BYTES)?;
        }
        "selection" => {
            if choice_ids.is_none() || choice_id.is_some() || text.is_some() {
                return Err(ProtocolError::bad_request(
                    None,
                    "selection decision requires exactly choiceIds",
                ));
            }
            let ids = choice_ids
                .and_then(Value::as_array)
                .ok_or_else(|| ProtocolError::bad_request(None, "choiceIds must be an array"))?;
            if ids.is_empty() {
                return Err(ProtocolError::bad_request(
                    None,
                    "selection decision requires at least one choiceId",
                ));
            }
            for id in ids {
                if id.as_str().is_none() || id.as_str().is_some_and(str::is_empty) {
                    return Err(ProtocolError::bad_request(
                        None,
                        "choiceIds entries must be non-empty strings",
                    ));
                }
            }
        }
        "input" => {
            if text.is_none() || choice_id.is_some() || choice_ids.is_some() {
                return Err(ProtocolError::bad_request(
                    None,
                    "input decision requires exactly text",
                ));
            }
            require_bounded(map, "text", MAX_CONTENT_BYTES)?;
        }
        _ => unreachable!("kind checked above"),
    }
    Ok(())
}

fn validate_cursor(cursor: &Value) -> Result<(), ProtocolError> {
    let map = as_object(cursor, "cursor")?;
    require_ulid(map, "streamId")?;
    require_ulid(map, "epoch")?;
    let after = get(map, "after")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::bad_request(None, "cursor.after is required"))?;
    if !crate::capability::is_canonical_uint64(after) {
        return Err(ProtocolError::bad_request(
            None,
            "cursor.after must be a canonical uint64 string",
        ));
    }
    let parsed: u64 = after.parse().map_err(|_| {
        ProtocolError::bad_request(None, "cursor.after must be a canonical uint64 string")
    })?;
    if parsed > MAX_POSITION {
        return Err(ProtocolError::bad_request(
            None,
            "cursor.after exceeds the persistable position range",
        ));
    }
    Ok(())
}

/// Agent 领域 input 严格校验：生成 Schema 形状 + 契约语义规则。
pub fn validate_agent_input(method: &str, argument: &Value) -> Result<(), ProtocolError> {
    let message = input_message(method).ok_or_else(|| {
        ProtocolError::bad_request(None, format!("{method} is not an agent method"))
    })?;
    crate::capability::validate_domain_schema(message, argument)?;
    let map = as_object(argument, "argument")?;
    match method {
        AGENT_ENDPOINTS_LIST => validate_page(map)?,
        AGENT_WORKSPACES_LIST | AGENT_CONVERSATIONS_LIST => {
            require_ulid(map, "endpointId")?;
            if let Some(workspace) = get(map, "workspaceId") {
                let _ = workspace;
                require_ulid(map, "workspaceId")?;
            }
            validate_page(map)?;
        }
        AGENT_CONVERSATIONS_CREATE => {
            require_ulid(map, "endpointId")?;
            require_ulid(map, "workspaceId")?;
            require_bounded(map, "idempotencyKey", MAX_PAGE_BYTES)?;
            optional_bounded(map, "title", MAX_PAGE_BYTES)?;
        }
        AGENT_CONVERSATIONS_GET | EVENTS_SNAPSHOT => {
            require_ulid(map, "conversationId")?;
        }
        AGENT_CONVERSATIONS_HISTORY => {
            require_ulid(map, "conversationId")?;
            if get(map, "beforeMessageId").is_some() {
                require_ulid(map, "beforeMessageId")?;
            }
            validate_page(map)?;
        }
        AGENT_MESSAGES_READ => {
            require_ulid(map, "conversationId")?;
            require_ulid(map, "messageId")?;
            let offset = get(map, "offsetBytes")
                .and_then(Value::as_str)
                .ok_or_else(|| ProtocolError::bad_request(None, "offsetBytes is required"))?;
            if !crate::capability::is_canonical_uint64(offset) {
                return Err(ProtocolError::bad_request(
                    None,
                    "offsetBytes must be a canonical uint64 string",
                ));
            }
            if let Some(max) = get(map, "maxBytes") {
                let n = max.as_u64().ok_or_else(|| {
                    ProtocolError::bad_request(None, "maxBytes must be a non-negative integer")
                })?;
                if n == 0 || n > MAX_MESSAGE_READ_BYTES {
                    return Err(ProtocolError::bad_request(
                        None,
                        "maxBytes must be within 1..=65536",
                    ));
                }
            }
        }
        AGENT_EXECUTIONS_SUBMIT | AGENT_EXECUTIONS_STEER => {
            if method == AGENT_EXECUTIONS_SUBMIT {
                require_ulid(map, "conversationId")?;
            } else {
                require_ulid(map, "executionId")?;
                let generation = get(map, "expectedGeneration")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ProtocolError::bad_request(None, "expectedGeneration is required")
                    })?;
                if !crate::capability::is_canonical_uint64(generation) {
                    return Err(ProtocolError::bad_request(
                        None,
                        "expectedGeneration must be a canonical uint64 string",
                    ));
                }
            }
            require_bounded(map, "idempotencyKey", MAX_PAGE_BYTES)?;
            require_bounded(map, "content", MAX_CONTENT_BYTES)?;
        }
        AGENT_EXECUTIONS_GET => require_ulid(map, "executionId")?,
        AGENT_EXECUTIONS_CANCEL | AGENT_SUBAGENTS_CANCEL => {
            require_ulid(map, "executionId")?;
            let generation = get(map, "expectedGeneration")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    ProtocolError::bad_request(None, "expectedGeneration is required")
                })?;
            if !crate::capability::is_canonical_uint64(generation) {
                return Err(ProtocolError::bad_request(
                    None,
                    "expectedGeneration must be a canonical uint64 string",
                ));
            }
        }
        AGENT_INTERACTIONS_LIST => {
            require_ulid(map, "conversationId")?;
            if get(map, "executionId").is_some() {
                require_ulid(map, "executionId")?;
            }
            validate_page(map)?;
        }
        AGENT_INTERACTIONS_RESPOND => {
            require_ulid(map, "interactionId")?;
            let revision = get(map, "expectedRevision")
                .and_then(Value::as_str)
                .ok_or_else(|| ProtocolError::bad_request(None, "expectedRevision is required"))?;
            if !crate::capability::is_canonical_uint64(revision) {
                return Err(ProtocolError::bad_request(
                    None,
                    "expectedRevision must be a canonical uint64 string",
                ));
            }
            let decision = get(map, "decision")
                .ok_or_else(|| ProtocolError::bad_request(None, "decision is required"))?;
            validate_decision(decision)?;
        }
        AGENT_RELATIONSHIPS_LIST => {
            require_ulid(map, "conversationId")?;
            validate_page(map)?;
        }
        AGENT_SUBAGENTS_LIST => {
            require_ulid(map, "executionId")?;
            validate_page(map)?;
        }
        EVENTS_READ => {
            let cursor = get(map, "cursor")
                .ok_or_else(|| ProtocolError::bad_request(None, "cursor is required"))?;
            validate_cursor(cursor)?;
            if let Some(limit) = get(map, "limit") {
                let n = limit.as_u64().ok_or_else(|| {
                    ProtocolError::bad_request(None, "limit must be a non-negative integer")
                })?;
                if !(1..=32).contains(&n) {
                    return Err(ProtocolError::bad_request(
                        None,
                        "events/read limit must be within 1..=32",
                    ));
                }
            }
        }
        _ => {
            return Err(ProtocolError::bad_request(
                None,
                format!("{method} is not an agent method"),
            ));
        }
    }
    Ok(())
}

/// 词表与规范 uint64 逐键检查；形状已由生成 Schema 保证，这里只追加语义。
fn check_semantics(value: &Value) -> Result<(), ProtocolError> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if UINT64_KEYS.contains(&key.as_str()) {
                    if let Some(s) = child.as_str() {
                        if !crate::capability::is_canonical_uint64(s) {
                            return Err(ProtocolError::bad_request(
                                None,
                                format!("{key} must be a canonical uint64 string"),
                            ));
                        }
                    } else if !child.is_null() {
                        return Err(ProtocolError::bad_request(
                            None,
                            format!("{key} must be a canonical uint64 string"),
                        ));
                    }
                }
                let vocab: &[&[&str]] = match key.as_str() {
                    "state" => &[&EXECUTION_STATES, &INTERACTION_STATES, &MESSAGE_STATES],
                    "kind" => &[&DECISION_KINDS],
                    "effect" => &[&EFFECTS],
                    "role" => &[&ROLES],
                    "quality" => &[&QUALITIES],
                    "source" => &[&SOURCES],
                    "availability" => &[&AVAILABILITY],
                    "catalogScope" => &[&CATALOG_SCOPES],
                    "adapter" => &[&ADAPTERS],
                    _ => &[],
                };
                if !vocab.is_empty()
                    && let Some(s) = child.as_str()
                    && !vocab.iter().any(|l| l.contains(&s))
                {
                    return Err(ProtocolError::bad_request(
                        None,
                        format!("{key} value {s:?} is outside the contract vocabulary"),
                    ));
                }
                check_semantics(child)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                check_semantics(item)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Agent 领域 output 严格校验：生成 Schema 形状 + 规范 uint64 与状态词表。
pub fn validate_agent_output(method: &str, result: &Value) -> Result<(), ProtocolError> {
    let message = result_message(method).ok_or_else(|| {
        ProtocolError::bad_request(None, format!("{method} is not an agent method"))
    })?;
    crate::capability::validate_domain_schema(message, result)?;
    check_semantics(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const VECTORS: &str = include_str!("../../../conformance/vectors/agent-domain.json");

    /// 向量值支持 {"repeat":{"char":c,"count":n}} 与可选 "append" 后缀；
    /// 展开按字符重复，UTF-8 字节长度由验证器按编码后长度测量。
    fn expand(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                if let Some(repeat) = map.get("repeat").and_then(|r| r.as_object()) {
                    let ch = repeat
                        .get("char")
                        .and_then(Value::as_str)
                        .expect("repeat.char");
                    let count = repeat
                        .get("count")
                        .and_then(Value::as_u64)
                        .expect("repeat.count") as usize;
                    let append = map.get("append").and_then(Value::as_str).unwrap_or("");
                    let mut out = ch.repeat(count);
                    out.push_str(append);
                    return Value::String(out);
                }
                Value::Object(map.iter().map(|(k, v)| (k.clone(), expand(v))).collect())
            }
            Value::Array(items) => Value::Array(items.iter().map(expand).collect()),
            other => other.clone(),
        }
    }

    #[test]
    fn shared_vectors_accept_and_reject() {
        let vectors: Value = serde_json::from_str(VECTORS).expect("vectors are JSON");
        for group in ["accept", "reject"] {
            for case in vectors[group].as_array().expect("cases") {
                let method = case["method"].as_str().expect("method");
                let value = expand(&case["value"]);
                let result = match case["direction"].as_str().expect("direction") {
                    "input" => validate_agent_input(method, &value).map(|_| ()),
                    "output" => validate_agent_output(method, &value).map(|_| ()),
                    other => panic!("unknown direction {other}"),
                };
                if group == "accept" {
                    assert!(result.is_ok(), "{method} accept case failed: {result:?}");
                } else {
                    let name = case["name"].as_str().expect("name");
                    assert!(result.is_err(), "{name} should be rejected");
                }
            }
        }
    }

    #[test]
    fn raw_duplicate_key_is_rejected_before_mapping() {
        let vectors: Value = serde_json::from_str(VECTORS).expect("vectors are JSON");
        for case in vectors["rawReject"].as_array().expect("cases") {
            let raw = case["rawJson"].as_str().expect("rawJson");
            // 原始信封重复键由 wire decoder 拒绝；此处直接经 StrictValue 解析路径。
            let error = serde_json::from_str::<crate::wire::StrictValue>(raw)
                .err()
                .expect("duplicate key must fail");
            assert!(error.to_string().contains("duplicate"), "{error}");
        }
    }

    #[test]
    fn execution_classes_partition_the_catalogue() {
        let mut read_only = 0;
        let mut non_replayable = 0;
        for method in AGENT_METHODS {
            match agent_execution_class(method) {
                Some("read_only") => read_only += 1,
                Some("non_replayable") => non_replayable += 1,
                other => panic!("{method} has no execution class: {other:?}"),
            }
            assert!(required_permission(method).is_some());
        }
        assert_eq!(read_only + non_replayable, AGENT_METHODS.len());
        assert_eq!(non_replayable, 6);
    }
}

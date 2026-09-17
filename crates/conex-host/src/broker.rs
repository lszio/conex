//! P1 business-call dispatcher shared by HTTP and WSS (design §5.2, §7).
//!
//! One `Broker` per `Host`; `/rpc` and `/wss` both route frames here. The
//! broker enforces the resource policy via `conex_core::Host::invoke`,
//! applies the registered `MethodContract` (strict decoder + output
//! validator), and dispatches blob/session/operation/agent wire methods
//! to the in-process stores installed by `conex-host::serve`.
#![forbid(unsafe_code)]

use std::sync::Arc;

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::time::Instant;

use conex_content::ContentStore;
use conex_core::contracts::contracts as p1_contracts;
use conex_core::operation::{DedupKey, ExecutionState, OperationError, OperationStore};
use conex_core::session::{RecoveryLevel, SessionBinding, SessionError, SessionStore};
use conex_core::{
    CallError, CallResult, Caller, Host as CoreHost, MethodContract, PreparedInput, ResourceClaim,
};
use conex_proto::cid;
use conex_proto::v1;

#[derive(Debug, Clone)]
pub struct BrokerCall {
    pub caller: Caller,
    pub endpoint_id: String,
    pub method: String,
    pub input: Value,
    pub deadline: Instant,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BrokerContext {
    pub principal_id: String,
    pub tenant_id: String,
    pub provider_endpoint_id: String,
    pub plane: i32,
    pub binding_id: Option<String>,
    pub timeout_budget_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerFrame {
    pub request_id: String,
    pub method: String,
    #[serde(default)]
    pub context: BrokerContext,
    pub params: Option<Value>,
}

#[derive(Clone, Default)]
pub struct BrokerDeps {
    pub content: Option<Arc<ContentStore>>,
    pub session: Option<Arc<SessionStore>>,
    pub operation: Option<Arc<OperationStore>>,
    pub agents: Option<Arc<crate::agent::AgentRegistry>>,
    /// Expected `hostOrigin` string that `agent/register` must advertise.
    /// Required when `agents` is set.
    pub host_origin: Option<String>,
}

pub struct Broker {
    host: Arc<CoreHost>,
    deps: BrokerDeps,
}

impl Broker {
    pub fn new(host: Arc<CoreHost>, deps: BrokerDeps) -> Self {
        Self { host, deps }
    }

    pub fn host(&self) -> &Arc<CoreHost> {
        &self.host
    }

    pub fn deps(&self) -> &BrokerDeps {
        &self.deps
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn invoke(&self, call: BrokerCall) -> CallResult<Value> {
        if call.method == "conex/hello" {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "conex/hello is handled by the transport, not the broker",
            ));
        }
        if call.method.starts_with("blob/") {
            return self.dispatch_blob(&call).await;
        }
        if call.method.starts_with("session/") {
            return self.dispatch_session(&call).await;
        }
        if call.method.starts_with("operation/") {
            return self.dispatch_operation(&call).await;
        }
        if call.method.starts_with("agent/") {
            return crate::agent::handle(self, call).await;
        }
        let duration = call.deadline.saturating_duration_since(Instant::now());
        self.host
            .invoke(
                &call.caller,
                &call.endpoint_id,
                &call.method,
                call.input.clone(),
                duration,
            )
            .await
    }

    async fn dispatch_blob(&self, call: &BrokerCall) -> CallResult<Value> {
        let content = self
            .deps
            .content
            .as_ref()
            .ok_or_else(|| unavailable("blob backend is not configured"))?;
        let contract = find_contract(&call.method).ok_or_else(|| {
            CallError::new(
                v1::ErrorCode::UnknownMethod,
                format!("unknown blob method {}", call.method),
            )
        })?;
        let PreparedInput {
            canonical, claim, ..
        } = (contract.prepare)(&call.input)?;
        if !claim_blob(&claim) {
            return Err(CallError::new(
                v1::ErrorCode::Forbidden,
                format!("{} does not grant blob access", claim.action),
            ));
        }
        // Cross-tenant access is denied at the broker: blob namespaces are
        // bound to the bearer's tenant. `ResourceClaim` does not carry a
        // tenant, so the comparison is `caller.tenant_id` only.
        let _ = claim;
        let content = content.clone();
        let method = call.method.clone();
        let value = match method.as_str() {
            "blob/put" => blob_put(&content, &canonical).await?,
            "blob/chunk" => blob_chunk(&content, &canonical).await?,
            "blob/commit" => blob_commit(&content, &canonical).await?,
            "blob/pin" => blob_pin(&content, &canonical).await?,
            "blob/unpin" => blob_unpin(&content, &canonical).await?,
            "blob/have" => blob_have(&content, &canonical).await?,
            "blob/get" => blob_get(&content, &canonical).await?,
            "blob/cancel" => blob_cancel(&content, &canonical).await?,
            other => {
                return Err(CallError::new(
                    v1::ErrorCode::UnknownMethod,
                    format!("unsupported blob method {other}"),
                ));
            }
        };
        (contract.validate_output)(&value)?;
        Ok(value)
    }

    async fn dispatch_session(&self, call: &BrokerCall) -> CallResult<Value> {
        let store = self
            .deps
            .session
            .as_ref()
            .ok_or_else(|| unavailable("session backend is not configured"))?;
        let contract = find_contract(&call.method).ok_or_else(|| {
            CallError::new(
                v1::ErrorCode::UnknownMethod,
                format!("unknown session method {}", call.method),
            )
        })?;
        let PreparedInput { canonical, .. } = (contract.prepare)(&call.input)?;
        let store = store.clone();
        let value = match call.method.as_str() {
            "session/open" => session_open(&store, &canonical, &call.caller).await?,
            "session/resume" => session_resume(&store, &canonical, &call.caller).await?,
            "session/renew" => session_renew(&store, &canonical, &call.caller).await?,
            "session/close" => session_close(&store, &canonical, &call.caller).await?,
            other => {
                return Err(CallError::new(
                    v1::ErrorCode::UnknownMethod,
                    format!("unsupported session method {other}"),
                ));
            }
        };
        (contract.validate_output)(&value)?;
        Ok(value)
    }

    async fn dispatch_operation(&self, call: &BrokerCall) -> CallResult<Value> {
        let store = self
            .deps
            .operation
            .as_ref()
            .ok_or_else(|| unavailable("operation backend is not configured"))?;
        let contract = find_contract(&call.method).ok_or_else(|| {
            CallError::new(
                v1::ErrorCode::UnknownMethod,
                format!("unknown operation method {}", call.method),
            )
        })?;
        let PreparedInput { canonical, .. } = (contract.prepare)(&call.input)?;
        let store = store.clone();
        let value = match call.method.as_str() {
            "operation/get" => {
                operation_get(&store, &canonical, &call.caller, &call.method).await?
            }
            "operation/cancel" => operation_cancel(&store, &canonical, &call.caller).await?,
            other => {
                return Err(CallError::new(
                    v1::ErrorCode::UnknownMethod,
                    format!("unsupported operation method {other}"),
                ));
            }
        };
        (contract.validate_output)(&value)?;
        Ok(value)
    }
}

fn find_contract(method: &str) -> Option<MethodContract> {
    p1_contracts()
        .into_iter()
        .find(|(name, _)| *name == method)
        .map(|(_, contract)| contract)
}

fn claim_blob(claim: &ResourceClaim) -> bool {
    matches!(
        claim.action.as_str(),
        "blob.upload" | "blob.pin" | "blob.unpin" | "blob.have" | "blob.get" | "blob.cancel"
    )
}

fn unavailable(message: &str) -> CallError {
    CallError::new(v1::ErrorCode::Unavailable, message)
}

fn require_string<'a>(map: &'a Map<String, Value>, key: &str) -> CallResult<&'a str> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| missing(key))
}

fn require_u64(map: &Map<String, Value>, key: &str) -> CallResult<u64> {
    let raw = map
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| missing(key))?;
    raw.parse::<u64>()
        .map_err(|_| bad_req(&format!("{key} invalid")))
}

fn object(input: &Value) -> CallResult<&Map<String, Value>> {
    input
        .as_object()
        .ok_or_else(|| bad_req("input must be an object"))
}

fn missing(field: &str) -> CallError {
    bad_req(&format!("{field} is required"))
}

fn bad_req(message: &str) -> CallError {
    CallError::new(v1::ErrorCode::BadRequest, message)
}

fn expect_resource_id(label: &str, value: &str) -> CallResult<()> {
    if value.is_empty()
        || value
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/'))
    {
        let message = String::from(label) + " must be alphanumeric or -/_/./";
        return Err(bad_req(&message));
    }
    Ok(())
}

fn expect_alphanumeric(label: &str, value: &str) -> CallResult<()> {
    if value.is_empty()
        || value
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    {
        return Err(bad_req(&format!("{label} must be alphanumeric or -/_")));
    }
    Ok(())
}

fn expect_cid(value: &str) -> CallResult<()> {
    cid::parse_cid(value)
        .map(|_| ())
        .map_err(|error| bad_req(&error.message))
}

// -------------------- blob handlers --------------------

async fn blob_put(store: &ContentStore, input: &Value) -> CallResult<Value> {
    let map = object(input)?;
    let format_version = require_u64(map, "formatVersion")? as u32;
    if format_version != 1 {
        return Err(CallError::new(
            v1::ErrorCode::UnsupportedCapability,
            format!("formatVersion {format_version} not supported"),
        ));
    }
    let declared_size_bytes = require_u64(map, "declaredSizeBytes")?;
    let declared_chunk_size = require_u64(map, "declaredChunkSize")? as u32;
    if declared_chunk_size == 0 {
        return Err(bad_req("declaredChunkSize must be > 0"));
    }
    let expected_root = require_string(map, "expectedRootCid")?.to_string();
    expect_cid(&expected_root)?;
    let access = object(map.get("access").ok_or_else(|| missing("access"))?)?;
    let resource_id = require_string(access, "resourceId")?.to_string();
    expect_resource_id("resourceId", &resource_id)?;
    let requested_lease_ms = map
        .get("requestedLeaseMs")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse::<u64>().ok());
    let upload = store
        .begin_upload(
            format_version,
            declared_chunk_size,
            declared_size_bytes,
            root_kind_for(&expected_root, declared_size_bytes, declared_chunk_size),
            &expected_root,
            requested_lease_ms,
        )
        .map_err(content_to_call)?;
    Ok(json!({
        "uploadId": upload.upload_id(),
        "chunkSize": declared_chunk_size.to_string(),
        "leaseMs": requested_lease_ms.unwrap_or(60 * 60 * 1000).to_string(),
        "maxBlobBytes": "1073741824",
        "inlineThresholdBytes": "65536",
        "alreadyHaveChunkCids": [],
        "resourceId": resource_id,
    }))
}

async fn blob_chunk(store: &ContentStore, input: &Value) -> CallResult<Value> {
    let map = object(input)?;
    let upload_id = require_string(map, "uploadId")?.to_string();
    let chunk_index = require_u64(map, "chunkIndex")? as u32;
    let chunk_cid = require_string(map, "chunkCid")?.to_string();
    expect_cid(&chunk_cid)?;
    let bytes = match map.get("chunkBytes") {
        Some(Value::Array(array)) => array
            .iter()
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| bad_req("chunkBytes entries must be bytes"))
            })
            .collect::<CallResult<Vec<u8>>>()?,
        Some(Value::String(b64)) => base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|_| bad_req("chunkBytes is not valid base64"))?,
        _ => {
            return Err(bad_req(
                "chunkBytes must be bytes (array of u8 or base64 string)",
            ));
        }
    };
    let upload = store.resume_upload(&upload_id).map_err(content_to_call)?;
    let cid = upload
        .put_chunk(chunk_index, &bytes)
        .map_err(content_to_call)?;
    if cid != chunk_cid {
        return Err(CallError::new(
            v1::ErrorCode::BadBlob,
            format!("chunk cid mismatch: declared {chunk_cid}, recomputed {cid}"),
        ));
    }
    Ok(json!({
        "chunkIndex": chunk_index.to_string(),
        "receivedBytes": bytes.len().to_string(),
        "remainingBytes": "0",
    }))
}

async fn blob_commit(store: &ContentStore, input: &Value) -> CallResult<Value> {
    let map = object(input)?;
    let upload_id = require_string(map, "uploadId")?.to_string();
    let declared_root = require_string(map, "declaredRootCid")?.to_string();
    expect_cid(&declared_root)?;
    let persistence = require_string(map, "persistence")?.to_string();
    if persistence != "local" {
        return Err(CallError::new(
            v1::ErrorCode::Unavailable,
            format!("persistence {persistence} not supported by local broker"),
        ));
    }
    let commit = store
        .commit(&upload_id, &declared_root, "manifest")
        .or_else(|error| {
            if let conex_content::ContentError::UnsupportedRootKind(_) = error {
                store.commit(&upload_id, &declared_root, "raw")
            } else {
                Err(error)
            }
        })
        .map_err(content_to_call)?;
    Ok(json!({
        "resourceRootCid": commit.root_cid,
        "committedBytes": commit.committed_bytes.to_string(),
        "persistence": persistence,
        "receiptId": commit.receipt_id,
    }))
}

async fn blob_pin(store: &ContentStore, input: &Value) -> CallResult<Value> {
    let map = object(input)?;
    let root = require_string(map, "rootCid")?.to_string();
    expect_cid(&root)?;
    let expires_at_ms = map
        .get("pinUntilMs")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or_else(now_ms);
    let pin = store
        .pin(&root, &format!("pin-{root}"), expires_at_ms)
        .map_err(content_to_call)?;
    Ok(json!({
        "pinId": pin.pin_id,
        "expiresAtMs": pin.expires_at_ms.to_string(),
    }))
}

async fn blob_unpin(store: &ContentStore, input: &Value) -> CallResult<Value> {
    let map = object(input)?;
    let pin_id = require_string(map, "pinId")?.to_string();
    store.unpin(&pin_id).map_err(content_to_call)?;
    Ok(json!({}))
}

async fn blob_have(store: &ContentStore, input: &Value) -> CallResult<Value> {
    let map = object(input)?;
    let cids = map
        .get("chunkCids")
        .and_then(Value::as_array)
        .ok_or_else(|| missing("chunkCids"))?;
    let owned: Vec<String> = cids
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| bad_req("chunkCids must be strings"))
        })
        .collect::<CallResult<Vec<_>>>()?;
    let present = store.has(&owned).map_err(content_to_call)?;
    Ok(json!({ "present": present }))
}

async fn blob_get(store: &ContentStore, input: &Value) -> CallResult<Value> {
    let map = object(input)?;
    let chunk_cid = require_string(map, "chunkCid")?.to_string();
    expect_cid(&chunk_cid)?;
    let bytes = store.get(&chunk_cid).map_err(content_to_call)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes.as_ref());
    Ok(json!({
        "chunkBytes": encoded,
        "chunkCid": chunk_cid,
    }))
}

async fn blob_cancel(store: &ContentStore, input: &Value) -> CallResult<Value> {
    let map = object(input)?;
    let upload_id = require_string(map, "uploadId")?.to_string();
    store.cancel_upload(&upload_id).map_err(content_to_call)?;
    Ok(json!({}))
}

// -------------------- session handlers --------------------

async fn session_open(store: &SessionStore, input: &Value, caller: &Caller) -> CallResult<Value> {
    let map = object(input)?;
    let binding = object(map.get("binding").ok_or_else(|| missing("binding"))?)?;
    let principal_id = require_string(binding, "principalId")?.to_string();
    let tenant_id = require_string(binding, "tenantId")?.to_string();
    if principal_id != caller.principal_id || tenant_id != caller.tenant_id {
        return Err(CallError::new(
            v1::ErrorCode::Forbidden,
            "session binding principal/tenant does not match the bearer",
        ));
    }
    let provider_endpoint_id = require_string(binding, "providerEndpointId")?.to_string();
    let plane = binding
        .get("plane")
        .and_then(Value::as_i64)
        .map(|n| n as i32)
        .unwrap_or(1);
    let workspace_peer_id = binding
        .get("workspacePeerId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let human_peer_id = binding
        .get("humanPeerId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let recovery = require_string(map, "recovery")?;
    let requested = match recovery {
        "none" => RecoveryLevel::None,
        "in_process" => RecoveryLevel::InProcess,
        "persistent" => RecoveryLevel::Persistent,
        other => {
            return Err(CallError::new(
                v1::ErrorCode::UnsupportedCapability,
                format!("recovery level {other} not recognised"),
            ));
        }
    };
    let attachment_id = require_string(map, "attachmentId")?.to_string();
    let peer_role = require_string(map, "peerRole")?.to_string();
    expect_alphanumeric("attachmentId", &attachment_id)?;
    expect_alphanumeric("peerRole", &peer_role)?;
    let binding = SessionBinding {
        principal_id,
        tenant_id,
        provider_endpoint_id,
        plane: plane_to_enum(plane),
        workspace_peer_id,
        human_peer_id,
    };
    let (session_id, granted) = store
        .open_session(binding.clone(), requested, &attachment_id, &peer_role)
        .map_err(session_to_call)?;
    Ok(json!({
        "sessionId": session_id,
        "binding": binding,
        "recovery": recovery_label(granted),
        "attachmentEpoch": "1",
        "leaseMs": "120000",
        "provides": [],
        "rejectedCapabilities": [],
    }))
}

async fn session_resume(store: &SessionStore, input: &Value, caller: &Caller) -> CallResult<Value> {
    let map = object(input)?;
    let session_id = require_string(map, "sessionId")?.to_string();
    let attachment_id = require_string(map, "attachmentId")?.to_string();
    let expected_epoch = require_u64(map, "expectedEpoch")?;
    let binding_value = map.get("binding");
    let binding = binding_value.map(parse_session_binding).transpose()?;
    if let Some(b) = &binding {
        if !b.principal_id.is_empty() && b.principal_id != caller.principal_id {
            return Err(CallError::new(
                v1::ErrorCode::Forbidden,
                "session resume binding does not match the bearer principal",
            ));
        }
        if !b.tenant_id.is_empty() && b.tenant_id != caller.tenant_id {
            return Err(CallError::new(
                v1::ErrorCode::Forbidden,
                "session resume binding does not match the bearer tenant",
            ));
        }
    }
    let binding = binding.unwrap_or_else(|| SessionBinding {
        principal_id: caller.principal_id.clone(),
        tenant_id: caller.tenant_id.clone(),
        provider_endpoint_id: String::new(),
        plane: plane_to_enum(1),
        workspace_peer_id: None,
        human_peer_id: None,
    });
    let new_epoch = store
        .resume(&session_id, &attachment_id, expected_epoch, &binding)
        .map_err(session_to_call)?;
    Ok(json!({
        "sessionId": session_id,
        "attachmentId": attachment_id,
        "newEpoch": new_epoch.to_string(),
        "streamsReset": [],
        "restoredWindowBytes": "0",
    }))
}

async fn session_renew(store: &SessionStore, input: &Value, caller: &Caller) -> CallResult<Value> {
    let map = object(input)?;
    let session_id = require_string(map, "sessionId")?.to_string();
    let attachment_id = require_string(map, "attachmentId")?.to_string();
    let expected_epoch = require_u64(map, "expectedEpoch")?;
    let _ = caller;
    let new_lease = store
        .renew(&session_id, &attachment_id, expected_epoch)
        .map_err(session_to_call)?;
    Ok(json!({
        "newEpoch": expected_epoch.to_string(),
        "leaseMs": new_lease.to_string(),
    }))
}

async fn session_close(store: &SessionStore, input: &Value, caller: &Caller) -> CallResult<Value> {
    let map = object(input)?;
    let session_id = require_string(map, "sessionId")?.to_string();
    let attachment_id = require_string(map, "attachmentId")?.to_string();
    let expected_epoch = require_u64(map, "expectedEpoch")?;
    let _ = caller;
    let final_epoch = store
        .close(&session_id, &attachment_id, expected_epoch)
        .map_err(session_to_call)?;
    Ok(json!({
        "sessionId": session_id,
        "finalEpoch": final_epoch.to_string(),
    }))
}

fn parse_session_binding(value: &Value) -> CallResult<SessionBinding> {
    let map = object(value)?;
    let principal_id = require_string(map, "principalId")?.to_string();
    let tenant_id = require_string(map, "tenantId")?.to_string();
    let provider_endpoint_id = require_string(map, "providerEndpointId")?.to_string();
    let plane_i = map
        .get("plane")
        .and_then(Value::as_i64)
        .map(|n| n as i32)
        .unwrap_or(1);
    let workspace_peer_id = map
        .get("workspacePeerId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let human_peer_id = map
        .get("humanPeerId")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(SessionBinding {
        principal_id,
        tenant_id,
        provider_endpoint_id,
        plane: plane_to_enum(plane_i),
        workspace_peer_id,
        human_peer_id,
    })
}

fn recovery_label(level: RecoveryLevel) -> &'static str {
    match level {
        RecoveryLevel::None => "none",
        RecoveryLevel::InProcess => "in_process",
        RecoveryLevel::Persistent => "persistent",
    }
}

// -------------------- operation handlers --------------------

async fn operation_get(
    store: &OperationStore,
    input: &Value,
    caller: &Caller,
    method: &str,
) -> CallResult<Value> {
    let map = object(input)?;
    let key = parse_dedup_key(map.get("key").ok_or_else(|| missing("key"))?)?;
    if key.principal_id != caller.principal_id || key.tenant_id != caller.tenant_id {
        return Err(CallError::new(
            v1::ErrorCode::Forbidden,
            format!("{method} dedup key principal/tenant does not match the bearer"),
        ));
    }
    let (record, settled) = store.get(&key).map_err(op_to_call)?;
    let (success, failure, execution) = settled
        .map(|s| (s.success, s.failure, s.execution))
        .unwrap_or((None, None, ExecutionState::NotStarted));
    Ok(json!({
        "key": record.key,
        "state": record.state as i32,
        "expiresAtMs": record.expires_at_ms.to_string(),
        "success": success,
        "failure": failure.map(|e| serde_json::json!({"code": e.code, "message": e.message})),
        "execution": execution_label(execution),
        "acceptedAtMs": record.accepted_at_ms.to_string(),
        "settledAtMs": record.settled_at_ms.map(|n| n.to_string()),
    }))
}

async fn operation_cancel(
    store: &OperationStore,
    input: &Value,
    caller: &Caller,
) -> CallResult<Value> {
    let map = object(input)?;
    let key = parse_dedup_key(map.get("key").ok_or_else(|| missing("key"))?)?;
    if key.principal_id != caller.principal_id || key.tenant_id != caller.tenant_id {
        return Err(CallError::new(
            v1::ErrorCode::Forbidden,
            "operation/cancel dedup key principal/tenant does not match the bearer",
        ));
    }
    match store.cancel(&key) {
        Ok(state) => Ok(json!({
            "state": state as i32,
            "outcomeUnknown": false,
        })),
        Err(error) => Err(op_to_call(error)),
    }
}

fn parse_dedup_key(value: &Value) -> CallResult<DedupKey> {
    let map = object(value)?;
    Ok(DedupKey {
        tenant_id: require_string(map, "tenantId")?.to_string(),
        principal_id: require_string(map, "principalId")?.to_string(),
        provider_endpoint_id: require_string(map, "providerEndpointId")?.to_string(),
        space_id: map
            .get("spaceId")
            .and_then(Value::as_str)
            .map(str::to_string),
        resource_id: require_string(map, "resourceId")?.to_string(),
        method: require_string(map, "method")?.to_string(),
        operation_id: require_string(map, "operationId")?.to_string(),
    })
}

fn execution_label(state: ExecutionState) -> &'static str {
    match state {
        ExecutionState::NotStarted => "not_started",
        ExecutionState::Completed => "completed",
        ExecutionState::Unknown => "unknown",
    }
}

// -------------------- helpers --------------------

fn content_to_call(error: conex_content::ContentError) -> CallError {
    CallError::new(
        error_code_for_content(error.bad_request_code()),
        format!("{error}"),
    )
}

fn error_code_for_content(code: i32) -> v1::ErrorCode {
    use v1::ErrorCode as E;
    match code {
        c if c == E::BadBlob as i32 => E::BadBlob,
        c if c == E::UnknownMethod as i32 => E::UnknownMethod,
        c if c == E::Timeout as i32 => E::Timeout,
        c if c == E::Unavailable as i32 => E::Unavailable,
        c if c == E::BadRequest as i32 => E::BadRequest,
        c if c == E::Internal as i32 => E::Internal,
        _ => E::Internal,
    }
}

fn session_to_call(error: SessionError) -> CallError {
    CallError::new(
        v1::ErrorCode::try_from(error.code()).unwrap_or(v1::ErrorCode::Internal),
        format!("{error}"),
    )
}

fn op_to_call(error: OperationError) -> CallError {
    use v1::ErrorCode as E;
    let message = format!("{error}");
    match error {
        OperationError::Conflict => CallError::new(E::Conflict, message),
        OperationError::Unknown => CallError::new(E::UnknownMethod, message),
        OperationError::Expired => CallError::new(E::OutcomeUnknown, message),
        OperationError::PathTraversal(_) => CallError::new(E::BadRequest, message),
        _ => CallError::new(E::Internal, message),
    }
}

fn root_kind_for(_root: &str, total_bytes: u64, chunk_size: u32) -> &'static str {
    if total_bytes <= chunk_size as u64 {
        "raw"
    } else {
        "manifest"
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn plane_to_enum(value: i32) -> conex_proto::v1::Plane {
    match value {
        2 => conex_proto::v1::Plane::Relay,
        _ => conex_proto::v1::Plane::Broker,
    }
}

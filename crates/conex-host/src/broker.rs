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
use conex_proto;
use conex_proto::cid;

#[derive(Debug, Clone)]
pub struct BrokerCall {
    pub caller: Caller,
    pub endpoint_id: String,
    pub method: String,
    pub input: Value,
    pub deadline: Instant,
    pub role: String,
    /// Web session link of the caller when the transport knows it; lets
    /// `connection/list` isolate anonymous visitors (plan M1.1).
    pub link_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
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
    pub catalog: Option<Arc<crate::catalog::EndpointCatalog>>,
    /// Expected `hostOrigin` string that `agent/register` must advertise.
    /// Required when `agents` is set.
    pub host_origin: Option<String>,
    /// Anonymous visitor principal (plan M1.1); `connection/list` isolates
    /// these callers to their own browser link and hides agent declarations.
    pub guest_principal: Option<String>,
    /// Visitor-facing client registry backing `client/list`, `client/profile`
    /// and `client/hello`.
    pub clients: Option<Arc<crate::clients::ClientRegistry>>,
    /// M3: reverse-agent links used to fetch remote content slices after the
    /// policy-authorized probe.
    pub remote: Option<Arc<crate::remote::RemoteConnections>>,
}

pub struct Broker {
    host: Arc<CoreHost>,
    deps: BrokerDeps,
    ui_links: std::sync::Arc<std::sync::Mutex<Option<Arc<crate::ui_links::UiLinkRegistry>>>>,
}

impl Broker {
    pub fn new(host: Arc<CoreHost>, deps: BrokerDeps) -> Self {
        Self {
            host,
            deps,
            ui_links: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }

    pub fn host(&self) -> &Arc<CoreHost> {
        &self.host
    }

    pub fn deps(&self) -> &BrokerDeps {
        &self.deps
    }

    pub fn attach_ui_links(&self, registry: Arc<crate::ui_links::UiLinkRegistry>) {
        *self.ui_links.lock().expect("broker ui_links slot poisoned") = Some(registry);
    }

    fn ui_links(&self) -> Option<Arc<crate::ui_links::UiLinkRegistry>> {
        self.ui_links
            .lock()
            .expect("broker ui_links slot poisoned")
            .clone()
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn invoke(&self, call: BrokerCall) -> CallResult<Value> {
        if call.method == "conex/hello" {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "conex/hello is handled by the transport, not the broker",
            ));
        }
        if call.method == "endpoint/list" {
            let catalog = self
                .deps
                .catalog
                .as_ref()
                .ok_or_else(|| unavailable("endpoint catalog is not configured"))?;
            return catalog.list(&call.caller, call.input).await;
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
        if call.method == "connection/list" {
            return self.dispatch_connection_list(&call).await;
        }
        if call.method.starts_with("client/") {
            return self.dispatch_client(&call).await;
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
                conex_proto::ErrorCode::UnknownMethod,
                format!("unknown blob method {}", call.method),
            )
        })?;
        let PreparedInput {
            canonical, claim, ..
        } = (contract.prepare)(&call.input)?;
        if !claim_blob(&claim) {
            return Err(CallError::new(
                conex_proto::ErrorCode::Forbidden,
                format!("{} does not grant blob access", claim.action),
            ));
        }
        // Cross-tenant access is denied at the broker: blob namespaces are
        // bound to the bearer's tenant. `ResourceClaim` does not carry a
        // tenant, so the comparison is `caller.tenant_id` only.
        let _ = claim;
        let owner = conex_content::Owner {
            principal_id: call.caller.principal_id.clone(),
            tenant_id: call.caller.tenant_id.clone(),
        };
        let content = content.clone();
        let method = call.method.clone();
        let value = match method.as_str() {
            "blob/put" => blob_put(&content, &canonical, &owner).await?,
            "blob/chunk" => blob_chunk(&content, &canonical, &owner).await?,
            "blob/commit" => blob_commit(&content, &canonical, &owner).await?,
            "blob/pin" => blob_pin(&content, &canonical, &owner).await?,
            "blob/unpin" => blob_unpin(&content, &canonical, &owner).await?,
            "blob/have" => blob_have(&content, &canonical, &owner).await?,
            "blob/get" => blob_get(self, &content, &canonical, &owner, &call.caller).await?,
            "blob/cancel" => blob_cancel(&content, &canonical, &owner).await?,
            other => {
                return Err(CallError::new(
                    conex_proto::ErrorCode::UnknownMethod,
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
                conex_proto::ErrorCode::UnknownMethod,
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
                    conex_proto::ErrorCode::UnknownMethod,
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
                conex_proto::ErrorCode::UnknownMethod,
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
                    conex_proto::ErrorCode::UnknownMethod,
                    format!("unsupported operation method {other}"),
                ));
            }
        };
        (contract.validate_output)(&value)?;
        Ok(value)
    }

    /// M3: fetch one bounded slice of a resource with raw bytes.
    ///
    /// Authorization is the ordinary source/read path: the same route
    /// lookup, contract decode, tenant check and policy probe the browser
    /// WSS/`/rpc` calls use. The URL never becomes a filesystem path here.
    /// Reverse-agent endpoints stream over the binary link; local endpoints
    /// read through the provider's [`RangeReader`] with the same revision
    /// binding, so one `/content` path serves both.
    #[allow(clippy::too_many_arguments)]
    pub async fn content_range_bytes(
        &self,
        caller: &Caller,
        endpoint_id: &str,
        resource_id: &str,
        revision: Option<&str>,
        offset: u64,
        length: usize,
        timeout: std::time::Duration,
    ) -> CallResult<conex_proto::DataChunk> {
        // Authorization + resource existence probe: same enforced path as
        // `source/read` (no second, weaker gate for content).
        self.host
            .invoke(
                caller,
                endpoint_id,
                "source/read",
                json!({ "resourceId": resource_id }),
                timeout,
            )
            .await?;
        let agent_id = self
            .deps
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.agent_for_endpoint(endpoint_id));
        if let (Some(agent_id), Some(connections)) = (agent_id, self.deps.remote.as_ref()) {
            let mut remote = json!({
                "endpointId": endpoint_id,
                "resourceId": resource_id,
                "offset": offset.to_string(),
                "length": length.to_string(),
            });
            if let Some(revision) = revision {
                remote["revision"] = json!(revision);
            }
            let input = json!({ "remote": remote });
            return connections
                .request_chunk(
                    &agent_id,
                    endpoint_id,
                    caller,
                    input,
                    tokio::time::Instant::now() + timeout,
                )
                .await;
        }
        // Local endpoint: read the bounded slice through the provider.
        let claim = conex_source::resource::read_claim(resource_id)?;
        let ctx = conex_core::CallContext {
            caller: caller.clone(),
            endpoint_id: endpoint_id.to_string(),
            plane: conex_proto::Plane::Broker,
            method: "source/read".to_string(),
            claim,
            policy_version: 0,
            deadline: tokio::time::Instant::now() + timeout,
        };
        let reader = self
            .host
            .registry()
            .routes_for_endpoint(endpoint_id)
            .into_iter()
            .find_map(|route| route.range_reader.clone())
            .ok_or_else(|| unavailable("endpoint cannot serve byte ranges"))?;
        let (bytes, revision, eof) = reader.read_range(&ctx, offset, length, revision).await?;
        Ok(conex_proto::DataChunk {
            request_id: String::new(),
            chunk: bytes.into(),
            revision,
            eof,
        })
    }

    async fn dispatch_connection_list(&self, call: &BrokerCall) -> CallResult<Value> {
        if call.role != "ui" {
            return Err(CallError::new(
                conex_proto::ErrorCode::Forbidden,
                "connection/list is restricted to the ui role",
            ));
        }
        let _ = object(&call.input)?;
        let principal_id = call.caller.principal_id.clone();
        let is_guest = self.deps.guest_principal.as_deref() == Some(principal_id.as_str());
        let mut browser_links: Vec<Value> = self
            .ui_links()
            .ok_or_else(|| unavailable("ui link registry is not configured"))?
            .list_for_principal(Some(&principal_id))
            .into_iter()
            .map(ui_link_to_json)
            .collect::<Vec<_>>();
        // Anonymous visitors share one principal: each sees only its own
        // session activity, never another visitor's (plan M1.1).
        if is_guest {
            browser_links
                .retain(|row| call.link_id.as_deref() == row.get("linkId").and_then(Value::as_str));
            return Ok(json!({
                "browserLinks": browser_links,
                // Agent resource declarations are management data; guests
                // already see tenant-filtered endpoint state via endpoint/list.
                "agentLinks": [],
            }));
        }
        let agent_links = self
            .deps
            .agents
            .as_ref()
            .map(|agents| {
                let mut rows: Vec<Value> =
                    agents.list().into_iter().map(agent_link_to_json).collect();
                rows.sort_by(|a, b| {
                    a.get("agentId")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .cmp(b.get("agentId").and_then(Value::as_str).unwrap_or(""))
                });
                rows
            })
            .unwrap_or_default();
        Ok(json!({
            "browserLinks": browser_links,
            "agentLinks": agent_links,
        }))
    }

    /// `client/*`: the visitor-facing client panel. Every method requires a
    /// bound UI link, because a client is defined by its link.
    async fn dispatch_client(&self, call: &BrokerCall) -> CallResult<Value> {
        if call.role != "ui" {
            return Err(CallError::new(
                conex_proto::ErrorCode::Forbidden,
                "client methods are restricted to the ui role",
            ));
        }
        let registry = self
            .deps
            .clients
            .as_ref()
            .ok_or_else(|| unavailable("client registry is not configured"))?;
        let link_id = call
            .link_id
            .as_deref()
            .ok_or_else(|| bad_req("client methods require a browser link"))?;
        match call.method.as_str() {
            "client/list" => {
                object(&call.input)?;
                // The group is the isolation boundary: a caller only ever sees
                // its own group's clients. A hidden client is not listed to
                // anyone including itself; the page renders its own row from
                // `client/profile`.
                let rows: Vec<Value> = registry
                    .list_for_group(link_id)
                    .into_iter()
                    .filter(|entry| entry.profile.visible)
                    .map(client_to_json)
                    .collect();
                Ok(json!({ "clients": rows }))
            }
            "client/status" => {
                object(&call.input)?;
                // Aggregate counters are host-wide on purpose: the status page
                // answers "how many clients and groups are on this host", which
                // is a fact about the process rather than about the caller. No
                // individual row, name or file is projected here, so it leaks
                // nothing a group-scoped `client/list` would not.
                let groups = registry.group_status();
                Ok(json!({
                    "clientsOnline": registry.online_count().to_string(),
                    "groupsOnline": groups.len().to_string(),
                    "groups": groups
                        .iter()
                        .map(|group| json!({
                            "groupKey": group.group_key,
                            "label": group.label,
                            "clientsOnline": group.clients_online.to_string(),
                            "lastRoundTripMs": group.last_round_trip_ms.to_string(),
                            "avgRoundTripMs": group.avg_round_trip_ms.to_string(),
                            "roundTrips": group.round_trips.to_string(),
                            "filesShared": group.files_shared.to_string(),
                            "sharedBytes": group.shared_bytes.to_string(),
                        }))
                        .collect::<Vec<Value>>(),
                }))
            }
            "client/profile" => {
                let input = object(&call.input)?;
                let profile_value = input.get("profile").cloned().unwrap_or(Value::Null);
                let profile = crate::clients::ClientProfile::from_input(&profile_value)
                    .ok_or_else(|| {
                        CallError::new(
                            conex_proto::ErrorCode::BadRequest,
                            "profile must be an object",
                        )
                    })?;
                let entry = registry
                    .set_profile(link_id, profile)
                    .ok_or_else(|| unavailable("client link is gone"))?;
                Ok(json!({ "self": client_to_json(entry) }))
            }
            "client/hello" => {
                let input = object(&call.input)?;
                let target = input
                    .get("targetLinkId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if target.is_empty() {
                    return Err(CallError::new(
                        conex_proto::ErrorCode::BadRequest,
                        "targetLinkId is required",
                    ));
                }
                let text = input
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("hello")
                    .chars()
                    .take(200)
                    .collect::<String>();
                // The callback payload is capped here so an oversized one is
                // refused to its own sender instead of timing out on the target.
                let payload = match input.get("payload") {
                    Some(value) => crate::clients::cap_payload(value)?,
                    None => None,
                };
                let outcome = registry
                    .send_hello(target, link_id, &text, payload.as_ref())
                    .await?;
                Ok(json!({
                    "targetLinkId": target,
                    "roundTripMs": outcome.round_trip_ms.to_string(),
                    "reply": outcome.reply,
                    // An absent answer is an empty object rather than a
                    // missing field: the shape is the same either way, so a
                    // client never has to distinguish "no answer" from
                    // "this host does not support answers".
                    "answer": outcome.answer.unwrap_or_else(|| json!({})),
                }))
            }
            other => Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                format!("unknown client method: {other}"),
            )),
        }
    }
}

fn client_to_json(entry: crate::clients::ClientEntry) -> Value {
    json!({
        "linkId": entry.link_id,
        "principalId": entry.principal_id,
        "tenantId": entry.tenant_id,
        "profile": {
            "displayName": entry.profile.label(&entry.link_id),
            "declaredName": entry.profile.display_name,
            "group": entry.profile.group,
            "visible": entry.profile.visible,
        },
        "connectedAtMs": entry.connected_at_ms.to_string(),
        "lastSeenAtMs": entry.last_seen_at_ms.to_string(),
        "userAgent": entry.user_agent,
        "groupKey": entry.group_key,
        "lastRoundTripMs": entry.last_round_trip_ms.to_string(),
        "avgRoundTripMs": entry.avg_round_trip_ms().to_string(),
        "filesShared": entry.files_shared.to_string(),
        "sharedBytes": entry.shared_bytes.to_string(),
    })
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
    CallError::new(conex_proto::ErrorCode::Unavailable, message)
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
    CallError::new(conex_proto::ErrorCode::BadRequest, message)
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

async fn blob_put(
    store: &ContentStore,
    input: &Value,
    owner: &conex_content::Owner,
) -> CallResult<Value> {
    let map = object(input)?;
    let format_version = require_u64(map, "formatVersion")? as u32;
    if format_version != 1 {
        return Err(CallError::new(
            conex_proto::ErrorCode::UnsupportedCapability,
            format!("formatVersion {format_version} not supported"),
        ));
    }
    let declared_size_bytes = require_u64(map, "declaredSizeBytes")?;
    const MAX_BLOB_BYTES: u64 = 1_073_741_824;
    if declared_size_bytes > MAX_BLOB_BYTES {
        return Err(content_to_call(conex_content::ContentError::TooLarge {
            limit: MAX_BLOB_BYTES,
            actual: declared_size_bytes,
        }));
    }
    let declared_chunk_size = require_u64(map, "declaredChunkSize")? as u32;
    if declared_chunk_size != conex_proto::cid::CHUNK_SIZE as u32 {
        return Err(bad_req(&format!(
            "declaredChunkSize must be {}",
            conex_proto::cid::CHUNK_SIZE
        )));
    }
    const MAX_LEASE_MS: u64 = 24 * 60 * 60 * 1000;
    let requested_lease_ms = map
        .get("requestedLeaseMs")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse::<u64>().ok());
    if requested_lease_ms.is_some_and(|lease| lease > MAX_LEASE_MS) {
        return Err(bad_req("requestedLeaseMs must not exceed 24h"));
    }
    let expected_root = require_string(map, "expectedRootCid")?.to_string();
    expect_cid(&expected_root)?;
    let access = object(map.get("access").ok_or_else(|| missing("access"))?)?;
    let resource_id = require_string(access, "resourceId")?.to_string();
    expect_resource_id("resourceId", &resource_id)?;
    // Opportunistic staging reaping; expired pre-M1.2 unbound staging is
    // never claimed, just dropped (plan M1.2).
    store.purge_expired_uploads();
    let upload = store
        .begin_upload(
            format_version,
            declared_chunk_size,
            declared_size_bytes,
            root_kind_for(&expected_root, declared_size_bytes, declared_chunk_size),
            &expected_root,
            requested_lease_ms,
            owner.clone(),
            &resource_id,
        )
        .map_err(content_to_call)?;
    // On resume the upload carries the chunks already received; report them
    // so the client skips re-upload (design §5.5).
    let already_have: Vec<String> = upload
        .state()
        .received_chunks
        .iter()
        .map(|chunk| chunk.cid.clone())
        .collect();
    Ok(json!({
        "uploadId": upload.upload_id(),
        "chunkSize": declared_chunk_size.to_string(),
        "leaseMs": requested_lease_ms.unwrap_or(60 * 60 * 1000).to_string(),
        "maxBlobBytes": "1073741824",
        "inlineThresholdBytes": "65536",
        "alreadyHaveChunkCids": already_have,
        "resourceId": resource_id,
    }))
}

async fn blob_chunk(
    store: &ContentStore,
    input: &Value,
    owner: &conex_content::Owner,
) -> CallResult<Value> {
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
    // Validate the bytes against the declared CID BEFORE touching the store:
    // a bad chunk must be rejected without persisting the block or recording
    // it in the upload receipt (design §5.4 "坏块...可保留此前已验证块").
    let recomputed = conex_proto::cid::cid_for_raw(&bytes);
    if recomputed != chunk_cid {
        return Err(CallError::new(
            conex_proto::ErrorCode::BadBlob,
            format!("chunk cid mismatch: declared {chunk_cid}, recomputed {recomputed}"),
        ));
    }
    let upload = store
        .resume_upload(&upload_id, owner)
        .map_err(content_to_call)?;
    upload
        .put_chunk(chunk_index, &bytes)
        .map_err(content_to_call)?;
    Ok(json!({
        "chunkIndex": chunk_index.to_string(),
        "receivedBytes": bytes.len().to_string(),
        "remainingBytes": "0",
    }))
}

async fn blob_commit(
    store: &ContentStore,
    input: &Value,
    owner: &conex_content::Owner,
) -> CallResult<Value> {
    let map = object(input)?;
    let upload_id = require_string(map, "uploadId")?.to_string();
    let declared_root = require_string(map, "declaredRootCid")?.to_string();
    expect_cid(&declared_root)?;
    let persistence = require_string(map, "persistence")?.to_string();
    if persistence != "local" {
        return Err(CallError::new(
            conex_proto::ErrorCode::Unavailable,
            format!("persistence {persistence} not supported by local broker"),
        ));
    }
    // The root kind was recorded at blob/put; deriving it from the upload
    // (owner-verified) instead of guessing avoids rejecting raw commits whose
    // first probe mismatches on kind.
    let upload = store
        .resume_upload(&upload_id, owner)
        .map_err(content_to_call)?;
    let kind = upload.state().declared_root_kind.clone();
    let commit = store
        .commit(&upload_id, &declared_root, &kind, owner)
        .map_err(content_to_call)?;
    Ok(json!({
        "resourceRootCid": commit.root_cid,
        "committedBytes": commit.committed_bytes.to_string(),
        "persistence": persistence,
        "receiptId": commit.receipt_id,
    }))
}

async fn blob_pin(
    store: &ContentStore,
    input: &Value,
    owner: &conex_content::Owner,
) -> CallResult<Value> {
    let map = object(input)?;
    let root = require_string(map, "rootCid")?.to_string();
    expect_cid(&root)?;
    let expires_at_ms = map
        .get("pinUntilMs")
        .and_then(Value::as_str)
        .and_then(|raw| raw.parse::<u64>().ok())
        .unwrap_or_else(now_ms);
    let pin = store
        .pin(&root, &format!("pin-{root}"), expires_at_ms, owner)
        .map_err(content_to_call)?;
    Ok(json!({
        "pinId": pin.pin_id,
        "expiresAtMs": pin.expires_at_ms.to_string(),
    }))
}

async fn blob_unpin(
    store: &ContentStore,
    input: &Value,
    owner: &conex_content::Owner,
) -> CallResult<Value> {
    let map = object(input)?;
    let pin_id = require_string(map, "pinId")?.to_string();
    store.unpin(&pin_id, owner).map_err(content_to_call)?;
    Ok(json!({}))
}

async fn blob_have(
    store: &ContentStore,
    input: &Value,
    owner: &conex_content::Owner,
) -> CallResult<Value> {
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
    let present = store.has(&owned, owner).map_err(content_to_call)?;
    Ok(json!({ "present": present }))
}

async fn blob_get(
    broker: &Broker,
    store: &ContentStore,
    input: &Value,
    owner: &conex_content::Owner,
    caller: &Caller,
) -> CallResult<Value> {
    let map = object(input)?;
    let committed = map.get("committed");
    let remote_target = map.get("remote");
    match (committed, remote_target) {
        (Some(target), None) => {
            let target = object(target)?;
            let chunk_cid = require_string(target, "chunkCid")?.to_string();
            expect_cid(&chunk_cid)?;
            let bytes = store.get(&chunk_cid, owner).map_err(content_to_call)?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes.as_ref());
            Ok(json!({
                "chunkBytes": encoded,
                "chunkCid": chunk_cid,
            }))
        }
        // M3: remote target fetches one bounded slice through the same
        // authorized path `/content` uses. The JSON plane reply carries
        // base64 because JSON has no native bytes; `/content` streams the
        // raw bytes straight off the binary link.
        (None, Some(target)) => {
            let target = object(target)?;
            let endpoint_id = require_string(target, "endpointId")?.to_string();
            let resource_id = require_string(target, "resourceId")?.to_string();
            let revision = target
                .get("revision")
                .and_then(Value::as_str)
                .map(str::to_string);
            let offset: u64 = require_u64(target, "offset")?;
            let length: u64 = require_u64(target, "length")?;
            let chunk = broker
                .content_range_bytes(
                    caller,
                    &endpoint_id,
                    &resource_id,
                    revision.as_deref(),
                    offset,
                    length.min(conex_proto::cid::CHUNK_SIZE as u64) as usize,
                    std::time::Duration::from_secs(30),
                )
                .await?;
            let encoded = base64::engine::general_purpose::STANDARD.encode(chunk.chunk.as_ref());
            Ok(json!({
                "chunkBytes": encoded,
                "revision": chunk.revision,
                "eof": chunk.eof,
            }))
        }
        _ => Err(CallError::new(
            conex_proto::ErrorCode::Internal,
            "blob/get reached handler without a contract-valid target",
        )),
    }
}

async fn blob_cancel(
    store: &ContentStore,
    input: &Value,
    owner: &conex_content::Owner,
) -> CallResult<Value> {
    let map = object(input)?;
    let upload_id = require_string(map, "uploadId")?.to_string();
    store
        .cancel_upload(&upload_id, owner)
        .map_err(content_to_call)?;
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
            conex_proto::ErrorCode::Forbidden,
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
                conex_proto::ErrorCode::UnsupportedCapability,
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
                conex_proto::ErrorCode::Forbidden,
                "session resume binding does not match the bearer principal",
            ));
        }
        if !b.tenant_id.is_empty() && b.tenant_id != caller.tenant_id {
            return Err(CallError::new(
                conex_proto::ErrorCode::Forbidden,
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
            conex_proto::ErrorCode::Forbidden,
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
            conex_proto::ErrorCode::Forbidden,
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

fn error_code_for_content(code: i32) -> conex_proto::ErrorCode {
    use conex_proto::ErrorCode as E;
    match code {
        c if c == E::BadBlob as i32 => E::BadBlob,
        c if c == E::UnknownMethod as i32 => E::UnknownMethod,
        c if c == E::Timeout as i32 => E::Timeout,
        c if c == E::Unavailable as i32 => E::Unavailable,
        c if c == E::BadRequest as i32 => E::BadRequest,
        c if c == E::Forbidden as i32 => E::Forbidden,
        c if c == E::PayloadTooLarge as i32 => E::PayloadTooLarge,
        c if c == E::UnknownProvider as i32 => E::UnknownProvider,
        c if c == E::Internal as i32 => E::Internal,
        _ => E::Internal,
    }
}

fn session_to_call(error: SessionError) -> CallError {
    CallError::new(
        conex_proto::ErrorCode::try_from(error.code()).unwrap_or(conex_proto::ErrorCode::Internal),
        format!("{error}"),
    )
}

fn op_to_call(error: OperationError) -> CallError {
    use conex_proto::ErrorCode as E;
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

fn ui_link_to_json(link: crate::ui_links::UiLink) -> Value {
    json!({
        "linkId": link.link_id,
        "principalId": link.principal_id,
        "tenantId": link.tenant_id,
        "connectedAtMs": link.connected_at_ms.to_string(),
        "lastSeenAtMs": link.last_seen_at_ms.to_string(),
        "ticketsIssued": link.tickets_issued.to_string(),
        "callsTotal": link.calls_total.to_string(),
        "callsInFlight": link.calls_in_flight.to_string(),
    })
}

fn agent_link_to_json(agent: crate::agent::AgentRegistration) -> Value {
    json!({
        "agentId": agent.agent_id,
        "principalId": agent.principal_id,
        "tenantId": agent.tenant_id,
        "registeredAtMs": agent.registered_at_ms.to_string(),
        "lastHeartbeatAtMs": agent.last_heartbeat_at_ms.to_string(),
        "methods": agent.methods(),
        "resources": agent.resources(),
        "endpoints": agent.endpoints.iter().map(|endpoint| json!({
            "endpointId": endpoint.endpoint_id,
            "root": endpoint.root,
            "methods": endpoint.methods,
        })).collect::<Vec<_>>(),
    })
}

fn plane_to_enum(value: i32) -> conex_proto::Plane {
    match value {
        2 => conex_proto::Plane::Relay,
        _ => conex_proto::Plane::Broker,
    }
}

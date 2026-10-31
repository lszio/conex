//! Private product entry (docs/plans/2026-10-29-conex-mvp-p5.md §3-§4):
//! anonymous access is refused by default, grants are issued only over an
//! authenticated path, and the hub never accepts a principal from a request
//! body. Node runtime epochs are learned exclusively from an authenticated
//! publication link (`access/node-identity`), never from grant requests.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use axum::extract::Json;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::grants::{
    AccessGrantResponse, AccessGrantScope, GrantIssuer, GrantPolicy, PURPOSE_DIRECT,
    VerifiedPrincipal,
};
use crate::private_store::{GrantLedgerRow, PrivateStore};
use crate::web_auth::WebAuth;

pub const BOOTSTRAP_TTL_MS: u64 = 5 * 60 * 1000;

/// Resolved `[private]` configuration. Presence on `HostConfig` enables the
/// private entry; there is no separate on/off flag to leave half-open.
#[derive(Debug, Clone)]
pub struct PrivateSettings {
    pub issuer: String,
    pub kid: String,
    pub tenant: String,
    pub signing_key_backend: String,
    pub store_path: std::path::PathBuf,
    /// The principal the bootstrap code authenticates as.
    pub principal: String,
    /// Principals allowed to call `/private/revoke`.
    pub admin_principals: BTreeSet<String>,
    /// Node publication credential principals a grant may target.
    pub node_principals: BTreeSet<String>,
}

/// Runtime state for the private entry. `None` in `HttpState` means the
/// private product is not configured and every `/private/*` route answers
/// 404/unavailable instead of falling back to guest behavior.
pub struct PrivateAccess {
    pub settings: PrivateSettings,
    pub issuer: GrantIssuer,
    pub store: PrivateStore,
    web_auth: Mutex<Option<Arc<WebAuth>>>,
    /// node publication principal -> identity learned from the node link.
    nodes: Mutex<BTreeMap<String, NodeIdentity>>,
}

#[derive(Debug, Clone)]
pub struct NodeIdentity {
    pub node_id: String,
    pub node_epoch: String,
    pub registered_at_ms: u64,
}

impl PrivateAccess {
    pub fn new(settings: PrivateSettings) -> Result<Self, conex_core::CallError> {
        let issuer = GrantIssuer::load(
            &settings.kid,
            &settings.issuer,
            &settings.signing_key_backend,
            GrantPolicy::default(),
        )
        .map_err(|error| {
            conex_core::CallError::new(conex_proto::ErrorCode::Internal, error.to_string())
        })?;
        let store = PrivateStore::open(&settings.store_path)?;
        Ok(Self {
            settings,
            issuer,
            store,
            web_auth: Mutex::new(None),
            nodes: Mutex::new(BTreeMap::new()),
        })
    }

    /// The browser session registry exists only in `build_router`; private
    /// handlers borrow it from here.
    pub fn set_web_auth(&self, web_auth: Option<Arc<WebAuth>>) {
        *self
            .web_auth
            .lock()
            .expect("private web_auth slot poisoned") = web_auth;
    }

    fn web(&self) -> Result<Arc<WebAuth>, conex_core::CallError> {
        self.web_auth
            .lock()
            .expect("private web_auth slot poisoned")
            .clone()
            .ok_or_else(|| unavailable("private web origin is not configured"))
    }

    pub fn store(&self) -> &PrivateStore {
        &self.store
    }

    /// The hub learns a node's runtime epoch only over its authenticated
    /// publication link. Re-registration refreshes the epoch; an old grant
    /// naming a previous epoch dies at the node's per-operation check.
    pub fn register_node_identity(
        &self,
        principal: &str,
        node_id: &str,
        node_epoch: &str,
        now_ms: u64,
    ) -> Result<(), conex_core::CallError> {
        if self.settings.tenant.is_empty() {
            return Err(unavailable("private tenant is not configured"));
        }
        if !is_ulid(node_id) || !is_ulid(node_epoch) {
            return Err(forbidden("node identity must be canonical ULIDs"));
        }
        self.nodes.lock().expect("node registry poisoned").insert(
            principal.to_owned(),
            NodeIdentity {
                node_id: node_id.to_owned(),
                node_epoch: node_epoch.to_owned(),
                registered_at_ms: now_ms,
            },
        );
        Ok(())
    }

    pub fn node_identity(&self, principal: &str) -> Option<NodeIdentity> {
        self.nodes
            .lock()
            .expect("node registry poisoned")
            .get(principal)
            .cloned()
    }

    /// Issue a grant for an authenticated caller. `requested` is the wire
    /// request; the principal always comes from `verified`, never from it.
    pub async fn issue_grant(
        &self,
        verified: &VerifiedPrincipal,
        node_id: &str,
        scope: &AccessGrantScope,
        now_ms: u64,
    ) -> Result<AccessGrantResponse, conex_core::CallError> {
        if verified.principal.is_empty() {
            return Err(unauthorized("authenticated principal is required"));
        }
        if scope.purpose != PURPOSE_DIRECT && scope.purpose != crate::grants::PURPOSE_RELAY {
            return Err(forbidden("grant purpose must be direct or relay"));
        }
        // The node epoch comes from the node's own authenticated publication,
        // not from this request: a grant names the epoch the node announced.
        let identity = self
            .node_identity(&self.principal_for_node(node_id)?)
            .ok_or_else(|| forbidden("node is not registered for private access"))?;
        if identity.node_id != node_id {
            return Err(forbidden("node identity mismatch"));
        }
        let ttl = GrantPolicy::default()
            .ceiling(&scope.purpose)
            .ok_or_else(|| forbidden("grant purpose must be direct or relay"))?;
        let response = self
            .issuer
            .issue(verified, node_id, &identity.node_epoch, scope, now_ms, ttl)
            .map_err(|error| forbidden(&error.to_string()))?;
        let scope_json = serde_json::to_string(scope).map_err(|error| {
            conex_core::CallError::new(conex_proto::ErrorCode::Internal, error.to_string())
        })?;
        self.store
            .record_grant(GrantLedgerRow {
                jti: response.jti.clone(),
                principal: verified.principal.clone(),
                tenant: verified.tenant.clone(),
                node_id: node_id.to_owned(),
                node_epoch: identity.node_epoch,
                purpose: scope.purpose.clone(),
                scope_json,
                issued_at_ms: now_ms,
                expires_at_ms: response.expires_at_ms,
            })
            .await?;
        Ok(response)
    }

    /// Map a node id to the publication principal that announced it.
    fn principal_for_node(&self, node_id: &str) -> Result<String, conex_core::CallError> {
        let nodes = self.nodes.lock().expect("node registry poisoned");
        nodes
            .iter()
            .find(|(_, identity)| identity.node_id == node_id)
            .map(|(principal, _)| principal.clone())
            .ok_or_else(|| forbidden("node is not registered for private access"))
    }

    /// Persist a revocation first, then apply hub-visible effects. The
    /// revision is only returned after the row is durable.
    pub async fn revoke(
        &self,
        kind: &str,
        target: &str,
        now_ms: u64,
    ) -> Result<u64, conex_core::CallError> {
        if !matches!(kind, "jti" | "principal" | "kid") {
            return Err(forbidden("revocation kind must be jti, principal or kid"));
        }
        if target.is_empty() || target.len() > 256 {
            return Err(forbidden("revocation target is required"));
        }
        let revision = self
            .store
            .append_revocation(kind.to_owned(), target.to_owned(), now_ms)
            .await?;
        if kind == "principal"
            && let Ok(web) = self.web()
        {
            // Hub sessions of a revoked principal die immediately; nodes
            // learn the same fact through the revision sync.
            web.revoke_all_for_principal(target);
        }
        Ok(revision)
    }
}

// ---------------------------------------------------------------------------
// HTTP handlers
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BootstrapBody {
    code: String,
}

/// `POST /private/session`: exchange the one-time bootstrap code for a
/// private cookie+CSRF session. The code travels only in the HTTPS body; it
/// never reaches a URL, log line or directory.
pub async fn private_session(
    axum::extract::Extension(state): axum::extract::Extension<Arc<crate::http::HttpState>>,
    headers: HeaderMap,
    Json(body): Json<BootstrapBody>,
) -> Response {
    let Some(private) = state.private.as_ref() else {
        return not_found();
    };
    let web = match private.web() {
        Ok(web) => web,
        Err(error) => return crate::web_auth::error_response(error),
    };
    if let Err(error) = web.check_origin(&headers) {
        return crate::web_auth::error_response(error);
    }
    if body.code.len() > 256 || body.code.trim() != body.code || body.code.is_empty() {
        return crate::web_auth::error_response(unauthorized("bootstrap code is malformed"));
    }
    let now = now_ms();
    let consumed = private
        .store
        .consume_bootstrap(bootstrap_hash(&body.code), now)
        .await;
    match consumed {
        Ok(true) => {}
        Ok(false) => {
            return crate::web_auth::error_response(unauthorized(
                "bootstrap code is invalid or already used",
            ));
        }
        Err(error) => return crate::web_auth::error_response(error),
    }
    let observed = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let session = match web.login_private(
        private.settings.principal.clone(),
        private.settings.tenant.clone(),
        observed,
    ) {
        Ok(session) => session,
        Err(error) => return crate::web_auth::error_response(error),
    };
    let mut response = (
        StatusCode::OK,
        axum::Json(json!({
            "principalId": session.caller.principal_id,
            "tenantId": session.caller.tenant_id,
            "role": session.role,
            "csrf": session.csrf,
            "expiresAtMs": session.expires_at_ms().to_string(),
        })),
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, web.set_cookie(&session));
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct GrantBody {
    node_id: String,
    scope: ScopeBody,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ScopeBody {
    space_id: String,
    provider_endpoint_id: String,
    contract_id: String,
    module_generation: String,
    methods: Vec<String>,
    states: Vec<String>,
    workspace_ids: Vec<String>,
    conversation_ids: Vec<String>,
    purpose: String,
}

impl ScopeBody {
    // axum Responses are the denial carrier for HTTP handlers; boxing
    // would obscure the one-error-one-response boundary.
    #[allow(clippy::result_large_err)]
    fn into_scope(self) -> Result<AccessGrantScope, Response> {
        let module_generation = parse_u64(&self.module_generation).ok_or_else(|| {
            crate::web_auth::error_response(forbidden(
                "moduleGeneration must be a canonical uint64 string",
            ))
        })?;
        if module_generation == 0 {
            return Err(crate::web_auth::error_response(forbidden(
                "moduleGeneration must be the target module's actual generation",
            )));
        }
        Ok(AccessGrantScope {
            space_id: self.space_id,
            provider_endpoint_id: self.provider_endpoint_id,
            contract_id: self.contract_id,
            module_generation,
            methods: self.methods,
            states: self.states,
            workspace_ids: self.workspace_ids,
            conversation_ids: self.conversation_ids,
            purpose: self.purpose,
        })
    }
}

/// `POST /private/grant`: two independent authenticated paths — browser
/// (cookie + exact Origin + CSRF) or dedicated client bearer. The principal
/// is resolved from authentication only; a `principal` in the body would be
/// an unknown field and rejected by the deserializer.
pub async fn private_grant(
    axum::extract::Extension(state): axum::extract::Extension<Arc<crate::http::HttpState>>,
    headers: HeaderMap,
    Json(body): Json<GrantBody>,
) -> Response {
    let Some(private) = state.private.as_ref() else {
        return not_found();
    };
    let scope = match body.scope.into_scope() {
        Ok(scope) => scope,
        Err(response) => return response,
    };
    let verified = match authenticate_caller(&state, private, &headers) {
        Ok(verified) => verified,
        Err(response) => return response,
    };
    let now = now_ms();
    match private
        .issue_grant(&verified, &body.node_id, &scope, now)
        .await
    {
        Ok(response) => (
            StatusCode::OK,
            axum::Json(json!({
                "token": response.token,
                "jti": response.jti,
                "nodeId": response.node_id,
                "nodeEpoch": response.node_epoch,
                "expiresAtMs": response.expires_at_ms.to_string(),
            })),
        )
            .into_response(),
        Err(error) => crate::web_auth::error_response(error),
    }
}

#[allow(clippy::result_large_err)]
fn authenticate_caller(
    state: &Arc<crate::http::HttpState>,
    private: &PrivateAccess,
    headers: &HeaderMap,
) -> Result<VerifiedPrincipal, Response> {
    let has_cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .is_some();
    if has_cookie {
        let web = private.web().map_err(crate::web_auth::error_response)?;
        let session = web
            .check_mutation(headers)
            .map_err(crate::web_auth::error_response)?;
        return Ok(VerifiedPrincipal {
            principal: session.caller.principal_id.clone(),
            tenant: session.caller.tenant_id.clone(),
        });
    }
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let inbound = state
        .auth
        .authenticate(authorization)
        .map_err(crate::web_auth::error_response)?;
    if inbound.role == "ui" {
        return Err(crate::web_auth::error_response(unauthorized(
            "UI credentials require a private session",
        )));
    }
    Ok(VerifiedPrincipal {
        principal: inbound.caller.principal_id,
        tenant: inbound.caller.tenant_id,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RevokeBody {
    kind: String,
    target: String,
}

/// `POST /private/revoke`: administrators only. The revocation commits to
/// the monotonic log before the revision is returned; nodes pick it up by
/// revision sync, never by wall-clock timestamps.
pub async fn private_revoke(
    axum::extract::Extension(state): axum::extract::Extension<Arc<crate::http::HttpState>>,
    headers: HeaderMap,
    Json(body): Json<RevokeBody>,
) -> Response {
    let Some(private) = state.private.as_ref() else {
        return not_found();
    };
    let verified = match authenticate_caller(&state, private, &headers) {
        Ok(verified) => verified,
        Err(response) => return response,
    };
    if !private
        .settings
        .admin_principals
        .contains(&verified.principal)
    {
        return crate::web_auth::error_response(forbidden("revocation requires an administrator"));
    }
    let now = now_ms();
    match private.revoke(&body.kind, &body.target, now).await {
        Ok(revision) => (
            StatusCode::OK,
            axum::Json(json!({"revision": revision.to_string()})),
        )
            .into_response(),
        Err(error) => crate::web_auth::error_response(error),
    }
}

// ---------------------------------------------------------------------------
// access/* control methods on the node publication link
// ---------------------------------------------------------------------------

/// `access/node-identity`: the node announces its runtime epoch over the
/// authenticated publication link. Handled from the broker's `access/*`
/// dispatch with the already-authenticated caller.
pub async fn access_node_identity(
    private: &PrivateAccess,
    principal: &str,
    input: &Value,
) -> Result<Value, conex_core::CallError> {
    if !private.settings.node_principals.contains(principal) {
        return Err(forbidden(
            "node identity registration requires an approved node principal",
        ));
    }
    let node_id = input
        .get("nodeId")
        .and_then(Value::as_str)
        .ok_or_else(|| forbidden("nodeId is required"))?;
    let node_epoch = input
        .get("nodeRuntimeEpoch")
        .and_then(Value::as_str)
        .ok_or_else(|| forbidden("nodeRuntimeEpoch is required"))?;
    private.register_node_identity(principal, node_id, node_epoch, now_ms())?;
    let revision = private.store.latest_revision().await?;
    Ok(json!({"accepted": true, "revocationRevision": revision.to_string()}))
}

/// `access/revocations/sync`: paged, revision-keyed. A page is at most 128
/// items and 196608 bytes; a gap means the node must resnapshot from zero.
pub async fn access_revocations_sync(
    private: &PrivateAccess,
    input: &Value,
) -> Result<Value, conex_core::CallError> {
    let after_revision = input
        .get("afterRevision")
        .and_then(Value::as_str)
        .and_then(parse_u64)
        .ok_or_else(|| forbidden("afterRevision must be a canonical uint64 string"))?;
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(crate::private_store::MAX_REVOKE_PAGE_ITEMS as u64);
    if limit == 0 || limit > crate::private_store::MAX_REVOKE_PAGE_ITEMS as u64 {
        return Err(forbidden("limit must be 1..=128"));
    }
    let page = private
        .store
        .sync_revocations(after_revision, limit as usize)
        .await?;
    let items: Vec<Value> = page
        .items
        .iter()
        .map(|row| {
            json!({
                "revision": row.revision.to_string(),
                "kind": row.kind,
                "target": row.target,
                "effectiveAtMs": row.effective_at_ms.to_string(),
            })
        })
        .collect();
    Ok(json!({
        "items": items,
        "nextRevision": page.next_revision.to_string(),
        "hasMore": page.has_more,
    }))
}

// ---------------------------------------------------------------------------
// shared helpers
// ---------------------------------------------------------------------------

/// `conex-host private-bootstrap`: generate a 256-bit one-time code, persist
/// only its hash, and print the plaintext once to the administrator.
pub fn generate_bootstrap_code() -> String {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system random source unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn bootstrap_hash(code: &str) -> String {
    hex::encode(Sha256::digest(code.as_bytes()))
}

pub use crate::private_store::put_bootstrap_blocking;

pub fn ensure_private_dir(path: &std::path::Path) -> Result<(), conex_core::CallError> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(path)
        .map_err(|error| store_failure(format!("private store directory: {error}")))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| store_failure(format!("private store directory mode: {error}")))?;
    Ok(())
}

fn store_failure(message: String) -> conex_core::CallError {
    conex_core::CallError::new(conex_proto::ErrorCode::Internal, message)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub fn parse_u64(text: &str) -> Option<u64> {
    if text.is_empty() || (text != "0" && text.starts_with('0')) {
        return None;
    }
    text.parse::<u64>().ok()
}

fn is_ulid(value: &str) -> bool {
    value.len() == 26 && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn unauthorized(message: &str) -> conex_core::CallError {
    conex_core::CallError::new(conex_proto::ErrorCode::Unauthorized, message.to_string())
}
fn forbidden(message: &str) -> conex_core::CallError {
    conex_core::CallError::new(conex_proto::ErrorCode::Forbidden, message.to_string())
}
fn unavailable(message: &str) -> conex_core::CallError {
    conex_core::CallError::new(conex_proto::ErrorCode::Unavailable, message.to_string())
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

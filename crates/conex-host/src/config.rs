//! Installation config. Only file paths, profile ids and credential references;
//! no executable fields are accepted (serde deny_unknown_fields).
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use conex_core::CallError;
use conex_proto;
use serde::Deserialize;

pub const SUPPORTED_KINDS: [&str; 3] = ["source-fs", "source-http-catalog", "source-remote"];
pub const SUPPORTED_TOKEN_ROLES: [&str; 3] = ["ui", "agent", "service"];
pub const SUPPORTED_REMOTE_METHODS: [&str; 3] = ["source/list", "source/read", "source/search"];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    pub listen: String,
    #[serde(default)]
    pub allow_loopback_http: bool,
    /// Plaintext listener on a non-loopback address, for a container whose
    /// port is only reachable from a TLS-terminating reverse proxy. Separate
    /// from `allow_loopback_http`, which means "plaintext never leaves this
    /// machine"; inside a container that assumption is false, so the
    /// exposure has to be acknowledged explicitly.
    #[serde(default)]
    pub allow_plaintext_bind: bool,
    #[serde(default)]
    pub audience: Option<String>,
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    #[serde(default)]
    pub audit_file: Option<PathBuf>,
    #[serde(default)]
    pub audit_capacity: Option<usize>,
    #[serde(default)]
    pub tokens: Vec<TokenConfig>,
    #[serde(default)]
    pub policy: Vec<PolicyConfig>,
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
    /// Same-origin browser origin for the connected landing page.
    #[serde(default)]
    pub web_origin: Option<String>,
    /// Additional accepted browser origins. A single image is deployed under
    /// several domains (production plus one preview domain per pull request),
    /// and every Origin/CSRF check compares against this set, so the preview
    /// build does not need a different config than production.
    #[serde(default)]
    pub web_origins: Vec<String>,
    /// Static UI root for the connected landing page.
    #[serde(default)]
    pub web_root: Option<PathBuf>,
    /// Pre-authorized reverse-connect agents.
    #[serde(default)]
    pub agents: Vec<AgentConfig>,
    /// Local persistent root for the P1 blob backend
    /// (`conex-content::ContentStore`). When `None` the `/rpc` host disables
    /// `blob/*` and `p1_e2e` semantics still work via direct broker tests.
    #[serde(default)]
    pub content_root: Option<PathBuf>,
    /// Local persistent root for `conex-core::session::SessionStore`.
    #[serde(default)]
    pub session_root: Option<PathBuf>,
    /// Local persistent root for `conex-core::operation::OperationStore`.
    #[serde(default)]
    pub operation_root: Option<PathBuf>,
    /// Public origin the host expects reverse-connection agents to advertise
    /// in `agent/register.hostOrigin`. Defaults to `audience()` when unset.
    /// Only the prefix is checked; full PKI pinning is a future item.
    #[serde(default)]
    pub host_origin: Option<String>,
    /// Real OIDC verification (RS256 + issuer/audience/nonce + JWKS).
    /// When set, `/oidc/token` verifies a presented `idToken` signature
    /// instead of the dev-only code/PKCE pass-through.
    #[serde(default)]
    pub oidc: Option<OidcConfig>,
    /// Landing-page share scene. The root is separate from `content_root` on
    /// purpose: the blob channel garbage-collects what no committed root
    /// reaches, and an in-flight upload is exactly that, so the two scenes
    /// must not share a root or one would delete the other's staging.
    #[serde(default)]
    pub share: Option<ShareConfig>,
    /// Anonymous visitor sessions for the public landing page (M1.1). When
    /// set, browsers without a cookie get a read-only `ui` session bound to
    /// the configured principal/tenant; only the UI method whitelist
    /// (`endpoint/list` + `connection/list` + `source/*`) is reachable.
    /// Absent (`None`) keeps the host closed to anonymous visitors.
    #[serde(default)]
    pub web_guest: Option<WebGuestConfig>,
    /// S1 预安装契约与静态批准（docs/contracts/s1-capabilities.md §4）。
    /// 配置装载时校验：批准必须引用已安装契约，模块键全局唯一；未知即拒绝。
    /// 仅在 P1 后端（content/session/operation root）就绪时生效。
    #[serde(default)]
    pub capabilities: Option<CapabilityConfig>,
}

/// S1 静态批准与预安装契约（docs/plans/2026-10-17-conex-s1.md T02）。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityConfig {
    /// 随软件安装的契约描述（安装期数据，不是运行时上传）。
    #[serde(default)]
    pub contracts: Vec<ContractConfig>,
    /// Node 可发布的精确 `(spaceId, integrationId, providerEndpointId,
    /// contractId)` 元组；未声明即拒绝（默认 deny）。
    #[serde(default)]
    pub publish: Vec<PublishApprovalConfig>,
    /// Principal 可发现/调用/订阅的目标与契约成员；缺省表示整个契约。
    #[serde(default)]
    pub consume: Vec<ConsumeApprovalConfig>,
}

/// 预安装契约描述（§4、§6.1）：typed 方法与状态源。
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContractConfig {
    pub contract_id: String,
    #[serde(default)]
    pub methods: Vec<MethodConfig>,
    #[serde(default)]
    pub states: Vec<StateConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MethodConfig {
    pub name: String,
    pub input_schema: String,
    pub output_schema: String,
    pub action: String,
    /// 只接受 `read_only` 与 `non_replayable`；其他执行类别在装载时显式
    /// 拒绝，不允许悄悄降级成 read_only。
    pub execution: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateConfig {
    pub name: String,
    pub value_schema: String,
    pub action: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishApprovalConfig {
    pub node_id: String,
    pub space_id: String,
    pub integration_id: String,
    pub provider_endpoint_id: String,
    pub contract_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumeApprovalConfig {
    pub principal_id: String,
    pub space_id: String,
    pub provider_endpoint_id: String,
    pub contract_id: String,
    #[serde(default)]
    pub methods: Option<Vec<String>>,
    #[serde(default)]
    pub states: Option<Vec<String>>,
}

/// Anonymous visitor policy. Quotas here are independent of the per-principal
/// cap that bounds authenticated users.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebGuestConfig {
    /// Principal bound to every guest session. Must not collide with any
    /// `[[tokens]]` principal.
    #[serde(default = "default_guest_principal")]
    pub principal_id: String,
    /// Public display tenant. Every policy entry for the guest principal
    /// must use this tenant; contradictions are rejected at load.
    pub tenant_id: String,
    /// Global bound on live anonymous sessions.
    #[serde(default = "default_guest_max_sessions")]
    pub max_sessions: usize,
    /// Sliding idle expiry: each authenticated use extends the session.
    #[serde(default = "default_guest_idle_ttl_ms")]
    pub idle_ttl_ms: u64,
    /// Session issuance rate limit per rolling minute.
    #[serde(default = "default_guest_issue_per_minute")]
    pub max_issue_per_minute: u32,
}

/// The landing page file/clip scene: where bytes live and how much is allowed.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareConfig {
    /// Persistent root for scene bytes. Defaults to a sibling of
    /// `content_root`, so a host that already configures P1 gets a share root
    /// without a second line.
    #[serde(default)]
    pub share_root: Option<PathBuf>,
    /// How long a staged upload stays resumable.
    #[serde(default = "default_share_lease_ms")]
    pub lease_ms: u64,
    /// Largest single item one client may offer.
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
    /// Largest total one group may hold at once.
    #[serde(default = "default_max_group_bytes")]
    pub max_group_bytes: u64,
    /// Most files one client may offer at a time.
    #[serde(default = "default_max_files_per_client")]
    pub max_files_per_client: usize,
    /// Most clips one client may offer at once. Separate from the file count
    /// so a busy clipboard cannot take the file scene's slots.
    #[serde(default = "default_max_clips_per_client")]
    pub max_clips_per_client: usize,
    /// Clip lifetime in milliseconds.
    #[serde(default = "default_clip_ttl_ms")]
    pub clip_ttl_ms: u64,
}

fn default_share_lease_ms() -> u64 {
    24 * 60 * 60 * 1000
}
fn default_max_file_bytes() -> u64 {
    8 * 1024 * 1024
}
fn default_max_group_bytes() -> u64 {
    32 * 1024 * 1024
}
fn default_max_files_per_client() -> usize {
    32
}
fn default_max_clips_per_client() -> usize {
    20
}
fn default_clip_ttl_ms() -> u64 {
    10 * 60 * 1000
}

fn default_guest_principal() -> String {
    "guest".to_string()
}
fn default_guest_max_sessions() -> usize {
    64
}
fn default_guest_idle_ttl_ms() -> u64 {
    15 * 60 * 1000
}
fn default_guest_issue_per_minute() -> u32 {
    60
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OidcConfig {
    /// `iss` claim the host pins to (exact string).
    pub id_token_issuer: String,
    /// `aud` claim the host accepts (the host's OIDC client id).
    pub client_id: String,
    /// Optional `nonce` the browser flow must present.
    #[serde(default)]
    pub nonce: Option<String>,
    /// Inline JWKS document (`{"keys":[...]}`); `jwks_json` and `jwks_path`
    /// are alternatives, exactly one must be set.
    #[serde(default)]
    pub jwks_json: Option<String>,
    /// Path to a JWKS JSON file.
    #[serde(default)]
    pub jwks_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TlsConfig {
    pub cert: PathBuf,
    pub key: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenConfig {
    pub token_hash: String,
    pub principal_id: String,
    pub tenant_id: String,
    pub audience: String,
    #[serde(default = "default_token_role")]
    pub role: String,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub id: String,
    pub tenant_id: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub credential_name: Option<String>,
    #[serde(default)]
    pub credential_backend: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    pub principal_id: String,
    pub tenant_id: String,
    pub endpoint_id: String,
    pub actions: Vec<String>,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub subtree: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    pub id: String,
    pub tenant_id: String,
    pub provider_id: String,
    pub kind: String,
    pub provides: Vec<String>,
    #[serde(default)]
    pub root: Option<PathBuf>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub fixed_path: Option<String>,
    #[serde(default)]
    pub ca_pem: Option<PathBuf>,
    #[serde(default)]
    pub expected_server_name: Option<String>,
    #[serde(default)]
    pub max_response_bytes: Option<u64>,
    #[serde(default)]
    pub allow_loopback_http: bool,
    #[serde(default)]
    pub credential_name: Option<String>,
    #[serde(default)]
    pub credential_backend: Option<String>,
    /// For `source-remote`, the pre-authorized reverse-connect agent.
    #[serde(default)]
    pub agent_id: Option<String>,
}

impl HostConfig {
    pub fn load(path: &Path) -> Result<HostConfig, CallError> {
        let text = std::fs::read_to_string(path).map_err(invalid)?;
        let config: HostConfig = toml::from_str(&text).map_err(invalid)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), CallError> {
        let address: SocketAddr = self
            .listen
            .parse()
            .map_err(|_| invalid(format!("listen must be a socket address: {}", self.listen)))?;
        if let Some(guest) = &self.web_guest {
            if guest.principal_id.trim().is_empty() {
                return Err(invalid("web_guest.principal_id must not be empty"));
            }
            if guest.tenant_id.trim().is_empty() {
                return Err(invalid("web_guest.tenant_id must not be empty"));
            }
            if guest.max_sessions == 0 {
                return Err(invalid("web_guest.max_sessions must be at least 1"));
            }
            if guest.idle_ttl_ms == 0 {
                return Err(invalid("web_guest.idle_ttl_ms must be at least 1"));
            }
            if guest.max_issue_per_minute == 0 {
                return Err(invalid("web_guest.max_issue_per_minute must be at least 1"));
            }
            if self
                .tokens
                .iter()
                .any(|token| token.principal_id == guest.principal_id)
            {
                return Err(invalid(
                    "web_guest.principal_id must not collide with a token principal",
                ));
            }
            for policy in &self.policy {
                if policy.principal_id == guest.principal_id && policy.tenant_id != guest.tenant_id
                {
                    return Err(invalid(format!(
                        "web_guest.tenant_id {} contradicts policy tenant {} for {}",
                        guest.tenant_id, policy.tenant_id, guest.principal_id
                    )));
                }
            }
        }
        if let Some(share) = &self.share {
            if share.max_file_bytes == 0 {
                return Err(invalid("share.max_file_bytes must be at least 1"));
            }
            if share.max_files_per_client == 0 {
                return Err(invalid("share.max_files_per_client must be at least 1"));
            }
            if share.max_clips_per_client == 0 {
                return Err(invalid("share.max_clips_per_client must be at least 1"));
            }
            if share.clip_ttl_ms == 0 {
                return Err(invalid("share.clip_ttl_ms must be at least 1"));
            }
            if let Some((share_root, _)) = self.share_storage().as_ref()
                && self.content_root.as_ref() == Some(share_root)
            {
                return Err(invalid(
                    "share.share_root must not be content_root: the scenes reclaim differently",
                ));
            }
        }
        if let Some(origin) = &self.web_origin
            && !valid_web_origin(origin)
        {
            return Err(invalid(
                "web_origin must be a bare http:// or https:// origin without path, query, userinfo, or trailing slash",
            ));
        }
        for origin in &self.web_origins {
            if !valid_web_origin(origin) {
                return Err(invalid(format!(
                    "web_origins entry {origin} must be a bare http:// or https:// origin without path, query, userinfo, or trailing slash"
                )));
            }
        }
        let mut agent_ids = HashSet::new();
        for agent in &self.agents {
            if agent.id.trim().is_empty() {
                return Err(invalid("agent id must not be empty"));
            }
            if !agent_ids.insert(agent.id.clone()) {
                return Err(invalid(format!("duplicate agent {}", agent.id)));
            }
            let credential_name = agent
                .credential_name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| invalid(format!("agent {} requires credential_name", agent.id)))?;
            let credential_backend = agent
                .credential_backend
                .as_deref()
                .filter(|backend| valid_credential_source(backend))
                .ok_or_else(|| {
                    invalid(format!(
                        "agent {} requires credential_backend env:NAME or file:PATH",
                        agent.id
                    ))
                })?;
            let has_agent_identity = self
                .tokens
                .iter()
                .any(|token| token.role == "agent" && token.principal_id == agent.id);
            let has_matching_agent_token = self.tokens.iter().any(|token| {
                token.role == "agent"
                    && token.principal_id == agent.id
                    && token.tenant_id == agent.tenant_id
            });
            if !has_matching_agent_token {
                let reason = if has_agent_identity {
                    "tenant mismatch"
                } else {
                    "matching role=agent token"
                };
                return Err(invalid(format!("agent {} requires {}", agent.id, reason)));
            }
            let _ = (credential_name, credential_backend);
        }
        let mut seen = HashSet::new();
        for endpoint in &self.endpoints {
            if !seen.insert(endpoint.id.clone()) {
                return Err(invalid(format!("duplicate endpoint {}", endpoint.id)));
            }
            if !SUPPORTED_KINDS.contains(&endpoint.kind.as_str()) {
                return Err(invalid(format!("unknown factory kind {}", endpoint.kind)));
            }
            if endpoint.provides.is_empty() {
                return Err(invalid(format!(
                    "endpoint {} advertises no capabilities",
                    endpoint.id
                )));
            }
            match endpoint.kind.as_str() {
                "source-fs" if endpoint.root.is_none() => {
                    return Err(invalid(format!(
                        "source-fs endpoint {} requires root",
                        endpoint.id
                    )));
                }
                "source-http-catalog" if endpoint.origin.is_none() => {
                    return Err(invalid(format!(
                        "source-http-catalog endpoint {} requires origin",
                        endpoint.id
                    )));
                }
                "source-remote" => {
                    let agent_id = endpoint.agent_id.as_ref().ok_or_else(|| {
                        invalid(format!(
                            "source-remote endpoint {} requires agent_id",
                            endpoint.id
                        ))
                    })?;
                    let agent = self
                        .agents
                        .iter()
                        .find(|agent| agent.id == *agent_id)
                        .ok_or_else(|| {
                            invalid(format!(
                                "endpoint {} references unknown agent {}",
                                endpoint.id, agent_id
                            ))
                        })?;
                    if agent.tenant_id != endpoint.tenant_id {
                        return Err(invalid(format!(
                            "endpoint {} and agent {} must use the same tenant",
                            endpoint.id, agent_id
                        )));
                    }
                    if endpoint.root.is_none() {
                        return Err(invalid(format!(
                            "source-remote endpoint {} requires root",
                            endpoint.id
                        )));
                    }
                    if endpoint
                        .provides
                        .iter()
                        .any(|method| !SUPPORTED_REMOTE_METHODS.contains(&method.as_str()))
                    {
                        return Err(invalid(format!(
                            "unsupported remote method on endpoint {}",
                            endpoint.id
                        )));
                    }
                }
                _ => {}
            }
            if endpoint.kind != "source-remote" && endpoint.agent_id.is_some() {
                return Err(invalid(format!(
                    "endpoint {} agent_id is only valid for source-remote",
                    endpoint.id
                )));
            }
            if let Some(backend) = &endpoint.credential_backend {
                if !backend.starts_with("env:") && !backend.starts_with("file:") {
                    return Err(invalid("credential_backend must be env:NAME or file:PATH"));
                }
                if endpoint.credential_name.is_none() {
                    return Err(invalid("credential_backend requires credential_name"));
                }
            }
        }
        for policy in &self.policy {
            let endpoint = self
                .endpoints
                .iter()
                .find(|endpoint| endpoint.id == policy.endpoint_id)
                .ok_or_else(|| invalid(format!("unknown endpoint {}", policy.endpoint_id)))?;
            if endpoint.tenant_id != policy.tenant_id {
                return Err(invalid(format!(
                    "policy {} and endpoint {} must use the same tenant",
                    policy.principal_id, policy.endpoint_id
                )));
            }
        }
        for token in &self.tokens {
            if !SUPPORTED_TOKEN_ROLES.contains(&token.role.as_str()) {
                return Err(invalid(format!("unknown token role {}", token.role)));
            }
            let hash = token.token_hash.trim();
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(invalid("token_hash must be 64 hex characters"));
            }
        }
        if self.tls.is_none() && !self.allow_loopback_http && !self.allow_plaintext_bind {
            return Err(invalid(
                "plaintext requires allow_loopback_http = true or allow_plaintext_bind = true",
            ));
        }
        if self.allow_loopback_http && !address.ip().is_loopback() {
            return Err(invalid(
                "allow_loopback_http requires a loopback listen address",
            ));
        }
        if let Some(tls) = &self.tls {
            if !tls.cert.exists() {
                return Err(invalid(format!(
                    "tls cert not found: {}",
                    tls.cert.display()
                )));
            }
            if !tls.key.exists() {
                return Err(invalid(format!("tls key not found: {}", tls.key.display())));
            }
        }
        Ok(())
    }

    pub fn audience(&self) -> String {
        self.audience
            .clone()
            .unwrap_or_else(|| "conex-host".to_string())
    }

    pub fn host_origin(&self) -> String {
        self.host_origin.clone().unwrap_or_else(|| {
            // Reasonable default: treat the audience as the host origin in
            // dev. Production deployments should set `host_origin`.
            self.audience()
        })
    }

    pub fn p1_backends(&self) -> P1Backends {
        P1Backends {
            content: self.content_root.clone(),
            session: self.session_root.clone(),
            operation: self.operation_root.clone(),
        }
    }

    /// Share scene limits, with the host's own ceilings applied.
    pub fn share_limits(&self) -> crate::share::ShareLimits {
        let mut limits = crate::share::ShareLimits::default();
        if let Some(share) = &self.share {
            limits.max_file_bytes = share.max_file_bytes;
            // A group always has room for one file: raising the per-file cap
            // without touching the group cap would otherwise make the scene
            // refuse every upload, and the error would not say why.
            limits.max_group_bytes = share.max_group_bytes.max(share.max_file_bytes);
            limits.max_files_per_client = share.max_files_per_client;
            limits.max_clips_per_client = share.max_clips_per_client;
            limits.clip_ttl_ms = share.clip_ttl_ms;
        }
        limits
    }

    /// Share root plus the lease a staged upload keeps.
    ///
    /// `None` when neither `[share] share_root` nor `content_root` is set,
    /// which leaves the scene unavailable rather than writing to a path the
    /// operator never named.
    pub fn share_storage(&self) -> Option<(PathBuf, u64)> {
        let lease_ms = self
            .share
            .as_ref()
            .map_or(default_share_lease_ms(), |s| s.lease_ms);
        if let Some(root) = self
            .share
            .as_ref()
            .and_then(|share| share.share_root.clone())
        {
            return Some((root, lease_ms));
        }
        // A host that configures P1 gets a sibling share root by default.
        self.content_root.as_ref().map(|content| {
            (
                content
                    .parent()
                    .map(|parent| parent.join("share"))
                    .unwrap_or_else(|| content.join("share")),
                lease_ms,
            )
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct P1Backends {
    pub content: Option<PathBuf>,
    pub session: Option<PathBuf>,
    pub operation: Option<PathBuf>,
}

impl P1Backends {
    pub fn is_complete(&self) -> bool {
        self.content.is_some() && self.session.is_some() && self.operation.is_some()
    }
}

fn default_token_role() -> String {
    "service".to_string()
}

fn invalid(message: impl std::fmt::Display) -> CallError {
    CallError::new(conex_proto::ErrorCode::Internal, message.to_string())
}
fn valid_web_origin(origin: &str) -> bool {
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    let Some(authority) = authority else {
        return false;
    };
    if authority.is_empty()
        || authority.contains(['/', '?', '#', '@'])
        || authority.chars().any(char::is_whitespace)
    {
        return false;
    }
    if authority.starts_with('[') {
        let Some(end) = authority.find(']') else {
            return false;
        };
        if end == 1 {
            return false;
        }
        return authority[end + 1..]
            .strip_prefix(':')
            .map_or(authority.len() == end + 1, |port| {
                !port.is_empty() && port.parse::<u16>().is_ok()
            });
    }
    if authority.matches(':').count() > 1 {
        return false;
    }
    let (host, port) = authority
        .split_once(':')
        .map_or((authority, None), |(host, port)| (host, Some(port)));
    !host.is_empty() && port.is_none_or(|port| !port.is_empty() && port.parse::<u16>().is_ok())
}

fn valid_credential_source(source: &str) -> bool {
    source
        .strip_prefix("env:")
        .or_else(|| source.strip_prefix("file:"))
        .is_some_and(|value| !value.trim().is_empty())
}

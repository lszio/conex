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
    /// Anonymous visitor sessions for the public landing page (M1.1). When
    /// set, browsers without a cookie get a read-only `ui` session bound to
    /// the configured principal/tenant; only the UI method whitelist
    /// (`endpoint/list` + `connection/list` + `source/*`) is reachable.
    /// Absent (`None`) keeps the host closed to anonymous visitors.
    #[serde(default)]
    pub web_guest: Option<WebGuestConfig>,
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

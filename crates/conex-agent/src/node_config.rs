//! Administrator-owned node environment, never supplied by an application package.
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    pub node_id: String,
    pub node_ref: String,
    pub operator_principal: String,
    pub data_dir: PathBuf,
    pub local_socket: PathBuf,
    pub applications: Vec<PathBuf>,
    pub permissions: BTreeSet<String>,
    #[serde(default)]
    pub configs: BTreeMap<String, ApprovedCounterConfig>,
    #[serde(default)]
    pub remote_endpoints: Vec<ApprovedRemoteConfig>,
    pub publication: PublicationConfig,
    /// P5 private access. Present = the node verifies hub grants and serves
    /// the direct-connection entry; absent keeps the P2 local-only behavior.
    #[serde(default)]
    pub private: Option<NodePrivateConfig>,
    /// P3 OMP adapter backend. Present = the "omp" implementation is installable.
    #[serde(default)]
    pub omp: Option<OmpAdapterConfig>,
    /// P4 Codex adapter backend. Present = the "codex" implementation is
    /// installable (owned stdio app-server form).
    #[serde(default)]
    pub codex: Option<CodexAdapterConfig>,
    /// P6 metrics module. Present = the "node-metrics" implementation is
    /// installable.
    #[serde(default)]
    pub metrics: Option<MetricsModuleConfig>,
}

/// 指标模块配置（P6.1）。采样间隔与保留上限当前为编译期常量（有界）；
/// 配置位留作批准项扩展。
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricsModuleConfig {
    #[serde(default)]
    pub enabled: bool,
}

/// Codex 适配器批准配置（docs/plans/2026-10-31-conex-mvp-p4.md §1）。
/// approvalPolicy/sandbox 全部来自批准配置（契约 §4 禁止客户端覆盖审批策略）。
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexAdapterConfig {
    pub backend_binary: String,
    /// e.g. "untrusted" / "on-request"; forwarded to thread/start.
    #[serde(default)]
    pub approval_policy: Option<String>,
    /// e.g. "read-only" / "workspace-write".
    #[serde(default)]
    pub sandbox: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<ApprovedWorkspace>,
}

/// OMP 适配器批准配置（docs/plans/2026-10-31-conex-mvp-p3.md §1）。二进制路径、
/// Workspace 真实根目录与并发上限全部由管理员固定；客户端 payload 不传任何
/// 启动参数（契约 §4）。
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OmpAdapterConfig {
    /// Approved binary (absolute path, or a name resolved via PATH at spawn).
    pub backend_binary: String,
    /// Approved model ("provider/model", exact). Ambient OMP auth is used;
    /// clients cannot select or override models.
    #[serde(default)]
    pub model: Option<String>,
    /// Optional tool allowlist passed as --tools (comma-separated). Absent =
    /// OMP default toolset; client payloads never choose tools.
    #[serde(default)]
    pub tools: Option<String>,
    /// Session approval mode. yolo and auto-approve are refused by validation:
    /// the adapter never runs an auto-approving backend.
    #[serde(default)]
    pub approval_mode: Option<String>,
    /// Concurrent owned OMP processes; new sessions beyond this fail
    /// unavailable instead of preempting an active session.
    #[serde(default)]
    pub max_concurrent_sessions: usize,
    /// rootRef key -> real root path. Real paths never leave the node.
    #[serde(default)]
    pub workspaces: Vec<ApprovedWorkspace>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedWorkspace {
    /// rootRef key clients reference; not a path.
    pub id: String,
    pub display_name: String,
    pub root: PathBuf,
}

/// 节点私有访问配置（docs/plans/2026-10-29-conex-mvp-p5.md §3/P5.3）：
/// 中心公钥允许表、直连地址与批准 Origin 都由管理员固定，不来自中心下发。
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodePrivateConfig {
    /// Grant issuer the node pins; grants from any other issuer are refused.
    pub hub_issuer: String,
    /// The private instance tenant; a valid grant with another tenant is a
    /// mismatch, rejected before any resource check.
    pub tenant: String,
    /// kid -> raw Ed25519 public key (hex). Installed by the administrator;
    /// nothing here is fetched from the network.
    pub approved_public_keys: BTreeMap<String, String>,
    /// Direct-entry listen address. Loopback by construction: a non-loopback
    /// address additionally requires `allow_remote_direct` and TLS.
    pub direct_listen: String,
    /// Exact web origins allowed to fetch direct tickets (CORS + Origin).
    #[serde(default)]
    pub approved_web_origins: Vec<String>,
    /// Explicit acknowledgement that the direct entry may leave this machine.
    #[serde(default)]
    pub allow_remote_direct: bool,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedCounterConfig {
    /// An external controller is a reference, never owned or terminated by Node.
    pub external_socket: Option<PathBuf>,
    pub external_token_backend: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovedRemoteConfig {
    pub node_ref: String,
    pub provider_endpoint_ref: String,
    pub contract_id: String,
    pub space_id: String,
    pub provider_endpoint_id: String,
    pub registration_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicationConfig {
    pub url: String,
    /// Private nodes keep the publication link even with nothing to publish:
    /// the link is how the hub learns the runtime epoch and how revocations
    /// propagate. Set by Node::boot when [private] is configured.
    #[serde(default)]
    pub always_link: bool,
    pub token_backend: String,
    pub space_id: String,
    pub integration_id: String,
    pub endpoint_prefix: String,
    #[serde(default)]
    pub allow_loopback_ws: bool,
    pub exposure_principals: BTreeSet<String>,
    pub exposure_tenants: BTreeSet<String>,
}

impl NodeConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|error| format!("node config read: {error}"))?;
        if bytes.len() > 256 * 1024 {
            return Err("node config too large".into());
        }
        let text = std::str::from_utf8(&bytes).map_err(|error| error.to_string())?;
        let config: Self = toml::from_str(text).map_err(|error: toml::de::Error| {
            format!("node config TOML parse failed at bytes {:?}", error.span())
        })?;
        for id in [&config.node_id, &config.operator_principal] {
            if !conex_proto::wire::is_ulid(id) {
                return Err("node_id and operator_principal require canonical ULIDs".into());
            }
        }
        if config.node_ref.is_empty()
            || config.applications.is_empty()
            || config.applications.len() > 64
        {
            return Err("node requires node_ref and 1..64 approved applications".into());
        }
        if config.data_dir.as_os_str().is_empty()
            || !config.local_socket.starts_with(&config.data_dir)
        {
            return Err("local socket must be inside the private data directory".into());
        }
        // Reuse the deployed transport's ws/wss and approved secret-source checks.
        let transport = crate::config::AgentConfig {
            agent_id: config.node_id.clone(),
            tenant_id: "node".into(),
            host_url: config.publication.url.clone(),
            token_name: "node-publication".into(),
            token_backend: config.publication.token_backend.clone(),
            ca_pem: None,
            expected_server_name: None,
            host_origin: None,
            allow_loopback_ws: config.publication.allow_loopback_ws,
            endpoints: Vec::new(),
        };
        transport.validate()?;
        for value in config.configs.values() {
            if value.external_socket.is_some() != value.external_token_backend.is_some() {
                return Err("external counter needs both approved socket and token backend".into());
            }
        }
        if let Some(private) = &config.private {
            if private.hub_issuer.trim().is_empty() || private.tenant.trim().is_empty() {
                return Err("node private entry requires hub issuer and tenant".into());
            }
            if private.approved_public_keys.is_empty() {
                return Err("node private entry requires at least one approved public key".into());
            }
            for (kid, key) in &private.approved_public_keys {
                if kid.is_empty() {
                    return Err("approved public key kid must not be empty".into());
                }
                if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(format!(
                        "approved public key {kid} must be 64 hex characters (raw Ed25519)"
                    ));
                }
            }
            let listen: std::net::SocketAddr = private
                .direct_listen
                .parse()
                .map_err(|_| "node direct_listen must be a socket address".to_string())?;
            if !listen.ip().is_loopback() && !private.allow_remote_direct {
                return Err(
                    "non-loopback direct_listen requires allow_remote_direct and TLS".into(),
                );
            }
            for origin in &private.approved_web_origins {
                if !is_valid_origin(origin) {
                    return Err(format!(
                        "approved_web_origins entry {origin} must be a bare http:// or https:// origin"
                    ));
                }
            }
        }
        if let Some(codex) = &config.codex {
            if codex.backend_binary.trim().is_empty() {
                return Err("codex backend_binary must not be empty".into());
            }
            let mut seen = std::collections::BTreeSet::new();
            for workspace in &codex.workspaces {
                if workspace.id.is_empty()
                    || workspace.display_name.is_empty()
                    || !seen.insert(workspace.id.clone())
                {
                    return Err("codex workspace ids must be non-empty and unique".into());
                }
                if !workspace.root.is_absolute() {
                    return Err("codex workspace root must be an absolute path".into());
                }
            }
        }
        if let Some(omp) = &config.omp {
            if omp.backend_binary.trim().is_empty() {
                return Err("omp backend_binary must not be empty".into());
            }
            if omp.max_concurrent_sessions == 0 || omp.max_concurrent_sessions > 16 {
                return Err("omp max_concurrent_sessions must be 1..=16".into());
            }
            if let Some(model) = &omp.model
                && (!model.contains('/') || model.trim() != model || model.is_empty())
            {
                return Err("omp model must be an exact \"provider/model\" string".into());
            }
            if let Some(mode) = &omp.approval_mode
                && !matches!(mode.as_str(), "always-ask" | "always-ask-cached" | "write")
            {
                return Err(
                    "omp approval_mode must be always-ask, always-ask-cached or write; \
                     auto-approving modes are refused"
                        .into(),
                );
            }
            let mut seen = std::collections::BTreeSet::new();
            for workspace in &omp.workspaces {
                if workspace.id.is_empty()
                    || workspace.display_name.is_empty()
                    || !seen.insert(workspace.id.clone())
                {
                    return Err("omp workspace ids must be non-empty and unique".into());
                }
                if !workspace.root.is_absolute() {
                    return Err("omp workspace root must be an absolute path".into());
                }
            }
        }
        Ok(config)
    }
}

/// Same shape as the host's origin check: a bare scheme+authority string.
fn is_valid_origin(origin: &str) -> bool {
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"));
    let Some(authority) = authority else {
        return false;
    };
    !authority.is_empty()
        && !authority.contains(['/', '?', '#', '@'])
        && authority.chars().all(|c| !c.is_whitespace())
}

pub fn read_secret(backend: &str) -> Result<String, String> {
    let secret = if let Some(name) = backend.strip_prefix("env:") {
        std::env::var(name)
            .map_err(|_| "approved credential environment variable missing".to_owned())?
    } else if let Some(path) = backend.strip_prefix("file:") {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|_| "approved credential file unavailable".to_owned())?;
        if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.len() > 4096 {
            return Err("credential file must be regular, <=4096 bytes and 0600".into());
        }
        std::fs::read_to_string(path)
            .map_err(|_| "approved credential file unreadable".to_owned())?
    } else {
        return Err("credential source requires env:NAME or file:PATH".into());
    };
    let secret = secret.trim().to_owned();
    if secret.is_empty() || secret.len() > 4096 {
        return Err("invalid credential size".into());
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    #[test]
    fn invalid_node_config_does_not_echo_secret_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.toml");
        let secret = "private-node-config-canary-26aa";
        std::fs::write(&path, format!("secret='{secret}'\n")).unwrap();
        let error = super::NodeConfig::load(&path)
            .err()
            .expect("unknown field rejected");
        assert!(!error.contains(secret));
    }
}

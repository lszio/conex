//! Installation config. Only file paths, profile ids and credential references;
//! no executable fields are accepted (serde deny_unknown_fields).
use std::collections::HashSet;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use conex_core::CallError;
use conex_proto::v1;
use serde::Deserialize;

pub const SUPPORTED_KINDS: [&str; 2] = ["source-fs", "source-http-catalog"];

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfig {
    pub listen: String,
    #[serde(default)]
    pub allow_loopback_http: bool,
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
                _ => {}
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
        for token in &self.tokens {
            let hash = token.token_hash.trim();
            if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(invalid("token_hash must be 64 hex characters"));
            }
        }
        if self.tls.is_none() && !self.allow_loopback_http {
            return Err(invalid("plaintext requires allow_loopback_http = true"));
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
}

fn invalid(message: impl std::fmt::Display) -> CallError {
    CallError::new(v1::ErrorCode::Internal, message.to_string())
}

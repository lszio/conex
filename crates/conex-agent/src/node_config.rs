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
        Ok(config)
    }
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

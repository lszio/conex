use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

const SUPPORTED_METHODS: [&str; 3] = ["source/list", "source/read", "source/search"];
fn default_resources() -> Vec<String> {
    vec!["*".into()]
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub agent_id: String,
    pub tenant_id: String,
    pub host_url: String,
    pub token_name: String,
    pub token_backend: String,
    #[serde(default)]
    pub ca_pem: Option<PathBuf>,
    #[serde(default)]
    pub expected_server_name: Option<String>,
    #[serde(default)]
    pub host_origin: Option<String>,
    #[serde(default)]
    pub allow_loopback_ws: bool,
    #[serde(default)]
    pub endpoints: Vec<EndpointConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    pub endpoint_id: String,
    pub root: PathBuf,
    pub methods: Vec<String>,
    #[serde(default = "default_resources")]
    pub resources: Vec<String>,
}

impl AgentConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
        let config: Self = toml::from_str(&text).map_err(|error| error.to_string())?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.agent_id.trim().is_empty() {
            return Err("agent_id must not be empty".into());
        }
        if self.tenant_id.trim().is_empty() {
            return Err("tenant_id must not be empty".into());
        }
        if self.token_name.trim().is_empty() {
            return Err("token_name must not be empty".into());
        }
        if !valid_token_source(&self.token_backend) {
            return Err("token_backend must be env:NAME or file:PATH".into());
        }
        let scheme = if self.host_url.starts_with("wss://") {
            Some("wss://")
        } else if self.host_url.starts_with("ws://") {
            Some("ws://")
        } else {
            None
        };
        let Some(scheme) = scheme else {
            return Err("host_url must use wss://, except explicit loopback ws://".into());
        };
        if scheme == "ws://" && !(self.allow_loopback_ws && loopback_ws_authority(&self.host_url))
        {
            return Err("host_url ws:// is allowed only for explicit loopback".into());
        }
        let mut ids = HashSet::new();
        for endpoint in &self.endpoints {
            if endpoint.endpoint_id.trim().is_empty() {
                return Err("endpoint_id must not be empty".into());
            }
            if !ids.insert(endpoint.endpoint_id.as_str()) {
                return Err(format!("duplicate endpoint {}", endpoint.endpoint_id));
            }
            if endpoint.root.as_os_str().is_empty() {
                return Err(format!("endpoint {} requires root", endpoint.endpoint_id));
            }
            if endpoint.methods.is_empty() {
                return Err(format!("endpoint {} must declare methods", endpoint.endpoint_id));
            }
            if endpoint
                .methods
                .iter()
                .any(|method| !SUPPORTED_METHODS.contains(&method.as_str()))
            {
                return Err(format!("unsupported method on endpoint {}", endpoint.endpoint_id));
            }
            if endpoint.resources.is_empty()
                || endpoint
                    .resources
                    .iter()
                    .any(|resource| resource != "*" && (resource.starts_with('/') || resource.split('/').any(|part| part == "..")))
            {
                return Err(format!("endpoint {} has invalid resources", endpoint.endpoint_id));
            }
        }
        Ok(())
    }
}

fn valid_token_source(source: &str) -> bool {
    source
        .strip_prefix("env:")
        .or_else(|| source.strip_prefix("file:"))
        .is_some_and(|value| !value.trim().is_empty())
}

fn loopback_ws_authority(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("ws://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let (host, port) = if authority.starts_with('[') {
        let Some(end) = authority.find(']') else {
            return false;
        };
        let suffix = &authority[end + 1..];
        let port = if suffix.is_empty() {
            true
        } else {
            suffix
                .strip_prefix(':')
                .is_some_and(|value| !value.is_empty() && value.parse::<u16>().is_ok())
        };
        (&authority[..=end], port)
    } else if authority.matches(':').count() <= 1 {
        let (host, port) = authority
            .split_once(':')
            .map_or((authority, true), |(host, port)| {
                (host, !port.is_empty() && port.parse::<u16>().is_ok())
            });
        (host, port)
    } else {
        return false;
    };
    port && matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

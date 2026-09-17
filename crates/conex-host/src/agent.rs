//! P1-05 reverse-connect agent registry + P1-09 browser entry helpers.
//!
//! Both are kept in-memory by design: this slice proves the wire surface
//! (provider registration, capability listing, ticket issuance) and the
//! OIDC code+PKCE redirect shape. The full reverse-connect transport
//! (`crates/conex-agent`) ships as a separate binary in P2; for P1 the
//! host accepts registration in-process over WSS so end-to-end behaviour is
//! verifiable.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use conex_core::CallError;
use conex_proto::v1;

use crate::broker::{Broker, BrokerCall};

const TICKET_TTL_MS: u64 = 30_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRegistration {
    pub agent_id: String,
    pub principal_id: String,
    pub tenant_id: String,
    pub provider_ids: Vec<String>,
    pub methods: Vec<String>,
    pub resources: Vec<String>,
    pub registered_at_ms: u64,
    pub last_heartbeat_at_ms: u64,
    pub host_origin: String, // expected host binding (anti-spoof)
}

#[derive(Debug, Default)]
pub struct AgentRegistry {
    inner: Mutex<HashMap<String, AgentRegistration>>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, agent: AgentRegistration) -> Result<(), AgentError> {
        if !is_safe_component(&agent.agent_id)
            || !is_safe_component(&agent.principal_id)
            || !is_safe_component(&agent.tenant_id)
        {
            return Err(AgentError::InvalidIdentifier);
        }
        if agent.methods.is_empty() {
            return Err(AgentError::NoCapabilities);
        }
        if agent.host_origin.is_empty() || !agent.host_origin.starts_with("conex://") {
            return Err(AgentError::InvalidHost);
        }
        let mut guard = self.inner.lock().expect("agent registry poisoned");
        guard.insert(agent.agent_id.clone(), agent);
        Ok(())
    }

    pub fn heartbeat(&self, agent_id: &str) -> Result<u64, AgentError> {
        let mut guard = self.inner.lock().expect("agent registry poisoned");
        let entry = guard.get_mut(agent_id).ok_or(AgentError::Unknown)?;
        entry.last_heartbeat_at_ms = now_ms();
        Ok(entry.last_heartbeat_at_ms)
    }

    pub fn list(&self) -> Vec<AgentRegistration> {
        let guard = self.inner.lock().expect("agent registry poisoned");
        let mut list: Vec<AgentRegistration> = guard.values().cloned().collect();
        list.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        list
    }

    pub fn resolve(
        &self,
        provider_id: &str,
        resource_id: &str,
        method: &str,
    ) -> Vec<AgentRegistration> {
        let guard = self.inner.lock().expect("agent registry poisoned");
        guard
            .values()
            .filter(|agent| {
                agent.provider_ids.iter().any(|id| id == provider_id)
                    && agent.resources.iter().any(|id| id == resource_id)
                    && agent.methods.iter().any(|id| id == method)
            })
            .cloned()
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("invalid identifier (must be alphanumeric/-/_ )")]
    InvalidIdentifier,
    #[error("agent must advertise at least one capability")]
    NoCapabilities,
    #[error("host binding must be a conex:// origin")]
    InvalidHost,
    #[error("unknown agent")]
    Unknown,
}

impl AgentError {
    fn into_call(self) -> CallError {
        let code = match &self {
            AgentError::Unknown => v1::ErrorCode::UnknownProvider,
            _ => v1::ErrorCode::BadRequest,
        };
        CallError::new(code, format!("{self}"))
    }
}

#[derive(Debug, Clone)]
pub struct WebTicket {
    pub ticket: String,
    pub principal_id: String,
    pub tenant_id: String,
    pub origin: String,
    pub target_host: String,
    pub peer_role: String,
    pub capability_caps: Vec<String>,
    pub session_id: Option<String>,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Default)]
pub struct TicketRegistry {
    inner: Mutex<HashMap<String, WebTicket>>,
}

impl TicketRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        &self,
        principal_id: &str,
        tenant_id: &str,
        origin: &str,
        target_host: &str,
        peer_role: &str,
        capability_caps: Vec<String>,
        session_id: Option<String>,
    ) -> Result<WebTicket, CallError> {
        if origin.is_empty() || !origin.starts_with("http") {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "origin must be a non-empty http(s) URL",
            ));
        }
        if !is_safe_component(principal_id) || !is_safe_component(tenant_id) {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "principal/tenant must be alphanumeric/-/_",
            ));
        }
        if !["ui", "agent"].contains(&peer_role) {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                format!("peer_role {peer_role} not allowed"),
            ));
        }
        let now = now_ms();
        let issued_at_ms = now;
        let expires_at_ms = now + TICKET_TTL_MS;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(principal_id.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(tenant_id.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(origin.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(target_host.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(issued_at_ms.to_string().as_bytes());
        let ticket =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(&bytes));
        let ticket = WebTicket {
            ticket,
            principal_id: principal_id.to_string(),
            tenant_id: tenant_id.to_string(),
            origin: origin.to_string(),
            target_host: target_host.to_string(),
            peer_role: peer_role.to_string(),
            capability_caps,
            session_id,
            issued_at_ms,
            expires_at_ms,
        };
        let mut guard = self.inner.lock().expect("ticket registry poisoned");
        guard.insert(ticket.ticket.clone(), ticket.clone());
        Ok(ticket)
    }

    pub fn consume(&self, ticket: &str) -> Result<WebTicket, CallError> {
        let mut guard = self.inner.lock().expect("ticket registry poisoned");
        let entry = guard.remove(ticket).ok_or_else(|| {
            CallError::new(
                v1::ErrorCode::Unauthorized,
                "ticket is unknown or already consumed",
            )
        })?;
        if entry.expires_at_ms < now_ms() {
            return Err(CallError::new(
                v1::ErrorCode::Unauthorized,
                "ticket has expired",
            ));
        }
        Ok(entry)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OidcCode {
    pub code: String,
    pub principal_id: String,
    pub tenant_id: String,
    pub audience: String,
    pub origin: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Default)]
pub struct OidcRegistry {
    inner: Mutex<HashMap<String, OidcCode>>,
    ttl_ms: u64,
}

impl OidcRegistry {
    pub fn new() -> Self {
        Self {
            ttl_ms: 60_000,
            ..Default::default()
        }
    }

    pub fn with_ttl(mut self, ttl_ms: u64) -> Self {
        self.ttl_ms = ttl_ms;
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        &self,
        principal_id: &str,
        tenant_id: &str,
        audience: &str,
        origin: &str,
        code_challenge: &str,
        code_challenge_method: &str,
    ) -> Result<OidcCode, CallError> {
        if !is_safe_component(principal_id) || !is_safe_component(tenant_id) {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "principal/tenant must be alphanumeric/-/_",
            ));
        }
        if code_challenge_method != "S256" {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "code_challenge_method must be S256",
            ));
        }
        if code_challenge.len() < 32 {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "code_challenge must be at least 32 bytes",
            ));
        }
        let now = now_ms();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(principal_id.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(origin.as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(code_challenge.as_bytes());
        bytes.extend_from_slice(now.to_string().as_bytes());
        let code = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(&bytes));
        let entry = OidcCode {
            code,
            principal_id: principal_id.to_string(),
            tenant_id: tenant_id.to_string(),
            audience: audience.to_string(),
            origin: origin.to_string(),
            code_challenge: code_challenge.to_string(),
            code_challenge_method: code_challenge_method.to_string(),
            issued_at_ms: now,
            expires_at_ms: now + self.ttl_ms,
        };
        self.inner
            .lock()
            .expect("oidc registry poisoned")
            .insert(entry.code.clone(), entry.clone());
        Ok(entry)
    }

    pub fn exchange(
        &self,
        code: &str,
        code_verifier: &str,
        origin: &str,
    ) -> Result<OidcCode, CallError> {
        let mut guard = self.inner.lock().expect("oidc registry poisoned");
        let entry = guard.remove(code).ok_or_else(|| {
            CallError::new(
                v1::ErrorCode::Unauthorized,
                "oidc code is unknown or already consumed",
            )
        })?;
        if entry.expires_at_ms < now_ms() {
            return Err(CallError::new(
                v1::ErrorCode::Unauthorized,
                "oidc code has expired",
            ));
        }
        if entry.origin != origin {
            return Err(CallError::new(
                v1::ErrorCode::Unauthorized,
                "origin does not match the issued oidc code",
            ));
        }
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(code_verifier.as_bytes()));
        if challenge != entry.code_challenge {
            return Err(CallError::new(
                v1::ErrorCode::Unauthorized,
                "code_verifier does not match the challenge",
            ));
        }
        Ok(entry)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn is_safe_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

pub fn object(input: &Value) -> Result<&Map<String, Value>, CallError> {
    input
        .as_object()
        .ok_or_else(|| CallError::new(v1::ErrorCode::BadRequest, "input must be an object"))
}

pub fn require_string<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a str, CallError> {
    map.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| CallError::new(v1::ErrorCode::BadRequest, format!("{key} required")))
}

pub fn optional_string<'a>(
    map: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, CallError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(CallError::new(
            v1::ErrorCode::BadRequest,
            format!("{key} must be a string"),
        )),
    }
}

pub fn string_list(map: &Map<String, Value>, key: &str) -> Result<Vec<String>, CallError> {
    let arr = map
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| CallError::new(v1::ErrorCode::BadRequest, format!("{key} required")))?;
    arr.iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                CallError::new(
                    v1::ErrorCode::BadRequest,
                    format!("{key} entries must be strings"),
                )
            })
        })
        .collect()
}

/// Dispatch agent wire methods to the in-process registry. The handler runs
/// inside the broker; audit/limits are wrapped in `BrokerDeps` once the
/// unified P1 audit sink lands.
pub async fn handle(broker: &Broker, call: BrokerCall) -> Result<Value, CallError> {
    let map = match call.input.as_object() {
        Some(map) => map,
        None => {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "agent input must be an object",
            ));
        }
    };
    match call.method.as_str() {
        "agent/register" => {
            let host_origin = require_string(map, "hostOrigin")?.to_string();
            if let Some(expected) = broker.deps().host_origin.as_deref() {
                if host_origin != expected {
                    return Err(CallError::new(
                        v1::ErrorCode::Forbidden,
                        format!("agent hostOrigin {host_origin} does not match host {expected}"),
                    ));
                }
            } else if !host_origin.starts_with("conex://") {
                return Err(CallError::new(
                    v1::ErrorCode::BadRequest,
                    "agent hostOrigin must start with conex://",
                ));
            }
            let registration = AgentRegistration {
                agent_id: require_string(map, "agentId")?.to_string(),
                principal_id: call.caller.principal_id.clone(),
                tenant_id: call.caller.tenant_id.clone(),
                provider_ids: string_list(map, "providerIds")?,
                methods: string_list(map, "methods")?,
                resources: string_list(map, "resources")?,
                registered_at_ms: now_ms(),
                last_heartbeat_at_ms: now_ms(),
                host_origin,
            };
            broker_agents(broker)
                .register(registration)
                .map_err(|e| e.into_call())?;
            Ok(json!({ "agentId": map.get("agentId").and_then(Value::as_str) }))
        }
        "agent/heartbeat" => {
            let agent_id = require_string(map, "agentId")?.to_string();
            let ts = broker_agents(broker)
                .heartbeat(&agent_id)
                .map_err(|e| e.into_call())?;
            Ok(json!({ "agentId": agent_id, "heartbeatAtMs": ts.to_string() }))
        }
        "agent/resolve" => {
            let provider_id = require_string(map, "providerId")?.to_string();
            let resource_id = require_string(map, "resourceId")?.to_string();
            let method = require_string(map, "method")?.to_string();
            let agents = broker_agents(broker).resolve(&provider_id, &resource_id, &method);
            let names: Vec<String> = agents.into_iter().map(|a| a.agent_id).collect();
            Ok(json!({ "agents": names }))
        }
        other => Err(CallError::new(
            v1::ErrorCode::UnknownMethod,
            format!("unknown agent method {other}"),
        )),
    }
}

pub fn broker_agents(broker: &Broker) -> Arc<AgentRegistry> {
    broker
        .deps()
        .agents
        .clone()
        .expect("host always installs an agent registry")
}

#[derive(Clone)]
pub struct HostSide {
    pub agents: Arc<AgentRegistry>,
    pub tickets: Arc<TicketRegistry>,
    pub oidc: Arc<OidcRegistry>,
    /// Real RS256 id_token verifier. `Some` when `[oidc]` is configured;
    /// `/oidc/token` then verifies presented id_tokens instead of the
    /// dev-only code/PKCE pass-through.
    pub verifier: Option<Arc<crate::oidc_jwt::OidcVerifier>>,
}

impl HostSide {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(AgentRegistry::new()),
            tickets: Arc::new(TicketRegistry::new()),
            oidc: Arc::new(OidcRegistry::new()),
            verifier: None,
        }
    }

    pub fn with_verifier(mut self, verifier: Arc<crate::oidc_jwt::OidcVerifier>) -> Self {
        self.verifier = Some(verifier);
        self
    }
}

impl Default for HostSide {
    fn default() -> Self {
        Self::new()
    }
}

pub fn _unused_keep_duration(_: Duration) {}

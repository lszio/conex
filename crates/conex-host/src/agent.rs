//! Reverse-connect agent registry and browser entry helpers.
//!
//! Agent links are authenticated at the WSS upgrade and fenced by generation:
//! reconnecting the same identity replaces the old owner, while old cleanup
//! and heartbeats can never affect the replacement.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use conex_core::CallError;
use conex_proto;

use crate::broker::{Broker, BrokerCall};
use crate::remote::{RemoteConnections, RemoteLink};

const TICKET_TTL_MS: u64 = 30_000;

/// Per-endpoint registration claim (plan M2): the agent declares each
/// endpoint it serves; the host validates each claim against its own config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EndpointRegistration {
    pub endpoint_id: String,
    /// Resource root the agent claims to serve; `*` serves the whole
    /// authorized root. The host's configured root always wins.
    pub root: String,
    pub methods: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRegistration {
    pub agent_id: String,
    pub principal_id: String,
    pub tenant_id: String,
    /// Accepted endpoints of the current generation, validated against the
    /// host-side authorization.
    pub endpoints: Vec<EndpointRegistration>,
    pub registered_at_ms: u64,
    pub last_heartbeat_at_ms: u64,
    pub host_origin: String, // expected host binding (anti-spoof)
}

impl AgentRegistration {
    /// Flat views kept for resolve and the connection panel projection.
    pub fn provider_ids(&self) -> Vec<String> {
        self.endpoints
            .iter()
            .map(|e| e.endpoint_id.clone())
            .collect()
    }

    pub fn methods(&self) -> Vec<String> {
        let mut methods: Vec<String> = self
            .endpoints
            .iter()
            .flat_map(|e| e.methods.iter().cloned())
            .collect();
        methods.sort();
        methods.dedup();
        methods
    }

    pub fn resources(&self) -> Vec<String> {
        let mut roots: Vec<String> = self.endpoints.iter().map(|e| e.root.clone()).collect();
        roots.sort();
        roots.dedup();
        roots
    }

    pub fn accepts_endpoint(&self, endpoint_id: &str) -> bool {
        self.endpoints.iter().any(|e| e.endpoint_id == endpoint_id)
    }
}

/// Host-configured authorization for one agent endpoint (plan M2): the
/// endpoint id must exist in config, the configured root scopes it, and
/// claimed methods must be a subset.
#[derive(Debug, Clone)]
pub struct AuthorizedEndpoint {
    pub endpoint_id: String,
    pub root: String,
    pub methods: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentAuthorization {
    pub agent_id: String,
    pub tenant_id: String,
    pub endpoints: Vec<AuthorizedEndpoint>,
}

impl AgentAuthorization {
    /// Per-endpoint review (plan M2): every claimed endpoint is validated
    /// against the host config independently; invalid ones are rejected and
    /// reported while valid ones are accepted. The host's configured root
    /// always wins over the agent's claim.
    fn review(
        &self,
        registration: &AgentRegistration,
    ) -> (Vec<EndpointRegistration>, Vec<(String, &'static str)>) {
        if self.agent_id != registration.agent_id || self.tenant_id != registration.tenant_id {
            return (Vec::new(), Vec::new());
        }
        let mut accepted = Vec::new();
        let mut rejected = Vec::new();
        for claimed in &registration.endpoints {
            let Some(authorized) = self
                .endpoints
                .iter()
                .find(|authorized| authorized.endpoint_id == claimed.endpoint_id)
            else {
                rejected.push((
                    claimed.endpoint_id.clone(),
                    "endpoint is not configured for this agent",
                ));
                continue;
            };
            if claimed.methods.is_empty()
                || claimed
                    .methods
                    .iter()
                    .any(|method| !authorized.methods.contains(method))
            {
                rejected.push((
                    claimed.endpoint_id.clone(),
                    "method outside the authorized set",
                ));
                continue;
            }
            let root_ok = claimed.root == "*"
                || authorized.root.is_empty()
                || authorized.root == claimed.root
                || claimed
                    .root
                    .strip_prefix(&authorized.root)
                    .is_some_and(|rest| rest.starts_with('/'));
            if !root_ok {
                rejected.push((
                    claimed.endpoint_id.clone(),
                    "claimed root escapes the authorized root",
                ));
                continue;
            }
            let mut methods = claimed.methods.clone();
            methods.sort();
            methods.dedup();
            accepted.push(EndpointRegistration {
                endpoint_id: claimed.endpoint_id.clone(),
                root: if claimed.root == "*" || authorized.root.is_empty() {
                    authorized.root.clone()
                } else {
                    claimed.root.clone()
                },
                methods,
            });
        }
        accepted.sort_by(|a, b| a.endpoint_id.cmp(&b.endpoint_id));
        (accepted, rejected)
    }
}

#[derive(Debug, Default)]
struct AgentState {
    registrations: HashMap<String, AgentRegistration>,
    generations: HashMap<String, u64>,
}

#[derive(Debug, Default)]
pub struct AgentRegistry {
    state: Mutex<AgentState>,
}

impl AgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn validate_registration(agent: &AgentRegistration) -> Result<(), AgentError> {
        if !is_safe_component(&agent.agent_id)
            || !is_safe_component(&agent.principal_id)
            || !is_safe_component(&agent.tenant_id)
        {
            return Err(AgentError::InvalidIdentifier);
        }
        if agent.endpoints.is_empty()
            || agent.endpoints.iter().any(|endpoint| {
                !is_safe_component(&endpoint.endpoint_id) || endpoint.methods.is_empty()
            })
        {
            return Err(AgentError::NoCapabilities);
        }
        if agent.host_origin.is_empty() || !agent.host_origin.starts_with("conex://") {
            return Err(AgentError::InvalidHost);
        }
        Ok(())
    }

    pub fn register(&self, agent: AgentRegistration) -> Result<(), AgentError> {
        Self::validate_registration(&agent)?;
        self.state
            .lock()
            .expect("agent registry poisoned")
            .registrations
            .insert(agent.agent_id.clone(), agent);
        Ok(())
    }

    /// Fenced registration (plan M2): each claimed endpoint is validated
    /// independently; endpoints that fail land in the rejected list of the
    /// response. Only when at least one endpoint is accepted does the new
    /// generation replace the old connection — a fully-rejected registration
    /// errors out and leaves the healthy old link untouched.
    pub fn register_link(
        &self,
        agent: AgentRegistration,
        generation: u64,
        authorization: &AgentAuthorization,
    ) -> Result<EndpointReview, AgentError> {
        Self::validate_registration(&agent)?;
        let (accepted, rejected) = authorization.review(&agent);
        if accepted.is_empty() {
            return Err(AgentError::NotAuthorized);
        }
        let mut state = self.state.lock().expect("agent registry poisoned");
        if state
            .generations
            .get(&agent.agent_id)
            .is_some_and(|current| generation <= *current)
        {
            return Err(AgentError::StaleGeneration);
        }
        let accepted_ids: Vec<String> = accepted.iter().map(|e| e.endpoint_id.clone()).collect();
        let mut registered = agent;
        registered.endpoints = accepted;
        state
            .generations
            .insert(registered.agent_id.clone(), generation);
        state
            .registrations
            .insert(registered.agent_id.clone(), registered);
        Ok(EndpointReview {
            accepted_endpoint_ids: accepted_ids,
            rejected_capabilities: rejected,
        })
    }

    /// Whether the agent's current generation accepted this endpoint; drives
    /// the per-endpoint ready projection (plan M2).
    pub fn accepts_endpoint(&self, agent_id: &str, endpoint_id: &str) -> bool {
        let state = self.state.lock().expect("agent registry poisoned");
        state
            .registrations
            .get(agent_id)
            .is_some_and(|registration| registration.accepts_endpoint(endpoint_id))
    }

    pub fn heartbeat_generation(&self, agent_id: &str, generation: u64) -> Result<u64, AgentError> {
        let mut state = self.state.lock().expect("agent registry poisoned");
        if state.generations.get(agent_id).copied() != Some(generation) {
            return Err(AgentError::StaleGeneration);
        }
        let entry = state
            .registrations
            .get_mut(agent_id)
            .ok_or(AgentError::Unknown)?;
        entry.last_heartbeat_at_ms = now_ms();
        Ok(entry.last_heartbeat_at_ms)
    }

    pub fn disconnect_generation(&self, agent_id: &str, generation: u64) {
        let mut state = self.state.lock().expect("agent registry poisoned");
        if state.generations.get(agent_id).copied() != Some(generation) {
            return;
        }
        state.generations.remove(agent_id);
        state.registrations.remove(agent_id);
    }

    pub fn heartbeat_expired(&self, agent_id: &str, generation: u64, max_age_ms: u64) -> bool {
        let state = self.state.lock().expect("agent registry poisoned");
        if state.generations.get(agent_id).copied() != Some(generation) {
            return true;
        }
        state
            .registrations
            .get(agent_id)
            .map(|entry| now_ms().saturating_sub(entry.last_heartbeat_at_ms) > max_age_ms)
            .unwrap_or(true)
    }

    pub fn heartbeat(&self, agent_id: &str) -> Result<u64, AgentError> {
        let mut state = self.state.lock().expect("agent registry poisoned");
        let entry = state
            .registrations
            .get_mut(agent_id)
            .ok_or(AgentError::Unknown)?;
        entry.last_heartbeat_at_ms = now_ms();
        Ok(entry.last_heartbeat_at_ms)
    }

    pub fn list(&self) -> Vec<AgentRegistration> {
        let state = self.state.lock().expect("agent registry poisoned");
        let mut list: Vec<AgentRegistration> = state.registrations.values().cloned().collect();
        list.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));
        list
    }

    pub fn resolve(
        &self,
        provider_id: &str,
        resource_id: &str,
        method: &str,
    ) -> Vec<AgentRegistration> {
        let state = self.state.lock().expect("agent registry poisoned");
        state
            .registrations
            .values()
            .filter(|agent| {
                agent.provider_ids().iter().any(|id| id == provider_id)
                    && (resource_id == "*"
                        || agent
                            .resources()
                            .iter()
                            .any(|id| id == resource_id || id == "*"))
                    && agent.methods().iter().any(|id| id == method)
            })
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct EndpointReview {
    pub accepted_endpoint_ids: Vec<String>,
    pub rejected_capabilities: Vec<(String, &'static str)>,
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
    #[error("agent is not pre-authorized for this host")]
    NotAuthorized,
    #[error("stale agent connection generation")]
    StaleGeneration,
}
impl AgentError {
    fn into_call(self) -> CallError {
        let code = match self {
            AgentError::Unknown => conex_proto::ErrorCode::UnknownProvider,
            AgentError::NotAuthorized => conex_proto::ErrorCode::Forbidden,
            AgentError::StaleGeneration => conex_proto::ErrorCode::Conflict,
            _ => conex_proto::ErrorCode::BadRequest,
        };
        CallError::new(code, self.to_string())
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
    /// The browser link this ticket opens. Minted when the tab asks for the
    /// ticket rather than taken from the session, so a second tab on the same
    /// cookie gets its own client instead of hijacking the first tab's.
    pub link_id: String,
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
        link_id: String,
    ) -> Result<WebTicket, CallError> {
        if origin.is_empty() || !(origin.starts_with("http://") || origin.starts_with("https://")) {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "origin must be a non-empty http(s) URL",
            ));
        }
        if !is_safe_component(principal_id) || !is_safe_component(tenant_id) {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "principal/tenant must be alphanumeric/-/_",
            ));
        }
        if !["ui", "agent"].contains(&peer_role) {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                format!("peer_role {peer_role} not allowed"),
            ));
        }
        let now = now_ms();
        let ticket = random_token();
        let ticket = WebTicket {
            ticket,
            principal_id: principal_id.to_string(),
            tenant_id: tenant_id.to_string(),
            origin: origin.to_string(),
            target_host: target_host.to_string(),
            peer_role: peer_role.to_string(),
            capability_caps,
            session_id,
            link_id,
            issued_at_ms: now,
            expires_at_ms: now + TICKET_TTL_MS,
        };
        let mut guard = self.inner.lock().expect("ticket registry poisoned");
        let now = now_ms();
        guard.retain(|_, entry| entry.expires_at_ms > now);
        if guard.len() >= 4096 {
            return Err(CallError::new(
                conex_proto::ErrorCode::QuotaExceeded,
                "ticket registry capacity exceeded",
            ));
        }
        if let Some(session_id) = ticket.session_id.as_deref()
            && guard
                .values()
                .filter(|entry| entry.session_id.as_deref() == Some(session_id))
                .count()
                >= 8
        {
            return Err(CallError::new(
                conex_proto::ErrorCode::QuotaExceeded,
                "session ticket capacity exceeded",
            ));
        }
        guard.insert(ticket.ticket.clone(), ticket.clone());
        Ok(ticket)
    }

    pub fn consume_for(
        &self,
        ticket: &str,
        origin: &str,
        target_host: &str,
    ) -> Result<WebTicket, CallError> {
        let mut guard = self.inner.lock().expect("ticket registry poisoned");
        let now = now_ms();
        guard.retain(|_, entry| entry.expires_at_ms > now);
        let entry = guard.get(ticket).ok_or_else(|| {
            CallError::new(
                conex_proto::ErrorCode::Unauthorized,
                "ticket is unknown, expired, or already consumed",
            )
        })?;
        if entry.origin != origin || entry.target_host != target_host {
            return Err(CallError::new(
                conex_proto::ErrorCode::Unauthorized,
                "ticket origin or target host does not match",
            ));
        }
        Ok(guard.remove(ticket).expect("ticket present while locked"))
    }

    pub fn consume(&self, ticket: &str) -> Result<WebTicket, CallError> {
        let mut guard = self.inner.lock().expect("ticket registry poisoned");
        let entry = guard.remove(ticket).ok_or_else(|| {
            CallError::new(
                conex_proto::ErrorCode::Unauthorized,
                "ticket is unknown or already consumed",
            )
        })?;
        if entry.expires_at_ms <= now_ms() {
            return Err(CallError::new(
                conex_proto::ErrorCode::Unauthorized,
                "ticket has expired",
            ));
        }
        Ok(entry)
    }

    pub fn revoke_session(&self, session_id: &str) {
        self.inner
            .lock()
            .expect("ticket registry poisoned")
            .retain(|_, entry| entry.session_id.as_deref() != Some(session_id));
    }
}

fn random_token() -> String {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system random source unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
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
                conex_proto::ErrorCode::BadRequest,
                "principal/tenant must be alphanumeric/-/_",
            ));
        }
        if code_challenge_method != "S256" {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "code_challenge_method must be S256",
            ));
        }
        if code_challenge.len() < 32 {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
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
                conex_proto::ErrorCode::Unauthorized,
                "oidc code is unknown or already consumed",
            )
        })?;
        if entry.expires_at_ms < now_ms() {
            return Err(CallError::new(
                conex_proto::ErrorCode::Unauthorized,
                "oidc code has expired",
            ));
        }
        if entry.origin != origin {
            return Err(CallError::new(
                conex_proto::ErrorCode::Unauthorized,
                "origin does not match the issued oidc code",
            ));
        }
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(code_verifier.as_bytes()));
        if challenge != entry.code_challenge {
            return Err(CallError::new(
                conex_proto::ErrorCode::Unauthorized,
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
    input.as_object().ok_or_else(|| {
        CallError::new(
            conex_proto::ErrorCode::BadRequest,
            "input must be an object",
        )
    })
}

pub fn require_string<'a>(map: &'a Map<String, Value>, key: &str) -> Result<&'a str, CallError> {
    map.get(key).and_then(Value::as_str).ok_or_else(|| {
        CallError::new(
            conex_proto::ErrorCode::BadRequest,
            format!("{key} required"),
        )
    })
}

pub fn optional_string<'a>(
    map: &'a Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, CallError> {
    match map.get(key) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(CallError::new(
            conex_proto::ErrorCode::BadRequest,
            format!("{key} must be a string"),
        )),
    }
}

pub fn string_list(map: &Map<String, Value>, key: &str) -> Result<Vec<String>, CallError> {
    let arr = map.get(key).and_then(Value::as_array).ok_or_else(|| {
        CallError::new(
            conex_proto::ErrorCode::BadRequest,
            format!("{key} required"),
        )
    })?;
    arr.iter()
        .map(|value| {
            value.as_str().map(str::to_string).ok_or_else(|| {
                CallError::new(
                    conex_proto::ErrorCode::BadRequest,
                    format!("{key} entries must be strings"),
                )
            })
        })
        .collect()
}

/// Parse the per-endpoint registration list (plan M2):
/// `endpoints: [{endpointId, root, methods}]`.
pub fn parse_endpoints(map: &Map<String, Value>) -> Result<Vec<EndpointRegistration>, CallError> {
    let arr = map
        .get("endpoints")
        .and_then(Value::as_array)
        .ok_or_else(|| CallError::new(conex_proto::ErrorCode::BadRequest, "endpoints required"))?;
    arr.iter()
        .map(|entry| {
            let map = entry.as_object().ok_or_else(|| {
                CallError::new(
                    conex_proto::ErrorCode::BadRequest,
                    "endpoints entries must be objects",
                )
            })?;
            Ok(EndpointRegistration {
                endpoint_id: require_string(map, "endpointId")?.to_string(),
                root: map
                    .get("root")
                    .and_then(Value::as_str)
                    .unwrap_or("*")
                    .to_string(),
                methods: string_list(map, "methods")?,
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
                conex_proto::ErrorCode::BadRequest,
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
                        conex_proto::ErrorCode::Forbidden,
                        format!("agent hostOrigin {host_origin} does not match host {expected}"),
                    ));
                }
            } else if !host_origin.starts_with("conex://") {
                return Err(CallError::new(
                    conex_proto::ErrorCode::BadRequest,
                    "agent hostOrigin must start with conex://",
                ));
            }
            let registration = AgentRegistration {
                agent_id: require_string(map, "agentId")?.to_string(),
                principal_id: call.caller.principal_id.clone(),
                tenant_id: call.caller.tenant_id.clone(),
                endpoints: parse_endpoints(map)?,
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
            conex_proto::ErrorCode::UnknownMethod,
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
    pub authorizations: Arc<HashMap<String, AgentAuthorization>>,
    pub connections: Arc<RemoteConnections>,
    /// Real RS256 id_token verifier. `Some` when `[oidc]` is configured;
    /// `/oidc/token` then verifies presented id_tokens instead of the
    /// dev-only code/PKCE pass-through.
    pub verifier: Option<Arc<crate::oidc_jwt::OidcVerifier>>,
}

impl HostSide {
    pub fn new() -> Self {
        Self::with_authorizations(Vec::new())
    }

    pub fn with_authorizations(authorizations: Vec<AgentAuthorization>) -> Self {
        let authorizations = authorizations
            .into_iter()
            .map(|authorization| (authorization.agent_id.clone(), authorization))
            .collect();
        Self {
            agents: Arc::new(AgentRegistry::new()),
            tickets: Arc::new(TicketRegistry::new()),
            oidc: Arc::new(OidcRegistry::new()),
            authorizations: Arc::new(authorizations),
            connections: Arc::new(RemoteConnections::new()),
            verifier: None,
        }
    }

    pub async fn stage_connection(&self, agent_id: &str, link: RemoteLink) {
        self.connections
            .stage(agent_id, link.generation, link)
            .await;
    }

    pub async fn discard_connection(&self, agent_id: &str, generation: u64) {
        self.connections.discard(agent_id, generation).await;
    }

    pub async fn activate_connection(
        &self,
        agent_id: &str,
        generation: u64,
        accepted_endpoints: &[String],
    ) -> Result<(), AgentError> {
        self.connections
            .activate(agent_id, generation, accepted_endpoints)
            .await
    }

    pub fn register_agent_link(
        &self,
        registration: AgentRegistration,
        generation: u64,
    ) -> Result<EndpointReview, AgentError> {
        let authorization = self
            .authorizations
            .get(&registration.agent_id)
            .ok_or(AgentError::NotAuthorized)?;
        if registration.principal_id != registration.agent_id {
            return Err(AgentError::NotAuthorized);
        }
        self.agents
            .register_link(registration, generation, authorization)
    }

    pub fn heartbeat_agent(&self, agent_id: &str, generation: u64) -> Result<u64, AgentError> {
        self.agents.heartbeat_generation(agent_id, generation)
    }
    pub fn agent_heartbeat_expired(
        &self,
        agent_id: &str,
        generation: u64,
        max_age_ms: u64,
    ) -> bool {
        self.agents
            .heartbeat_expired(agent_id, generation, max_age_ms)
    }

    pub fn disconnect_agent(&self, agent_id: &str, generation: u64) {
        self.agents.disconnect_generation(agent_id, generation);
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

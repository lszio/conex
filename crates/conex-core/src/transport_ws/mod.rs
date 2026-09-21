//! WSS bootstrap state machine (design §4.4, §5.2).
//!
//! The conex WSS Link runs a fixed-format handshake before any business
//! frame is allowed:
//!
//!   hello → hello result → conex/ready → ready result
//!
//! - Both sides speak UTF-8 JSON-RPC 2.0; one JSON message = one WS text
//!   message; each JSON message ≤ 64 KiB.
//! - `conex/ready` echoes a server-generated one-shot `negotiation_id`.
//! - Until the server writes `ready result`, the client may NOT send any
//!   business frame. After it does, the client may send the first
//!   business frame.
//! - The server may not advertise a profile that differs from `hello`;
//!   repeated `ready` is rejected.
//!
//! This module is a pure state machine; no Tokio / WS I/O. The transport
//! crate (`conex-transport-ws`) is responsible for wiring it onto axum/hyper.
#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use conex_proto::v1;

/// UTF-8 JSON-RPC 2.0 envelope, capped at 64 KiB per message.
pub const MAX_BOOTSTRAP_MESSAGE_BYTES: usize = 65_536;

/// Default handshake timeout: 10 seconds.
pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 10_000;

/// Identifier for the two bootstrap profiles the demo slice recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileId {
    #[serde(rename = "conex-jsonrpc2-wss-v1")]
    JsonRpc2WssV1,
    #[serde(rename = "conex-protobuf-wss-v1")]
    ProtobufWssV1,
}

impl ProfileId {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProfileId::JsonRpc2WssV1 => "conex-jsonrpc2-wss-v1",
            ProfileId::ProtobufWssV1 => "conex-protobuf-wss-v1",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapState {
    AwaitingHello,
    SentHello,
    AwaitingReady,
    Ready {
        profile: ProfileId,
        negotiation_id: String,
    },
    Failed {
        reason: BootstrapError,
    },
}

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BootstrapError {
    #[error("early business frame before ready result")]
    EarlyBusiness,
    #[error("malformed bootstrap envelope: {0}")]
    MalformedEnvelope(String),
    #[error("unknown method: {0}")]
    UnknownMethod(String),
    #[error("unsupported profile: {0}")]
    UnsupportedProfile(String),
    #[error("unsupported required capability: {0}")]
    UnsupportedCapability(String),
    #[error("negotiated limits mismatch")]
    LimitsMismatch,
    #[error("repeated ready call")]
    RepeatedReady,
    #[error("ready negotiation id mismatch")]
    NegotiationIdMismatch,
    #[error("handshake timeout")]
    Timeout,
    #[error("buffer exceeded {MAX_BOOTSTRAP_MESSAGE_BYTES} bytes")]
    BufferExceeded,
    #[error("missing plane")]
    MissingPlane,
    #[error("plane mismatch: client said {client}, server expected {server}")]
    PlaneMismatch { client: String, server: String },
}
 

pub fn plane_name(plane: v1::Plane) -> &'static str {
    match plane {
        v1::Plane::Broker => "broker",
        v1::Plane::Relay => "relay",
        v1::Plane::Unspecified => "unspecified",
    }
}
fn plane_from_str(s: &str) -> Option<v1::Plane> {
    match s {
        "broker" => Some(v1::Plane::Broker),
        "relay" => Some(v1::Plane::Relay),
        _ => None,
    }
}



/// Generated control type used for both JSON ready projection and protobuf
/// mapping. JSON uses explicit lowercase field/plane mapping below because
/// prost messages do not implement serde.
pub type ReadyRequest = v1::ReadyRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapFrame {
    pub json: String,
}

fn profile_from_str(s: &str) -> Option<ProfileId> {
    match s {
        "conex-jsonrpc2-wss-v1" => Some(ProfileId::JsonRpc2WssV1),
        "conex-protobuf-wss-v1" => Some(ProfileId::ProtobufWssV1),
        _ => None,
    }
}
fn validate_bootstrap_envelope(value: &serde_json::Value) -> Result<(), BootstrapError> {
    if value.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0") {
        return Err(BootstrapError::MalformedEnvelope(
            "jsonrpc must be the string 2.0".into(),
        ));
    }
    if !value.get("id").is_some_and(|id| !id.is_null()) {
        return Err(BootstrapError::MalformedEnvelope(
            "bootstrap request/response requires an id".into(),
        ));
    }
    Ok(())
}

static FALLBACK_NEGOTIATION_COUNTER: AtomicU64 = AtomicU64::new(1);

fn generate_negotiation_id() -> String {
    use base64::Engine as _;
    use std::io::Read as _;
    let mut bytes = [0u8; 32];
    let secure = std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .is_ok();
    if !secure {
        let counter = FALLBACK_NEGOTIATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos() as u64);
        bytes[..8].copy_from_slice(&counter.to_le_bytes());
        bytes[8..16].copy_from_slice(&nanos.to_le_bytes());
    }
    format!(
        "neg-{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    )
}

fn default_limits() -> v1::Limits {
    v1::Limits {
        max_frame_bytes: 1_048_576,
        max_inflight: 4,
        max_queued_bytes: 8_388_608,
        timeout_ms: 8_000,
    }
}

fn limits_json(limits: v1::Limits) -> serde_json::Value {
    serde_json::json!({
        "maxFrameBytes": limits.max_frame_bytes,
        "maxInflight": limits.max_inflight,
        "maxQueuedBytes": limits.max_queued_bytes,
        "timeoutMs": limits.timeout_ms,
    })
}

fn limits_from_json(value: &serde_json::Value) -> Result<v1::Limits, BootstrapError> {
    let number = |field: &str| {
        value
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(|| BootstrapError::MalformedEnvelope(format!("invalid limits.{field}")))
    };
    Ok(v1::Limits {
        max_frame_bytes: number("maxFrameBytes")?,
        max_inflight: number("maxInflight")?,
        max_queued_bytes: number("maxQueuedBytes")?,
        timeout_ms: number("timeoutMs")?,
    })
}

fn string_list(
    value: &serde_json::Value,
    field: &str,
) -> Result<Vec<String>, BootstrapError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| BootstrapError::MalformedEnvelope(format!("missing {field}")))?
        .iter()
        .map(|item| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| BootstrapError::MalformedEnvelope(format!("{field} must be strings")))
        })
        .collect()
}

fn hello_request_from_json(value: &serde_json::Value) -> Result<v1::HelloRequest, BootstrapError> {
    let profile_id = value
        .get("profileId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| BootstrapError::MalformedEnvelope("missing profileId".into()))?
        .to_owned();
    let plane = value
        .get("plane")
        .and_then(serde_json::Value::as_str)
        .and_then(plane_from_str)
        .ok_or_else(|| BootstrapError::MalformedEnvelope("missing or invalid plane".into()))?;
    Ok(v1::HelloRequest {
        profile_id,
        plane: plane as i32,
        provides: string_list(value, "provides")?,
        requires: string_list(value, "requires")?,
    })
}
fn ready_request_from_json(value: &serde_json::Value) -> Result<v1::ReadyRequest, BootstrapError> {
    let negotiation_id = value
        .get("negotiationId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| BootstrapError::MalformedEnvelope("missing negotiationId".into()))?
        .to_owned();
    let profile_id = value
        .get("profileId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| BootstrapError::MalformedEnvelope("missing profileId".into()))?
        .to_owned();
    let plane = value
        .get("plane")
        .and_then(serde_json::Value::as_str)
        .and_then(plane_from_str)
        .ok_or_else(|| BootstrapError::MalformedEnvelope("missing or invalid plane".into()))?;
    let limits = value
        .get("limits")
        .map(limits_from_json)
        .transpose()?;
    Ok(v1::ReadyRequest {
        negotiation_id,
        profile_id,
        plane: plane as i32,
        provides: string_list(value, "provides")?,
        limits,
    })
}

/// Server-side handshake.
pub struct ServerHandshake {
    state: BootstrapState,
    negotiation_id: String,
    server_profiles: Vec<ProfileId>,
    /// Profile the client negotiated in `conex/hello`; echoed in ready.
    agreed_profile: Option<ProfileId>,

    server_plane: v1::Plane,
    server_capabilities: v1::NegotiatedCapabilities,
    link_identity: v1::LinkIdentity,
    limits: v1::Limits,
}

impl ServerHandshake {
    pub fn new(server_profile: ProfileId, server_plane: v1::Plane) -> Self {
        Self::new_with_profiles(&[server_profile], server_plane)
    }

    /// The server may accept more than one profile (design §4.4). The first
    /// entry is the default advertised when the client does not pin one.
    pub fn new_with_profiles(server_profiles: &[ProfileId], server_plane: v1::Plane) -> Self {
        Self::new_with_profiles_and_capabilities(
            server_profiles,
            server_plane,
            Vec::new(),
            generate_negotiation_id(),
        )
    }

    /// Build a handshake from host-assembled capabilities and injected
    /// identity material. The generated protobuf types are the source of
    /// truth for the negotiated capability and link summaries.
    pub fn new_with_profiles_and_capabilities(
        server_profiles: &[ProfileId],
        server_plane: v1::Plane,
        server_provides: Vec<String>,
        negotiation_id: String,
    ) -> Self {
        let mut server_provides = server_provides;
        server_provides.sort();
        server_provides.dedup();
        Self {
            state: BootstrapState::AwaitingHello,
            negotiation_id: negotiation_id.clone(),
            server_profiles: server_profiles.to_vec(),
            agreed_profile: None,
            server_plane,
            server_capabilities: v1::NegotiatedCapabilities {
                provides: server_provides,
                requires: Vec::new(),
                rejected_capabilities: Vec::new(),
            },
            link_identity: v1::LinkIdentity {
                link_id: negotiation_id,
                peer_id: String::new(),
                tenant_id: String::new(),
            },
            limits: default_limits(),
        }
    }

    pub fn state(&self) -> &BootstrapState {
        &self.state
    }

    pub fn link_identity(&self) -> &v1::LinkIdentity {
        &self.link_identity
    }

    pub fn ingest(&mut self, frame: BootstrapFrame) -> Result<BootstrapFrame, BootstrapError> {
        if frame.json.len() > MAX_BOOTSTRAP_MESSAGE_BYTES {
            return Err(BootstrapError::BufferExceeded);
        }
        let parsed: serde_json::Value = serde_json::from_str(&frame.json)
            .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?;
        validate_bootstrap_envelope(&parsed)?;
        match self.state.clone() {
            BootstrapState::AwaitingHello => self.handle_hello(parsed),
            BootstrapState::AwaitingReady => self.handle_frame_after_hello(parsed),
            BootstrapState::Ready { .. } => Err(BootstrapError::EarlyBusiness),
            _ => Err(BootstrapError::MalformedEnvelope(
                "server in unexpected state".into(),
            )),
        }
    }

    fn handle_hello(&mut self, value: serde_json::Value) -> Result<BootstrapFrame, BootstrapError> {
        let method = value
            .get("method")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BootstrapError::MalformedEnvelope("missing method".into()))?;
        if method != "conex/hello" {
            return Err(BootstrapError::UnknownMethod(method.to_string()));
        }
        let params = value
            .get("params")
            .ok_or_else(|| BootstrapError::MalformedEnvelope("missing params".into()))?;
        let req = hello_request_from_json(params)?;
        let negotiated = self
            .server_profiles
            .iter()
            .find(|p| p.as_str() == req.profile_id)
            .copied()
            .ok_or_else(|| BootstrapError::UnsupportedProfile(req.profile_id.clone()))?;
        let req_plane = v1::Plane::try_from(req.plane)
            .unwrap_or(v1::Plane::Unspecified);
        if req_plane != self.server_plane {
            return Err(BootstrapError::PlaneMismatch {
                client: plane_name(req_plane).to_string(),
                server: plane_name(self.server_plane).to_string(),
            });
        }
        let supported: HashSet<&str> = self
            .server_capabilities
            .provides
            .iter()
            .map(String::as_str)
            .collect();
        if let Some(missing) = req
            .requires
            .iter()
            .find(|required| !supported.contains(required.as_str()))
        {
            return Err(BootstrapError::UnsupportedCapability(missing.clone()));
        }
        let response = v1::HelloResponse {
            binding_id: String::new(),
            expires_in_ms: 0,
            profile_id: negotiated.as_str().to_owned(),
            plane: self.server_plane as i32,
            provides: self.server_capabilities.provides.clone(),
            rejected_capabilities: Vec::new(),
            limits: Some(self.limits),
        };
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": value.get("id").cloned().unwrap_or(serde_json::Value::Null),
            "result": {
                "negotiationId": self.negotiation_id,
                "profileId": response.profile_id,
                "plane": plane_name(v1::Plane::try_from(response.plane).unwrap_or(self.server_plane)),
                "limits": limits_json(response.limits.unwrap_or_else(default_limits)),
                "provides": response.provides,
                "requires": self.server_capabilities.requires,
                "linkIdentity": {
                    "linkId": self.link_identity.link_id,
                    "peerId": self.link_identity.peer_id,
                    "tenantId": self.link_identity.tenant_id,
                },
            }
        });
        let json = serde_json::to_string(&response)
            .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?;
        if json.len() > MAX_BOOTSTRAP_MESSAGE_BYTES {
            return Err(BootstrapError::BufferExceeded);
        }
        self.agreed_profile = Some(negotiated);
        self.state = BootstrapState::AwaitingReady;
        Ok(BootstrapFrame { json })
    }

    fn handle_frame_after_hello(
        &mut self,
        value: serde_json::Value,
    ) -> Result<BootstrapFrame, BootstrapError> {
        let method = value
            .get("method")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BootstrapError::MalformedEnvelope("missing method".into()))?;
        match method {
            "conex/ready" => self.handle_ready(value),
            "conex/hello" => Err(BootstrapError::MalformedEnvelope(
                "conex/hello received after handshake started".into(),
            )),
            _ => Err(BootstrapError::UnknownMethod(method.to_string())),
        }
    }

    fn handle_ready(&mut self, value: serde_json::Value) -> Result<BootstrapFrame, BootstrapError> {
        if matches!(self.state, BootstrapState::Ready { .. }) {
            return Err(BootstrapError::RepeatedReady);
        }
        let method = value
            .get("method")
            .and_then(|v| v.as_str())
            .ok_or_else(|| BootstrapError::MalformedEnvelope("missing method".into()))?;
        if method != "conex/ready" {
            return Err(BootstrapError::UnknownMethod(method.to_string()));
        }
        let params = value
            .get("params")
            .ok_or_else(|| BootstrapError::MalformedEnvelope("missing params".into()))?;
        let incoming_negotiation_id = params
            .get("negotiationId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| BootstrapError::MalformedEnvelope("missing negotiationId".into()))?;
        if incoming_negotiation_id != self.negotiation_id {
            return Err(BootstrapError::NegotiationIdMismatch);
        }
        let req = ready_request_from_json(params)?;
        let agreed = self.agreed_profile.unwrap_or_else(|| {
            self.server_profiles
                .first()
                .copied()
                .unwrap_or(ProfileId::JsonRpc2WssV1)
        });
        if req.profile_id != agreed.as_str() {
            return Err(BootstrapError::UnsupportedProfile(req.profile_id));
        }
        if req.negotiation_id != self.negotiation_id {
            return Err(BootstrapError::NegotiationIdMismatch);
        }
        let req_plane = v1::Plane::try_from(req.plane)
            .unwrap_or(v1::Plane::Unspecified);
        if req_plane != self.server_plane {
            return Err(BootstrapError::PlaneMismatch {
                client: plane_name(req_plane).to_string(),
                server: plane_name(self.server_plane).to_string(),
            });
        }
        if req.provides != self.server_capabilities.provides {
            let missing = self
                .server_capabilities
                .provides
                .iter()
                .find(|capability| !req.provides.contains(capability))
                .cloned()
                .unwrap_or_else(|| "capability set mismatch".into());
            return Err(BootstrapError::UnsupportedCapability(missing));
        }
        if req.limits.as_ref().ok_or(BootstrapError::LimitsMismatch)? != &self.limits {
            return Err(BootstrapError::LimitsMismatch);
        }
        let ready_response = v1::ReadyResponse {
            negotiation_id: self.negotiation_id.clone(),
            profile_id: agreed.as_str().to_owned(),
            plane: self.server_plane as i32,
            provides: self.server_capabilities.provides.clone(),
            limits: Some(self.limits),
            link_identity: Some(self.link_identity.clone()),
        };
        let identity = ready_response.link_identity.clone().unwrap_or_default();
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": value.get("id").cloned().unwrap_or(serde_json::Value::Null),
            "result": {
                "negotiationId": ready_response.negotiation_id,
                "profileId": ready_response.profile_id,
                "plane": plane_name(v1::Plane::try_from(ready_response.plane).unwrap_or(self.server_plane)),
                "provides": ready_response.provides,
                "requires": self.server_capabilities.requires,
                "limits": limits_json(ready_response.limits.unwrap_or_else(default_limits)),
                "linkIdentity": {
                    "linkId": identity.link_id,
                    "peerId": identity.peer_id,
                    "tenantId": identity.tenant_id,
                },
            }
        });
        let json = serde_json::to_string(&response)
            .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?;
        if json.len() > MAX_BOOTSTRAP_MESSAGE_BYTES {
            return Err(BootstrapError::BufferExceeded);
        }
        self.state = BootstrapState::Ready {
            profile: agreed,
            negotiation_id: self.negotiation_id.clone(),
        };
        Ok(BootstrapFrame { json })
    }
}

/// Client-side handshake.
pub struct ClientHandshake {
    state: BootstrapState,
    expected_negotiation_id: String,
    agreed_profile: Option<ProfileId>,
    server_hello_plane: v1::Plane,
    server_hello_profile: String,
    server_hello_id: Option<serde_json::Value>,
    request_counter: u64,
    client_requires: Vec<String>,
    server_provides: Vec<String>,
    server_limits: Option<serde_json::Value>,
}

impl Default for ClientHandshake {
    fn default() -> Self {
        Self::new()
    }
}

impl ClientHandshake {
    pub fn new() -> Self {
        Self {
            state: BootstrapState::AwaitingHello,
            expected_negotiation_id: String::new(),
            agreed_profile: None,
            server_hello_plane: v1::Plane::Broker,
            server_hello_profile: String::new(),
            server_hello_id: None,
            request_counter: 0,
            client_requires: Vec::new(),
            server_provides: Vec::new(),
            server_limits: None,
        }
    }

    pub fn state(&self) -> &BootstrapState {
        &self.state
    }

    pub fn build_hello(
        &mut self,
        profile: ProfileId,
        plane: v1::Plane,
        provides: Vec<String>,
        requires: Vec<String>,
    ) -> Result<BootstrapFrame, BootstrapError> {
        if !matches!(self.state, BootstrapState::AwaitingHello) {
            return Err(BootstrapError::RepeatedReady);
        }
        if plane == v1::Plane::Unspecified {
            return Err(BootstrapError::MissingPlane);
        }
        self.request_counter += 1;
        let id = format!("req-{}", self.request_counter);
        let hello = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "conex/hello",
            "params": {
                "profileId": profile.as_str(),
                "plane": plane_name(plane),
                "provides": provides,
                "requires": requires,
            }
        });
        let json = serde_json::to_string(&hello)
            .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?;
        if json.len() > MAX_BOOTSTRAP_MESSAGE_BYTES {
            return Err(BootstrapError::BufferExceeded);
        }
        self.client_requires = requires;
        self.agreed_profile = Some(profile);
        self.state = BootstrapState::SentHello;
        Ok(BootstrapFrame { json })
    }

    pub fn ingest(&mut self, frame: BootstrapFrame) -> Result<ReadyRequest, BootstrapError> {
        if frame.json.len() > MAX_BOOTSTRAP_MESSAGE_BYTES {
            return Err(BootstrapError::BufferExceeded);
        }
        let parsed: serde_json::Value = serde_json::from_str(&frame.json)
            .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?;
        validate_bootstrap_envelope(&parsed)?;
        match self.state.clone() {
            BootstrapState::AwaitingHello => Err(BootstrapError::MalformedEnvelope(
                "client received before sending hello".into(),
            )),
            BootstrapState::SentHello => {
                let result = parsed
                    .get("result")
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing result".into()))?;
                let negotiation_id = result
                    .get("negotiationId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        BootstrapError::MalformedEnvelope("missing negotiationId".into())
                    })?
                    .to_string();
                let profile_str = result
                    .get("profileId")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing profileId".into()))?
                    .to_string();
                let profile = profile_from_str(&profile_str)
                    .ok_or_else(|| BootstrapError::UnsupportedProfile(profile_str.clone()))?;
                if Some(profile) != self.agreed_profile {
                    return Err(BootstrapError::UnsupportedProfile(profile_str));
                }
                let plane_str = result
                    .get("plane")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing plane".into()))?
                    .to_string();
                let plane = plane_from_str(&plane_str)
                    .ok_or_else(|| BootstrapError::MalformedEnvelope(format!("unknown plane: {plane_str}")))?;
                let provides_values = result
                    .get("provides")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing provides".into()))?
                    .iter()
                    .map(|value| {
                        value.as_str().map(str::to_owned).ok_or_else(|| {
                            BootstrapError::MalformedEnvelope("provides must be strings".into())
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let provides: HashSet<&str> =
                    provides_values.iter().map(String::as_str).collect();
                if let Some(missing) = self
                    .client_requires
                    .iter()
                    .find(|required| !provides.contains(required.as_str()))
                {
                    return Err(BootstrapError::UnsupportedCapability(missing.clone()));
                }
                let limits = result
                    .get("limits")
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing limits".into()))?;
                let limits = limits_from_json(limits)?;
                let limits_value = limits_json(limits);
                self.server_provides = provides_values.clone();
                self.server_limits = Some(limits_value);
                self.expected_negotiation_id = negotiation_id.clone();
                self.server_hello_plane = plane;
                self.server_hello_profile = profile_str.clone();
                self.server_hello_id = parsed.get("id").cloned();
                self.state = BootstrapState::AwaitingReady;
                Ok(ReadyRequest {
                    negotiation_id,
                    profile_id: profile_str,
                    plane: plane as i32,
                    provides: provides_values,
                    limits: Some(limits),
                })
            }
            BootstrapState::AwaitingReady => {
                let result = parsed
                    .get("result")
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing result".into()))?;
                let negotiation_id = result
                    .get("negotiationId")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing negotiationId".into()))?
                    .to_owned();
                if negotiation_id != self.expected_negotiation_id {
                    return Err(BootstrapError::NegotiationIdMismatch);
                }
                let profile_str = result
                    .get("profileId")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing profileId".into()))?
                    .to_owned();
                if profile_str != self.server_hello_profile {
                    return Err(BootstrapError::UnsupportedProfile(profile_str));
                }
                let plane = result
                    .get("plane")
                    .and_then(serde_json::Value::as_str)
                    .and_then(plane_from_str)
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing plane".into()))?;
                if plane != self.server_hello_plane {
                    return Err(BootstrapError::PlaneMismatch {
                        client: plane_name(plane).into(),
                        server: plane_name(self.server_hello_plane).into(),
                    });
                }
                let provides = result
                    .get("provides")
                    .and_then(serde_json::Value::as_array)
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing provides".into()))?
                    .iter()
                    .map(|value| {
                        value.as_str().map(str::to_owned).ok_or_else(|| {
                            BootstrapError::MalformedEnvelope("provides must be strings".into())
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if provides != self.server_provides {
                    return Err(BootstrapError::UnsupportedCapability(
                        "ready result capability set mismatch".into(),
                    ));
                }
                let limits_value = result
                    .get("limits")
                    .ok_or(BootstrapError::LimitsMismatch)?;
                let limits = limits_from_json(limits_value)?;
                let expected_limits = limits_from_json(
                    self.server_limits
                        .as_ref()
                        .ok_or(BootstrapError::LimitsMismatch)?,
                )?;
                if limits != expected_limits {
                    return Err(BootstrapError::LimitsMismatch);
                }
                self.state = BootstrapState::Ready {
                    profile: self.agreed_profile.unwrap_or(ProfileId::JsonRpc2WssV1),
                    negotiation_id: self.expected_negotiation_id.clone(),
                };
                Ok(ReadyRequest {
                    negotiation_id,
                    profile_id: profile_str,
                    plane: plane as i32,
                    provides,
                    limits: Some(limits),
                })
            }
            BootstrapState::Ready { .. } => Err(BootstrapError::RepeatedReady),
            BootstrapState::Failed { .. } => Err(BootstrapError::MalformedEnvelope(
                "client already failed".into(),
            )),
        }
    }

    pub fn build_ready_frame(
        &self,
        ready: &ReadyRequest,
    ) -> Result<BootstrapFrame, BootstrapError> {
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.server_hello_id.clone().unwrap_or(serde_json::Value::Null),
            "method": "conex/ready",
            "params": {
                "negotiationId": ready.negotiation_id,
                "profileId": ready.profile_id,
                "plane": plane_name(v1::Plane::try_from(ready.plane).unwrap_or(v1::Plane::Unspecified)),
                "provides": ready.provides,
                "limits": ready.limits.map(limits_json),
            }
        });
        let json = serde_json::to_string(&frame)
            .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?;
        if json.len() > MAX_BOOTSTRAP_MESSAGE_BYTES {
            return Err(BootstrapError::BufferExceeded);
        }
        Ok(BootstrapFrame { json })
    }
}

impl fmt::Display for BootstrapFrame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_happy_path_round_trip() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2WssV1, v1::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2WssV1,
                v1::Plane::Broker,
                vec!["source/read".into()],
                vec![],
            )
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let ready = client.ingest(hello_result).unwrap();
        let ready_frame = client.build_ready_frame(&ready).unwrap();
        let ready_result = server.ingest(ready_frame).unwrap();
        let _ = client.ingest(ready_result).unwrap();
        assert!(matches!(server.state(), BootstrapState::Ready { .. }));
        assert!(matches!(client.state(), BootstrapState::Ready { .. }));
    }

    #[test]
    fn profile_mismatch_rejected() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2WssV1, v1::Plane::Broker);
        let hello = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": "conex/hello",
            "params": {
                "profileId": "conex-protobuf-wss-v1",
                "plane": "broker",
                "provides": [],
                "requires": [],
            }
        })
        .to_string();
        let err = server.ingest(BootstrapFrame { json: hello }).unwrap_err();
        assert!(matches!(err, BootstrapError::UnsupportedProfile(_)));
    }

    #[test]
    fn plane_mismatch_rejected() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2WssV1, v1::Plane::Broker);
        let hello = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": "conex/hello",
            "params": {
                "profileId": "conex-jsonrpc2-wss-v1",
                "plane": "relay",
                "provides": [],
                "requires": [],
            }
        })
        .to_string();
        let err = server.ingest(BootstrapFrame { json: hello }).unwrap_err();
        assert!(matches!(err, BootstrapError::PlaneMismatch { .. }));
    }

    #[test]
    fn unknown_method_rejected() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2WssV1, v1::Plane::Broker);
        let bad = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": "conex/foobar",
            "params": {
                "profileId": "conex-jsonrpc2-wss-v1",
                "plane": "broker",
            }
        })
        .to_string();
        let err = server.ingest(BootstrapFrame { json: bad }).unwrap_err();
        assert!(matches!(err, BootstrapError::UnknownMethod(_)));
    }

    #[test]
    fn repeated_hello_rejected_after_hello() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2WssV1, v1::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(ProfileId::JsonRpc2WssV1, v1::Plane::Broker, vec![], vec![])
            .unwrap();
        let _ = server.ingest(hello.clone()).unwrap();
        let second = server.ingest(hello);
        assert!(matches!(second, Err(BootstrapError::MalformedEnvelope(_))));
    }

    #[test]
    fn ready_frame_after_ready_rejected_as_early_business() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2WssV1, v1::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(ProfileId::JsonRpc2WssV1, v1::Plane::Broker, vec![], vec![])
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let ready = client.ingest(hello_result).unwrap();
        let ready_frame = client.build_ready_frame(&ready).unwrap();
        let _ = server.ingest(ready_frame.clone()).unwrap();
        let second = server.ingest(ready_frame);
        assert!(matches!(second, Err(BootstrapError::EarlyBusiness)));
    }

    #[test]
    fn oversized_bootstrap_message_rejected() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2WssV1, v1::Plane::Broker);
        let huge = "x".repeat(MAX_BOOTSTRAP_MESSAGE_BYTES + 1);
        let err = server.ingest(BootstrapFrame { json: huge }).unwrap_err();
        assert!(matches!(err, BootstrapError::BufferExceeded));
    }

    #[test]
    fn negotiation_id_mismatch_rejected_after_hello() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2WssV1, v1::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(ProfileId::JsonRpc2WssV1, v1::Plane::Broker, vec![], vec![])
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let _ = client.ingest(hello_result).unwrap();
        let bad = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "ready-spoof",
            "method": "conex/ready",
            "params": {
                "negotiationId": "spoofed",
                "profileId": "conex-jsonrpc2-wss-v1",
                "plane": "broker",
            }
        })
        .to_string();
        let err = server.ingest(BootstrapFrame { json: bad }).unwrap_err();
        assert!(matches!(err, BootstrapError::NegotiationIdMismatch));
    }

    #[test]
    fn both_profiles_known() {
        let s1 = ProfileId::JsonRpc2WssV1.as_str();
        let s2 = ProfileId::ProtobufWssV1.as_str();
        assert_eq!(s1, "conex-jsonrpc2-wss-v1");
        assert_eq!(s2, "conex-protobuf-wss-v1");
    }

    #[test]
    fn early_business_frame_rejected_after_ready() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2WssV1, v1::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(ProfileId::JsonRpc2WssV1, v1::Plane::Broker, vec![], vec![])
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let ready = client.ingest(hello_result).unwrap();
        let ready_frame = client.build_ready_frame(&ready).unwrap();
        let _ = server.ingest(ready_frame).unwrap();
        let business = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "biz-1",
            "method": "source/read",
            "params": {}
        })
        .to_string();
        let err = server
            .ingest(BootstrapFrame { json: business })
            .unwrap_err();
        assert!(matches!(err, BootstrapError::EarlyBusiness));
    }

    #[test]
    fn unspecified_plane_rejected_on_hello() {
        let mut client = ClientHandshake::new();
        let err = client
            .build_hello(
                ProfileId::JsonRpc2WssV1,
                v1::Plane::Unspecified,
                vec![],
                vec![],
            )
            .unwrap_err();
        assert!(matches!(err, BootstrapError::MissingPlane));
    }

    #[test]
    fn protobuf_profile_negotiates() {
        let mut server = ServerHandshake::new(ProfileId::ProtobufWssV1, v1::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::ProtobufWssV1,
                v1::Plane::Broker,
                vec!["blob/commit".into()],
                vec![],
            )
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let ready = client.ingest(hello_result).unwrap();
        assert_eq!(ready.profile_id, "conex-protobuf-wss-v1");
    }
 
    #[test]
    fn server_advertises_own_capabilities_and_rejects_hard_requires() {
        let mut server = ServerHandshake::new_with_profiles_and_capabilities(
            &[ProfileId::JsonRpc2WssV1],
            v1::Plane::Broker,
            vec!["source/read".into()],
            "neg-test".into(),
        );
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2WssV1,
                v1::Plane::Broker,
                vec!["ui/probe".into()],
                vec!["source/read".into()],
            )
            .unwrap();
        let response = server.ingest(hello).unwrap();
        let value: serde_json::Value = serde_json::from_str(&response.json).unwrap();
        assert_eq!(value["result"]["provides"], serde_json::json!(["source/read"]));
        assert_ne!(value["result"]["provides"], serde_json::json!(["ui/probe"]));

        let mut rejecting = ServerHandshake::new_with_profiles_and_capabilities(
            &[ProfileId::JsonRpc2WssV1],
            v1::Plane::Broker,
            vec!["source/read".into()],
            "neg-test".into(),
        );
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2WssV1,
                v1::Plane::Broker,
                vec![],
                vec!["source/search".into()],
            )
            .unwrap();
        assert!(matches!(
            rejecting.ingest(hello),
            Err(BootstrapError::UnsupportedCapability(_))
        ));
    }

    #[test]
    fn ready_rejects_capability_or_limit_mismatch() {
        let mut server = ServerHandshake::new_with_profiles_and_capabilities(
            &[ProfileId::JsonRpc2WssV1],
            v1::Plane::Broker,
            vec!["source/read".into()],
            "neg-ready".into(),
        );
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2WssV1,
                v1::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let ready = client.ingest(hello_result).unwrap();
        let mut ready_value: serde_json::Value =
            serde_json::from_str(&client.build_ready_frame(&ready).unwrap().json).unwrap();
        ready_value["params"]["provides"] = serde_json::json!([]);
        let err = server
            .ingest(BootstrapFrame {
                json: ready_value.to_string(),
            })
            .unwrap_err();
        assert!(matches!(err, BootstrapError::UnsupportedCapability(_)));
    }

    #[test]
    fn client_rejects_ready_result_capability_mismatch() {
        let mut server = ServerHandshake::new_with_profiles_and_capabilities(
            &[ProfileId::JsonRpc2WssV1],
            v1::Plane::Broker,
            vec!["source/read".into()],
            "neg-client-ready".into(),
        );
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2WssV1,
                v1::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let ready = client.ingest(hello_result).unwrap();
        let ready_result = server.ingest(client.build_ready_frame(&ready).unwrap()).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&ready_result.json).unwrap();
        value["result"]["provides"] = serde_json::json!([]);
        assert!(matches!(
            client.ingest(BootstrapFrame {
                json: value.to_string(),
            }),
            Err(BootstrapError::UnsupportedCapability(_))
        ));
    }
}

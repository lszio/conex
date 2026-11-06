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
//! This module is a pure state machine; no Tokio / WS I/O. Wiring onto
//! axum/hyper lives with the processes that speak this protocol today:
//! `conex-host` (server side) and `conex-agent` (client side).
#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use conex_proto;

/// UTF-8 JSON-RPC 2.0 envelope, capped at 64 KiB per message.
pub const MAX_BOOTSTRAP_MESSAGE_BYTES: usize = 65_536;

/// Default handshake timeout: 10 seconds.
pub const DEFAULT_HANDSHAKE_TIMEOUT_MS: u64 = 10_000;

/// Identifier for the two bootstrap profiles the demo slice recognises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileId {
    #[serde(rename = "conex-jsonrpc2-wss")]
    JsonRpc2Wss,
    #[serde(rename = "conex-protobuf-wss")]
    ProtobufWss,
}

impl ProfileId {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProfileId::JsonRpc2Wss => "conex-jsonrpc2-wss",
            ProfileId::ProtobufWss => "conex-protobuf-wss",
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

pub fn plane_name(plane: conex_proto::Plane) -> &'static str {
    match plane {
        conex_proto::Plane::Broker => "broker",
        conex_proto::Plane::Relay => "relay",
        conex_proto::Plane::Unspecified => "unspecified",
    }
}
fn plane_from_str(s: &str) -> Option<conex_proto::Plane> {
    match s {
        "broker" => Some(conex_proto::Plane::Broker),
        "relay" => Some(conex_proto::Plane::Relay),
        _ => None,
    }
}

/// Plane as a handshake result carries it: the legacy JSON projection uses
/// lowercase names ("broker"), the S1 pbjson projection uses the proto enum
/// names ("PLANE_BROKER") and also accepts numbers. `Unspecified` is not a
/// negotiable plane in either shape.
fn plane_from_result(value: &serde_json::Value) -> Result<conex_proto::Plane, BootstrapError> {
    let unknown = |v: String| BootstrapError::MalformedEnvelope(format!("unknown plane: {v}"));
    if let Some(text) = value.as_str() {
        if let Some(plane) = plane_from_str(text) {
            return Ok(plane);
        }
        return match text {
            "PLANE_BROKER" => Ok(conex_proto::Plane::Broker),
            "PLANE_RELAY" => Ok(conex_proto::Plane::Relay),
            _ => Err(unknown(text.to_owned())),
        };
    }
    let number = value
        .as_i64()
        .and_then(|n| i32::try_from(n).ok())
        .ok_or_else(|| unknown(value.to_string()))?;
    match conex_proto::Plane::try_from(number) {
        Ok(plane @ (conex_proto::Plane::Broker | conex_proto::Plane::Relay)) => Ok(plane),
        _ => Err(unknown(number.to_string())),
    }
}

/// Generated control type used for both JSON ready projection and protobuf
/// mapping. JSON uses explicit lowercase field/plane mapping below because
/// prost messages do not implement serde.
pub type ReadyRequest = conex_proto::ReadyRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapFrame {
    pub json: String,
}

fn profile_from_str(s: &str) -> Option<ProfileId> {
    match s {
        "conex-jsonrpc2-wss" => Some(ProfileId::JsonRpc2Wss),
        "conex-protobuf-wss" => Some(ProfileId::ProtobufWss),
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

fn default_limits() -> conex_proto::Limits {
    conex_proto::Limits {
        max_frame_bytes: 1_048_576,
        max_inflight: 4,
        max_queued_bytes: 8_388_608,
        timeout_ms: 8_000,
        // Legacy bootstrap has no module or subscription capabilities.
        max_subscriptions: 0,
        max_modules: 0,
    }
}

fn limits_json(limits: conex_proto::Limits) -> serde_json::Value {
    let mut value = serde_json::json!({
        "maxFrameBytes": limits.max_frame_bytes,
        "maxInflight": limits.max_inflight,
        "maxQueuedBytes": limits.max_queued_bytes,
        "timeoutMs": limits.timeout_ms,
    });
    if limits.max_subscriptions != 0 {
        value["maxSubscriptions"] = limits.max_subscriptions.into();
    }
    if limits.max_modules != 0 {
        value["maxModules"] = limits.max_modules.into();
    }
    value
}

/// Componentwise "a rises above b" test shared by both handshake sides: a
/// negotiated limit must never exceed the counterpart's ceiling (S1 契约
/// 「各方协商取可支持的下限」，下调合法，上调即违约).
fn limits_exceed(a: &conex_proto::Limits, b: &conex_proto::Limits) -> bool {
    a.max_frame_bytes > b.max_frame_bytes
        || a.max_inflight > b.max_inflight
        || a.max_queued_bytes > b.max_queued_bytes
        || a.timeout_ms > b.timeout_ms
        || a.max_subscriptions > b.max_subscriptions
        || a.max_modules > b.max_modules
}

fn limits_from_json(value: &serde_json::Value) -> Result<conex_proto::Limits, BootstrapError> {
    let number = |field: &str| {
        value
            .get(field)
            .and_then(serde_json::Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(|| BootstrapError::MalformedEnvelope(format!("invalid limits.{field}")))
    };
    Ok(conex_proto::Limits {
        max_frame_bytes: number("maxFrameBytes")?,
        max_inflight: number("maxInflight")?,
        max_queued_bytes: number("maxQueuedBytes")?,
        timeout_ms: number("timeoutMs")?,
        max_subscriptions: value
            .get("maxSubscriptions")
            .map(|_| number("maxSubscriptions"))
            .transpose()?
            .unwrap_or(0),
        max_modules: value
            .get("maxModules")
            .map(|_| number("maxModules"))
            .transpose()?
            .unwrap_or(0),
    })
}

fn string_list(value: &serde_json::Value, field: &str) -> Result<Vec<String>, BootstrapError> {
    value
        .get(field)
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| BootstrapError::MalformedEnvelope(format!("missing {field}")))?
        .iter()
        .map(|item| {
            item.as_str().map(str::to_owned).ok_or_else(|| {
                BootstrapError::MalformedEnvelope(format!("{field} must be strings"))
            })
        })
        .collect()
}

fn hello_request_from_json(
    value: &serde_json::Value,
) -> Result<conex_proto::HelloRequest, BootstrapError> {
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
    Ok(conex_proto::HelloRequest {
        profile_id,
        plane: plane as i32,
        provides: string_list(value, "provides")?,
        requires: string_list(value, "requires")?,
    })
}
fn ready_request_from_json(
    value: &serde_json::Value,
) -> Result<conex_proto::ReadyRequest, BootstrapError> {
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
    let limits = value.get("limits").map(limits_from_json).transpose()?;
    Ok(conex_proto::ReadyRequest {
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

    server_plane: conex_proto::Plane,
    server_capabilities: conex_proto::NegotiatedCapabilities,
    link_identity: conex_proto::LinkIdentity,
    limits: conex_proto::Limits,
    /// S1 ready confirms the client's own hello declarations, not the host's.
    s1_client_provides: Option<Vec<String>>,
}

impl ServerHandshake {
    pub fn new(server_profile: ProfileId, server_plane: conex_proto::Plane) -> Self {
        Self::new_with_profiles(&[server_profile], server_plane)
    }

    /// The server may accept more than one profile (design §4.4). The first
    /// entry is the default advertised when the client does not pin one.
    pub fn new_with_profiles(
        server_profiles: &[ProfileId],
        server_plane: conex_proto::Plane,
    ) -> Self {
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
        server_plane: conex_proto::Plane,
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
            server_capabilities: conex_proto::NegotiatedCapabilities {
                provides: server_provides,
                requires: Vec::new(),
                rejected_capabilities: Vec::new(),
            },
            link_identity: conex_proto::LinkIdentity {
                link_id: negotiation_id,
                peer_id: String::new(),
                tenant_id: String::new(),
            },
            limits: default_limits(),
            s1_client_provides: None,
        }
    }

    /// Supply host limits and authenticated link identity before ingesting hello.
    pub fn configure_s1(
        &mut self,
        limits: conex_proto::Limits,
        identity: conex_proto::LinkIdentity,
    ) {
        self.limits = limits;
        self.link_identity = identity;
    }

    /// Offered limits before ready, negotiated limits after successful ready.
    pub fn limits(&self) -> conex_proto::Limits {
        self.limits
    }

    pub fn state(&self) -> &BootstrapState {
        &self.state
    }

    pub fn link_identity(&self) -> &conex_proto::LinkIdentity {
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
        let req_plane =
            conex_proto::Plane::try_from(req.plane).unwrap_or(conex_proto::Plane::Unspecified);
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
        let s1 = req.requires.iter().any(|method| {
            matches!(
                method.as_str(),
                "module/register"
                    | "module/unregister"
                    | "module/list"
                    | "module/invoke"
                    | "module/cancel"
                    | "state/subscribe"
                    | "state/unsubscribe"
            )
        });
        if s1 {
            conex_proto::capability::validate_limits(&self.limits)
                .map_err(|_| BootstrapError::LimitsMismatch)?;
            if !conex_proto::wire::is_ulid(&self.negotiation_id) {
                return Err(BootstrapError::MalformedEnvelope(
                    "S1 bindingId must be a ULID".into(),
                ));
            }
        }
        let limits = if s1 { self.limits } else { default_limits() };
        let response = conex_proto::HelloResponse {
            binding_id: self.negotiation_id.clone(),
            expires_in_ms: 0,
            profile_id: negotiated.as_str().to_owned(),
            plane: self.server_plane as i32,
            provides: self.server_capabilities.provides.clone(),
            rejected_capabilities: Vec::new(),
            limits: Some(limits),
        };
        let result = if s1 {
            let mut result = serde_json::to_value(&response)
                .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?;
            // Explicit zero satisfies the generated JSON schema's required scalar.
            result["expiresInMs"] = response.expires_in_ms.into();
            result
        } else {
            serde_json::json!({
                "negotiationId": self.negotiation_id,
                "profileId": response.profile_id,
                "plane": plane_name(self.server_plane),
                "limits": limits_json(limits),
                "provides": response.provides,
                "requires": self.server_capabilities.requires,
                "linkIdentity": {
                    "linkId": self.link_identity.link_id,
                    "peerId": self.link_identity.peer_id,
                    "tenantId": self.link_identity.tenant_id,
                },
            })
        };
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": value.get("id").cloned().unwrap_or(serde_json::Value::Null),
            "result": result,
        });
        let json = serde_json::to_string(&response)
            .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?;
        if json.len() > MAX_BOOTSTRAP_MESSAGE_BYTES {
            return Err(BootstrapError::BufferExceeded);
        }
        self.agreed_profile = Some(negotiated);
        self.limits = limits;
        self.s1_client_provides = s1.then_some(req.provides);
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
                .unwrap_or(ProfileId::JsonRpc2Wss)
        });
        if req.profile_id != agreed.as_str() {
            return Err(BootstrapError::UnsupportedProfile(req.profile_id));
        }
        if req.negotiation_id != self.negotiation_id {
            return Err(BootstrapError::NegotiationIdMismatch);
        }
        let req_plane =
            conex_proto::Plane::try_from(req.plane).unwrap_or(conex_proto::Plane::Unspecified);
        if req_plane != self.server_plane {
            return Err(BootstrapError::PlaneMismatch {
                client: plane_name(req_plane).to_string(),
                server: plane_name(self.server_plane).to_string(),
            });
        }
        let expected_provides = self
            .s1_client_provides
            .as_ref()
            .unwrap_or(&self.server_capabilities.provides);
        let provides_match = if self.s1_client_provides.is_some() {
            expected_provides
                .iter()
                .all(|capability| req.provides.contains(capability))
                && req
                    .provides
                    .iter()
                    .all(|capability| expected_provides.contains(capability))
        } else {
            req.provides == *expected_provides
        };
        if !provides_match {
            let missing = expected_provides
                .iter()
                .find(|capability| !req.provides.contains(capability))
                .cloned()
                .unwrap_or_else(|| "capability set mismatch".into());
            return Err(BootstrapError::UnsupportedCapability(missing));
        }
        let limits = req.limits.ok_or(BootstrapError::LimitsMismatch)?;
        if self.s1_client_provides.is_some() {
            conex_proto::capability::validate_limits(&limits)
                .map_err(|_| BootstrapError::LimitsMismatch)?;
            if limits_exceed(&limits, &self.limits) {
                return Err(BootstrapError::LimitsMismatch);
            }
        } else if limits != self.limits {
            return Err(BootstrapError::LimitsMismatch);
        }
        let ready_response = conex_proto::ReadyResponse {
            negotiation_id: self.negotiation_id.clone(),
            profile_id: agreed.as_str().to_owned(),
            plane: self.server_plane as i32,
            provides: self.server_capabilities.provides.clone(),
            limits: Some(limits),
            link_identity: Some(self.link_identity.clone()),
        };
        let result = if self.s1_client_provides.is_some() {
            serde_json::to_value(&ready_response)
                .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?
        } else {
            serde_json::json!({
                "negotiationId": ready_response.negotiation_id,
                "profileId": ready_response.profile_id,
                "plane": plane_name(self.server_plane),
                "provides": ready_response.provides,
                "requires": self.server_capabilities.requires,
                "limits": limits_json(limits),
                "linkIdentity": {
                    "linkId": self.link_identity.link_id,
                    "peerId": self.link_identity.peer_id,
                    "tenantId": self.link_identity.tenant_id,
                },
            })
        };
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": value.get("id").cloned().unwrap_or(serde_json::Value::Null),
            "result": result,
        });
        let json = serde_json::to_string(&response)
            .map_err(|e| BootstrapError::MalformedEnvelope(e.to_string()))?;
        if json.len() > MAX_BOOTSTRAP_MESSAGE_BYTES {
            return Err(BootstrapError::BufferExceeded);
        }
        self.limits = limits;
        self.state = BootstrapState::Ready {
            profile: agreed,
            negotiation_id: self.negotiation_id.clone(),
        };
        Ok(BootstrapFrame { json })
    }
}

/// Client-side handshake.
///
/// Limits follow the S1 negotiation rule (契约「各方协商取可支持的下限」):
/// without [`ClientHandshake::with_client_limits`] the ready proposal echoes
/// the server's hello offer; with it, the proposal is the client's own
/// limits lowered by the offer via [`lower_limits`]. The ready result must
/// never rise above the offer or the recorded proposal.
pub struct ClientHandshake {
    state: BootstrapState,
    expected_negotiation_id: String,
    agreed_profile: Option<ProfileId>,
    server_hello_plane: conex_proto::Plane,
    server_hello_profile: String,
    server_hello_id: Option<serde_json::Value>,
    request_counter: u64,
    client_requires: Vec<String>,
    client_provides: Vec<String>,
    server_provides: Vec<String>,
    server_limits: Option<conex_proto::Limits>,
    proposed_limits: Option<conex_proto::Limits>,
    client_limits: Option<conex_proto::Limits>,
    s1_mode: bool,
    link_identity: Option<conex_proto::LinkIdentity>,
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
            server_hello_plane: conex_proto::Plane::Broker,
            server_hello_profile: String::new(),
            server_hello_id: None,
            request_counter: 0,
            client_requires: Vec::new(),
            client_provides: Vec::new(),
            server_provides: Vec::new(),
            server_limits: None,
            proposed_limits: None,
            client_limits: None,
            s1_mode: false,
            link_identity: None,
        }
    }

    /// Declare the client's own limits before hello. The ready proposal
    /// returned by `ingest` then becomes `lower_limits(client, server)`
    /// instead of a plain echo of the server's offer.
    pub fn with_client_limits(mut self, limits: conex_proto::Limits) -> Self {
        self.client_limits = Some(limits);
        self
    }

    /// Authenticated hub identity from the ready result (S1 parity with the
    /// SDK's `linkIdentity` check; `None` until a well-formed identity was
    /// seen, always `Some` after a successful S1 handshake).
    pub fn link_identity(&self) -> Option<&conex_proto::LinkIdentity> {
        self.link_identity.as_ref()
    }

    pub fn state(&self) -> &BootstrapState {
        &self.state
    }

    pub fn build_hello(
        &mut self,
        profile: ProfileId,
        plane: conex_proto::Plane,
        provides: Vec<String>,
        requires: Vec<String>,
    ) -> Result<BootstrapFrame, BootstrapError> {
        if !matches!(self.state, BootstrapState::AwaitingHello) {
            return Err(BootstrapError::RepeatedReady);
        }
        if plane == conex_proto::Plane::Unspecified {
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
        self.client_provides = provides;
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
                // The S1 pbjson projection names the binding "bindingId";
                // the legacy JSON projection names it "negotiationId".
                let s1_mode = result.get("bindingId").is_some();
                let negotiation_id = result
                    .get("negotiationId")
                    .or_else(|| result.get("bindingId"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        BootstrapError::MalformedEnvelope("missing negotiationId".into())
                    })?
                    .to_string();
                // SDK parity: an S1 binding is a ULID (the server enforces
                // the same on its own negotiation id).
                if s1_mode && !conex_proto::wire::is_ulid(&negotiation_id) {
                    return Err(BootstrapError::MalformedEnvelope(
                        "S1 bindingId must be a ULID".into(),
                    ));
                }
                if let Some(rejected) = result.get("rejectedCapabilities") {
                    let rejected = rejected.as_array().ok_or_else(|| {
                        BootstrapError::MalformedEnvelope("invalid rejectedCapabilities".into())
                    })?;
                    if let Some(first) = rejected.iter().find_map(|v| v.as_str()) {
                        return Err(BootstrapError::UnsupportedCapability(first.to_owned()));
                    }
                }
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
                let plane =
                    plane_from_result(result.get("plane").ok_or_else(|| {
                        BootstrapError::MalformedEnvelope("missing plane".into())
                    })?)?;
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
                let provides: HashSet<&str> = provides_values.iter().map(String::as_str).collect();
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
                // SDK parity: an S1 offer must be six positive fields with
                // the control reserve (legacy offers may omit the S1 caps).
                if s1_mode {
                    conex_proto::capability::validate_limits(&limits)
                        .map_err(|_| BootstrapError::LimitsMismatch)?;
                }
                let proposal = match self.client_limits {
                    Some(client) => conex_proto::capability::lower_limits(&client, &limits),
                    None => limits,
                };
                self.server_provides = provides_values.clone();
                self.server_limits = Some(limits);
                self.proposed_limits = Some(proposal);
                self.s1_mode = s1_mode;
                self.expected_negotiation_id = negotiation_id.clone();
                self.server_hello_plane = plane;
                self.server_hello_profile = profile_str.clone();
                self.server_hello_id = parsed.get("id").cloned();
                self.state = BootstrapState::AwaitingReady;
                // The ready frame declares the client's capability set. S1
                // servers check it set-equal against the client's own hello
                // declaration; legacy servers require a byte match of the
                // server's list.
                let ready_provides = if s1_mode {
                    self.client_provides.clone()
                } else {
                    provides_values
                };
                Ok(ReadyRequest {
                    negotiation_id,
                    profile_id: profile_str,
                    plane: plane as i32,
                    provides: ready_provides,
                    limits: Some(proposal),
                })
            }
            BootstrapState::AwaitingReady => {
                let result = parsed
                    .get("result")
                    .ok_or_else(|| BootstrapError::MalformedEnvelope("missing result".into()))?;
                let negotiation_id = result
                    .get("negotiationId")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        BootstrapError::MalformedEnvelope("missing negotiationId".into())
                    })?
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
                let plane =
                    plane_from_result(result.get("plane").ok_or_else(|| {
                        BootstrapError::MalformedEnvelope("missing plane".into())
                    })?)?;
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
                let limits_value = result.get("limits").ok_or(BootstrapError::LimitsMismatch)?;
                let limits = limits_from_json(limits_value)?;
                // The server may lower further, never raise: the reply is
                // bounded by both its hello offer and our recorded proposal.
                let offered = self
                    .server_limits
                    .as_ref()
                    .ok_or(BootstrapError::LimitsMismatch)?;
                if limits_exceed(&limits, offered) {
                    return Err(BootstrapError::LimitsMismatch);
                }
                if let Some(proposed) = self.proposed_limits.as_ref()
                    && limits_exceed(&limits, proposed)
                {
                    return Err(BootstrapError::LimitsMismatch);
                }
                // SDK parity: the negotiated result itself must stay a valid
                // six-field offer (a lower bound of valid limits is valid,
                // but a misbehaving server could send zeros).
                if self.s1_mode {
                    conex_proto::capability::validate_limits(&limits)
                        .map_err(|_| BootstrapError::LimitsMismatch)?;
                }
                // Authenticated hub identity (SDK parity): S1 requires it in
                // the ready result — ULID linkId plus non-empty peer/tenant.
                // Legacy servers also send it; record when well-formed and
                // otherwise leave it absent without failing the handshake.
                self.link_identity = result.get("linkIdentity").and_then(|v| {
                    let link_id = v.get("linkId").and_then(serde_json::Value::as_str)?;
                    let peer_id = v.get("peerId").and_then(serde_json::Value::as_str)?;
                    let tenant_id = v.get("tenantId").and_then(serde_json::Value::as_str)?;
                    if !conex_proto::wire::is_ulid(link_id)
                        || peer_id.is_empty()
                        || tenant_id.is_empty()
                    {
                        return None;
                    }
                    Some(conex_proto::LinkIdentity {
                        link_id: link_id.to_owned(),
                        peer_id: peer_id.to_owned(),
                        tenant_id: tenant_id.to_owned(),
                    })
                });
                if self.s1_mode && self.link_identity.is_none() {
                    return Err(BootstrapError::MalformedEnvelope(
                        "missing authenticated hub identity".into(),
                    ));
                }
                self.state = BootstrapState::Ready {
                    profile: self.agreed_profile.unwrap_or(ProfileId::JsonRpc2Wss),
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
        &mut self,
        ready: &ReadyRequest,
    ) -> Result<BootstrapFrame, BootstrapError> {
        // The invariant check on the ready result must compare against what
        // actually went on the wire, so record the sent proposal (callers
        // may have lowered the returned ReadyRequest further by hand).
        if ready.limits.is_some() {
            self.proposed_limits = ready.limits;
        }
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "id": self.server_hello_id.clone().unwrap_or(serde_json::Value::Null),
            "method": "conex/ready",
            "params": {
                "negotiationId": ready.negotiation_id,
                "profileId": ready.profile_id,
                "plane": plane_name(conex_proto::Plane::try_from(ready.plane).unwrap_or(conex_proto::Plane::Unspecified)),
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
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
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
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        let hello = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": "conex/hello",
            "params": {
                "profileId": "conex-protobuf-wss",
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
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        let hello = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": "conex/hello",
            "params": {
                "profileId": "conex-jsonrpc2-wss",
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
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        let bad = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": "conex/foobar",
            "params": {
                "profileId": "conex-jsonrpc2-wss",
                "plane": "broker",
            }
        })
        .to_string();
        let err = server.ingest(BootstrapFrame { json: bad }).unwrap_err();
        assert!(matches!(err, BootstrapError::UnknownMethod(_)));
    }

    #[test]
    fn repeated_hello_rejected_after_hello() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let _ = server.ingest(hello.clone()).unwrap();
        let second = server.ingest(hello);
        assert!(matches!(second, Err(BootstrapError::MalformedEnvelope(_))));
    }

    #[test]
    fn ready_frame_after_ready_rejected_as_early_business() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
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
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        let huge = "x".repeat(MAX_BOOTSTRAP_MESSAGE_BYTES + 1);
        let err = server.ingest(BootstrapFrame { json: huge }).unwrap_err();
        assert!(matches!(err, BootstrapError::BufferExceeded));
    }

    #[test]
    fn negotiation_id_mismatch_rejected_after_hello() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let _ = client.ingest(hello_result).unwrap();
        let bad = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "ready-spoof",
            "method": "conex/ready",
            "params": {
                "negotiationId": "spoofed",
                "profileId": "conex-jsonrpc2-wss",
                "plane": "broker",
            }
        })
        .to_string();
        let err = server.ingest(BootstrapFrame { json: bad }).unwrap_err();
        assert!(matches!(err, BootstrapError::NegotiationIdMismatch));
    }

    #[test]
    fn both_profiles_known() {
        let s1 = ProfileId::JsonRpc2Wss.as_str();
        let s2 = ProfileId::ProtobufWss.as_str();
        assert_eq!(s1, "conex-jsonrpc2-wss");
        assert_eq!(s2, "conex-protobuf-wss");
    }

    #[test]
    fn early_business_frame_rejected_after_ready() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
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
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Unspecified,
                vec![],
                vec![],
            )
            .unwrap_err();
        assert!(matches!(err, BootstrapError::MissingPlane));
    }

    #[test]
    fn protobuf_profile_negotiates() {
        let mut server = ServerHandshake::new(ProfileId::ProtobufWss, conex_proto::Plane::Broker);
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::ProtobufWss,
                conex_proto::Plane::Broker,
                vec!["blob/commit".into()],
                vec![],
            )
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let ready = client.ingest(hello_result).unwrap();
        assert_eq!(ready.profile_id, "conex-protobuf-wss");
    }

    #[test]
    fn server_advertises_own_capabilities_and_rejects_hard_requires() {
        let mut server = ServerHandshake::new_with_profiles_and_capabilities(
            &[ProfileId::JsonRpc2Wss],
            conex_proto::Plane::Broker,
            vec!["source/read".into()],
            "neg-test".into(),
        );
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec!["ui/probe".into()],
                vec!["source/read".into()],
            )
            .unwrap();
        let response = server.ingest(hello).unwrap();
        let value: serde_json::Value = serde_json::from_str(&response.json).unwrap();
        assert_eq!(
            value["result"]["provides"],
            serde_json::json!(["source/read"])
        );
        assert_ne!(value["result"]["provides"], serde_json::json!(["ui/probe"]));

        let mut rejecting = ServerHandshake::new_with_profiles_and_capabilities(
            &[ProfileId::JsonRpc2Wss],
            conex_proto::Plane::Broker,
            vec!["source/read".into()],
            "neg-test".into(),
        );
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
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
            &[ProfileId::JsonRpc2Wss],
            conex_proto::Plane::Broker,
            vec!["source/read".into()],
            "neg-ready".into(),
        );
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
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
            &[ProfileId::JsonRpc2Wss],
            conex_proto::Plane::Broker,
            vec!["source/read".into()],
            "neg-client-ready".into(),
        );
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let ready = client.ingest(hello_result).unwrap();
        let ready_result = server
            .ingest(client.build_ready_frame(&ready).unwrap())
            .unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&ready_result.json).unwrap();
        value["result"]["provides"] = serde_json::json!([]);
        assert!(matches!(
            client.ingest(BootstrapFrame {
                json: value.to_string(),
            }),
            Err(BootstrapError::UnsupportedCapability(_))
        ));
    }

    #[test]
    fn client_models_limits_lowering_and_rejects_raised_ready_result() {
        // S1-mode servers speak the pbjson envelope (bindingId, numeric
        // plane), which `ClientHandshake` does not parse yet — the SDK drives
        // that shape today. Until the 4c migration, exercise the lowering
        // semantics with hand-crafted legacy-shaped frames from a server that
        // lowers, which is exactly what the state machine must accept.
        let offered = conex_proto::Limits {
            max_frame_bytes: 262_144,
            max_inflight: 16,
            max_queued_bytes: 1_048_576,
            timeout_ms: 8_000,
            max_subscriptions: 32,
            max_modules: 16,
        };
        let wanted = conex_proto::Limits {
            max_frame_bytes: 65_536,
            timeout_ms: 4_000,
            ..offered
        };
        let proposal = conex_proto::capability::lower_limits(&wanted, &offered);
        let hello_result = || {
            serde_json::json!({
                "jsonrpc": "2.0", "id": "req-1",
                "result": {
                    "negotiationId": "neg-lowered",
                    "profileId": "conex-jsonrpc2-wss",
                    "plane": "broker",
                    "provides": ["module/invoke", "state/subscribe"],
                    "limits": limits_json(offered),
                },
            })
            .to_string()
        };
        let ready_result = |limits: serde_json::Value| {
            serde_json::json!({
                "jsonrpc": "2.0", "id": "req-1",
                "result": {
                    "negotiationId": "neg-lowered",
                    "profileId": "conex-jsonrpc2-wss",
                    "plane": "broker",
                    "provides": ["module/invoke", "state/subscribe"],
                    "limits": limits,
                },
            })
            .to_string()
        };

        // The proposal is the client's wish lowered by the offer; the ready
        // frame carries it; an echoed lowered reply completes the handshake.
        let mut client = ClientHandshake::new().with_client_limits(wanted);
        client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec!["module/invoke".to_owned(), "state/subscribe".to_owned()],
                vec![],
            )
            .unwrap();
        let returned = client
            .ingest(BootstrapFrame {
                json: hello_result(),
            })
            .unwrap();
        assert_eq!(returned.limits, Some(proposal));
        let ready_frame = client.build_ready_frame(&returned).unwrap();
        let frame_value: serde_json::Value = serde_json::from_str(&ready_frame.json).unwrap();
        assert_eq!(frame_value["params"]["limits"], limits_json(proposal));
        client
            .ingest(BootstrapFrame {
                json: ready_result(limits_json(proposal)),
            })
            .unwrap();
        assert!(matches!(client.state(), BootstrapState::Ready { .. }));

        // A reply above the proposal (still within the offer) is a violation.
        let mut client = ClientHandshake::new().with_client_limits(wanted);
        client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let returned = client
            .ingest(BootstrapFrame {
                json: hello_result(),
            })
            .unwrap();
        client.build_ready_frame(&returned).unwrap();
        let mut raised = limits_json(proposal);
        // proposal 4000 < 6000 <= offered 8000: only the proposal bound trips.
        raised["timeoutMs"] = serde_json::json!(6_000u64);
        assert!(matches!(
            client.ingest(BootstrapFrame {
                json: ready_result(raised),
            }),
            Err(BootstrapError::LimitsMismatch)
        ));

        // A reply above the hello offer is a violation too.
        let mut client = ClientHandshake::new().with_client_limits(wanted);
        client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let returned = client
            .ingest(BootstrapFrame {
                json: hello_result(),
            })
            .unwrap();
        client.build_ready_frame(&returned).unwrap();
        let mut raised = limits_json(proposal);
        raised["timeoutMs"] = serde_json::json!(9_000u64);
        assert!(matches!(
            client.ingest(BootstrapFrame {
                json: ready_result(raised),
            }),
            Err(BootstrapError::LimitsMismatch)
        ));
    }

    #[test]
    fn client_completes_s1_pbjson_round_trip_with_core_server() {
        // The pairing no test ever exercised: the core client against the
        // core server's S1 mode, whose results are the pbjson shape
        // (bindingId, PLANE_BROKER, linkIdentity) rather than legacy JSON.
        let offered = conex_proto::Limits {
            max_frame_bytes: 262_144,
            max_inflight: 16,
            max_queued_bytes: 1_048_576,
            timeout_ms: 8_000,
            max_subscriptions: 32,
            max_modules: 16,
        };
        let wanted = conex_proto::Limits {
            max_frame_bytes: 65_536,
            timeout_ms: 4_000,
            ..offered
        };
        let identity = conex_proto::LinkIdentity {
            link_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            peer_id: "node-a".into(),
            tenant_id: "tenant-a".into(),
        };
        // The server advertises more than the client declares; an S1 ready
        // must carry the client's own set (set-equality server-side), not
        // the server's list.
        let mut server = ServerHandshake::new_with_profiles_and_capabilities(
            &[ProfileId::ProtobufWss],
            conex_proto::Plane::Broker,
            vec![
                "module/invoke".to_owned(),
                "state/subscribe".to_owned(),
                "agent/heartbeat".to_owned(),
            ],
            "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
        );
        server.configure_s1(offered, identity.clone());
        let mut client = ClientHandshake::new().with_client_limits(wanted);
        let hello = client
            .build_hello(
                ProfileId::ProtobufWss,
                conex_proto::Plane::Broker,
                vec!["module/invoke".to_owned(), "state/subscribe".to_owned()],
                vec!["module/invoke".to_owned()],
            )
            .unwrap();
        let returned = client.ingest(server.ingest(hello).unwrap()).unwrap();
        assert_eq!(
            returned.limits,
            Some(conex_proto::capability::lower_limits(&wanted, &offered))
        );
        assert_eq!(
            returned.provides,
            vec!["module/invoke".to_owned(), "state/subscribe".to_owned()]
        );
        let ready_result = server
            .ingest(client.build_ready_frame(&returned).unwrap())
            .unwrap();
        client.ingest(ready_result).unwrap();
        assert!(matches!(client.state(), BootstrapState::Ready { .. }));
        assert_eq!(client.link_identity(), Some(&identity));
    }

    #[test]
    fn client_rejects_rejected_capabilities_in_s1_hello() {
        let mut client = ClientHandshake::new();
        client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let hello_result = serde_json::json!({
            "jsonrpc": "2.0", "id": "req-1",
            "result": {
                "bindingId": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "expiresInMs": 0,
                "profileId": "conex-jsonrpc2-wss",
                "plane": "PLANE_BROKER",
                "provides": ["module/invoke"],
                "rejectedCapabilities": ["module/invoke"],
                "limits": limits_json(conex_proto::Limits {
                    max_subscriptions: 4,
                    max_modules: 4,
                    ..default_limits()
                }),
            },
        })
        .to_string();
        assert!(matches!(
            client.ingest(BootstrapFrame { json: hello_result }),
            Err(BootstrapError::UnsupportedCapability(_))
        ));
    }

    #[test]
    fn client_requires_hub_identity_in_s1_ready_result() {
        let limits = limits_json(conex_proto::Limits {
            max_subscriptions: 4,
            max_modules: 4,
            ..default_limits()
        });
        let mut client = ClientHandshake::new();
        client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec!["module/invoke".to_owned()],
                vec![],
            )
            .unwrap();
        let hello_result = serde_json::json!({
            "jsonrpc": "2.0", "id": "req-1",
            "result": {
                "bindingId": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "expiresInMs": 0,
                "profileId": "conex-jsonrpc2-wss",
                "plane": "PLANE_BROKER",
                "provides": ["module/invoke"],
                "limits": limits,
            },
        })
        .to_string();
        client
            .ingest(BootstrapFrame { json: hello_result })
            .unwrap();
        let ready_result = serde_json::json!({
            "jsonrpc": "2.0", "id": "req-1",
            "result": {
                "negotiationId": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "profileId": "conex-jsonrpc2-wss",
                "plane": "PLANE_BROKER",
                "provides": ["module/invoke"],
                "limits": limits,
            },
        })
        .to_string();
        assert!(matches!(
            client.ingest(BootstrapFrame { json: ready_result }),
            Err(BootstrapError::MalformedEnvelope(_))
        ));
    }

    #[test]
    fn client_accepts_pbjson_enum_numbers_for_plane() {
        // pbjson deserializers accept enum numbers; the client mirrors that
        // parity for results while still sending legacy lowercase frames.
        let limits = limits_json(conex_proto::Limits {
            max_subscriptions: 4,
            max_modules: 4,
            ..default_limits()
        });
        let mut client = ClientHandshake::new();
        client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let hello_result = serde_json::json!({
            "jsonrpc": "2.0", "id": "req-1",
            "result": {
                "negotiationId": "neg-numeric-plane",
                "profileId": "conex-jsonrpc2-wss",
                "plane": 1,
                "provides": [],
                "limits": limits,
            },
        })
        .to_string();
        client
            .ingest(BootstrapFrame { json: hello_result })
            .unwrap();
        assert!(matches!(client.state(), BootstrapState::AwaitingReady));
    }

    #[test]
    fn client_applies_sdk_parity_checks_in_s1_mode() {
        let limits = serde_json::json!({
            "maxFrameBytes": 262_144u64, "maxInflight": 16u64,
            "maxQueuedBytes": 1_048_576u64, "timeoutMs": 8_000u64,
            "maxSubscriptions": 4u64, "maxModules": 4u64,
        });
        let s1_hello = |binding: &str, limits: serde_json::Value| {
            serde_json::json!({
                "jsonrpc": "2.0", "id": "req-1",
                "result": {
                    "bindingId": binding,
                    "expiresInMs": 0,
                    "profileId": "conex-jsonrpc2-wss",
                    "plane": "PLANE_BROKER",
                    "provides": ["module/invoke"],
                    "limits": limits,
                },
            })
            .to_string()
        };

        // A non-ULID bindingId is not an S1 binding.
        let mut client = ClientHandshake::new();
        client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        assert!(matches!(
            client.ingest(BootstrapFrame {
                json: s1_hello("not-a-ulid", limits.clone()),
            }),
            Err(BootstrapError::MalformedEnvelope(_))
        ));

        // An S1 offer without the subscription cap (zero is not positive)
        // is rejected before it can become a proposal.
        let mut client = ClientHandshake::new();
        client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let mut capped = limits.clone();
        capped.as_object_mut().unwrap().remove("maxSubscriptions");
        assert!(matches!(
            client.ingest(BootstrapFrame {
                json: s1_hello("01ARZ3NDEKTSV4RRFFQ69G5FAV", capped),
            }),
            Err(BootstrapError::LimitsMismatch)
        ));

        // A ready result that lowers a cap to zero violates the negotiated
        // validity even though it stays below the proposal.
        let mut client = ClientHandshake::new();
        client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec!["module/invoke".to_owned()],
                vec![],
            )
            .unwrap();
        client
            .ingest(BootstrapFrame {
                json: s1_hello("01ARZ3NDEKTSV4RRFFQ69G5FAV", limits.clone()),
            })
            .unwrap();
        let mut zeroed = limits;
        zeroed.as_object_mut().unwrap().remove("maxSubscriptions");
        let ready_result = serde_json::json!({
            "jsonrpc": "2.0", "id": "req-1",
            "result": {
                "negotiationId": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
                "profileId": "conex-jsonrpc2-wss",
                "plane": "PLANE_BROKER",
                "provides": ["module/invoke"],
                "limits": zeroed,
            },
        })
        .to_string();
        assert!(matches!(
            client.ingest(BootstrapFrame { json: ready_result }),
            Err(BootstrapError::LimitsMismatch)
        ));
    }

    fn s1_handshake() -> (ServerHandshake, serde_json::Value, Vec<String>) {
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let mut server = ServerHandshake::new_with_profiles_and_capabilities(
            &[ProfileId::JsonRpc2Wss],
            conex_proto::Plane::Broker,
            [
                "agent/heartbeat",
                "module/list",
                "module/invoke",
                "state/subscribe",
            ]
            .map(str::to_owned)
            .to_vec(),
            id.into(),
        );
        server.configure_s1(
            conex_proto::Limits {
                max_subscriptions: 64,
                max_modules: 32,
                ..default_limits()
            },
            conex_proto::LinkIdentity {
                link_id: id.into(),
                peer_id: "node-a".into(),
                tenant_id: "tenant-a".into(),
            },
        );
        let provides = ["module/invoke", "state/subscribe"]
            .map(str::to_owned)
            .to_vec();
        let hello = server
            .ingest(BootstrapFrame {
                json: serde_json::json!({
                    "jsonrpc": "2.0", "id": id, "method": "conex/hello",
                    "params": {
                        "profileId": "conex-jsonrpc2-wss", "plane": "broker",
                        "provides": provides, "requires": ["module/list"],
                    },
                })
                .to_string(),
            })
            .unwrap();
        (server, serde_json::from_str(&hello.json).unwrap(), provides)
    }

    fn s1_ready(
        hello: &serde_json::Value,
        provides: &[String],
        limits: serde_json::Value,
    ) -> BootstrapFrame {
        BootstrapFrame {
            json: serde_json::json!({
                "jsonrpc": "2.0", "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "method": "conex/ready",
                "params": {
                    "negotiationId": hello["result"]["bindingId"],
                    "profileId": "conex-jsonrpc2-wss", "plane": "broker",
                    "provides": provides, "limits": limits,
                },
            })
            .to_string(),
        }
    }

    #[test]
    fn s1_accepts_asymmetric_provides_and_six_lowered_limits() {
        let (mut server, hello, mut provides) = s1_handshake();
        let typed: conex_proto::HelloResponse =
            serde_json::from_value(hello["result"].clone()).unwrap();
        assert!(conex_proto::wire::is_ulid(&typed.binding_id));
        assert_eq!(typed.limits, Some(server.limits()));
        assert_ne!(typed.provides, provides);
        let lowered = conex_proto::Limits {
            max_frame_bytes: 524_288,
            max_inflight: 2,
            max_queued_bytes: 4_194_304,
            timeout_ms: 4_000,
            max_subscriptions: 16,
            max_modules: 8,
        };
        provides.reverse();
        let response = server
            .ingest(s1_ready(&hello, &provides, limits_json(lowered)))
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&response.json).unwrap();
        let ready: conex_proto::ReadyResponse =
            serde_json::from_value(value["result"].clone()).unwrap();
        assert_eq!(ready.negotiation_id, typed.binding_id);
        assert_eq!(ready.provides, typed.provides);
        assert_eq!(ready.limits, Some(lowered));
        assert_eq!(server.limits(), lowered);
        assert_eq!(ready.link_identity.as_ref(), Some(server.link_identity()));
        assert!(matches!(server.state(), BootstrapState::Ready { .. }));
    }

    #[test]
    fn s1_rejects_raised_missing_or_zero_limits() {
        for field in [
            "maxFrameBytes",
            "maxInflight",
            "maxQueuedBytes",
            "timeoutMs",
            "maxSubscriptions",
            "maxModules",
        ] {
            for invalid in 0..3 {
                let (mut server, hello, provides) = s1_handshake();
                let offered = server.limits();
                let mut limits = hello["result"]["limits"].clone();
                match invalid {
                    0 => limits[field] = serde_json::json!(limits[field].as_u64().unwrap() + 1),
                    1 => {
                        limits.as_object_mut().unwrap().remove(field);
                    }
                    _ => limits[field] = serde_json::json!(0),
                }
                assert!(
                    server.ingest(s1_ready(&hello, &provides, limits)).is_err(),
                    "{field}"
                );
                assert_eq!(server.limits(), offered);
                assert!(matches!(server.state(), BootstrapState::AwaitingReady));
            }
        }
    }

    #[test]
    fn s1_rejects_changed_client_provides() {
        let (mut server, hello, _) = s1_handshake();
        let provides = vec!["module/invoke".into()];
        assert!(matches!(
            server.ingest(s1_ready(
                &hello,
                &provides,
                hello["result"]["limits"].clone()
            )),
            Err(BootstrapError::UnsupportedCapability(_))
        ));
    }

    #[test]
    fn legacy_limits_omit_s1_fields_and_default_them_to_zero() {
        let limits = limits_json(default_limits());
        assert!(limits.get("maxSubscriptions").is_none());
        assert!(limits.get("maxModules").is_none());
        assert_eq!(limits_from_json(&limits).unwrap(), default_limits());
    }

    #[test]
    fn s1_configured_server_preserves_legacy_limit_echo() {
        let mut server = ServerHandshake::new(ProfileId::JsonRpc2Wss, conex_proto::Plane::Broker);
        server.configure_s1(
            conex_proto::Limits {
                max_subscriptions: 64,
                max_modules: 32,
                ..default_limits()
            },
            conex_proto::LinkIdentity::default(),
        );
        let mut client = ClientHandshake::new();
        let hello = client
            .build_hello(
                ProfileId::JsonRpc2Wss,
                conex_proto::Plane::Broker,
                vec![],
                vec![],
            )
            .unwrap();
        let hello_result = server.ingest(hello).unwrap();
        let ready = client.ingest(hello_result).unwrap();
        assert_eq!(ready.limits, Some(default_limits()));
        let ready_result = server
            .ingest(client.build_ready_frame(&ready).unwrap())
            .unwrap();
        client.ingest(ready_result).unwrap();
        assert_eq!(server.limits(), default_limits());
        assert!(matches!(server.state(), BootstrapState::Ready { .. }));
    }
}

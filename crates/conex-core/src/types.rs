//! Cross-task core types. Later tasks fill implementations; names are fixed here.
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use bytes::Bytes;
use conex_proto::v1;

use crate::ports::{Connection, Handler};

pub type CallResult<T> = Result<T, CallError>;

/// Core newtype over the wire error. Business code uses the frozen ErrorCode.
#[derive(Debug, Clone, PartialEq)]
pub struct CallError {
    error: Box<v1::Error>,
}

impl CallError {
    pub fn new(code: v1::ErrorCode, message: impl Into<String>) -> Self {
        Self {
            error: Box::new(v1::Error {
                code: code as i32,
                message: message.into(),
                diagnostic_id: String::new(),
                execution: String::new(),
                retry: String::new(),
                details: None,
            }),
        }
    }

    pub fn from_wire(error: v1::Error) -> Self {
        Self {
            error: Box::new(error),
        }
    }

    pub fn code(&self) -> i32 {
        self.error.code
    }

    pub fn code_enum(&self) -> Option<v1::ErrorCode> {
        v1::ErrorCode::try_from(self.error.code).ok()
    }

    pub fn message(&self) -> &str {
        &self.error.message
    }

    pub fn wire(&self) -> &v1::Error {
        &self.error
    }

    pub fn into_wire(self) -> v1::Error {
        *self.error
    }

    pub fn with_diagnostic_id(mut self, id: impl Into<String>) -> Self {
        self.error.diagnostic_id = id.into();
        self
    }

    pub fn with_execution(mut self, execution: impl Into<String>) -> Self {
        self.error.execution = execution.into();
        self
    }

    pub fn with_retry(mut self, retry: impl Into<String>) -> Self {
        self.error.retry = retry.into();
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ResourceClaim {
    pub resource_id: String,
    pub action: String,
    pub subtree: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PrincipalTenant {
    pub principal_id: String,
    pub tenant_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PreparedInput {
    pub canonical: serde_json::Value,
    pub claim: ResourceClaim,
    /// Principal/tenant expected to be filled by the host's dispatcher from
    /// the authenticated caller. Contracts that compute it themselves may set
    /// it directly; the dispatcher will refuse to overwrite a non-empty
    /// value with the caller's claim.
    pub binding: PrincipalTenant,
}

/// A method's behavior contract. `prepare` is the authoritative strict decoder.
#[derive(Clone)]
pub struct MethodContract {
    pub adapter_id: &'static str,
    pub input_schema: &'static str,
    pub output_schema: &'static str,
    pub prepare: fn(&serde_json::Value) -> CallResult<PreparedInput>,
    pub validate_output: fn(&serde_json::Value) -> CallResult<()>,
}

impl fmt::Debug for MethodContract {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MethodContract")
            .field("input_schema", &self.input_schema)
            .field("output_schema", &self.output_schema)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IdentityKey {
    Oidc { issuer: String, subject: String },
    Service { issuer: String, service_id: String },
    Local { machine_id: String, user_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    pub principal_id: String,
    pub tenant_id: String,
    pub actor_peer_id: String,
}

#[derive(Debug, Clone)]
pub struct CallContext {
    pub caller: Caller,
    pub endpoint_id: String,
    pub plane: v1::Plane,
    pub method: String,
    pub claim: ResourceClaim,
    pub policy_version: u64,
    pub deadline: tokio::time::Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_frame_bytes: u32,
    pub max_inflight: u32,
    pub max_queued_bytes: u32,
    pub timeout_ms: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_frame_bytes: 1_048_576,
            max_inflight: 4,
            max_queued_bytes: 8_388_608,
            timeout_ms: 8_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub id: String,
    pub provider_id: String,
    pub tenant_id: String,
    pub plane: v1::Plane,
    pub provides: Vec<String>,
    pub limits: Limits,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TlsTrust {
    pub ca_pem: Option<Vec<u8>>,
    pub expected_server_name: Option<String>,
    pub pinned_cert: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: String,
    pub origin: String,
    pub fixed_path: String,
    pub allowed_addresses: Vec<IpAddr>,
    pub tls_trust: TlsTrust,
    pub allow_loopback_http: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedTarget {
    pub target_id: String,
    pub audience: String,
    pub scheme: String,
    pub hostname: String,
    pub server_name: String,
    pub port: u16,
    pub pinned_address: SocketAddr,
    pub fixed_path: String,
    pub allow_loopback_http: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerVerification {
    TlsServer,
    PinnedCertificate,
    InsecureLoopback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPeer {
    pub target_id: String,
    pub audience: String,
    pub address: SocketAddr,
    pub verification: PeerVerification,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CredentialKey {
    pub tenant_id: String,
    pub holder: String,
    pub provider_id: String,
    pub audience: String,
    pub name: String,
    pub principal_id: Option<String>,
}

/// Secret wrapper: Debug is always redacted; handlers must call `expose`.
pub struct Secret(secrecy::SecretString);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(secrecy::SecretString::from(value.into()))
    }

    pub fn expose(&self) -> &str {
        use secrecy::ExposeSecret;
        self.0.expose_secret()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone)]
pub struct OutboundRequest {
    pub method: http::Method,
    pub path: String,
    pub headers: http::HeaderMap,
    pub body: Bytes,
}

#[derive(Debug, Clone)]
pub struct OutboundResponse {
    pub status: u16,
    pub headers: http::HeaderMap,
    pub body: Bytes,
}

pub struct ExecutionIo {
    pub connection: Option<Box<dyn Connection>>,
    pub secret: Option<Secret>,
}

impl fmt::Debug for ExecutionIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExecutionIo")
            .field("has_connection", &self.connection.is_some())
            .field("has_secret", &self.secret.is_some())
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FactoryKey {
    pub kind: String,
    pub protocol: String,
    pub version: u32,
}

pub struct Route {
    pub protocol: String,
    pub version: u32,
    pub endpoint: Endpoint,
    pub method: String,
    pub contract: MethodContract,
    pub handler: Arc<dyn Handler>,
    pub target: Option<Target>,
    pub credential: Option<CredentialKey>,
}

impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Route")
            .field("protocol", &self.protocol)
            .field("version", &self.version)
            .field("endpoint", &self.endpoint.id)
            .field("method", &self.method)
            .field("has_target", &self.target.is_some())
            .field("has_credential", &self.credential.is_some())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Installation {
    pub endpoint: Endpoint,
    pub factory: FactoryKey,
    pub provider: serde_json::Value,
    pub target: Option<Target>,
    pub credential: Option<CredentialKey>,
}

pub type FactoryFn = fn(&Installation) -> CallResult<Vec<Route>>;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("duplicate registration: {0}")]
    DuplicateKey(String),
    #[error("unknown factory: {0}")]
    UnknownFactory(String),
    #[error("invalid installation: {0}")]
    InvalidInstallation(String),
}

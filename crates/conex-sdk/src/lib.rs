//! Symmetric S1 JSON WSS client, using the caller's Tokio runtime.
//!
//! No automatic retry, reconnect, subscription restoration, HTTP transport or agent daemon.
//! See the crate README for the provider's atomic snapshot/observer boundary.
#![forbid(unsafe_code)]
pub mod capability;
pub mod client;
pub mod plugin;
pub mod state;
pub use capability::{
    Cancellation, Contract, Execution, Exposure, InvocationContext, Method, ModuleRef, State,
};
pub use client::{Call, CallControl, Client, ConnectOptions};
pub use conex_proto as proto;
use conex_proto::ErrorCode;
pub use plugin::{
    Deps, Instance, InstanceConfig, ModuleContext, ModuleDecl, Plugin, PluginError, RemoteBinding,
    RemoteDep, Service, StartedModule, StopOutcome,
};
pub use state::{StateEvent, StateLease, StateObserver, Subscription};
pub type Result<T, E = Error> = std::result::Result<T, E>;
/// Retains the generated machine-readable Error, including diagnostic/details fields.
#[derive(Clone, Debug, PartialEq)]
pub struct Error(pub Box<conex_proto::Error>);
impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>, execution: &str, retry: &str) -> Self {
        Self(Box::new(conex_proto::Error {
            code: code as i32,
            message: message.into(),
            execution: execution.into(),
            retry: retry.into(),
            ..Default::default()
        }))
    }
    pub fn code(&self) -> i32 {
        self.0.code
    }
    pub fn execution(&self) -> &str {
        &self.0.execution
    }
    pub fn retry(&self) -> &str {
        &self.0.retry
    }
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::BadRequest, message, "not_started", "never")
    }
    pub(crate) fn protocol(error: conex_proto::wire::ProtocolError) -> Self {
        Self(Box::new(conex_proto::Error {
            code: error.rpc_code,
            message: error.message,
            execution: "not_started".into(),
            retry: "never".into(),
            ..Default::default()
        }))
    }
    pub(crate) fn decode(error: impl std::fmt::Display) -> Self {
        Self::bad_request(error.to_string())
    }
    pub(crate) fn unavailable() -> Self {
        Self::new(ErrorCode::Unavailable, "link closed", "not_started", "safe")
    }
    pub(crate) fn quota() -> Self {
        Self::new(
            ErrorCode::QuotaExceeded,
            "SDK resource limit",
            "not_started",
            "never",
        )
    }
    pub(crate) fn ended(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(
            code,
            message,
            "completed",
            if code == ErrorCode::Forbidden {
                "never"
            } else {
                "safe"
            },
        )
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: {} (execution={}, retry={})",
            self.0.code, self.0.message, self.0.execution, self.0.retry
        )
    }
}
impl std::error::Error for Error {}

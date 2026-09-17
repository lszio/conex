//! conex core: data-driven registration, method contracts and execution ports.
//!
//! This crate has no concrete provider or HTTP client dependency; providers and
//! transports plug in through the ports defined here (design J1/J2/P1/P2).
#![forbid(unsafe_code)]

pub mod aggregate;
pub mod audit;
pub mod contracts;
pub mod execution;
pub mod host;
pub mod identity;
pub mod limits;
pub mod operation;
pub mod policy;
pub mod ports;
pub mod registry;
pub mod session;
pub mod stream;
pub mod target_policy;
pub mod transport_ws;
pub mod types;

pub use aggregate::*;
pub use audit::*;
pub use host::*;
pub use identity::*;
pub use limits::*;
pub use policy::*;
pub use ports::*;
pub use registry::*;
pub use target_policy::*;
pub use types::*;

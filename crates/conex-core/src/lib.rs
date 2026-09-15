//! conex core: data-driven registration, method contracts and execution ports.
//!
//! This crate has no concrete provider or HTTP client dependency; providers and
//! transports plug in through the ports defined here (design J1/J2/P1/P2).
#![forbid(unsafe_code)]

pub mod ports;
pub mod registry;
pub mod types;

pub use ports::*;
pub use registry::*;
pub use types::*;

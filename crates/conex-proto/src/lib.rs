//! conex protocol types, generated from `.proto` sources.
//!
//! `.proto` is the single structural type source (design §13.1). Generated code
//! lives in `OUT_DIR`; the JSON Schema under `schema/generated/jsonschema` is a
//! derived artifact consumed by both the Rust and TypeScript boundaries.
//!
//! The API surface is unversioned: generated messages are re-exported from the
//! crate root (package `conex`), not nested under a version module.
#![forbid(unsafe_code)]

pub mod agent_domain;
#[cfg(has_conex)]
pub mod capability;
#[cfg(has_conex)]
pub mod cid;
pub mod validation;
#[cfg(has_conex)]
pub mod wire;

#[allow(clippy::all)]
pub mod test {
    //! Test-only schema (`conformance/schema`), never advertised as a capability.
    include!(concat!(env!("OUT_DIR"), "/conex.test.rs"));
    include!(concat!(env!("OUT_DIR"), "/conex.test.serde.rs"));
}

#[cfg(has_conex)]
#[allow(clippy::all)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/conex.rs"));
    include!(concat!(env!("OUT_DIR"), "/conex.serde.rs"));
}

#[cfg(has_conex)]
pub use generated::*;

/// Milliseconds since the UNIX epoch; the single clock every crate reads.
/// Wall clocks are the persisted-deadline semantics (receipts, sessions);
/// monotonic timing stays local to each call site.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

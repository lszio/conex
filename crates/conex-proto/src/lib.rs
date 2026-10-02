//! conex protocol types, generated from `.proto` sources.
//!
//! `.proto` is the single structural type source (design §13.1). Generated code
//! lives in `OUT_DIR`; the JSON Schema under `schema/generated/jsonschema` is a
//! derived artifact consumed by both the Rust and TypeScript boundaries.
//!
//! The API surface is unversioned: generated messages are re-exported from the
//! crate root (package `conex`), not nested under a version module.
#![forbid(unsafe_code)]

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

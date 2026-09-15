//! conex/1 protocol types, generated from `.proto` sources.
//!
//! `.proto` is the single structural type source (design §13.1). Generated code
//! lives in `OUT_DIR`; the JSON Schema under `schema/generated/jsonschema` is a
//! derived artifact consumed by both the Rust and TypeScript boundaries.
#![forbid(unsafe_code)]

pub mod validation;
#[cfg(has_conex_v1)]
pub mod wire;

#[allow(clippy::all)]
pub mod test {
    //! Test-only schema (`conformance/schema`), never advertised as a capability.
    pub mod v1 {
        include!(concat!(env!("OUT_DIR"), "/conex.test.v1.rs"));
        include!(concat!(env!("OUT_DIR"), "/conex.test.v1.serde.rs"));
    }
}

#[cfg(has_conex_v1)]
#[allow(clippy::all)]
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/conex.v1.rs"));
    include!(concat!(env!("OUT_DIR"), "/conex.v1.serde.rs"));
}

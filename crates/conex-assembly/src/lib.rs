//! Assembly root: the only place provider factories are registered.
#![forbid(unsafe_code)]
use conex_core::{FactoryFn, FactoryKey, Installation, Registry, RegistryError};

pub const P0_PROTOCOL: &str = "conex";
pub const P0_VERSION: u32 = 1;

pub fn factories() -> Vec<(FactoryKey, FactoryFn)> {
    vec![
        (
            FactoryKey {
                kind: "source-fs".into(),
                protocol: P0_PROTOCOL.into(),
                version: P0_VERSION,
            },
            conex_provider_fs::factory,
        ),
        (
            FactoryKey {
                kind: "source-http-catalog".into(),
                protocol: P0_PROTOCOL.into(),
                version: P0_VERSION,
            },
            conex_provider_http_catalog::factory,
        ),
    ]
}

pub fn register_all(registry: &mut Registry) -> Result<(), RegistryError> {
    for (key, factory) in factories() {
        registry.register_factory(key, factory)?;
    }
    Ok(())
}

pub fn install_all(
    registry: &mut Registry,
    installations: Vec<Installation>,
) -> Result<(), RegistryError> {
    for installation in installations {
        registry.install(installation)?;
    }
    Ok(())
}

//! Source-wide contracts: the shared validation with the source-wide claim
//! policy attached as route data (`MethodContract.claim_post`).
use conex_core::{MethodContract, ResourceClaim};
use conex_source::contracts::{
    self as shared, SOURCE_LIST, SOURCE_READ, SOURCE_SEARCH, source_wide_post,
};

pub fn contracts() -> Vec<(&'static str, MethodContract)> {
    shared::contracts()
        .into_iter()
        .map(|(method, base)| {
            let claim_post: Option<fn(ResourceClaim) -> ResourceClaim> = match method {
                SOURCE_LIST | SOURCE_READ | SOURCE_SEARCH => Some(source_wide_post),
                _ => None,
            };
            (
                method,
                MethodContract {
                    adapter_id: "conex-source-http-catalog",
                    claim_post,
                    ..base
                },
            )
        })
        .collect()
}

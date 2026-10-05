//! DNS resolution bounded by the caller deadline.
use std::net::IpAddr;

use async_trait::async_trait;
use conex_core::{CallError, CallResult, Resolver};
use conex_proto;
use tokio::time::{Instant, timeout_at};

pub struct TokioResolver;

#[async_trait]
impl Resolver for TokioResolver {
    async fn resolve(&self, hostname: &str, deadline: Instant) -> CallResult<Vec<IpAddr>> {
        let lookup = tokio::net::lookup_host((hostname, 0));
        let resolved = timeout_at(deadline, lookup)
            .await
            .map_err(|_| {
                CallError::new(
                    conex_proto::ErrorCode::Timeout,
                    "dns resolution exceeded the deadline",
                )
            })?
            .map_err(|error| {
                CallError::new(
                    conex_proto::ErrorCode::Unavailable,
                    format!("dns resolution failed: {error}"),
                )
            })?;
        let mut addresses: Vec<IpAddr> = resolved.map(|socket| socket.ip()).collect();
        addresses.sort();
        addresses.dedup();
        if addresses.is_empty() {
            return Err(CallError::new(
                conex_proto::ErrorCode::Unavailable,
                "dns returned no addresses",
            ));
        }
        Ok(addresses)
    }
}

//! Core execution ports. Concrete providers and transports implement these.
use std::net::IpAddr;

use async_trait::async_trait;

use crate::types::{
    AllowedTarget, CallContext, CallResult, CredentialKey, ExecutionIo, OutboundRequest,
    OutboundResponse, Secret, VerifiedPeer,
};

#[async_trait]
pub trait Handler: Send + Sync {
    async fn execute(
        &self,
        ctx: &CallContext,
        input: serde_json::Value,
        io: ExecutionIo,
    ) -> CallResult<serde_json::Value>;
}

/// M3: bounded byte-range reader. Providers that can serve raw slices
/// implement it so the same `/content` path works for local and remote
/// endpoints: the Host prefers the reverse-agent link and falls back to the
/// provider itself.
#[async_trait]
pub trait RangeReader: Send + Sync {
    /// Read `[offset, offset + length)` bound to `expected_revision`; a moved
    /// revision returns `stale_revision` rather than mixed-version bytes.
    async fn read_range(
        &self,
        ctx: &CallContext,
        offset: u64,
        length: usize,
        expected_revision: Option<&str>,
    ) -> CallResult<(Vec<u8>, String, bool)>;
}

#[async_trait]
pub trait Resolver: Send + Sync {
    async fn resolve(
        &self,
        hostname: &str,
        deadline: tokio::time::Instant,
    ) -> CallResult<Vec<IpAddr>>;
}

#[async_trait]
pub trait Connection: Send {
    fn peer(&self) -> &VerifiedPeer;

    async fn request(
        &mut self,
        request: OutboundRequest,
        deadline: tokio::time::Instant,
    ) -> CallResult<OutboundResponse>;
}

#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(
        &self,
        target: &AllowedTarget,
        deadline: tokio::time::Instant,
    ) -> CallResult<Box<dyn Connection>>;
}

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn resolve(&self, key: &CredentialKey, peer: &VerifiedPeer) -> CallResult<Secret>;
}

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

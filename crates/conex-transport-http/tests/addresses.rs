//! Resolution and pinned-address dialing.
mod support;

use std::time::Duration;

use conex_core::{Connector, Resolver};
use conex_transport_http::{HttpConnector, TlsTrustConfig, TokioResolver};
use tokio::time::Instant;

#[tokio::test]
async fn resolver_returns_loopback_for_localhost() {
    let addresses = TokioResolver
        .resolve("localhost", Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    assert!(addresses.iter().any(|ip| ip.is_loopback()));
}

#[tokio::test]
async fn connector_uses_the_admitted_address_only() {
    // Reserve then drop a port so the pinned address is closed.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let connector = HttpConnector::new(TlsTrustConfig::default(), 1024).unwrap();
    let target = support::allowed_target(addr, "http", "127.0.0.1", "/");
    let result = connector
        .connect(&target, Instant::now() + Duration::from_millis(500))
        .await;
    assert!(result.is_err());
}

//! A peer with the wrong identity fails before any HTTP request is sent.
mod support;

use std::time::Duration;

use conex_core::Connector;
use conex_transport_http::HttpConnector;
use tokio::time::Instant;

use support::tls_fixture::TlsFixture;

#[tokio::test]
async fn wrong_peer_fails_before_http_request() {
    let fixture = TlsFixture::wrong_hostname().await;
    let connector = HttpConnector::new(fixture.client_trust(), 1_048_576).unwrap();
    let result = connector
        .connect(
            &fixture.allowed_target(),
            Instant::now() + Duration::from_secs(2),
        )
        .await;
    assert!(result.is_err(), "wrong-hostname handshake must fail");
    assert_eq!(
        fixture.http_requests(),
        0,
        "no business request may reach the server"
    );
}

//! Response bounding, fixed-path enforcement and a successful round trip.
mod support;

use std::time::Duration;

use bytes::Bytes;
use conex_core::{Connector, OutboundRequest};
use conex_proto::v1;
use conex_transport_http::{HttpConnector, TlsTrustConfig};
use tokio::time::Instant;

use support::plain_server;

fn request(path: &str) -> OutboundRequest {
    OutboundRequest {
        method: http::Method::GET,
        path: path.into(),
        headers: http::HeaderMap::new(),
        body: Bytes::new(),
    }
}

#[tokio::test]
async fn oversized_response_is_rejected() {
    let (addr, _count) = plain_server(4096).await;
    let connector = HttpConnector::new(TlsTrustConfig::default(), 1024).unwrap();
    let mut connection = connector
        .connect(
            &support::allowed_target(addr, "http", "127.0.0.1", "/"),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
    let error = connection
        .request(request("/"), Instant::now() + Duration::from_secs(2))
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(v1::ErrorCode::PayloadTooLarge));
}

#[tokio::test]
async fn small_response_round_trips() {
    let (addr, count) = plain_server(4).await;
    let connector = HttpConnector::new(TlsTrustConfig::default(), 1024).unwrap();
    let mut connection = connector
        .connect(
            &support::allowed_target(addr, "http", "127.0.0.1", "/"),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
    let response = connection
        .request(request("/"), Instant::now() + Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(&response.body[..], b"xxxx");
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn path_outside_fixed_path_is_rejected() {
    let (addr, _count) = plain_server(4).await;
    let connector = HttpConnector::new(TlsTrustConfig::default(), 1024).unwrap();
    let mut connection = connector
        .connect(
            &support::allowed_target(addr, "http", "127.0.0.1", "/api"),
            Instant::now() + Duration::from_secs(2),
        )
        .await
        .unwrap();
    let error = connection
        .request(request("/other"), Instant::now() + Duration::from_secs(2))
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(v1::ErrorCode::Forbidden));
}

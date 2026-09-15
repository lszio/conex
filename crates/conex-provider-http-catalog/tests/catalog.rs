//! Second provider behaves like the first: same contract, bounded, honest errors.
mod support;

use std::time::Duration;

use conex_proto::v1;
use conex_provider_http_catalog::Catalog;
use serde_json::json;

use support::CatalogFixture;

#[test]
fn parse_rejects_duplicate_resource_ids() {
    let body = br#"{"entries":[{"resourceId":"a.md","title":"a","mime":"text/markdown","text":"x"},{"resourceId":"a.md","title":"b","mime":"text/markdown","text":"y"}]}"#;
    assert_eq!(
        Catalog::parse(body).unwrap_err().code_enum(),
        Some(v1::ErrorCode::BadRequest)
    );
}

#[test]
fn parse_rejects_malformed_documents() {
    assert!(Catalog::parse(b"not json").is_err());
    assert!(Catalog::parse(br#"{"entries":[{"resourceId":"a.md"}]}"#).is_err());
    assert!(Catalog::parse(br#"{"entries":[{"resourceId":"../escape.md","title":"a","mime":"text/markdown","text":"x"}]}"#).is_err());
}

#[test]
fn parse_rejects_oversized_entry() {
    let huge = "x".repeat(300 * 1024);
    let body = serde_json::to_vec(&json!({
        "entries": [{"resourceId": "a.md", "title": "a", "mime": "text/markdown", "text": huge}]
    }))
    .unwrap();
    assert_eq!(
        Catalog::parse(&body).unwrap_err().code_enum(),
        Some(v1::ErrorCode::PayloadTooLarge)
    );
}

#[tokio::test]
async fn read_list_search_go_through_the_real_host_and_connector() {
    let fixture = CatalogFixture::source_wide().await;
    let read = fixture
        .host
        .invoke(
            &fixture.caller,
            "catalog-work",
            "source/read",
            json!({"resourceId": "hello.md"}),
            Duration::from_secs(3),
        )
        .await
        .unwrap();
    assert_eq!(read["text"], "hello conex\n");
    assert_eq!(
        read["cid"],
        serde_json::Value::String(conex_proto::cid::cid_for_raw(b"hello conex\n"))
    );
    assert!(fixture.server.get_count() >= 1);

    let list = fixture
        .host
        .invoke(
            &fixture.caller,
            "catalog-work",
            "source/list",
            json!({"root": ""}),
            Duration::from_secs(3),
        )
        .await
        .unwrap();
    assert_eq!(list["items"].as_array().unwrap().len(), 2);

    let search = fixture
        .host
        .invoke(
            &fixture.caller,
            "catalog-work",
            "source/search",
            json!({"root": "", "query": "conex"}),
            Duration::from_secs(3),
        )
        .await
        .unwrap();
    assert_eq!(search["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn upstream_redirect_is_rejected() {
    let fixture = CatalogFixture::with_status(302).await;
    let result = fixture
        .host
        .invoke(
            &fixture.caller,
            "catalog-work",
            "source/read",
            json!({"resourceId": "hello.md"}),
            Duration::from_secs(3),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Unavailable)
    );
}

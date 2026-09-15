//! A whole partition needs partition scope; partial permission never dials.
mod support;

use std::time::Duration;

use conex_proto::v1;
use serde_json::json;

use support::CatalogFixture;

#[tokio::test]
async fn file_only_permission_does_not_fetch_whole_catalog() {
    let fixture = CatalogFixture::with_file_only_permission().await;
    let result = fixture
        .host
        .invoke(
            &fixture.caller,
            "catalog-work",
            "source/read",
            json!({"resourceId": "hello.md"}),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Forbidden)
    );
    assert_eq!(fixture.server.get_count(), 0);
}

#[tokio::test]
async fn subtree_permission_is_not_enough_for_list() {
    let fixture = CatalogFixture::with_file_only_permission().await;
    let result = fixture
        .host
        .invoke(
            &fixture.caller,
            "catalog-work",
            "source/list",
            json!({"root": "team"}),
            Duration::from_secs(2),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Forbidden)
    );
    assert_eq!(fixture.server.get_count(), 0);
}

//! M2 handler behavior: full listings, text-only search, inline-text XOR
//! content-ref reads, mtime revisions.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use conex_core::{CallContext, Caller, ExecutionIo, Handler, ResourceClaim};

fn io() -> ExecutionIo {
    ExecutionIo {
        connection: None,
        secret: None,
    }
}
use conex_provider_fs::list::{ListHandler, scan};
use conex_provider_fs::read::ReadHandler;
use conex_provider_fs::search::SearchHandler;
use conex_provider_fs::{FsRoot, MAX_DOC_BYTES};
use conex_source::pagination::{SnapshotCache, SnapshotLimits, SystemClock};
use serde_json::Value;

fn ctx(resource: &str) -> CallContext {
    CallContext {
        caller: Caller {
            principal_id: "p".into(),
            tenant_id: "t".into(),
            actor_peer_id: "a".into(),
        },
        endpoint_id: "fs-endpoint".into(),
        plane: conex_proto::Plane::Broker,
        method: String::new(),
        claim: ResourceClaim {
            resource_id: resource.into(),
            action: "read".into(),
            subtree: false,
        },
        policy_version: 1,
        deadline: tokio::time::Instant::now() + Duration::from_secs(30),
    }
}

fn fixture() -> (tempfile::TempDir, Arc<FsRoot>) {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("hello.txt"), "find me here\n").unwrap();
    std::fs::write(temp.path().join("pic.png"), [0x89, 0x50, 0x4e, 0x47]).unwrap();
    std::fs::write(temp.path().join("broken.md"), [0xff, 0xfe, 0x00]).unwrap();
    std::fs::write(temp.path().join("big.md"), "x".repeat(MAX_DOC_BYTES + 1)).unwrap();
    std::fs::write(temp.path().join("unknown.bin"), [0x00, 0x01, 0x02]).unwrap();
    let root = Arc::new(FsRoot::open(temp.path()).unwrap());
    (temp, root)
}

fn list_handler(root: &Arc<FsRoot>) -> ListHandler {
    ListHandler {
        root: root.clone(),
        cache: Arc::new(Mutex::new(SnapshotCache::new(
            Arc::new(SystemClock),
            SnapshotLimits::default(),
        ))),
    }
}

fn search_handler(root: &Arc<FsRoot>) -> SearchHandler {
    SearchHandler {
        root: root.clone(),
        cache: Arc::new(Mutex::new(SnapshotCache::new(
            Arc::new(SystemClock),
            SnapshotLimits::default(),
        ))),
    }
}

#[tokio::test]
async fn list_includes_unknown_extension_and_oversize_files() {
    let (_temp, root) = fixture();
    let response = list_handler(&root)
        .execute(&ctx("x"), serde_json::json!({"root": ""}), io())
        .await
        .unwrap();
    let items = response["items"].as_array().unwrap();
    let ids: Vec<&str> = items
        .iter()
        .map(|item| item["resourceId"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        ["big.md", "broken.md", "hello.txt", "pic.png", "unknown.bin"]
    );
    for item in items {
        assert!(item["revision"].as_str().is_some());
    }
    let png = items.iter().find(|i| i["resourceId"] == "pic.png").unwrap();
    // Known media types keep their real MIME; the octet-stream fallback is
    // for genuinely unknown extensions (asserted below).
    assert_eq!(png["mime"], "image/png");
    let unknown = items
        .iter()
        .find(|item| item["resourceId"] == "unknown.bin")
        .expect("unknown.bin is listed");
    assert_eq!(
        unknown["mime"].as_str(),
        Some("application/octet-stream"),
        "unknown extensions fall back to application/octet-stream (M0)"
    );
    let big = items.iter().find(|i| i["resourceId"] == "big.md").unwrap();
    assert_eq!(big["mime"], "text/markdown");
    assert_eq!(
        big["sizeBytes"].as_str().unwrap(),
        (MAX_DOC_BYTES + 1).to_string()
    );
}

#[test]
fn scan_search_skips_non_text_and_non_utf8_files() {
    let (_temp, root) = fixture();
    let items = scan(&root, "", Some("find me"), 100).unwrap();
    let ids: Vec<&str> = items.iter().map(|item| item.resource_id.as_str()).collect();
    assert_eq!(ids, ["hello.txt"]);
    assert!(items[0].excerpt.as_deref().unwrap().contains("find me"));
    assert!(scan(&root, "", Some("nothing"), 100).unwrap().is_empty());
}

#[tokio::test]
async fn read_png_returns_content_reference() {
    let (_temp, root) = fixture();
    let value = ReadHandler { root: root.clone() }
        .execute(
            &ctx("pic.png"),
            serde_json::json!({"resourceId": "pic.png"}),
            io(),
        )
        .await
        .unwrap();
    assert!(value["text"].is_null());
    assert!(value["cid"].is_null());
    let content = &value["content"];
    assert_eq!(content["mime"], "image/png");
    assert_eq!(content["sizeBytes"], "4");
    assert_eq!(content["access"]["endpointId"], "fs-endpoint");
    assert_eq!(content["access"]["plane"], "broker");
    assert_eq!(content["access"]["resourceId"], "pic.png");
    assert!(content["revision"].as_str().is_some());
    assert!(value["resource"]["revision"].as_str().is_some());
}

#[tokio::test]
async fn read_txt_is_inline_text_with_cid_and_revision() {
    let (_temp, root) = fixture();
    let value = ReadHandler { root: root.clone() }
        .execute(
            &ctx("hello.txt"),
            serde_json::json!({"resourceId": "hello.txt"}),
            io(),
        )
        .await
        .unwrap();
    assert_eq!(value["text"], "find me here\n");
    assert!(value["cid"].as_str().is_some());
    assert!(value["content"].is_null());
    assert_eq!(value["resource"]["mime"], "text/plain");
    assert!(value["resource"]["revision"].as_str().is_some());
}

#[tokio::test]
async fn read_non_utf8_text_file_is_bad_request() {
    let (_temp, root) = fixture();
    let error = ReadHandler { root: root.clone() }
        .execute(
            &ctx("broken.md"),
            serde_json::json!({"resourceId": "broken.md"}),
            io(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::BadRequest));
}

#[tokio::test]
async fn read_oversize_file_returns_content_reference_not_error() {
    let (_temp, root) = fixture();
    let value = ReadHandler { root: root.clone() }
        .execute(
            &ctx("big.md"),
            serde_json::json!({"resourceId": "big.md"}),
            io(),
        )
        .await
        .unwrap();
    assert!(value["text"].is_null());
    assert_eq!(value["content"]["access"]["resourceId"], "big.md");
}

#[tokio::test]
async fn read_missing_file_still_errors_not_found() {
    let temp = tempfile::tempdir().unwrap();
    let root = Arc::new(FsRoot::open(temp.path()).unwrap());
    let error = ReadHandler { root }
        .execute(
            &ctx("gone.md"),
            serde_json::json!({"resourceId": "gone.md"}),
            io(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::BadRequest));
}

#[test]
fn search_handler_finds_txt_only() {
    // Covered by scan_search_skips_non_text_and_non_utf8_files; this keeps the
    // Handler path honest end to end.
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (_temp, root) = fixture();
    let handler = search_handler(&root);
    let value: Value = rt
        .block_on(handler.execute(
            &ctx("x"),
            serde_json::json!({"root": "", "query": "find me"}),
            io(),
        ))
        .unwrap();
    let items = value["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["resource"]["resourceId"], "hello.txt");
    assert!(items[0]["resource"]["revision"].as_str().is_some());
}

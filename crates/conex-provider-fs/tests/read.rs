//! Reads return the exact bytes and the shared content CID.
use std::path::PathBuf;

use conex_provider_fs::{FsRoot, MAX_DOC_BYTES};

fn fixture_root() -> FsRoot {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/p0/notes");
    FsRoot::open(&path).unwrap()
}

#[test]
fn reads_fixture_and_matches_content_cid() {
    let root = fixture_root();
    let snapshot = root.read("hello.md", 262144).unwrap();
    assert_eq!(&snapshot.bytes[..], b"hello conex\n");
    assert_eq!(
        snapshot.cid,
        conex_proto::cid::cid_for_raw(b"hello conex\n")
    );
}

#[test]
fn read_cid_uses_the_shared_content_function_at_the_size_cap() {
    // The per-document cap equals one chunk, so the largest readable document
    // must still address as a single-chunk (raw) CID of the same function that
    // P1 `blob/*` uses. If the cap ever grows past one chunk this assertion
    // fails, which is the point: addresses must not silently become manifests.
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("big.md"), vec![b'x'; MAX_DOC_BYTES]).unwrap();
    let root = FsRoot::open(tmp.path()).unwrap();
    let snapshot = root.read("big.md", MAX_DOC_BYTES).unwrap();
    assert_eq!(snapshot.bytes.len(), MAX_DOC_BYTES);
    assert_eq!(
        snapshot.cid,
        conex_proto::cid::content_cid(&snapshot.bytes, conex_proto::cid::CHUNK_SIZE)
    );
    assert_eq!(snapshot.cid, conex_proto::cid::cid_for_raw(&snapshot.bytes));
}

#[test]
fn reads_nested_resource() {
    let root = fixture_root();
    let snapshot = root.read("team/design.org", 262144).unwrap();
    let text = String::from_utf8(snapshot.bytes.to_vec()).unwrap();
    assert!(text.contains("conex routing"));
}

#[test]
fn factory_exposes_all_source_methods_with_shared_contracts() {
    use conex_core::{Endpoint, FactoryKey, Installation, Limits};

    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/p0/notes");
    let endpoint = Endpoint {
        id: "notes-local".into(),
        provider_id: "source".into(),
        tenant_id: "t".into(),
        plane: conex_proto::v1::Plane::Broker,
        provides: vec![
            "source/list".into(),
            "source/read".into(),
            "source/search".into(),
        ],
        limits: Limits::default(),
    };
    let installation = Installation {
        endpoint,
        factory: FactoryKey {
            kind: "source-fs".into(),
            protocol: "conex".into(),
            version: 1,
        },
        provider: serde_json::json!({"root": path}),
        target: None,
        credential: None,
    };
    let routes = conex_provider_fs::factory(&installation).unwrap();
    assert_eq!(routes.len(), 3);
    let shared = conex_source::contracts();
    for route in &routes {
        let (_, contract) = shared
            .iter()
            .find(|(method, _)| *method == route.method)
            .expect("shared contract");
        assert_eq!(contract.input_schema, route.contract.input_schema);
        assert_eq!(contract.output_schema, route.contract.output_schema);
    }
}

#[test]
fn oversized_document_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("big.md"), "x".repeat(300 * 1024)).unwrap();
    let root = FsRoot::open(temp.path()).unwrap();
    let error = root.read("big.md", 256 * 1024).unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::v1::ErrorCode::PayloadTooLarge)
    );
}

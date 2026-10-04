//! M1.2 blob ownership and authorization behavior: a bare CID, upload id or
//! pin id is never a credential; only the owning principal can read, resume,
//! cancel, commit, pin or unpin its own content.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use tempfile::TempDir;

use conex_content::ContentStore;
use conex_core::Caller;
use conex_host::agent::HostSide;
use conex_host::broker::{Broker, BrokerCall, BrokerDeps};

fn tmp() -> TempDir {
    tempfile::tempdir().unwrap()
}

fn caller(principal: &str, tenant: &str) -> Caller {
    Caller {
        principal_id: principal.into(),
        tenant_id: tenant.into(),
        actor_peer_id: "test".into(),
    }
}

async fn make_broker(
    content_root: PathBuf,
    session_root: PathBuf,
    operation_root: PathBuf,
) -> Arc<Broker> {
    let content = ContentStore::open(&content_root, 60_000).unwrap();
    let session = conex_core::session::SessionStore::open(&session_root).unwrap();
    let operation = conex_core::operation::OperationStore::open(&operation_root).unwrap();
    let registry = conex_core::Registry::new();
    let host = Arc::new(
        conex_core::Host::new(
            registry,
            Arc::new(conex_core::StaticPolicy::new(Vec::new())),
            Arc::new(conex_core::TargetPolicy::new()),
            Arc::new(conex_transport_http::TokioResolver),
            Arc::new(
                conex_transport_http::HttpConnector::new(
                    conex_transport_http::TlsTrustConfig::default(),
                    1024 * 1024,
                )
                .unwrap(),
            ),
            Arc::new(conex_host::credentials::EnvFileStore::new(Vec::new()).unwrap()),
            Arc::new(conex_host::audit_file::NullAudit),
            conex_core::HostLimits::default(),
        )
        .unwrap(),
    );
    let side = HostSide::new();
    let deps = BrokerDeps {
        content: Some(Arc::new(content)),
        session: Some(Arc::new(session)),
        operation: Some(Arc::new(operation)),
        agents: Some(side.agents.clone()),
        catalog: None,
        host_origin: Some("conex://broker.local".into()),
        guest_principal: None,
        remote: None,
    };
    Arc::new(Broker::new(host, deps))
}

fn broker_roots(dir: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
    (
        dir.path().join("content"),
        dir.path().join("session"),
        dir.path().join("operation"),
    )
}

async fn call(
    broker: &Broker,
    who: Caller,
    method: &str,
    input: Value,
) -> Result<Value, conex_core::CallError> {
    broker
        .invoke(BrokerCall {
            caller: who,
            endpoint_id: "blob-store".into(),
            method: method.into(),
            input,
            deadline: tokio::time::Instant::now() + Duration::from_secs(5),
            role: "service".into(),
            link_id: None,
        })
        .await
}

fn blob_put(root_cid: &str, size: u64) -> Value {
    json!({
        "formatVersion": "1",
        "declaredSizeBytes": size.to_string(),
        "declaredChunkSize": "262144",
        "expectedRoot": { "rawCid": root_cid },
        "access": { "endpointId": "", "plane": "broker", "resourceId": "notes/a.md" }
    })
}

fn chunk_input(upload_id: &str, cid: &str, payload: &[u8]) -> Value {
    json!({
        "uploadId": upload_id,
        "chunkIndex": "0",
        "chunkCid": cid,
        "chunkBytes": base64(payload),
    })
}

const PAYLOAD_LEN: usize = 64;

/// Principal A uploads, commits and pins; B — same tenant or not — cannot
/// touch any of it by knowing the identifiers.
#[tokio::test]
async fn blob_content_is_owner_scoped_end_to_end() {
    let dir = tmp();
    let roots = broker_roots(&dir);
    let broker = make_broker(roots.0, roots.1, roots.2).await;
    let alice = caller("alice", "tenant-a");
    let mallory_same_tenant = caller("mallory", "tenant-a");
    let mallory_other_tenant = caller("mallory", "tenant-b");

    let payload = vec![0x5Au8; PAYLOAD_LEN];
    let root = cid_for_raw(&payload);
    let put = call(
        &broker,
        alice.clone(),
        "blob/put",
        blob_put(&root, payload.len() as u64),
    )
    .await
    .expect("put");
    let upload_id = put["uploadId"].as_str().unwrap().to_string();
    call(
        &broker,
        alice.clone(),
        "blob/chunk",
        chunk_input(&upload_id, &root, &payload),
    )
    .await
    .expect("chunk");
    let commit = call(
        &broker,
        alice.clone(),
        "blob/commit",
        json!({ "uploadId": upload_id, "declaredRoot": { "rawCid": root }, "persistence": "local" }),
    )
    .await
    .expect("commit");
    assert_eq!(commit["resourceRootCid"], root.as_str());
    let pin = call(
        &broker,
        alice.clone(),
        "blob/pin",
        json!({ "root": { "rawCid": root }, "pinUntilMs": "4102444800000" }),
    )
    .await
    .expect("pin");
    let pin_id = pin["pinId"].as_str().unwrap().to_string();

    for attacker in [mallory_same_tenant, mallory_other_tenant] {
        // blob/get: a valid CID alone must not read A's content.
        let error = call(
            &broker,
            attacker.clone(),
            "blob/get",
            json!({ "committed": { "chunkCid": root } }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));

        // blob/have must not leak the existence of A's chunk.
        let have = call(
            &broker,
            attacker.clone(),
            "blob/have",
            json!({ "chunkCids": [root] }),
        )
        .await
        .expect("have");
        assert_eq!(have["present"][0], false, "foreign chunk presence leaks");

        // A's upload id grants nothing: chunk, commit and cancel all fail.
        for (method, input) in [
            ("blob/chunk", chunk_input(&upload_id, &root, &payload)),
            (
                "blob/commit",
                json!({ "uploadId": upload_id, "declaredRoot": { "rawCid": root }, "persistence": "local" }),
            ),
            ("blob/cancel", json!({ "uploadId": upload_id })),
        ] {
            let error = call(&broker, attacker.clone(), method, input)
                .await
                .unwrap_err();
            assert_eq!(
                error.code_enum(),
                Some(conex_proto::ErrorCode::UnknownProvider),
                "{method} with a foreign upload id must fail"
            );
        }

        // A's pin id cannot be released by anyone else.
        let error = call(
            &broker,
            attacker.clone(),
            "blob/unpin",
            json!({ "pinId": pin_id }),
        )
        .await
        .unwrap_err();
        assert_eq!(
            error.code_enum(),
            Some(conex_proto::ErrorCode::UnknownProvider)
        );

        // Pinning a root the attacker does not own is rejected.
        let error = call(
            &broker,
            attacker.clone(),
            "blob/pin",
            json!({ "root": { "rawCid": root } }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));
    }

    // The owner retains full access.
    let got = call(
        &broker,
        alice.clone(),
        "blob/get",
        json!({ "committed": { "chunkCid": root } }),
    )
    .await
    .expect("owner read");
    assert_eq!(got["chunkCid"], root.as_str());
    call(
        &broker,
        alice.clone(),
        "blob/unpin",
        json!({ "pinId": pin_id }),
    )
    .await
    .expect("owner unpin");
}

/// B starting an upload with the same content identity must never resume A's
/// staging progress, and A's in-flight chunks stay private.
#[tokio::test]
async fn staging_resume_never_crosses_owners() {
    let dir = tmp();
    let roots = broker_roots(&dir);
    let broker = make_broker(roots.0, roots.1, roots.2).await;
    let alice = caller("alice", "tenant-a");
    let mallory = caller("mallory", "tenant-a");

    let payload = vec![0x77u8; PAYLOAD_LEN];
    let root = cid_for_raw(&payload);
    let put = call(
        &broker,
        alice.clone(),
        "blob/put",
        blob_put(&root, payload.len() as u64),
    )
    .await
    .expect("put");
    let upload_id = put["uploadId"].as_str().unwrap().to_string();
    call(
        &broker,
        alice.clone(),
        "blob/chunk",
        chunk_input(&upload_id, &root, &payload),
    )
    .await
    .expect("chunk");

    // Mallory's identical blob/put must open a separate upload, not resume
    // Alice's, and must not see her chunk as already-present.
    let mallory_put = call(
        &broker,
        mallory.clone(),
        "blob/put",
        blob_put(&root, payload.len() as u64),
    )
    .await
    .expect("mallory put");
    assert_ne!(
        mallory_put["uploadId"].as_str().unwrap(),
        upload_id,
        "resume-by-root crossed owners"
    );
    assert!(
        mallory_put["alreadyHaveChunkCids"]
            .as_array()
            .unwrap()
            .is_empty(),
        "foreign staging chunks leaked into alreadyHaveChunkCids"
    );

    // Mallory cannot write into Alice's upload id either.
    let error = call(
        &broker,
        mallory.clone(),
        "blob/chunk",
        chunk_input(&upload_id, &root, &payload),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));
}

/// Entry and cumulative caps: oversized declarations, wrong chunk geometry
/// and over-declared writes are rejected (plan M1.2).
#[tokio::test]
async fn blob_entry_and_cumulative_limits_are_enforced() {
    let dir = tmp();
    let roots = broker_roots(&dir);
    let broker = make_broker(roots.0, roots.1, roots.2).await;
    let alice = caller("alice", "tenant-a");

    let oversize = blob_put(&cid_for_raw(b"x"), 1_073_741_825);
    let error = call(&broker, alice.clone(), "blob/put", oversize)
        .await
        .unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::PayloadTooLarge)
    );

    let mut wrong_geometry = blob_put(&cid_for_raw(b"x"), 64);
    wrong_geometry["declaredChunkSize"] = json!("65536");
    let error = call(&broker, alice.clone(), "blob/put", wrong_geometry)
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::BadRequest));

    let mut long_lease = blob_put(&cid_for_raw(b"x"), 64);
    long_lease["requestedLeaseMs"] = json!("259200001");
    let error = call(&broker, alice.clone(), "blob/put", long_lease)
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::BadRequest));

    // Cumulative: writing beyond the declared size is rejected. The chunk
    // bytes are internally consistent (CID matches the bytes) but exceed the
    // declared total.
    let payload = [0x33u8; 64];
    let root = cid_for_raw(&payload);
    let put = call(&broker, alice.clone(), "blob/put", blob_put(&root, 32))
        .await
        .expect("put");
    let upload_id = put["uploadId"].as_str().unwrap().to_string();
    let error = call(
        &broker,
        alice,
        "blob/chunk",
        chunk_input(&upload_id, &root, &payload),
    )
    .await
    .expect_err("over-declared write");
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::PayloadTooLarge)
    );
}

/// Ownership rules survive a restart: the same staging stays sealed against
/// foreign principals while the owner can still resume it.
#[tokio::test]
async fn ownership_survives_restart() {
    let dir = tmp();
    let roots = broker_roots(&dir);
    let broker = make_broker(roots.0.clone(), roots.1.clone(), roots.2.clone()).await;
    let alice = caller("alice", "tenant-a");
    let mallory = caller("mallory", "tenant-a");

    let payload = vec![0x99u8; PAYLOAD_LEN];
    let root = cid_for_raw(&payload);
    let put = call(
        &broker,
        alice.clone(),
        "blob/put",
        blob_put(&root, payload.len() as u64),
    )
    .await
    .expect("put");
    let upload_id = put["uploadId"].as_str().unwrap().to_string();

    drop(broker);
    let broker = make_broker(roots.0, roots.1, roots.2).await;
    let error = call(
        &broker,
        mallory.clone(),
        "blob/chunk",
        chunk_input(&upload_id, &root, &payload),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));

    let owner_resume = call(
        &broker,
        alice.clone(),
        "blob/chunk",
        chunk_input(&upload_id, &root, &payload),
    )
    .await;
    assert!(
        owner_resume.is_ok(),
        "the owner must still resume after restart"
    );
}

fn cid_for_raw(bytes: &[u8]) -> String {
    conex_proto::cid::cid_for_raw(bytes)
}

fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

//! P1 joint e2e (broker-only): in-memory stores + `Broker::invoke` directly,
//! no socket. Covers the subset of design §14 P1 acceptance that fits a
//! synchronous broker test. The wire-level acceptance (HTTP `/rpc` + WSS
//! `/wss` + ticket/OIDC) lives in the host integration test once the
//! dispatcher is mounted in `serve::build`; do not reintroduce a
//! `p1_wss_smoke.rs` reference until that test exists.
#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Map, Value, json};
use tempfile::TempDir;

use base64::Engine as _;

use conex_content::ContentStore;
use conex_core::Caller;

use conex_host::agent::HostSide;
use conex_host::broker::{Broker, BrokerCall, BrokerDeps};

fn tmp(name: &str) -> (TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join(name);
    std::fs::create_dir_all(&path).unwrap();
    (tmp, path)
}

fn caller(principal: &str, tenant: &str) -> Caller {
    Caller {
        principal_id: principal.into(),
        tenant_id: tenant.into(),
        actor_peer_id: "test".into(),
    }
}

fn deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(10)
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
        host_origin: Some("conex://broker.local".into()),
    };
    Arc::new(Broker::new(host, deps))
}

fn cid_for_raw(bytes: &[u8]) -> String {
    conex_proto::cid::cid_for_raw(bytes)
}

fn known_manifest_root() -> String {
    let leaves = vec![cid_for_raw(&vec![0u8; 262144]); 2];
    let chunk_size: u32 = 262_144;
    let total_bytes: u64 = 524_288;
    conex_proto::cid::content_cid_for_parts(chunk_size, total_bytes, &leaves)
        .expect("manifest root")
}

fn base64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

async fn invoke(
    broker: &Broker,
    caller: Caller,
    endpoint: &str,
    method: &str,
    input: Value,
) -> Value {
    broker
        .invoke(BrokerCall {
            caller,
            endpoint_id: endpoint.into(),
            method: method.into(),
            input,
            deadline: deadline(),
        })
        .await
        .expect("broker call")
}

#[tokio::test]
async fn blob_upload_round_trip() {
    let (_t1, content_root) = tmp("content");
    let (_t2, session_root) = tmp("session");
    let (_t3, operation_root) = tmp("operation");
    let broker = make_broker(content_root, session_root, operation_root).await;
    let caller = caller("alice", "tenant-a");
    let expected = known_manifest_root();

    // debug probe removed
    let put = invoke(
        &broker,
        caller.clone(),
        "blob-store",
        "blob/put",
        json!({
            "formatVersion": "1",
            "declaredSizeBytes": "524288",
            "declaredChunkSize": "262144",
            "expectedRoot": { "manifestCid": expected },
            "access": {
                "providerId": "fs",
                "plane": "broker",
                "resourceId": "notes/a.md"
            }
        }),
    )
    .await;
    let upload_id = put
        .get("uploadId")
        .and_then(Value::as_str)
        .expect("uploadId present")
        .to_string();

    let chunk_payload = vec![0u8; 262144];
    let chunk_cid = cid_for_raw(&chunk_payload);
    invoke(
        &broker,
        caller.clone(),
        "blob-store",
        "blob/chunk",
        json!({
            "uploadId": upload_id,
            "chunkIndex": "0",
            "chunkCid": chunk_cid,
            "chunkBytes": base64_encode(&chunk_payload)
        }),
    )
    .await;

    invoke(
        &broker,
        caller.clone(),
        "blob-store",
        "blob/chunk",
        json!({
            "uploadId": upload_id,
            "chunkIndex": "1",
            "chunkCid": chunk_cid,
            "chunkBytes": base64_encode(&chunk_payload)
        }),
    )
    .await;

    let commit = invoke(
        &broker,
        caller.clone(),
        "blob-store",
        "blob/commit",
        json!({
            "uploadId": upload_id,
            "declaredRoot": { "manifestCid": expected },
            "persistence": "local"
        }),
    )
    .await;
    assert_eq!(
        commit.get("resourceRootCid").and_then(Value::as_str),
        Some(expected.as_str())
    );
}

#[tokio::test]
async fn session_lifecycle_and_binding_mismatch() {
    let (_t1, content_root) = tmp("content");
    let (_t2, session_root) = tmp("session");
    let (_t3, operation_root) = tmp("operation");
    let broker = make_broker(content_root, session_root, operation_root).await;
    let caller = caller("alice", "tenant-a");

    let open = invoke(
        &broker,
        caller.clone(),
        "session-mgr",
        "session/open",
        json!({
            "requested": {
                "principalId": "alice",
                "tenantId": "tenant-a",
                "providerEndpointId": "notes-local",
                "plane": 1
            },
            "recovery": "in_process",
            "attachmentId": "attach-1",
            "peerRole": "ui"
        }),
    )
    .await;
    let session_id = open
        .get("sessionId")
        .and_then(Value::as_str)
        .expect("sessionId present")
        .to_string();

    let error = broker
        .invoke(BrokerCall {
            caller: caller.clone(),
            endpoint_id: "session-mgr".into(),
            method: "session/resume".into(),
            input: json!({
                "sessionId": session_id,
                "attachmentId": "attach-1",
                "expectedEpoch": "1",
                "binding": {
                    "principalId": "eve",
                    "tenantId": "tenant-a",
                    "providerEndpointId": "notes-local",
                    "plane": 1
                }
            }),
            deadline: deadline(),
        })
        .await
        .expect_err("binding mismatch must reject");
    // The broker layer reports cross-principal binding mismatches as
    // Forbidden. `Unauthorized` is reserved for missing/invalid bearer.
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::v1::ErrorCode::Forbidden)
    );
}

#[tokio::test]
async fn operation_dedup_round_trip() {
    let (_t1, content_root) = tmp("content");
    let (_t2, session_root) = tmp("session");
    let (_t3, operation_root) = tmp("operation");
    let broker = make_broker(content_root, session_root, operation_root).await;
    let caller = caller("alice", "tenant-a");
    let key_json = json!({
        "tenantId": "tenant-a",
        "principalId": "alice",
        "providerEndpointId": "notes-local",
        "resourceId": "notes/a.md",
        "method": "source/write",
        "operationId": "op-1"
    });
    let key = decode_dedup_key(&key_json);

    // Seed the operation record first via the store (the broker exposes
    // /cancel and /get; accept is the upstream's responsibility).
    broker
        .deps()
        .operation
        .as_ref()
        .expect("operation backend configured")
        .accept(
            key.clone(),
            conex_core::operation::ExecutionClass::NonReplayable,
            "test".into(),
        )
        .expect("accept seed");

    let cancel = invoke(
        &broker,
        caller.clone(),
        "operation-store",
        "operation/cancel",
        json!({ "key": key_json }),
    )
    .await;
    assert_eq!(
        cancel.get("outcomeUnknown").and_then(Value::as_bool),
        Some(false)
    );

    // A second get on the now-recorded operation returns Cancelled (= 4).
    let get = invoke(
        &broker,
        caller.clone(),
        "operation-store",
        "operation/get",
        json!({ "key": key_json }),
    )
    .await;
    let state = get.get("state").and_then(Value::as_i64).unwrap();
    assert_eq!(state, 4);
}

fn decode_dedup_key(value: &Value) -> conex_core::operation::DedupKey {
    use conex_core::operation::DedupKey;
    let map = value.as_object().expect("key object");
    DedupKey {
        tenant_id: map["tenantId"].as_str().unwrap().into(),
        principal_id: map["principalId"].as_str().unwrap().into(),
        provider_endpoint_id: map["providerEndpointId"].as_str().unwrap().into(),
        space_id: map
            .get("spaceId")
            .and_then(Value::as_str)
            .map(str::to_string),
        resource_id: map["resourceId"].as_str().unwrap().into(),
        method: map["method"].as_str().unwrap().into(),
        operation_id: map["operationId"].as_str().unwrap().into(),
    }
}

#[tokio::test]
async fn agent_registration_and_resolve() {
    let (_t1, content_root) = tmp("content");
    let (_t2, session_root) = tmp("session");
    let (_t3, operation_root) = tmp("operation");
    let broker = make_broker(content_root, session_root, operation_root).await;
    let caller = caller("alice", "tenant-a");

    invoke(
        &broker,
        caller.clone(),
        "agent-mgr",
        "agent/register",
        json!({
            "agentId": "agent-1",
            "providerIds": ["notes-local"],
            "methods": ["source/read"],
            "resources": ["notes/a.md"],
            "hostOrigin": "conex://broker.local"
        }),
    )
    .await;

    let resolve = invoke(
        &broker,
        caller.clone(),
        "agent-mgr",
        "agent/resolve",
        json!({
            "providerId": "notes-local",
            "resourceId": "notes/a.md",
            "method": "source/read"
        }),
    )
    .await;
    let agents = resolve.get("agents").and_then(Value::as_array).unwrap();
    assert!(agents.iter().any(|value| value.as_str() == Some("agent-1")));
}

#[allow(dead_code)]
fn _unused() {
    let _ = Map::<String, Value>::new();
    let _ = "ok";
}

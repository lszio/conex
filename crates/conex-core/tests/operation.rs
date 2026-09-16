//! P1-08 operation dedup / execution / state machine tests.
use std::fs;
use std::path::Path;

use conex_core::operation::{
    DEFAULT_TTL_HOURS, DedupKey, ExecutionClass, ExecutionState, OperationError, OperationState,
    OperationStore, RetryPolicy, read_record_file,
};
use conex_proto::v1;
use tempfile::TempDir;

fn tmp_store(tmp: &TempDir) -> OperationStore {
    let path = tmp.path().join("operations");
    OperationStore::open(&path).expect("open operation store")
}

fn key(tenant: &str, method: &str, op: &str) -> DedupKey {
    DedupKey {
        tenant_id: tenant.to_string(),
        principal_id: "alice".to_string(),
        provider_endpoint_id: "notes-local".to_string(),
        space_id: None,
        resource_id: "notes/hello.md".to_string(),
        method: method.to_string(),
        operation_id: op.to_string(),
    }
}

#[test]
fn execution_class_to_retry_is_fixed() {
    assert_eq!(ExecutionClass::ReadOnly.retry(), RetryPolicy::Safe);
    assert_eq!(ExecutionClass::Idempotent.retry(), RetryPolicy::Safe);
    assert_eq!(
        ExecutionClass::Deduplicated.retry(),
        RetryPolicy::WithOperationId
    );
    assert_eq!(ExecutionClass::NonReplayable.retry(), RetryPolicy::Never);
}

#[test]
fn execution_state_after_upstream_send() {
    assert_eq!(
        ExecutionClass::NonReplayable.execution_state(true),
        ExecutionState::Unknown
    );
    assert_eq!(
        ExecutionClass::NonReplayable.execution_state(false),
        ExecutionState::Completed
    );
    assert_eq!(
        ExecutionClass::ReadOnly.execution_state(true),
        ExecutionState::Completed
    );
}

#[test]
fn accept_then_settle_success_roundtrip() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp_store(&tmp);
    let k = key("tenant-a", "source/read", "op-1");
    let record = store
        .accept(
            k.clone(),
            ExecutionClass::Deduplicated,
            "digest-1".to_string(),
        )
        .unwrap();
    assert_eq!(record.state, OperationState::Accepted);
    let running = store.mark_running(&k).unwrap();
    assert_eq!(running.state, OperationState::Running);
    let settled = store
        .settle_success(&k, serde_json::json!({"text": "hi"}))
        .unwrap();
    assert_eq!(settled.state, OperationState::Succeeded);
    let (read_back, result) = store.get(&k).unwrap();
    assert_eq!(read_back.state, OperationState::Succeeded);
    let result = result.unwrap();
    assert_eq!(result.execution, ExecutionState::Completed);
    assert_eq!(result.success, Some(serde_json::json!({"text": "hi"})));
}

#[test]
fn same_key_different_param_digest_returns_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp_store(&tmp);
    let k = key("tenant-a", "source/read", "op-2");
    store
        .accept(
            k.clone(),
            ExecutionClass::Idempotent,
            "digest-A".to_string(),
        )
        .unwrap();
    let err = store.accept(
        k.clone(),
        ExecutionClass::Idempotent,
        "digest-B".to_string(),
    );
    assert!(matches!(err, Err(OperationError::Conflict)));
}

#[test]
fn same_key_same_param_digest_replays_idempotently() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp_store(&tmp);
    let k = key("tenant-a", "source/write", "op-3");
    let r1 = store
        .accept(
            k.clone(),
            ExecutionClass::Idempotent,
            "digest-A".to_string(),
        )
        .unwrap();
    let r2 = store
        .accept(
            k.clone(),
            ExecutionClass::Idempotent,
            "digest-A".to_string(),
        )
        .unwrap();
    assert_eq!(r1.accepted_at_ms, r2.accepted_at_ms);
}

#[test]
fn response_lost_deduplicated_recovers_via_operation_get() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp_store(&tmp);
    let k = key("tenant-a", "source/write", "op-4");
    store
        .accept(
            k.clone(),
            ExecutionClass::Deduplicated,
            "digest-A".to_string(),
        )
        .unwrap();
    store.mark_running(&k).unwrap();
    store
        .settle_success(&k, serde_json::json!({"ok": true}))
        .unwrap();
    let (record, result) = store.get(&k).unwrap();
    assert_eq!(record.state, OperationState::Succeeded);
    assert_eq!(
        result.unwrap().success,
        Some(serde_json::json!({"ok": true}))
    );
}

#[test]
fn response_lost_non_replayable_marks_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp_store(&tmp);
    let k = key("tenant-a", "workspace/shell.run", "op-5");
    store
        .accept(
            k.clone(),
            ExecutionClass::NonReplayable,
            "digest-A".to_string(),
        )
        .unwrap();
    store.mark_running(&k).unwrap();
    store.settle_unknown(&k).unwrap();
    let (record, result) = store.get(&k).unwrap();
    assert_eq!(record.state, OperationState::Unknown);
    assert_eq!(result.unwrap().execution, ExecutionState::Unknown);
    assert_eq!(record.execution_class.retry(), RetryPolicy::Never);
}

#[test]
fn operation_id_expired_no_silent_replay() {
    // Deterministic expiry: backdate `expires_at_ms` to 0 on disk so the
    // next reload reports the record as expired regardless of wall-clock
    // resolution. No `sleep` is needed.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("operations");
    let store = OperationStore::open(&root).unwrap();
    let k = key("tenant-a", "source/write", "op-6");
    store
        .accept(
            k.clone(),
            ExecutionClass::Idempotent,
            "digest-A".to_string(),
        )
        .unwrap();
    let sanitized = k.sanitize();
    let record_path = root.join(format!("{sanitized}.json"));
    let mut text = fs::read_to_string(&record_path).unwrap();
    let mut obj: serde_json::Value = serde_json::from_str(&text).unwrap();
    obj.as_object_mut()
        .unwrap()
        .insert("expires_at_ms".into(), serde_json::json!(0u64));
    text = serde_json::to_string(&obj).unwrap();
    fs::write(&record_path, text).unwrap();
    drop(store);
    let store2 = OperationStore::open(&root).unwrap();
    let err = store2.get(&k);
    assert!(matches!(err, Err(OperationError::Expired)));
}

#[test]
fn crash_recovery_rebuilds_records_and_results() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("operations");
    {
        let store = OperationStore::open(&root).unwrap();
        let k = key("tenant-a", "source/read", "op-7");
        store
            .accept(
                k.clone(),
                ExecutionClass::Deduplicated,
                "digest-A".to_string(),
            )
            .unwrap();
        store.mark_running(&k).unwrap();
        store
            .settle_success(&k, serde_json::json!({"v": 7}))
            .unwrap();
    }
    let store2 = OperationStore::open(&root).unwrap();
    let k = key("tenant-a", "source/read", "op-7");
    let (record, result) = store2.get(&k).unwrap();
    assert_eq!(record.state, OperationState::Succeeded);
    assert_eq!(record.execution_class, ExecutionClass::Deduplicated);
    assert_eq!(result.unwrap().success, Some(serde_json::json!({"v": 7})));
}

#[test]
fn gc_drops_expired_records_on_disk() {
    // Deterministic expiry: backdate the record to a past timestamp.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("operations");
    let store = OperationStore::open(&root).unwrap();
    let k = key("tenant-a", "source/read", "op-8");
    store
        .accept(k.clone(), ExecutionClass::ReadOnly, "digest".to_string())
        .unwrap();
    store.mark_running(&k).unwrap();
    let sanitized = k.sanitize();
    let record_path = root.join(format!("{sanitized}.json"));
    let mut text = fs::read_to_string(&record_path).unwrap();
    let mut obj: serde_json::Value = serde_json::from_str(&text).unwrap();
    obj.as_object_mut()
        .unwrap()
        .insert("expires_at_ms".into(), serde_json::json!(0u64));
    text = serde_json::to_string(&obj).unwrap();
    fs::write(&record_path, text).unwrap();
    drop(store);
    let store2 = OperationStore::open(&root).unwrap();
    let dropped = store2.collect_garbage().unwrap();
    assert!(dropped >= 1);
    let record_path_check: &Path = &record_path;
    assert!(!record_path_check.exists());
}

#[test]
fn unauthorized_principal_cannot_query() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp_store(&tmp);
    let k = key("tenant-a", "source/read", "op-9");
    store
        .accept(k.clone(), ExecutionClass::ReadOnly, "digest".to_string())
        .unwrap();
    let mut wrong = k.clone();
    wrong.tenant_id = "tenant-b".to_string();
    let err = store.get(&wrong);
    assert!(matches!(err, Err(OperationError::Unknown)));
}

#[test]
fn failure_settled_preserves_error_code() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp_store(&tmp);
    let k = key("tenant-a", "source/write", "op-10");
    store
        .accept(
            k.clone(),
            ExecutionClass::Deduplicated,
            "digest".to_string(),
        )
        .unwrap();
    store.mark_running(&k).unwrap();
    let err = v1::Error {
        code: v1::ErrorCode::Forbidden as i32,
        message: "denied".into(),
        diagnostic_id: "d".into(),
        execution: "not_started".into(),
        retry: "never".into(),
        details: None,
    };
    let settled = store.settle_failure(&k, err.clone()).unwrap();
    assert_eq!(settled.state, OperationState::Failed);
    let (_, result) = store.get(&k).unwrap();
    let result = result.unwrap();
    assert_eq!(result.success, None);
    assert_eq!(
        result.failure.unwrap().code,
        v1::ErrorCode::Forbidden as i32
    );
}

#[test]
fn default_ttl_is_24_hours() {
    assert_eq!(DEFAULT_TTL_HOURS, 24);
}

#[test]
fn persisted_files_exist_after_accept() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("operations");
    let store = OperationStore::open(&root).unwrap();
    let k = key("tenant-a", "source/read", "op-11");
    store
        .accept(k.clone(), ExecutionClass::ReadOnly, "digest".to_string())
        .unwrap();
    let sanitized = k.sanitize();
    let record_path = root.join(format!("{sanitized}.json"));
    assert!(record_path.exists());
    let (_r, result) = read_record_file(&record_path).unwrap();
    assert!(result.is_none());
}

#[test]
fn settled_failure_writes_result_file() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("operations");
    let store = OperationStore::open(&root).unwrap();
    let k = key("tenant-a", "source/write", "op-12");
    store
        .accept(
            k.clone(),
            ExecutionClass::Deduplicated,
            "digest".to_string(),
        )
        .unwrap();
    store.mark_running(&k).unwrap();
    let err = v1::Error {
        code: v1::ErrorCode::Conflict as i32,
        message: "duplicate".into(),
        diagnostic_id: "d2".into(),
        execution: "completed".into(),
        retry: "never".into(),
        details: None,
    };
    store.settle_failure(&k, err).unwrap();
    let sanitized = k.sanitize();
    let result_path = root.join(format!("{sanitized}.result.json"));
    assert!(result_path.exists());
    let text = fs::read_to_string(&result_path).unwrap();
    assert!(text.contains("duplicate"));
}

#[test]
fn path_traversal_in_key_component_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let store = tmp_store(&tmp);
    let mut k = key("tenant-a", "source/read", "op-13");
    k.tenant_id = "../escape".to_string();
    let err = store.accept(k, ExecutionClass::ReadOnly, "digest".to_string());
    assert!(matches!(err, Err(OperationError::PathTraversal(_))));
}

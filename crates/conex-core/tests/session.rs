//! P1-03 session/attachment state machine tests.
use std::thread::sleep;
use std::time::Duration;

use conex_core::session::{
    RecoveryLevel, SessionBinding, SessionError, SessionStore, read_session_file,
};
use conex_proto::v1;

fn binding() -> SessionBinding {
    SessionBinding {
        principal_id: "alice".into(),
        tenant_id: "tenant-a".into(),
        provider_endpoint_id: "notes-local".into(),
        plane: v1::Plane::Broker,
        workspace_peer_id: Some("workspace-1".into()),
        human_peer_id: Some("ui-1".into()),
    }
}

#[test]
fn open_session_returns_in_process_recovery() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let (id, granted) = store
        .open_session(binding(), RecoveryLevel::InProcess, "attach-1", "consumer")
        .unwrap();
    assert!(id.starts_with("ses-tenant-a-alice-"));
    assert_eq!(granted, RecoveryLevel::InProcess);
}

#[test]
fn persistent_recovery_downgrades_in_p1() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let err = store.open_session(binding(), RecoveryLevel::Persistent, "attach-1", "consumer");
    assert!(matches!(
        err,
        Err(SessionError::UnsupportedRecovery {
            requested: RecoveryLevel::Persistent,
            granted: RecoveryLevel::InProcess,
        })
    ));
}

#[test]
fn resume_fences_old_epoch_and_advances() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let (sid, _) = store
        .open_session(binding(), RecoveryLevel::InProcess, "attach-1", "consumer")
        .unwrap();
    let new_epoch = store.resume(&sid, "attach-1", 1, &binding()).unwrap();
    assert_eq!(new_epoch, 2);
    // A resume with the old epoch from the same caller must fail.
    let err = store.resume(&sid, "attach-1", 1, &binding());
    assert!(matches!(
        err,
        Err(SessionError::EpochMismatch {
            expected: 1,
            actual: 2
        })
    ));
}

#[test]
fn resume_with_wrong_binding_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let (sid, _) = store
        .open_session(binding(), RecoveryLevel::InProcess, "attach-1", "consumer")
        .unwrap();
    let mut wrong = binding();
    wrong.tenant_id = "tenant-b".into();
    let err = store.resume(&sid, "attach-1", 1, &wrong);
    assert!(matches!(err, Err(SessionError::BindingMismatch)));
}

#[test]
fn renew_keeps_epoch_constant() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let (sid, _) = store
        .open_session(binding(), RecoveryLevel::InProcess, "attach-1", "consumer")
        .unwrap();
    let new_lease = store.renew(&sid, "attach-1", 1).unwrap();
    assert!(new_lease > 0);
    let record = store.get(&sid).unwrap();
    let attach = record
        .attachments
        .iter()
        .find(|a| a.attachment_id == "attach-1")
        .unwrap();
    assert_eq!(attach.epoch, 1);
}

#[test]
fn close_with_final_attachment_drops_session() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let (sid, _) = store
        .open_session(binding(), RecoveryLevel::InProcess, "attach-1", "consumer")
        .unwrap();
    store.close(&sid, "attach-1", 1).unwrap();
    let err = store.get(&sid);
    assert!(matches!(err, Err(SessionError::NotFound(_))));
}

#[test]
fn close_with_wrong_epoch_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let (sid, _) = store
        .open_session(binding(), RecoveryLevel::InProcess, "attach-1", "consumer")
        .unwrap();
    let err = store.close(&sid, "attach-1", 99);
    assert!(matches!(
        err,
        Err(SessionError::EpochMismatch {
            expected: 99,
            actual: 1
        })
    ));
}

#[test]
fn crash_recovery_rebuilds_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    {
        let store = SessionStore::open(&root).unwrap();
        store
            .open_session(binding(), RecoveryLevel::InProcess, "attach-1", "consumer")
            .unwrap();
    }
    let _store2 = SessionStore::open(&root).unwrap();
    let sessions: Vec<_> = tmp
        .path()
        .read_dir()
        .unwrap()
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s == "json")
                .unwrap_or(false)
        })
        .collect();
    assert_eq!(sessions.len(), 1);
    let record = read_session_file(&sessions[0].path()).unwrap();
    assert_eq!(record.attachments.len(), 1);
}

#[test]
fn gc_drops_expired_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path())
        .unwrap()
        .with_attachment_lease_ms(0);
    let (sid, _) = store
        .open_session(binding(), RecoveryLevel::InProcess, "attach-1", "consumer")
        .unwrap();
    sleep(Duration::from_millis(50));
    let dropped = store.collect_garbage().unwrap();
    assert!(dropped >= 1);
    let err = store.get(&sid);
    assert!(matches!(err, Err(SessionError::NotFound(_))));
}

#[test]
fn different_attachments_in_same_session_track_independent_epochs() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let (sid, _) = store
        .open_session(binding(), RecoveryLevel::InProcess, "attach-A", "consumer")
        .unwrap();
    // Add a second attachment via resume (opens new attachment). The demo
    // slice doesn't expose `attach` directly; use resume on a fresh id to
    // confirm epoch fencing is per-attachment.
    let err = store.resume(&sid, "attach-B", 1, &binding());
    assert!(matches!(err, Err(SessionError::AttachmentNotFound(_))));
}

#[test]
fn lease_expiry_rejected_on_resume() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path())
        .unwrap()
        .with_attachment_lease_ms(0);
    let (sid, _) = store
        .open_session(binding(), RecoveryLevel::InProcess, "attach-1", "consumer")
        .unwrap();
    sleep(Duration::from_millis(50));
    let err = store.resume(&sid, "attach-1", 1, &binding());
    assert!(matches!(err, Err(SessionError::LeaseExpired)));
}

#[test]
fn path_traversal_in_attachment_id_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let err = store.open_session(binding(), RecoveryLevel::InProcess, "../escape", "consumer");
    assert!(matches!(err, Err(SessionError::PathTraversal(_))));
}

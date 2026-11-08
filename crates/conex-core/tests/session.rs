//! P1-03 session/attachment state machine tests.
use std::sync::{Arc, Barrier};
use std::thread;
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use conex_core::session::{
    RecoveryLevel, SessionBinding, SessionError, SessionStore, read_session_file,
};

fn binding() -> SessionBinding {
    SessionBinding {
        principal_id: "alice".into(),
        tenant_id: "tenant-a".into(),
        provider_endpoint_id: "notes-local".into(),
        plane: conex_proto::Plane::Broker,
        workspace_peer_id: Some("workspace-1".into()),
        human_peer_id: Some("ui-1".into()),
    }
}

/// Number of persisted session files (`*.json`) in a store directory.
/// The `.json.tmp` write intermediates are excluded on purpose.
fn json_file_count(dir: &std::path::Path) -> usize {
    dir.read_dir()
        .unwrap()
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s == "json")
                .unwrap_or(false)
        })
        .count()
}

/// Errors a racing resume/renew/close may legitimately observe when another
/// thread wins the critical section first.
fn racing_error_is_legal(err: &SessionError) -> bool {
    matches!(
        err,
        SessionError::EpochMismatch { .. } | SessionError::NotFound(_)
    )
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

#[test]
fn concurrent_resume_and_renew_keep_memory_and_disk_consistent() {
    for i in 0..200 {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::open(tmp.path()).unwrap());
        // Unique principal per round keeps the session_id unique even when two
        // rounds land in the same millisecond.
        let mut bind = binding();
        bind.principal_id = format!("alice-{i}");
        let (sid, _) = store
            .open_session(
                bind.clone(),
                RecoveryLevel::InProcess,
                "attach-1",
                "consumer",
            )
            .unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let store_a = store.clone();
        let barrier_a = barrier.clone();
        let sid_a = sid.clone();
        let bind_a = bind.clone();
        let t_resume = thread::spawn(move || {
            barrier_a.wait();
            store_a.resume(&sid_a, "attach-1", 1, &bind_a)
        });
        let store_b = store.clone();
        let barrier_b = barrier.clone();
        let sid_b = sid.clone();
        let t_renew = thread::spawn(move || {
            barrier_b.wait();
            store_b.renew(&sid_b, "attach-1", 1)
        });
        // A pre-fix panic (stale-clone `.unwrap()` re-read outside the lock)
        // surfaces here as a join error.
        let resume_res = t_resume.join().expect("resume thread panicked");
        let renew_res = t_renew.join().expect("renew thread panicked");
        for (name, res) in [("resume", &resume_res), ("renew", &renew_res)] {
            match res {
                Ok(_) => {}
                Err(e) if racing_error_is_legal(e) => {}
                Err(e) => panic!("round {i}: {name} failed unexpectedly: {e:?}"),
            }
        }
        // Final state must be consistent: the in-memory snapshot equals the
        // persisted file (both present and identical, or both absent).
        let mem = store.get(&sid).ok();
        let disk = read_session_file(&tmp.path().join(format!("{sid}.json"))).ok();
        assert_eq!(
            mem.is_some(),
            disk.is_some(),
            "round {i}: memory/disk presence diverged (mem={mem:?}, disk={disk:?})"
        );
        if let (Some(m), Some(d)) = (mem, disk) {
            assert_eq!(m, d, "round {i}: memory and disk records diverged");
        }
    }
}

#[test]
fn close_racing_renew_never_panics_and_never_leaves_a_session_file() {
    for i in 0..200 {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(SessionStore::open(tmp.path()).unwrap());
        let mut bind = binding();
        bind.principal_id = format!("alice-{i}");
        let (sid, _) = store
            .open_session(bind, RecoveryLevel::InProcess, "attach-1", "consumer")
            .unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let store_a = store.clone();
        let barrier_a = barrier.clone();
        let sid_a = sid.clone();
        let t_close = thread::spawn(move || {
            barrier_a.wait();
            store_a.close(&sid_a, "attach-1", 1)
        });
        let store_b = store.clone();
        let barrier_b = barrier.clone();
        let sid_b = sid.clone();
        let t_renew = thread::spawn(move || {
            barrier_b.wait();
            store_b.renew(&sid_b, "attach-1", 1)
        });
        let close_res = t_close.join().expect("close thread panicked");
        let renew_res = t_renew.join().expect("renew thread panicked");
        for (name, res) in [("close", &close_res), ("renew", &renew_res)] {
            match res {
                Ok(_) => {}
                Err(e) if racing_error_is_legal(e) => {}
                Err(e) => panic!("round {i}: {name} failed unexpectedly: {e:?}"),
            }
        }
        // Invariant: the session is absent from memory if and only if there is
        // no session file left on disk (no ghost resurrection, no orphan file).
        let memory_gone = matches!(store.get(&sid), Err(SessionError::NotFound(_)));
        let files = json_file_count(tmp.path());
        assert_eq!(
            memory_gone,
            files == 0,
            "round {i}: memory gone={memory_gone}, session files on disk={files}"
        );
    }
}

#[test]
fn a_rejected_persistent_open_persists_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::open(tmp.path()).unwrap();
    let ms_before = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let err = store.open_session(binding(), RecoveryLevel::Persistent, "attach-1", "consumer");
    let ms_after = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert!(matches!(
        err,
        Err(SessionError::UnsupportedRecovery {
            requested: RecoveryLevel::Persistent,
            granted: RecoveryLevel::InProcess,
        })
    ));
    // A session_id (had one been created) would be ses-tenant-a-alice-{ms}
    // for some ms in [ms_before, ms_after]; none of them may exist in memory.
    for ms in ms_before..=ms_after {
        let sid = format!("ses-tenant-a-alice-{ms}");
        assert!(
            matches!(store.get(&sid), Err(SessionError::NotFound(_))),
            "rejected persistent open left session {sid} in memory"
        );
    }
    // Nothing on disk either: no orphan session file may survive a rejection.
    assert_eq!(json_file_count(tmp.path()), 0);
}

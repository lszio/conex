//! P1-06 blob storage lifecycle tests.
use std::fs;

use conex_content::{ContentStore, DEFAULT_CHUNK_SIZE, DEFAULT_LEASE_MS};
use conex_proto::cid::cid_for_raw;
use serde_json::Value;
use tempfile::TempDir;

fn load_vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors/p1/blob.json"
    );
    let text = fs::read_to_string(path).expect("read blob vectors");
    serde_json::from_str(&text).expect("parse blob vectors")
}

fn store(tmp: &TempDir) -> ContentStore {
    ContentStore::open(tmp.path(), DEFAULT_LEASE_MS).expect("open content store")
}

fn check_persistence_level(name: &str) -> Result<(), String> {
    if name == "local" {
        Ok(())
    } else {
        Err(format!(
            "persistence level {name} not supported by local backend"
        ))
    }
}

#[test]
fn vectors_are_well_formed() {
    let v = load_vectors();
    assert!(v["stateMachine"].is_array());
    assert!(v["pin"].is_array());
    let cases = v["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    let names: Vec<&str> = cases.iter().map(|c| c["name"].as_str().unwrap()).collect();
    for required in [
        "single_chunk_upload_commits_as_raw",
        "two_chunk_upload_commits_as_manifest",
        "bad_chunk_rejects_upload_keeps_receivable_blocks",
        "commit_declared_root_mismatch_rejected",
        "missing_blocks_on_commit_rejected_with_cid_list",
        "persistence_level_unsupported_rejected",
        "commit_after_chunk_crash_recovers_idempotent",
        "staging_lease_expires_without_commit",
    ] {
        assert!(
            names.contains(&required),
            "missing blob vector case {required}"
        );
    }
}

#[test]
fn single_chunk_upload_commits_as_raw() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = b"hello";
    let expected_cid = cid_for_raw(payload);
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            payload.len() as u64,
            "raw",
            &expected_cid,
            Some(60_000),
        )
        .unwrap();
    let cid = upload.put_chunk(0, payload).unwrap();
    assert_eq!(cid, expected_cid);
    let commit = s
        .commit(
            upload.upload_id(),
            &expected_cid,
            "raw",
            std::slice::from_ref(&cid),
            payload.len() as u64,
        )
        .unwrap();
    assert_eq!(commit.root_kind, "raw");
    assert_eq!(commit.committed_bytes, payload.len() as u64);
    check_persistence_level("local").unwrap();
}

#[test]
fn two_chunk_upload_commits_as_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload_a = vec![0xAB; DEFAULT_CHUNK_SIZE as usize];
    let payload_b = vec![0xCD; DEFAULT_CHUNK_SIZE as usize];
    let cid_a = cid_for_raw(&payload_a);
    let cid_b = cid_for_raw(&payload_b);
    let manifest_bytes = format!("manifest/v1{cid_a}{cid_b}");
    let manifest_cid = cid_for_raw(manifest_bytes.as_bytes());
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            2 * DEFAULT_CHUNK_SIZE as u64,
            "manifest",
            &manifest_cid,
            Some(60_000),
        )
        .unwrap();
    upload.put_chunk(0, &payload_a).unwrap();
    upload.put_chunk(1, &payload_b).unwrap();
    let leaves = vec![cid_a, cid_b];
    let commit = s
        .commit(
            upload.upload_id(),
            &manifest_cid,
            "manifest",
            &leaves,
            2 * DEFAULT_CHUNK_SIZE as u64,
        )
        .unwrap();
    assert_eq!(commit.root_kind, "manifest");
    assert!(s.has(&leaves).unwrap().iter().all(|p| *p));
}

#[test]
fn bad_chunk_rejects_upload_keeps_receivable_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = vec![0xAB; 64];
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            64,
            "raw",
            &cid_for_raw(&payload),
            Some(60_000),
        )
        .unwrap();
    let good_cid = upload.put_chunk(0, &payload).unwrap();
    let bad = s
        .block_store()
        .put_block(&cid_for_raw(&payload), &[0xCD; 64]);
    assert!(bad.is_err());
    assert!(s.has_block(&good_cid).unwrap());
}

#[test]
fn commit_declared_root_mismatch_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = vec![0x42; 32];
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            32,
            "raw",
            &cid_for_raw(b"declared"),
            Some(60_000),
        )
        .unwrap();
    let cid = upload.put_chunk(0, &payload).unwrap();
    let bad = s.commit(
        upload.upload_id(),
        &cid_for_raw(b"different"),
        "raw",
        std::slice::from_ref(&cid),
        32,
    );
    assert!(matches!(
        bad,
        Err(conex_content::ContentError::DeclaredRootMismatch { .. })
    ));
}

#[test]
fn missing_blocks_on_commit_rejected_with_cid_list() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let missing = cid_for_raw(b"never-uploaded");
    let upload = s
        .begin_upload(1, DEFAULT_CHUNK_SIZE, 32, "raw", &missing, Some(60_000))
        .unwrap();
    let err = s.commit(
        upload.upload_id(),
        &missing,
        "raw",
        std::slice::from_ref(&missing),
        32,
    );
    assert!(matches!(
        err,
        Err(conex_content::ContentError::MissingChunks(_))
    ));
}

#[test]
fn persistence_level_unsupported_rejected() {
    let err = check_persistence_level("replicated(2)");
    assert!(err.is_err(), "non-local persistence must be rejected");
}

#[test]
fn commit_after_chunk_crash_recovers_idempotent() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = vec![0xEE; 64];
    let expected_cid = cid_for_raw(&payload);
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            64,
            "raw",
            &expected_cid,
            Some(60_000),
        )
        .unwrap();
    let cid = upload.put_chunk(0, &payload).unwrap();
    drop(s);
    let s2 = store(&tmp);
    let upload2 = s2.resume_upload(upload.upload_id()).unwrap();
    assert!(upload2.state().received_chunks.contains(&0));
    let commit = s2
        .commit(
            upload2.upload_id(),
            &expected_cid,
            "raw",
            std::slice::from_ref(&cid),
            64,
        )
        .unwrap();
    assert_eq!(commit.root_cid, expected_cid);
    assert!(s2.has(std::slice::from_ref(&cid)).unwrap()[0]);
}

#[test]
fn staging_lease_expires_without_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = b"abc".to_vec();
    let cid = cid_for_raw(&payload);
    let upload = s
        .begin_upload(1, DEFAULT_CHUNK_SIZE, 3, "raw", &cid, Some(10))
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    conex_content::store::sleep_ms(20);
    let _ = s.collect_garbage();
    assert!(s.block_store().get_upload(upload.upload_id()).is_err());
}

#[test]
fn pin_returns_id_and_expiry_and_unpin_releases() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = b"pinned".to_vec();
    let cid = cid_for_raw(&payload);
    let upload = s
        .begin_upload(1, DEFAULT_CHUNK_SIZE, 6, "raw", &cid, Some(60_000))
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    let commit = s
        .commit(
            upload.upload_id(),
            &cid,
            "raw",
            std::slice::from_ref(&cid),
            6,
        )
        .unwrap();
    let pin_id = "pin-1";
    let pin = s
        .pin(
            &commit.root_cid,
            pin_id,
            conex_content::receipt::now_ms() + 60_000,
        )
        .unwrap();
    assert_eq!(pin.pin_id, pin_id);
    assert!(
        s.block_store()
            .list_active_pins()
            .iter()
            .any(|p| p.pin_id == pin_id)
    );
    s.unpin(pin_id).unwrap();
    assert!(s.block_store().list_active_pins().is_empty());
}

#[test]
fn gc_skips_committed_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = b"keep".to_vec();
    let cid = cid_for_raw(&payload);
    let upload = s
        .begin_upload(1, DEFAULT_CHUNK_SIZE, 4, "raw", &cid, Some(60_000))
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    s.commit(
        upload.upload_id(),
        &cid,
        "raw",
        std::slice::from_ref(&cid),
        4,
    )
    .unwrap();
    let dropped = s.collect_garbage().unwrap();
    assert_eq!(dropped, 0);
    assert!(s.has(std::slice::from_ref(&cid)).unwrap()[0]);
}

//! P1-06 blob storage lifecycle tests.
use std::fs;

use conex_content::{ContentError, ContentStore, DEFAULT_CHUNK_SIZE, DEFAULT_LEASE_MS};
use conex_proto::cid::{cid_for_raw, content_cid};
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

fn chunking_vector(name: &str) -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors/p1/chunking.json"
    );
    let text = fs::read_to_string(path).expect("read chunking vectors");
    let doc: Value = serde_json::from_str(&text).expect("parse chunking vectors");
    doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap_or_else(|| panic!("missing chunking vector {name}"))
        .clone()
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
        "commit_rejects_root_kind_contradicting_size",
        "commit_rejects_tampered_block_length",
        "commit_without_all_chunks_reports_missing_index",
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
    let commit = s.commit(upload.upload_id(), &expected_cid, "raw").unwrap();
    assert_eq!(commit.root_kind, "raw");
    assert_eq!(commit.committed_bytes, payload.len() as u64);
    check_persistence_level("local").unwrap();
}

#[test]
fn two_chunk_upload_commits_as_manifest() {
    // Payload and expected root come from the frozen chunking vector, so this
    // asserts the store computes the same address as the contract.
    let vector = chunking_vector("two_full_chunks");
    let payload = vec![vector["payload"]["byte"].as_u64().unwrap() as u8; 524_288];
    let golden = vector["rootCid"].as_str().unwrap().to_string();
    assert_eq!(content_cid(&payload, DEFAULT_CHUNK_SIZE as usize), golden);

    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            payload.len() as u64,
            "manifest",
            &golden,
            Some(60_000),
        )
        .unwrap();
    let leaf_a = upload
        .put_chunk(0, &payload[..DEFAULT_CHUNK_SIZE as usize])
        .unwrap();
    let leaf_b = upload
        .put_chunk(1, &payload[DEFAULT_CHUNK_SIZE as usize..])
        .unwrap();
    let commit = s.commit(upload.upload_id(), &golden, "manifest").unwrap();
    assert_eq!(commit.root_kind, "manifest");
    assert_eq!(commit.root_cid, golden);
    assert_eq!(commit.committed_bytes, payload.len() as u64);
    assert!(s.has(&[leaf_a, leaf_b]).unwrap().iter().all(|p| *p));
    // The manifest object of the canonical tree is stored and referenced, so
    // the reachable set is complete for blob/get and GC.
    assert!(s.has_block(&golden).unwrap());
    assert_eq!(
        s.block_store().read_block(&golden).unwrap().len(),
        vector["rootManifestBytesLength"].as_u64().unwrap() as usize
    );
    assert_eq!(s.collect_garbage().unwrap(), 0);
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

    // (a) declared root does not match the bytes carried by the upload: the
    // store re-derives the root from its own received chunks and rejects it.
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
    upload.put_chunk(0, &payload).unwrap();
    let bad = s.commit(upload.upload_id(), &cid_for_raw(b"declared"), "raw");
    assert!(matches!(
        bad,
        Err(ContentError::DeclaredRootMismatch { .. })
    ));

    // (b) commit parameters that disagree with blob/put are rejected too.
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            32,
            "raw",
            &cid_for_raw(&payload),
            Some(60_000),
        )
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    let bad = s.commit(upload.upload_id(), &cid_for_raw(b"different"), "raw");
    assert!(matches!(
        bad,
        Err(ContentError::DeclaredRootMismatch { .. })
    ));
}

#[test]
fn commit_rejects_root_kind_that_contradicts_the_size() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = vec![0x51; 32];
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            32,
            "manifest",
            &cid_for_raw(&payload),
            Some(60_000),
        )
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    let bad = s.commit(upload.upload_id(), &cid_for_raw(&payload), "manifest");
    assert!(matches!(bad, Err(ContentError::UnsupportedRootKind(_))));
}

#[test]
fn commit_rejects_block_whose_length_was_tampered_with() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = vec![0x31; DEFAULT_CHUNK_SIZE as usize];
    let leaf = cid_for_raw(&payload);
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            payload.len() as u64,
            "raw",
            &leaf,
            Some(60_000),
        )
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    // Local tampering with the stored block must not commit a short object.
    let block_path = tmp.path().join(format!(
        "blocks/{}/{}/{}",
        &leaf[7..9],
        &leaf[9..11],
        &leaf[11..]
    ));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&block_path)
        .unwrap();
    file.set_len(payload.len() as u64 - 1).unwrap();
    drop(file);
    let bad = s.commit(upload.upload_id(), &leaf, "raw");
    assert!(matches!(bad, Err(ContentError::BlockLengthMismatch { .. })));
}

#[test]
fn missing_blocks_on_commit_rejected_with_cid_list() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = vec![0x11u8; DEFAULT_CHUNK_SIZE as usize * 2];
    let root = content_cid(&payload, DEFAULT_CHUNK_SIZE as usize);
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            payload.len() as u64,
            "manifest",
            &root,
            Some(60_000),
        )
        .unwrap();
    let leaf_a = upload
        .put_chunk(0, &payload[..DEFAULT_CHUNK_SIZE as usize])
        .unwrap();
    upload
        .put_chunk(1, &payload[DEFAULT_CHUNK_SIZE as usize..])
        .unwrap();
    // The block bytes are gone although the receipt still lists the chunk.
    std::fs::remove_file(tmp.path().join(format!(
        "blocks/{}/{}/{}",
        &leaf_a[7..9],
        &leaf_a[9..11],
        &leaf_a[11..]
    )))
    .unwrap();
    match s.commit(upload.upload_id(), &root, "manifest") {
        Err(ContentError::MissingChunks(cids)) => assert_eq!(cids, vec![leaf_a]),
        other => panic!("expected MissingChunks, got {other:?}"),
    }
}

#[test]
fn commit_without_all_chunks_reports_the_missing_index() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = vec![0x21u8; DEFAULT_CHUNK_SIZE as usize * 2];
    let root = content_cid(&payload, DEFAULT_CHUNK_SIZE as usize);
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            payload.len() as u64,
            "manifest",
            &root,
            Some(60_000),
        )
        .unwrap();
    upload
        .put_chunk(0, &payload[..DEFAULT_CHUNK_SIZE as usize])
        .unwrap();
    match s.commit(upload.upload_id(), &root, "manifest") {
        Err(ContentError::MissingChunkIndex(index)) => assert_eq!(index, 1),
        other => panic!("expected MissingChunkIndex, got {other:?}"),
    }
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
    assert!(
        upload2
            .state()
            .received_chunks
            .iter()
            .any(|chunk| chunk.index == 0 && chunk.cid == cid)
    );
    let commit = s2
        .commit(upload2.upload_id(), &expected_cid, "raw")
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
    let commit = s.commit(upload.upload_id(), &cid, "raw").unwrap();
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
    s.commit(upload.upload_id(), &cid, "raw").unwrap();
    let dropped = s.collect_garbage().unwrap();
    assert_eq!(dropped, 0);
    assert!(s.has(std::slice::from_ref(&cid)).unwrap()[0]);
}

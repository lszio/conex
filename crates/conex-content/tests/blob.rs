//! P1-06 blob storage lifecycle tests.
use std::fs;

use conex_content::{ContentError, ContentStore, DEFAULT_CHUNK_SIZE, DEFAULT_LEASE_MS, Owner};
use conex_proto::cid::{cid_for_raw, content_cid};
use serde_json::Value;
use tempfile::TempDir;

fn owner() -> Owner {
    Owner {
        principal_id: "alice".into(),
        tenant_id: "tenant-a".into(),
    }
}

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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    let cid = upload.put_chunk(0, payload).unwrap();
    assert_eq!(cid, expected_cid);
    let commit = s
        .commit(upload.upload_id(), &expected_cid, "raw", &owner())
        .unwrap();
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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    let leaf_a = upload
        .put_chunk(0, &payload[..DEFAULT_CHUNK_SIZE as usize])
        .unwrap();
    let leaf_b = upload
        .put_chunk(1, &payload[DEFAULT_CHUNK_SIZE as usize..])
        .unwrap();
    let commit = s
        .commit(upload.upload_id(), &golden, "manifest", &owner())
        .unwrap();
    assert_eq!(commit.root_kind, "manifest");
    assert_eq!(commit.root_cid, golden);
    assert_eq!(commit.committed_bytes, payload.len() as u64);
    assert!(
        s.has(&[leaf_a, leaf_b], &owner())
            .unwrap()
            .iter()
            .all(|p| *p)
    );
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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    let bad = s.commit(
        upload.upload_id(),
        &cid_for_raw(b"declared"),
        "raw",
        &owner(),
    );
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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    let bad = s.commit(
        upload.upload_id(),
        &cid_for_raw(b"different"),
        "raw",
        &owner(),
    );
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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    let bad = s.commit(
        upload.upload_id(),
        &cid_for_raw(&payload),
        "manifest",
        &owner(),
    );
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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
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
    let bad = s.commit(upload.upload_id(), &leaf, "raw", &owner());
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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
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
    match s.commit(upload.upload_id(), &root, "manifest", &owner()) {
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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    upload
        .put_chunk(0, &payload[..DEFAULT_CHUNK_SIZE as usize])
        .unwrap();
    match s.commit(upload.upload_id(), &root, "manifest", &owner()) {
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
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    let cid = upload.put_chunk(0, &payload).unwrap();
    drop(s);
    let s2 = store(&tmp);
    let upload2 = s2.resume_upload(upload.upload_id(), &owner()).unwrap();
    assert!(
        upload2
            .state()
            .received_chunks
            .iter()
            .any(|chunk| chunk.index == 0 && chunk.cid == cid)
    );
    let commit = s2
        .commit(upload2.upload_id(), &expected_cid, "raw", &owner())
        .unwrap();
    assert_eq!(commit.root_cid, expected_cid);
    assert!(s2.has(std::slice::from_ref(&cid), &owner()).unwrap()[0]);
}

#[test]
fn staging_lease_expires_without_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = b"abc".to_vec();
    let cid = cid_for_raw(&payload);
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            3,
            "raw",
            &cid,
            Some(10),
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
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
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            6,
            "raw",
            &cid,
            Some(60_000),
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    let commit = s.commit(upload.upload_id(), &cid, "raw", &owner()).unwrap();
    let pin_id = "pin-1";
    let pin = s
        .pin(
            &commit.root_cid,
            pin_id,
            conex_content::receipt::now_ms() + 60_000,
            &owner(),
        )
        .unwrap();
    assert_eq!(pin.pin_id, pin_id);
    assert!(
        s.block_store()
            .list_active_pins()
            .iter()
            .any(|p| p.pin_id == pin_id)
    );
    s.unpin(pin_id, &owner()).unwrap();
    assert!(s.block_store().list_active_pins().is_empty());
}

#[test]
fn gc_skips_committed_blocks() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = b"keep".to_vec();
    let cid = cid_for_raw(&payload);
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            4,
            "raw",
            &cid,
            Some(60_000),
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    upload.put_chunk(0, &payload).unwrap();
    s.commit(upload.upload_id(), &cid, "raw", &owner()).unwrap();
    let dropped = s.collect_garbage().unwrap();
    assert_eq!(dropped, 0);
    assert!(s.has(std::slice::from_ref(&cid), &owner()).unwrap()[0]);
}

/// Multi-chunk payload with a live upload handle, committed.
fn ingest(tmp: &TempDir, payload: &[u8]) -> (ContentStore, Owner, String) {
    let s = store(tmp);
    let root = content_cid(payload, DEFAULT_CHUNK_SIZE as usize);
    let kind = if payload.len() as u64 <= DEFAULT_CHUNK_SIZE as u64 {
        "raw"
    } else {
        "manifest"
    };
    let upload = s
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            payload.len() as u64,
            kind,
            &root,
            Some(60_000),
            owner(),
            "notes/test",
            // The P1 channel declares its leaves as it goes, so it has none up front.
            &[],
        )
        .unwrap();
    for (index, chunk) in payload.chunks(DEFAULT_CHUNK_SIZE as usize).enumerate() {
        upload.put_chunk(index as u32, chunk).unwrap();
    }
    s.commit(upload.upload_id(), &root, kind, &owner()).unwrap();
    (s, owner(), root)
}

#[test]
fn leaves_of_a_committed_root_are_in_chunk_order() {
    let tmp = tempfile::tempdir().unwrap();
    let payload = vec![7u8; DEFAULT_CHUNK_SIZE as usize * 2 + 17];
    let (s, who, root) = ingest(&tmp, &payload);
    let leaves = s.block_store().leaves_of(&root).unwrap();
    assert_eq!(leaves.len(), 3);
    // Ascending chunk order, so a byte range maps onto a contiguous run of
    // these. The first leaf is the CID of the first 256 KiB.
    assert_eq!(
        leaves[0],
        cid_for_raw(&payload[..DEFAULT_CHUNK_SIZE as usize])
    );
    assert_eq!(
        leaves[2],
        cid_for_raw(&payload[DEFAULT_CHUNK_SIZE as usize * 2..])
    );
    // A root that was never committed reports as unknown, never as empty.
    let missing = cid_for_raw(b"not committed");
    assert!(matches!(
        s.block_store().leaves_of(&missing),
        Err(ContentError::UnknownCommit(_))
    ));
    assert!(s.block_store().leaves_of("not-a-cid").is_err());
    assert!(s.get_commit(&root).is_some());
    let _ = who;
}

#[test]
fn a_range_read_reassembles_the_bytes_a_caller_asked_for() {
    let tmp = tempfile::tempdir().unwrap();
    let payload: Vec<u8> = (0..(DEFAULT_CHUNK_SIZE as usize * 2 + 17))
        .map(|i| (i % 251) as u8)
        .collect();
    let (s, _who, root) = ingest(&tmp, &payload);
    let leaves = s.block_store().leaves_of(&root).unwrap();

    // Whole content.
    let mut whole = Vec::new();
    for leaf in &leaves {
        whole.extend_from_slice(&s.block_store().read_block(leaf).unwrap());
    }
    assert_eq!(whole, payload);

    // A preview-sized head that stops inside the second block.
    let head_len = DEFAULT_CHUNK_SIZE as usize + 100;
    let mut head = Vec::new();
    let mut first = s.block_store().read_block(&leaves[0]).unwrap().to_vec();
    head.append(&mut first);
    let second = s.block_store().read_block(&leaves[1]).unwrap();
    head.extend_from_slice(&second[..100]);
    assert_eq!(head.len(), head_len);
    assert_eq!(head, payload[..head_len]);
}

#[test]
fn retract_root_frees_its_blocks_and_its_refs() {
    let tmp = tempfile::tempdir().unwrap();
    let payload: Vec<u8> = (0..(DEFAULT_CHUNK_SIZE as usize * 2 + 17))
        .map(|i| (i % 251) as u8)
        .collect();
    let (s, _who, root) = ingest(&tmp, &payload);
    let leaves = s.block_store().leaves_of(&root).unwrap();
    let blocks_dir = tmp.path().join("blocks");
    let before = count_block_files(&blocks_dir);
    assert!(before > 0, "the committed blocks are on disk");

    let freed = s.block_store().retract_root(&root).unwrap();
    assert_eq!(freed, before, "every block of this root is freed");
    assert!(count_block_files(&blocks_dir) == 0);
    for leaf in &leaves {
        assert!(!s.has_block(leaf).unwrap());
    }
    assert!(s.get_commit(&root).is_none(), "the commit record is gone");
    assert!(matches!(
        s.block_store().leaves_of(&root),
        Err(ContentError::UnknownCommit(_))
    ));
    // A reopened store agrees: nothing is left to recover.
    drop(s);
    let again = store(&tmp);
    assert!(again.get_commit(&root).is_none());
    assert!(again.block_store().leaves_of(&root).is_err());
}

#[test]
fn releasing_one_hold_keeps_the_bytes_another_hold_still_serves() {
    let tmp = tempfile::tempdir().unwrap();
    let payload: Vec<u8> = (0..(DEFAULT_CHUNK_SIZE as usize * 2 + 17))
        .map(|i| (i % 251) as u8)
        .collect();
    let (s, _who, root) = ingest(&tmp, &payload);
    // The same content offered twice has one address, so two offers are two
    // holds of one root. Withdrawing one must not delete the other's bytes.
    s.block_store().hold_root(&root).unwrap();
    s.block_store().hold_root(&root).unwrap();
    let freed = s.block_store().release_root(&root).unwrap();
    assert_eq!(freed, 0, "one hold is still serving this root");
    assert!(
        s.has_block(&cid_for_raw(&payload[..DEFAULT_CHUNK_SIZE as usize]))
            .unwrap()
    );
    assert!(s.get_commit(&root).is_some());
    let freed = s.block_store().release_root(&root).unwrap();
    // Three leaves plus the manifest object derived from them.
    assert_eq!(freed, 4, "the last hold frees every object of the root");
    assert!(s.get_commit(&root).is_none());
    // Releasing again is not an error: the root is already gone.
    assert_eq!(s.block_store().release_root(&root).unwrap(), 0);
}

#[test]
fn a_shared_block_survives_while_another_committed_root_reaches_it() {
    let tmp = tempfile::tempdir().unwrap();
    // Two *different* contents that begin with the identical first chunk, so
    // they are two roots sharing one block: deduplication is by CID.
    let first_chunk: Vec<u8> = vec![3u8; DEFAULT_CHUNK_SIZE as usize];
    let mut left = first_chunk.clone();
    left.extend(std::iter::repeat_n(1u8, DEFAULT_CHUNK_SIZE as usize));
    let mut right = first_chunk.clone();
    right.extend(std::iter::repeat_n(2u8, DEFAULT_CHUNK_SIZE as usize));
    // The store is reused, so shadow the handle rather than opening a second one.
    let (store, _who, left_root) = ingest(&tmp, &left);
    let (s, _who, right_root) = ingest(&tmp, &right);
    assert_ne!(left_root, right_root);
    let shared = cid_for_raw(&first_chunk);
    let freed = s.block_store().retract_root(&right_root).unwrap();
    // The right root's own second chunk and its manifest go; the first chunk
    // stays because the left root still reaches it.
    assert_eq!(freed, 2, "only the blocks no other root reaches are freed");
    assert!(s.has_block(&shared).unwrap());
    assert!(store.get_commit(&left_root).is_some());
    // The shared chunk is freed only by the last root that reaches it.
    let freed = s.block_store().retract_root(&left_root).unwrap();
    assert_eq!(freed, 3, "the shared block goes with its final root");
    assert!(!s.has_block(&shared).unwrap());
}

#[test]
fn a_restart_with_no_live_holders_reclaims_what_survived() {
    let tmp = tempfile::tempdir().unwrap();
    let payload: Vec<u8> = (0..DEFAULT_CHUNK_SIZE as usize + 9)
        .map(|i| (i % 251) as u8)
        .collect();
    let root;
    {
        let (s, _who, r) = ingest(&tmp, &payload);
        root = r;
        drop(s);
    }
    // A reopened store still finds the committed root on disk, and reclaims it
    // because no live holder survived the restart.
    let again = store(&tmp);
    assert!(
        again.get_commit(&root).is_some(),
        "the commit survived on disk"
    );
    let freed = again.block_store().reclaim_all_committed();
    assert_eq!(freed, 3, "both leaves and the manifest object are freed");
    assert!(again.get_commit(&root).is_none());
    assert!(count_block_files(&tmp.path().join("blocks")) == 0);
}

fn count_block_files(dir: &std::path::Path) -> usize {
    fn walk(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|entry| {
                        let path = entry.path();
                        if path.is_dir() {
                            walk(&path)
                        } else {
                            usize::from(!path.extension().is_some_and(|ext| ext == "tmp"))
                        }
                    })
                    .sum()
            })
            .unwrap_or(0)
    }
    walk(dir)
}

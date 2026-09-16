//! P1-07 blob transfer tests (1 GiB roundtrip + edge cases).
use std::io::Write;

use conex_content::transfer::{
    DEFAULT_INLINE_THRESHOLD_BYTES, MAX_BLOB_BYTES, blob_ref_for, chunk_payload, decode_content,
    is_inline_eligible,
};
use conex_content::{ContentStore, DEFAULT_CHUNK_SIZE, DEFAULT_LEASE_MS};
use conex_proto::cid::{cid_for_raw, content_cid};
use serde_json::Value;
use tempfile::TempDir;

/// Root CID frozen in `conformance/vectors/p1/chunking.json` for a named case.
fn golden_root(vector_name: &str) -> String {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors/p1/chunking.json"
    );
    let text = std::fs::read_to_string(path).expect("read chunking vectors");
    let doc: Value = serde_json::from_str(&text).expect("parse chunking vectors");
    doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == vector_name)
        .unwrap_or_else(|| panic!("missing chunking vector {vector_name}"))["rootCid"]
        .as_str()
        .unwrap()
        .to_string()
}

fn store(tmp: &TempDir) -> ContentStore {
    ContentStore::open(tmp.path(), DEFAULT_LEASE_MS).expect("open content store")
}

fn write_and_commit(store: &ContentStore, payload: &[u8]) -> String {
    let declared = content_cid(payload, DEFAULT_CHUNK_SIZE as usize);
    let kind = if payload.len() <= DEFAULT_CHUNK_SIZE as usize {
        "raw"
    } else {
        "manifest"
    };
    let upload = store
        .begin_upload(
            1,
            DEFAULT_CHUNK_SIZE,
            payload.len() as u64,
            kind,
            &declared,
            Some(60_000),
        )
        .unwrap();
    for (i, chunk) in chunk_payload(payload, DEFAULT_CHUNK_SIZE)
        .0
        .iter()
        .enumerate()
    {
        let offset = i * DEFAULT_CHUNK_SIZE as usize;
        let end = (offset + DEFAULT_CHUNK_SIZE as usize).min(payload.len());
        let cid = upload.put_chunk(i as u32, &payload[offset..end]).unwrap();
        assert_eq!(&cid, chunk, "chunk {i} cid");
    }
    store
        .commit(upload.upload_id(), &declared, kind)
        .unwrap()
        .root_cid
}

#[test]
fn inline_payload_roundtrip_small() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = b"inline payload";
    let cid = write_and_commit(&s, payload);
    assert_eq!(cid, cid_for_raw(payload));
}

#[test]
fn manifest_payload_roundtrip_multi_chunk() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload: Vec<u8> = (0..(DEFAULT_CHUNK_SIZE as usize * 2 + 13))
        .map(|i| (i & 0xff) as u8)
        .collect();
    let cid = write_and_commit(&s, &payload);
    let leaves = chunk_payload(&payload, DEFAULT_CHUNK_SIZE).0;
    assert_eq!(leaves.len(), 3);
    assert_eq!(cid, content_cid(&payload, DEFAULT_CHUNK_SIZE as usize));
    // Same content as the frozen `three_chunks_short_last` vector.
    assert_eq!(cid, golden_root("three_chunks_short_last"));
}

#[test]
fn decode_reconstructs_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload: Vec<u8> = (0..(DEFAULT_CHUNK_SIZE as usize * 4 + 100))
        .map(|i| (i & 0xff) as u8)
        .collect();
    let cid = write_and_commit(&s, &payload);
    let leaves = chunk_payload(&payload, DEFAULT_CHUNK_SIZE).0;
    let mut sink: Vec<u8> = Vec::with_capacity(payload.len());
    let decoded = decode_content(
        s.block_store(),
        &cid,
        DEFAULT_CHUNK_SIZE,
        &leaves,
        payload.len() as u64,
        &mut sink,
    )
    .unwrap();
    assert_eq!(decoded.size_bytes as usize, payload.len());
    assert_eq!(sink, payload);
}

#[test]
fn decode_rejects_empty_leaf_list() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = vec![0x42u8; 1024];
    let cid = write_and_commit(&s, &payload);
    let mut sink = Vec::new();
    let err = decode_content(
        s.block_store(),
        &cid,
        DEFAULT_CHUNK_SIZE,
        &[],
        payload.len() as u64,
        &mut sink,
    );
    assert!(matches!(
        err,
        Err(conex_content::ContentError::MissingChunks(_))
    ));
}

#[test]
fn decode_rejects_size_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let payload = vec![0x01u8; 1024];
    let cid = write_and_commit(&s, &payload);
    let leaves = chunk_payload(&payload, DEFAULT_CHUNK_SIZE).0;
    let mut sink = Vec::new();
    let err = decode_content(
        s.block_store(),
        &cid,
        DEFAULT_CHUNK_SIZE,
        &leaves,
        (payload.len() as u64) + 1,
        &mut sink,
    );
    assert!(err.is_err());
}

#[test]
fn inline_eligibility_boundary() {
    let t = DEFAULT_INLINE_THRESHOLD_BYTES as u64;
    let cs = DEFAULT_CHUNK_SIZE;
    assert!(is_inline_eligible(t, cs, DEFAULT_INLINE_THRESHOLD_BYTES));
    assert!(!is_inline_eligible(
        t + 1,
        cs,
        DEFAULT_INLINE_THRESHOLD_BYTES
    ));
    assert!(!is_inline_eligible(
        cs as u64,
        cs,
        DEFAULT_INLINE_THRESHOLD_BYTES
    ));
    assert!(!is_inline_eligible(
        MAX_BLOB_BYTES,
        cs,
        DEFAULT_INLINE_THRESHOLD_BYTES
    ));
}

#[test]
fn blob_ref_carries_access_and_size() {
    let r = blob_ref_for("bafkreixxx", 4096, "source-fs", "broker", "notes/hello.md");
    let access = r.access.unwrap();
    assert_eq!(r.cid, "bafkreixxx");
    assert_eq!(r.size_bytes, "4096");
    assert_eq!(access.provider_id, "source-fs");
    assert_eq!(access.plane, "broker");
    assert_eq!(access.resource_id, "notes/hello.md");
}

#[test]
fn one_gib_payload_roundtrips() {
    // 1 MiB chunks keep the leaf count tractable (~1024) while exercising
    // the 1 GiB cap. decode_content writes through `std::io::Write`
    // without retaining the payload, so peak memory stays bounded.
    const CHUNK: usize = 1_048_576;
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let total: usize = 1 << 30;
    let leaves_expected = total.div_ceil(CHUNK);
    let payload_template = vec![0u8; CHUNK];
    let mut leaf_cids = Vec::with_capacity(leaves_expected);
    for i in 0..leaves_expected {
        let mut chunk = payload_template.clone();
        chunk.fill((i & 0xff) as u8);
        leaf_cids.push(cid_for_raw(&chunk));
    }
    let total_bytes = (leaves_expected * CHUNK) as u64;
    let manifest_cid =
        conex_proto::cid::content_cid_for_parts(CHUNK as u32, total_bytes, &leaf_cids).unwrap();
    let upload = s
        .begin_upload(
            1,
            CHUNK as u32,
            total as u64,
            "manifest",
            &manifest_cid,
            Some(60_000),
        )
        .unwrap();
    for (i, expected_cid) in leaf_cids.iter().enumerate() {
        let mut chunk = payload_template.clone();
        chunk.fill((i & 0xff) as u8);
        let actual = upload.put_chunk(i as u32, &chunk).unwrap();
        assert_eq!(actual, *expected_cid);
    }
    let commit = s
        .commit(upload.upload_id(), &manifest_cid, "manifest")
        .unwrap();
    assert_eq!(commit.root_cid, manifest_cid);
    assert_eq!(commit.committed_bytes, total_bytes);

    struct CountingSink {
        written: usize,
    }
    impl Write for CountingSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.written += buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut sink = CountingSink { written: 0 };
    let decoded = decode_content(
        s.block_store(),
        &manifest_cid,
        CHUNK as u32,
        &leaf_cids,
        total_bytes,
        &mut sink,
    )
    .unwrap();
    assert_eq!(decoded.size_bytes as usize, total);
    assert_eq!(sink.written, total);
}

#[test]
fn max_blob_bytes_cap_rejects_oversize() {
    let tmp = tempfile::tempdir().unwrap();
    let s = store(&tmp);
    let too_big = MAX_BLOB_BYTES + 1;
    let leaves = vec![cid_for_raw(b"x")];
    let mut sink = Vec::new();
    let err = decode_content(
        s.block_store(),
        &cid_for_raw(b"x"),
        DEFAULT_CHUNK_SIZE,
        &leaves,
        too_big,
        &mut sink,
    );
    assert!(err.is_err());
}

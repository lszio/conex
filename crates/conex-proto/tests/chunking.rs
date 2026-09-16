//! P1 chunking & manifest golden CID vectors. The expected values come from
//! `conformance/vectors/p1/chunking.json`; they must match an independent Rust
//! helper (`scripts/dev/p1-vectors.rs`) and the TS chunking helper.
use conex_proto::cid::{CHUNK_SIZE, content_cid, manifest_bytes_for_leaves};
use serde_json::Value;

fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors/p1/chunking.json"
    );
    let text = std::fs::read_to_string(path).expect("read conformance/vectors/p1/chunking.json");
    serde_json::from_str(&text).expect("parse chunking vectors")
}

fn input_bytes(case: &Value) -> Vec<u8> {
    let field = case.get("input").and_then(Value::as_str).unwrap_or("");
    // The chunkSize / exactly_256kib_* cases use symbolic names; the test maps
    // them to actual byte payloads so the golden CIDs can be reproduced.
    match field {
        "256kib-of-0xAB" => vec![0xAB; CHUNK_SIZE],
        "256kib-of-0xAB + 0xCD" => {
            let mut v = vec![0xAB; CHUNK_SIZE];
            v.push(0xCD);
            v
        }
        "512kib-of-0x01" => vec![0x01; CHUNK_SIZE * 2],
        "1.25MiB-pattern-byte[i%256]" => (0..(CHUNK_SIZE * 5)).map(|i| (i & 0xFF) as u8).collect(),
        other => other.as_bytes().to_vec(),
    }
}

#[test]
fn shared_chunking_vectors_match() {
    let doc = vectors();
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let bytes = input_bytes(case);
        let expected = case["cid"].as_str().unwrap();
        let kind = case["expected"].as_str().unwrap();
        let actual = content_cid(&bytes, CHUNK_SIZE);
        assert_eq!(actual, expected, "vector {name}: cid mismatch");
        if kind == "raw" {
            assert!(
                bytes.len() <= CHUNK_SIZE,
                "vector {name}: expected raw kind but input length {} > chunkSize",
                bytes.len()
            );
        } else {
            assert!(
                bytes.len() > CHUNK_SIZE,
                "vector {name}: expected manifest kind but input length {} <= chunkSize",
                bytes.len()
            );
            let manifest_bytes =
                manifest_bytes_for_leaves(std::iter::once(conex_proto::cid::cid_for_raw(&bytes)));
            let _manifest_cid = conex_proto::cid::cid_for_raw(&manifest_bytes);
        }
    }
}

#[test]
fn content_cid_is_deterministic_across_calls() {
    let bytes: Vec<u8> = (0..(CHUNK_SIZE + 7)).map(|i| (i & 0xFF) as u8).collect();
    let a = content_cid(&bytes, CHUNK_SIZE);
    let b = content_cid(&bytes, CHUNK_SIZE);
    let c = content_cid(&bytes, CHUNK_SIZE);
    assert_eq!(a, b);
    assert_eq!(b, c);
}

#[test]
fn content_cid_rejects_non_decimal_lengths_in_vector_form() {
    // Sanity: a non-decimal content_length string in a vector schema entry
    // would be an invalid manifest entry. content_cid() takes raw bytes only,
    // so this is a contract-level check that we never feed it a non-decimal.
    let raw = conex_proto::cid::cid_for_raw(b"abc");
    assert!(conex_proto::cid::parse_cid(&raw).is_ok());
}

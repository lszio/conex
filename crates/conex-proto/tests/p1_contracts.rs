//! P1 contract classification vectors.
//!
//! These vectors encode the acceptance criteria from docs/contracts/p1-*.md.
//! The Rust side classifies each case against the same contract text the TS
//! side does; both must agree on `expect` classifications. Cases that require
//! running a full broker/stream/operation stack will gain specific
//! implementations in P1-02..10 — for now we enforce the static invariants
//! (vector shape, field presence, deterministic ordering).
use serde_json::Value;

fn load(name: &str) -> Value {
    let path = format!(
        "{}/../../conformance/vectors/p1/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path}: {e}"))
}

fn all_cases_have_expect(doc: &Value, key: &str) {
    let cases = doc["cases"]
        .as_array()
        .unwrap_or_else(|| panic!("{key}: cases array missing"));
    assert!(!cases.is_empty(), "{key}: empty cases");
    for case in cases {
        assert!(
            case["expect"].is_string(),
            "{key} case {}: missing string expect",
            case["name"].as_str().unwrap_or("?")
        );
    }
}

fn flat_cases_have_expect(doc: &Value, key: &str, container: &str) {
    let cases = doc[container]
        .as_array()
        .unwrap_or_else(|| panic!("{key}: {container} missing"));
    assert!(!cases.is_empty(), "{key}: empty {container}");
    for case in cases {
        assert!(
            case["expect"].is_string(),
            "{key}/{container} case {}: missing string expect",
            case["name"].as_str().unwrap_or("?")
        );
    }
}

#[test]
fn blob_vectors_are_well_formed() {
    let doc = load("blob.json");
    assert!(doc["stateMachine"].is_array());
    assert!(doc["pin"].is_array());
    all_cases_have_expect(&doc, "blob");
}

#[test]
fn stream_vectors_cover_ack_credit_reset_epoch_slow_consumer() {
    let doc = load("stream.json");
    for key in [
        "ackCases",
        "creditCases",
        "resetCases",
        "slowConsumerCases",
        "epochCases",
    ] {
        flat_cases_have_expect(&doc, "stream", key);
    }
    assert_eq!(
        doc["frameShape"]["zeroByteMessage"].as_str(),
        Some("rejected"),
        "zero-byte data frame must be rejected"
    );
    assert_eq!(
        doc["frameShape"]["seqStart"].as_u64(),
        Some(1),
        "seq must start at 1"
    );
}

#[test]
fn operation_vectors_cover_dedup_execution_classes_and_conditional_write() {
    let doc = load("operation.json");
    let classes = doc["executionClasses"]
        .as_array()
        .expect("executionClasses");
    let names: Vec<&str> = classes
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    for required in ["read_only", "idempotent", "deduplicated", "non_replayable"] {
        assert!(
            names.contains(&required),
            "operation.json: missing execution class {required}"
        );
    }
    flat_cases_have_expect(&doc, "operation", "cases");
    flat_cases_have_expect(&doc, "operation", "expectedRevisionCases");
}

#[test]
fn chunking_rejects_invalid_cid_shapes() {
    let doc = load("chunking.json");
    let rejects = doc["rejects"].as_array().expect("chunking rejects array");
    assert!(
        rejects.iter().any(|r| r["name"] == "non_raw_cid_rejected"),
        "chunking must enumerate non-raw codec rejection"
    );
    assert!(
        rejects
            .iter()
            .any(|r| r["name"] == "non_sha256_multihash_rejected"),
        "chunking must enumerate non-sha256 rejection"
    );
}

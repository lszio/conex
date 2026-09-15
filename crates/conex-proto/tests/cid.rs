//! CIDv1 raw/SHA-256 standard vectors and rejection cases.
use conex_proto::cid::{cid_for_raw, parse_cid};
use serde_json::Value;

fn vectors() -> Vec<Value> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors/p0/cid.json"
    );
    let text = std::fs::read_to_string(path).expect("read conformance/vectors/p0/cid.json");
    serde_json::from_str(&text).expect("parse cid vectors")
}

#[test]
fn raw_hello_matches_standard_cid_bytes() {
    let text = cid_for_raw(b"hello");
    let cid = parse_cid(&text).unwrap();
    assert_eq!(
        hex::encode(cid.to_bytes()),
        "015512202cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

#[test]
fn shared_cid_vectors_match() {
    let cases = vectors();
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let input = case["input_utf8"].as_str().unwrap();
        let expected = case["cid"].as_str().unwrap();
        let bytes_hex = case["bytes_hex"].as_str().unwrap();
        let text = cid_for_raw(input.as_bytes());
        assert_eq!(text, expected, "vector {name}: text CID");
        let cid = parse_cid(&text).unwrap();
        assert_eq!(
            hex::encode(cid.to_bytes()),
            bytes_hex,
            "vector {name}: bytes"
        );
        // Other accepted text base (base32upper) normalizes to the same bytes.
        let upper = parse_cid(&text.to_uppercase()).unwrap();
        assert_eq!(
            upper.to_bytes(),
            cid.to_bytes(),
            "vector {name}: normalized"
        );
    }
}

#[test]
fn rejects_wrong_digest_length_and_unsupported_multihash() {
    use cid::Cid;
    use multihash::Multihash;

    // sha2-256 code but a 31-byte digest.
    let short = Multihash::<64>::wrap(0x12, &[0u8; 31]).unwrap();
    assert!(parse_cid(&Cid::new_v1(0x55, short).to_string()).is_err());

    // unsupported multihash code (sha2-512) under a raw codec.
    let other = Multihash::<64>::wrap(0x13, &[0u8; 64]).unwrap();
    assert!(parse_cid(&Cid::new_v1(0x55, other).to_string()).is_err());
}

#[test]
fn rejects_unsupported_or_malformed_cids() {
    // CIDv0 (dag-pb) is not a P0 content address.
    assert!(parse_cid("QmYwAPJzv5CZsnAzt8auVZRnAn7p9HRw1L1TqQ6T6t9K9Q").is_err());
    // trailing byte after a valid raw/SHA-256 CID
    assert!(parse_cid("bafkreibm6jg3ux5qumhcn2b3flc3tyu6dmlb4xa7u5bf44yegnrjhc4yeqA").is_err());
    // not a CID at all
    assert!(parse_cid("not-a-cid").is_err());
}

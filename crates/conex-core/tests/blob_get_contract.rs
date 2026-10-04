//! M0 contract invariants for blob/get: exclusive committed/remote targets,
//! 64-bit-safe decimal ranges, and exclusive output identity.

use conex_core::contracts::prepare_blob_get;
use serde_json::{Value, json};

fn valid_committed() -> Value {
    json!({"committed": {"chunkCid": "bafkreicnbk2gkwoowxiq3loyrlyfdd42wwhcyvv67ny33drroaybh4qvxy"}})
}

fn valid_remote() -> Value {
    json!({
        "remote": {
            "endpointId": "endpoint-a",
            "resourceId": "notes/a.md",
            "offset": "0",
            "length": "65536"
        }
    })
}

#[test]
fn committed_target_canonicalizes() {
    let prepared = prepare_blob_get(&valid_committed()).unwrap();
    assert_eq!(
        prepared.canonical,
        json!({"committed": {"chunkCid": "bafkreicnbk2gkwoowxiq3loyrlyfdd42wwhcyvv67ny33drroaybh4qvxy"}})
    );
    assert_eq!(prepared.claim.action, "blob.get");
}

#[test]
fn remote_target_canonicalizes_with_decimal_ranges() {
    let prepared = prepare_blob_get(&valid_remote()).unwrap();
    assert_eq!(prepared.claim.action, "read");
    assert_eq!(prepared.claim.resource_id, "notes/a.md");
    assert_eq!(
        prepared.canonical["remote"]["offset"],
        json!("0"),
        "offset stays a decimal string"
    );
    assert_eq!(prepared.canonical["remote"]["length"], json!("65536"));
}

#[test]
fn mixed_targets_are_rejected() {
    let mut mixed = valid_committed();
    mixed["remote"] = valid_remote()["remote"].clone();
    assert!(prepare_blob_get(&mixed).is_err());
}

#[test]
fn missing_target_is_rejected() {
    assert!(prepare_blob_get(&json!({})).is_err());
}

#[test]
fn remote_requires_endpoint() {
    let mut input = valid_remote();
    input["remote"]["endpointId"] = json!("");
    assert!(prepare_blob_get(&input).is_err());
}

#[test]
fn remote_rejects_invalid_resource() {
    for bad in ["../escape", "a//b", "/abs", "a\\b", "a/./b"] {
        let mut input = valid_remote();
        input["remote"]["resourceId"] = json!(bad);
        assert!(prepare_blob_get(&input).is_err(), "{bad} must be rejected");
    }
}

#[test]
fn remote_rejects_non_decimal_and_zero_length() {
    let mut input = valid_remote();
    input["remote"]["offset"] = json!("not-a-number");
    assert!(prepare_blob_get(&input).is_err());

    let mut input = valid_remote();
    input["remote"]["length"] = json!("0");
    assert!(
        prepare_blob_get(&input).is_err(),
        "empty range is not a read"
    );
}

#[test]
fn remote_rejects_64bit_overflow() {
    let mut input = valid_remote();
    input["remote"]["offset"] = json!(u64::MAX.to_string());
    input["remote"]["length"] = json!("1");
    assert!(
        prepare_blob_get(&input).is_err(),
        "offset+length must not wrap"
    );
}

#[test]
fn output_identity_is_exclusive() {
    let contract = conex_core::contracts::contracts()
        .into_iter()
        .find(|(method, _)| *method == "blob/get")
        .map(|(_, contract)| contract)
        .unwrap();

    let committed = json!({
        "chunkBytes": "AAEC",
        "chunkCid": "bafkreicnbk2gkwoowxiq3loyrlyfdd42wwhcyvv67ny33drroaybh4qvxy"
    });
    (contract.validate_output)(&committed).unwrap();

    let remote = json!({
        "chunkBytes": "AAEC",
        "revision": "rev-1",
        "eof": false
    });
    (contract.validate_output)(&remote).unwrap();

    let both = json!({
        "chunkBytes": "AAEC",
        "chunkCid": "bafkreicnbk2gkwoowxiq3loyrlyfdd42wwhcyvv67ny33drroaybh4qvxy",
        "revision": "rev-1"
    });
    assert!((contract.validate_output)(&both).is_err());

    let bad_cid = json!({
        "chunkBytes": "AAEC",
        "chunkCid": "not-a-cid"
    });
    assert!((contract.validate_output)(&bad_cid).is_err());
}

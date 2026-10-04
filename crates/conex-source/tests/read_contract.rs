//! M0 read-output invariants: text ⊕ content exclusivity, BlobRef shape,
//! and CID-format enforcement.

use conex_core::MethodContract;
use serde_json::json;

fn read_contract() -> MethodContract {
    conex_source::contracts()
        .into_iter()
        .find(|(method, _)| *method == "source/read")
        .map(|(_, contract)| contract)
        .unwrap()
}

fn inline_text() -> serde_json::Value {
    json!({
        "resource": {
            "resourceId": "notes/hello.md",
            "title": "hello.md",
            "mime": "text/markdown",
            "sizeBytes": "12",
            "kind": "ENTRY_KIND_FILE"
        },
        "text": "hello conex\n",
        "cid": "bafkreicnbk2gkwoowxiq3loyrlyfdd42wwhcyvv67ny33drroaybh4qvxy"
    })
}

fn content_ref() -> serde_json::Value {
    json!({
        "resource": {
            "resourceId": "media/clip.mp4",
            "title": "clip.mp4",
            "mime": "application/octet-stream",
            "sizeBytes": "1073741824",
            "kind": "ENTRY_KIND_FILE"
        },
        "content": {
            "sizeBytes": "1073741824",
            "mime": "video/mp4",
            "access": {
                "endpointId": "endpoint-a",
                "plane": "broker",
                "resourceId": "media/clip.mp4"
            },
            "revision": "1717200000000000000"
        }
    })
}

#[test]
fn inline_text_output_is_accepted() {
    (read_contract().validate_output)(&inline_text()).unwrap();
}

#[test]
fn content_reference_output_is_accepted() {
    (read_contract().validate_output)(&content_ref()).unwrap();
}

#[test]
fn text_and_content_are_mutually_exclusive() {
    let mut both = inline_text();
    both["content"] = content_ref()["content"].clone();
    assert!(
        (read_contract().validate_output)(&both).is_err(),
        "a read result is either inline text or a content reference"
    );
}

#[test]
fn content_cid_must_parse() {
    let mut bad = content_ref();
    bad["content"]["cid"] = json!("not-a-cid");
    assert!((read_contract().validate_output)(&bad).is_err());
}

#[test]
fn inline_cid_must_parse() {
    let mut bad = inline_text();
    bad["cid"] = json!("../resource-id-as-cid");
    assert!(
        (read_contract().validate_output)(&bad).is_err(),
        "resource paths must not pass as CIDs"
    );
}

#[test]
fn content_requires_decimal_size() {
    let mut bad = content_ref();
    bad["content"]["sizeBytes"] = json!(4096);
    assert!((read_contract().validate_output)(&bad).is_err());
}

#[test]
fn summary_size_must_be_decimal_string() {
    let mut bad = inline_text();
    bad["resource"]["sizeBytes"] = json!(12);
    assert!((read_contract().validate_output)(&bad).is_err());
}

#[test]
fn summary_kind_must_be_known() {
    let mut bad = inline_text();
    bad["resource"]["kind"] = json!("ENTRY_KIND_UNRECOGNIZED");
    assert!((read_contract().validate_output)(&bad).is_err());
}

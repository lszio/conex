//! Shared wire vectors: envelope classification, error codes and rejection.
#![allow(clippy::collapsible_if)]

use conex_proto::{v1, wire};
use serde_json::Value;

fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors/p0/wire.json"
    );
    let text = std::fs::read_to_string(path).expect("read conformance/vectors/p0/wire.json");
    serde_json::from_str(&text).expect("parse wire vectors")
}

fn kind(message: &v1::Message) -> &'static str {
    match message.body.as_ref() {
        Some(v1::message::Body::Request(_)) => "request",
        Some(v1::message::Body::Success(_)) => "success",
        Some(v1::message::Body::Failure(_)) => "failure",
        Some(v1::message::Body::Notification(_)) => "notification",
        None => "none",
    }
}

fn request_id(message: &v1::Message) -> Option<&str> {
    match message.body.as_ref() {
        Some(v1::message::Body::Request(r)) => Some(r.request_id.as_str()),
        Some(v1::message::Body::Success(s)) => Some(s.request_id.as_str()),
        Some(v1::message::Body::Failure(f)) => f.request_id.as_deref(),
        _ => None,
    }
}

fn method(message: &v1::Message) -> Option<&str> {
    match message.body.as_ref() {
        Some(v1::message::Body::Request(r)) => Some(r.method.as_str()),
        Some(v1::message::Body::Notification(n)) => Some(n.method.as_str()),
        _ => None,
    }
}

fn error_code(message: &v1::Message) -> Option<i32> {
    match message.body.as_ref() {
        Some(v1::message::Body::Failure(f)) => f.error.as_ref().map(|e| e.code),
        _ => None,
    }
}

#[test]
fn error_code_table_is_frozen() {
    let doc = vectors();
    for entry in doc["errorCodes"].as_array().expect("errorCodes") {
        let name = entry["name"].as_str().unwrap();
        let value = entry["value"].as_i64().unwrap() as i32;
        assert_eq!(
            wire::error_code_name(value),
            name,
            "semantic name for {value}"
        );
        assert_eq!(
            v1::ErrorCode::try_from(value).unwrap() as i32,
            value,
            "enum number for {name}"
        );
    }
}

#[test]
fn shared_wire_vectors_classify_identically() {
    let doc = vectors();
    let cases = doc["cases"].as_array().expect("cases");
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let expect = &case["expect"];
        let bytes = case["wire"].as_str().unwrap().as_bytes();
        match wire::decode_wire(bytes) {
            Ok(message) => {
                assert_eq!(
                    Some(kind(&message)),
                    expect["kind"].as_str(),
                    "vector {name}: kind"
                );
                if let Some(id) = expect["requestId"].as_str() {
                    assert_eq!(request_id(&message), Some(id), "vector {name}: requestId");
                }
                if let Some(m) = expect["method"].as_str() {
                    assert_eq!(method(&message), Some(m), "vector {name}: method");
                }
                if kind(&message) == "failure" {
                    if let Some(code) = expect["errorCode"].as_i64() {
                        assert_eq!(
                            error_code(&message),
                            Some(code as i32),
                            "vector {name}: errorCode"
                        );
                    }
                }
                match wire::validate_message(&message) {
                    Ok(()) => assert_eq!(
                        expect["valid"].as_bool(),
                        Some(true),
                        "vector {name}: unexpectedly valid"
                    ),
                    Err(e) => {
                        assert_eq!(
                            expect["valid"].as_bool(),
                            Some(false),
                            "vector {name}: unexpected validation error {e:?}"
                        );
                        if let Some(code) = expect["errorCode"].as_i64() {
                            assert_eq!(e.rpc_code, code as i32, "vector {name}: validation code");
                        }
                    }
                }
            }
            Err(e) => {
                assert!(
                    expect.get("kind").is_none(),
                    "vector {name}: unexpectedly rejected: {e:?}"
                );
                assert_eq!(
                    Some(e.rpc_code),
                    expect["errorCode"].as_i64().map(|v| v as i32),
                    "vector {name}: reject code"
                );
            }
        }
    }
}

#[test]
fn request_round_trips_through_encode() {
    let doc = vectors();
    let case = doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "request_roundtrip")
        .unwrap();
    let original = wire::decode_wire(case["wire"].as_str().unwrap().as_bytes()).unwrap();
    let encoded = wire::encode_wire(&original).unwrap();
    let decoded = wire::decode_wire(&encoded).unwrap();
    assert_eq!(original, decoded);
}

#[test]
fn failure_encodes_numeric_code_and_semantic_name() {
    let message = v1::Message {
        body: Some(v1::message::Body::Failure(v1::Failure {
            request_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".into()),
            error: Some(v1::Error {
                code: v1::ErrorCode::Forbidden as i32,
                message: "denied".into(),
                diagnostic_id: "d1".into(),
                execution: "not_started".into(),
                retry: "never".into(),
                details: None,
            }),
        })),
    };
    let encoded = wire::encode_wire(&message).unwrap();
    let json: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(json["error"]["code"], Value::from(-32002));
    assert_eq!(json["error"]["data"]["code"], Value::from("forbidden"));
    assert_eq!(
        json["error"]["data"]["execution"],
        Value::from("not_started")
    );
}

#[test]
fn deeply_nested_json_is_rejected() {
    let mut text = String::from(
        "{\"jsonrpc\":\"2.0\",\"id\":\"01ARZ3NDEKTSV4RRFFQ69G5FAV\",\"method\":\"m\",\"params\":",
    );
    text.push_str(&"[".repeat(300));
    text.push_str(&"]".repeat(300));
    text.push('}');
    assert!(wire::decode_wire(text.as_bytes()).is_err());
}

const SOURCE_READ_SCHEMA: &str =
    include_str!("../../../schema/generated/jsonschema/conex.v1.SourceReadRequest.schema.json");

#[test]
fn source_request_rejects_unknown_business_field() {
    use conex_proto::validation;
    let schema: Value = serde_json::from_str(SOURCE_READ_SCHEMA).unwrap();
    assert!(validation::validate(&schema, &serde_json::json!({"resourceId": "hello.md"})).is_ok());
    assert!(
        validation::validate(
            &schema,
            &serde_json::json!({"resourceId": "hello.md", "principalId": "spoofed"})
        )
        .is_err()
    );
}

#[test]
fn ulid_validation_matches_spec() {
    assert!(wire::is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
    assert!(!wire::is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FA")); // too short
    assert!(!wire::is_ulid("81ARZ3NDEKTSV4RRFFQ69G5FAV")); // first char overflow
    assert!(!wire::is_ulid("01ARZ3NDEKTSV4RRFFQ69G5FAI")); // excluded letter I
}

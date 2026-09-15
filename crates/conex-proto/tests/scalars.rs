//! Shared scalar vectors: Rust and TS must classify every case identically.
use conex_proto::validation;
use serde_json::Value;

const SCHEMA: &str =
    include_str!("../../../schema/generated/jsonschema/conex.test.v1.Scalars.schema.json");

fn vectors() -> Vec<Value> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors/p0/scalars.json"
    );
    let text = std::fs::read_to_string(path).expect("read conformance/vectors/p0/scalars.json");
    serde_json::from_str(&text).expect("parse vectors")
}

#[test]
fn shared_scalar_vectors_match_schema() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    let cases = vectors();
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let accept = case["accept"].as_bool().unwrap();
        let result = validation::validate(&schema, &case["input"]);
        assert_eq!(result.is_ok(), accept, "vector {name}: {result:?}");
    }
}

#[test]
fn max_u64_and_false_keep_presence() {
    use conex_proto::test::v1::Scalars;
    let value: Scalars =
        serde_json::from_str(r#"{"count":"18446744073709551615","enabled":false}"#).unwrap();
    assert_eq!(value.count, Some(u64::MAX));
    assert_eq!(value.enabled, Some(false));
}

#[test]
fn empty_object_is_all_none() {
    use conex_proto::test::v1::Scalars;
    let value: Scalars = serde_json::from_str("{}").unwrap();
    assert_eq!(value.count, None);
    assert_eq!(value.enabled, None);
}

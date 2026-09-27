
use serde_json::json;

use conex_proto;
use conex_proto::wire::{decode_wire, encode_wire};

use super::{LocalProvider, dispatch_local, handle_request};
use crate::config::EndpointConfig;

#[tokio::test]
async fn local_read_uses_canonical_wire_envelope() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("hello.md"), "hello agent").expect("write fixture");
    let endpoint = EndpointConfig {
        endpoint_id: "notes".into(),
        root: dir.path().into(),
        methods: vec!["source/read".into()],
        resources: vec!["*".into()],
    };
    let provider = LocalProvider::new(&endpoint).expect("provider");
    let request = conex_proto::Message {
        body: Some(conex_proto::message::Body::Request(conex_proto::Request {
            request_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            method: "source/read".into(),
            params: Some(conex_proto::CallParams {
                context: Some(conex_proto::RequestContext {
                    provider_endpoint_id: "notes".into(),
                    plane: conex_proto::Plane::Broker as i32,
                    binding_id: None,
                }),
                timeout_budget_ms: 1_000,
                input: Some(serde_json::from_value(json!({"resourceId":"hello.md"})).unwrap()),
            }),
        })),
    };
    let encoded = encode_wire(&request).expect("encode request");
    decode_wire(&encoded).expect("request decodes");
    let response = handle_request(&[provider], &encoded).await.expect("response");
    let message = decode_wire(response.as_bytes()).expect("decode response");
    let Some(conex_proto::message::Body::Success(success)) = message.body else {
        panic!("expected success");
    };
    let result = success.result.expect("result");
    let value = serde_json::to_value(&result).expect("result json");
    assert_eq!(value["text"].as_str(), Some("hello agent"));
}

#[tokio::test]
async fn local_provider_rejects_traversal_and_disabled_methods() {
    let dir = tempfile::tempdir().expect("tempdir");
    let endpoint = EndpointConfig {
        endpoint_id: "notes".into(),
        root: dir.path().into(),
        methods: vec!["source/read".into()],
        resources: vec!["*".into()],
    };
    let provider = LocalProvider::new(&endpoint).expect("provider");
    let traversal = dispatch_local(&[provider], "source/read", json!({"resourceId":"../secret"}), "notes").await;
    assert!(traversal.is_err());
    let disabled = dispatch_local(&[], "source/read", json!({"resourceId":"a.md"}), "notes").await;
    assert!(disabled.is_err());
}

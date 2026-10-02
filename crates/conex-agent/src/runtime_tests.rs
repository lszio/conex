use serde_json::json;

use conex_proto;
use conex_proto::wire::{decode_wire, encode_wire};

use super::{EndpointProvider, dispatch_local};
use crate::config::EndpointConfig;

fn endpoint(dir: &std::path::Path, methods: Vec<&str>) -> EndpointConfig {
    EndpointConfig {
        endpoint_id: "notes".into(),
        root: dir.into(),
        methods: methods.into_iter().map(String::from).collect(),
        resources: vec!["*".into()],
    }
}

#[tokio::test]
async fn local_read_routes_through_provider_handlers() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("hello.md"), "hello agent").expect("write fixture");
    let provider =
        EndpointProvider::new(&endpoint(dir.path(), vec!["source/read"])).expect("provider");
    let value = dispatch_local(
        &[provider],
        "source/read",
        json!({"resourceId":"hello.md"}),
        "notes",
        "alice",
        "tenant-a",
    )
    .await
    .expect("read");
    assert_eq!(value["text"].as_str(), Some("hello agent"));
    assert_eq!(value["resource"]["revision"].as_str().is_some(), true);
}

#[tokio::test]
async fn local_provider_rejects_traversal_unknown_endpoint_and_disabled_methods() {
    let dir = tempfile::tempdir().expect("tempdir");
    let provider =
        EndpointProvider::new(&endpoint(dir.path(), vec!["source/read"])).expect("provider");
    let traversal = dispatch_local(
        &[provider],
        "source/read",
        json!({"resourceId":"../secret"}),
        "notes",
        "alice",
        "tenant-a",
    )
    .await;
    assert!(traversal.is_err());
    let disabled = dispatch_local(
        &[],
        "source/read",
        json!({"resourceId":"a.md"}),
        "notes",
        "alice",
        "tenant-a",
    )
    .await;
    assert!(disabled.is_err());
    // Method not enabled on the endpoint config.
    let dir2 = tempfile::tempdir().expect("tempdir");
    let provider =
        EndpointProvider::new(&endpoint(dir2.path(), vec!["source/read"])).expect("provider");
    let wrong_method = dispatch_local(
        &[provider],
        "source/list",
        json!({"root":""}),
        "notes",
        "alice",
        "tenant-a",
    )
    .await;
    assert!(wrong_method.is_err());
}

#[test]
fn registration_payload_carries_per_endpoint_claims() {
    // The wire shape the runtime sends: endpoints with endpointId/root/methods.
    let payload = json!({
        "agentId": "agent-a",
        "endpoints": [{"endpointId": "notes-a", "root": "*", "methods": ["source/read"]}],
        "hostOrigin": "conex://host.local",
    });
    let endpoints = payload["endpoints"].as_array().expect("endpoints");
    assert_eq!(endpoints[0]["endpointId"], "notes-a");
    assert_eq!(endpoints[0]["root"], "*");
    // The wire round-trips through the same envelope the host parses.
    let request = conex_proto::Message {
        body: Some(conex_proto::message::Body::Request(conex_proto::Request {
            request_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            method: "agent/register".into(),
            params: Some(conex_proto::CallParams {
                context: Some(conex_proto::RequestContext {
                    provider_endpoint_id: "agent-mgr".into(),
                    plane: conex_proto::Plane::Broker as i32,
                    binding_id: None,
                    principal_id: String::new(),
                    tenant_id: String::new(),
                }),
                timeout_budget_ms: 8_000,
                input: Some(serde_json::from_value(payload).unwrap()),
            }),
        })),
    };
    let encoded = encode_wire(&request).expect("encode");
    let decoded = decode_wire(&encoded).expect("decode");
    match decoded.body.expect("body") {
        conex_proto::message::Body::Request(request) => {
            let input = request.params.expect("params").input.expect("input");
            let value = serde_json::to_value(&input).expect("json");
            assert_eq!(value["endpoints"][0]["endpointId"], "notes-a");
        }
        other => panic!("expected request, got {other:?}"),
    }
}

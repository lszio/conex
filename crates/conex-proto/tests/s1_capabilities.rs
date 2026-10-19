//! S1 共享向量：九个方法目录、input/result 边界、规范 uint64、limits 与
//! counter 域契约。Rust 与 TS（sdk/typescript/tests/s1_wire.test.ts）读同一份
//! conformance/vectors/s1/capabilities.json，两语言必须给出相同接受/拒绝。
#![allow(clippy::collapsible_if)]

use conex_proto::capability::{self, Boundary};
use conex_proto::wire;
use serde_json::Value;

fn vectors() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors/s1/capabilities.json"
    );
    let text =
        std::fs::read_to_string(path).expect("read conformance/vectors/s1/capabilities.json");
    serde_json::from_str(&text).expect("parse S1 capability vectors")
}

fn kind(message: &conex_proto::Message) -> &'static str {
    match message.body.as_ref() {
        Some(conex_proto::message::Body::Request(_)) => "request",
        Some(conex_proto::message::Body::Success(_)) => "success",
        Some(conex_proto::message::Body::Failure(_)) => "failure",
        Some(conex_proto::message::Body::Notification(_)) => "notification",
        Some(conex_proto::message::Body::DataChunk(_)) => "data_chunk",
        None => "none",
    }
}

fn request_id(message: &conex_proto::Message) -> Option<&str> {
    match message.body.as_ref() {
        Some(conex_proto::message::Body::Request(r)) => Some(r.request_id.as_str()),
        Some(conex_proto::message::Body::Success(s)) => Some(s.request_id.as_str()),
        Some(conex_proto::message::Body::Failure(f)) => f.request_id.as_deref(),
        _ => None,
    }
}

fn method(message: &conex_proto::Message) -> Option<&str> {
    match message.body.as_ref() {
        Some(conex_proto::message::Body::Request(r)) => Some(r.method.as_str()),
        Some(conex_proto::message::Body::Notification(n)) => Some(n.method.as_str()),
        _ => None,
    }
}

/// Decode and validate the complete frame, including correlated result semantics.
fn pipeline(case: &Value, boundary: Boundary) -> Result<conex_proto::Message, wire::ProtocolError> {
    let message = capability::decode_frame(case["wire"].as_str().unwrap().as_bytes(), boundary)?;
    if case["transport"] == "http" {
        capability::validate_http_frame(&message)?;
    }
    if let Some(conex_proto::message::Body::Success(s)) = message.body.as_ref() {
        if let Some(method) = case["expect"]["method"].as_str() {
            let result = serde_json::to_value(s.result.as_ref().unwrap()).unwrap();
            capability::validate_result(method, &result)?;
        }
    }
    Ok(message)
}

#[test]
fn shared_s1_vectors_accept_and_reject_identically() {
    let doc = vectors();
    let cases = doc["cases"].as_array().expect("cases");
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let expect = &case["expect"];
        let result = pipeline(case, Boundary::Client);
        if let Some(host_accept) = expect["hostAccept"].as_bool() {
            assert_eq!(
                pipeline(case, Boundary::Host).is_ok(),
                host_accept,
                "vector {name}: host boundary"
            );
        }
        match result {
            Ok(message) => {
                assert_eq!(
                    expect["accept"].as_bool(),
                    Some(true),
                    "vector {name}: expected accept but the client boundary rejected"
                );
                if let Some(k) = expect["kind"].as_str() {
                    assert_eq!(kind(&message), k, "vector {name}: kind");
                }
                if let Some(id) = expect["requestId"].as_str() {
                    assert_eq!(request_id(&message), Some(id), "vector {name}: requestId");
                }
                if let Some(m) = expect["method"].as_str() {
                    if kind(&message) != "success" {
                        assert_eq!(method(&message), Some(m), "vector {name}: method");
                    }
                }
            }
            Err(e) => {
                assert_ne!(
                    expect["accept"].as_bool(),
                    Some(true),
                    "vector {name}: client boundary unexpectedly rejected: {e:?}"
                );
                if let Some(code) = expect["errorCode"].as_i64() {
                    assert_eq!(e.rpc_code, code as i32, "vector {name}: reject code");
                }
            }
        }
    }
}

#[test]
fn s1_limits_vectors_reject_non_positive_subscription_caps() {
    let doc = vectors();
    for case in doc["limits"].as_array().expect("limits") {
        let name = case["name"].as_str().unwrap();
        let accept = case["accept"].as_bool().unwrap();
        let result = capability::limits_from_json(&case["json"]);
        assert_eq!(result.is_ok(), accept, "limits vector {name}: {result:?}");
    }
}

#[test]
fn counter_argument_vectors_match_generated_schema() {
    let doc = vectors();
    for case in doc["counterArguments"]
        .as_array()
        .expect("counterArguments")
    {
        let name = case["name"].as_str().unwrap();
        let schema = case["schema"].as_str().unwrap();
        let argument = case["argument"].clone();
        let accept = case["accept"].as_bool().unwrap();
        let result = capability::validate_domain_schema(schema, &argument);
        assert_eq!(result.is_ok(), accept, "counter vector {name}: {result:?}");
    }
}

#[test]
fn canonical_uint64_boundaries_match_the_contract() {
    let ok = [
        "0",
        "1",
        "18446744073709551615", // u64::MAX，超过 JS 安全整数
    ];
    let bad = [
        "",                     // 空
        "00",                   // 前导零非规范
        "007",                  // 前导零非规范
        "-1",                   // 负数
        "+1",                   // 符号
        "1.5",                  // 小数
        "18446744073709551616", // u64::MAX + 1
        "1e3",                  // 指数
    ];
    for s in ok {
        assert!(
            capability::is_canonical_uint64(s),
            "expected canonical: {s:?}"
        );
        assert!(s.parse::<u64>().is_ok(), "expected u64 parse: {s:?}");
    }
    for s in bad {
        assert!(
            !capability::is_canonical_uint64(s),
            "expected rejected: {s:?}"
        );
    }
}

#[test]
fn state_end_error_json_uses_enum_name_not_numeric_code_shape() {
    let error = conex_proto::Error {
        code: conex_proto::ErrorCode::SlowConsumer as i32,
        message: "queue full".into(),
        diagnostic_id: "d1".into(),
        execution: "completed".into(),
        retry: "safe".into(),
        details: None,
    };
    let json = capability::state_end_error_json(&error).expect("encode");
    let map = json.as_object().expect("error object");
    assert_eq!(
        map["code"],
        Value::String("ERROR_CODE_SLOW_CONSUMER".into())
    );
    assert_eq!(map["message"], Value::String("queue full".into()));
    // 与 JSON-RPC Failure 的 data 形状不同：没有嵌套 data/numeric code。
    assert!(map.get("data").is_none());

    let parsed = capability::parse_state_end_error(&json).expect("decode round trip");
    assert_eq!(parsed.code, conex_proto::ErrorCode::SlowConsumer as i32);

    // Proto3 defaults do not permit omitted subscription termination metadata.
    let minimal = serde_json::json!({
        "code": "ERROR_CODE_SESSION_LOST",
        "message": "ordering lost",
        "diagnosticId": "",
    });
    assert!(capability::parse_state_end_error(&minimal).is_err());

    // 数值 code（JSON-RPC Failure 形状）拒绝。
    let numeric = serde_json::json!({"code": -32015, "message": "x"});
    assert!(capability::parse_state_end_error(&numeric).is_err());

    // UNSPECIFIED 不是终止码。
    let unspecified = serde_json::json!({"code": "ERROR_CODE_UNSPECIFIED"});
    assert!(capability::parse_state_end_error(&unspecified).is_err());

    // 未知字段拒绝。
    let unknown = serde_json::json!({"code": "ERROR_CODE_TIMEOUT", "data": {"code": "timeout"}});
    assert!(capability::parse_state_end_error(&unknown).is_err());
}

#[test]
fn state_update_input_keeps_uint64_as_canonical_strings() {
    let sample = conex_proto::StateSample {
        revision: u64::MAX,
        value: Some(serde_json::from_value(serde_json::json!({"count": 1})).unwrap()),
        sampled_at_ms: 1_792_195_200_000,
        quality: "current".into(),
    };
    let json = capability::state_sample_json(&sample).expect("sample encode");
    assert_eq!(
        json["revision"],
        Value::String("18446744073709551615".into())
    );
    assert!(json["sampledAtMs"].is_string());

    let input = capability::state_update_input("01ARZ3NDEKTSV4RRFFQ69G5FAX", 1, &sample)
        .expect("update input builds and self-validates");
    assert_eq!(input["seq"], Value::String("1".into()));

    let decoded = capability::state_sample_from_json(&json).expect("sample decode");
    assert_eq!(decoded.revision, u64::MAX);
    assert_eq!(decoded.sampled_at_ms, 1_792_195_200_000);

    // 非规范 revision 不可反向解析。
    let bad = serde_json::json!({
        "revision": "007",
        "value": {},
        "sampledAtMs": "1",
        "quality": "current",
    });
    assert!(capability::state_sample_from_json(&bad).is_err());
}

#[test]
fn s1_context_rules_split_client_and_host_boundaries() {
    fn frame(principal: &str, tenant: &str) -> conex_proto::Message {
        use conex_proto::message::Body;
        conex_proto::Message {
            body: Some(Body::Request(conex_proto::Request {
                request_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                method: "module/invoke".into(),
                params: Some(conex_proto::CallParams {
                    context: Some(conex_proto::RequestContext {
                        provider_endpoint_id: "counter-rust".into(),
                        plane: conex_proto::Plane::Broker as i32,
                        binding_id: None,
                        principal_id: principal.into(),
                        tenant_id: tenant.into(),
                        space_id: "dev".into(),
                        registration_id: "01ARZ3NDEKTSV4RRFFQ69G5FAW".into(),
                    }),
                    timeout_budget_ms: 8000,
                    input: Some(
                        serde_json::from_value(serde_json::json!({
                            "method": "counter/read",
                            "argument": {},
                        }))
                        .unwrap(),
                    ),
                }),
            })),
        }
    }
    // Host 盖章身份：Host 边界接受，Client 边界拒绝。
    let stamped = frame("principal-1", "tenant-1");
    assert!(capability::validate_frame(&stamped, Boundary::Host).is_ok());
    assert!(capability::validate_frame(&stamped, Boundary::Client).is_err());
    // 无身份帧两个边界都接受。
    let plain = frame("", "");
    assert!(capability::validate_frame(&plain, Boundary::Host).is_ok());
    assert!(capability::validate_frame(&plain, Boundary::Client).is_ok());
}

#[test]
fn counter_arguments_survive_the_protobuf_value_boundary() {
    let doc = vectors();
    let invoke = doc["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "invoke_accept")
        .unwrap();
    let message = capability::decode_frame(
        invoke["wire"].as_str().unwrap().as_bytes(),
        Boundary::Client,
    )
    .unwrap();
    let encoded = wire::encode_wire(&message).unwrap();
    let forwarded: Value = serde_json::from_slice(&encoded).unwrap();
    let argument: conex_proto::CounterAddInput =
        serde_json::from_value(forwarded["params"]["input"]["argument"].clone()).unwrap();
    assert_eq!(argument.delta, 1);
    let Some(conex_proto::message::Body::Request(request)) = message.body else {
        panic!("invoke request")
    };
    let input = serde_json::to_value(request.params.unwrap().input.unwrap()).unwrap();
    capability::validate_domain_schema("conex.CounterAddInput", &input["argument"]).unwrap();
    let overflow: pbjson_types::Value =
        serde_json::from_value(serde_json::json!({"delta": 2147483648u64})).unwrap();
    let overflow = serde_json::to_value(overflow).unwrap();
    assert!(capability::validate_domain_schema("conex.CounterAddInput", &overflow).is_err());
}

#[test]
fn wire_values_preserve_fractional_numbers_and_uint64_strings() {
    let payload = serde_json::json!({
        "nested": [3, -2, 1.25, "18446744073709551615"],
        "safe": 9007199254740991i64,
        "unsafe": 9007199254740992.0,
    });
    let message = conex_proto::Message {
        body: Some(conex_proto::message::Body::Success(conex_proto::Success {
            request_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            result: Some(serde_json::from_value(payload).unwrap()),
        })),
    };
    let encoded = wire::encode_wire(&message).unwrap();
    let forwarded: Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(forwarded["result"]["nested"][0].as_i64(), Some(3));
    assert_eq!(forwarded["result"]["nested"][1].as_i64(), Some(-2));
    assert_eq!(forwarded["result"]["nested"][2].as_f64(), Some(1.25));
    assert_eq!(forwarded["result"]["nested"][3], "18446744073709551615");
    assert_eq!(forwarded["result"]["safe"].as_i64(), Some(9007199254740991));
    assert!(forwarded["result"]["unsafe"].is_f64());
}

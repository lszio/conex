//! S1 模块注册/撤销/发现与代次 fence（docs/plans/2026-10-17-conex-s1.md T02；
//! docs/contracts/s1-capabilities.md §4.2、§6）。
//!
//! 测试层级：`Broker::invoke`（p1_e2e 同级的真实宿主 API，代次经
//! `BrokerCall.generation` 传入，模拟两条 WSS 连接），外加对注册表与 wire
//! 边界的直接测试。完整双 socket/M2 staged 传输路径属于 T08 跨进程验收，
//! 本文件不重复搭 TLS/WSS 支架。
#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::Instant;

use conex_core::capability::{
    ApprovalSet, CapabilityMethod, CapabilityState, ConsumeApproval, ContractDescription,
    ContractRegistry, ExecutionClass, PublishApproval,
};
use conex_core::{CallError, Caller, Limits};
use conex_host::broker::{Broker, BrokerCall, BrokerDeps};
use conex_host::capability::{HandlerDeclaration, ModuleError, ModuleRegistry, ModuleRequest};

fn caller(principal: &str) -> Caller {
    Caller {
        principal_id: principal.into(),
        tenant_id: "tenant-a".into(),
        actor_peer_id: "test".into(),
    }
}

fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

fn counter_contract() -> ContractDescription {
    ContractDescription {
        contract_id: "conex.counter".into(),
        methods: vec![
            CapabilityMethod {
                name: "counter/read".into(),
                input_schema: "conex.CounterReadInput".into(),
                output_schema: "conex.CounterResult".into(),
                action: "counter.read".into(),
                execution: ExecutionClass::ReadOnly,
            },
            CapabilityMethod {
                name: "counter/add".into(),
                input_schema: "conex.CounterAddInput".into(),
                output_schema: "conex.CounterResult".into(),
                action: "counter.add".into(),
                execution: ExecutionClass::NonReplayable,
            },
        ],
        states: vec![CapabilityState {
            name: "counter/value".into(),
            value_schema: "conex.CounterStateValue".into(),
            action: "counter.value".into(),
        }],
    }
}

fn publish(node: &str, endpoint: &str) -> PublishApproval {
    PublishApproval {
        node_id: node.into(),
        space_id: "dev".into(),
        integration_id: "integration-a".into(),
        provider_endpoint_id: endpoint.into(),
        contract_id: "conex.counter".into(),
    }
}

fn consume(principal: &str, endpoint: &str) -> ConsumeApproval {
    ConsumeApproval {
        principal_id: principal.into(),
        space_id: "dev".into(),
        provider_endpoint_id: endpoint.into(),
        contract_id: "conex.counter".into(),
        methods: None,
        states: None,
    }
}

/// 标准装配：node-a 与 node-b 各一个批准 endpoint；alice/bob 有消费批准，
/// mallory 没有。
fn registry() -> Arc<ModuleRegistry> {
    let contracts = Arc::new(ContractRegistry::new());
    contracts
        .register(counter_contract())
        .expect("fixture contract");
    let approvals = ApprovalSet::new(
        vec![
            publish("node-a", "counter-ts"),
            publish("node-b", "counter-rust"),
        ],
        vec![
            ConsumeApproval {
                methods: Some(vec!["counter/read".into()]),
                states: None,
                ..consume("bob", "counter-ts")
            },
            consume("alice", "counter-ts"),
            consume("alice", "counter-rust"),
        ],
    );
    approvals
        .validate_contracts(&contracts)
        .expect("fixture approvals");
    approvals.validate_keys().expect("fixture approvals");
    ModuleRegistry::new(contracts, Arc::new(approvals))
}

fn broker(registry: Option<Arc<ModuleRegistry>>) -> Arc<Broker> {
    let host = Arc::new(
        conex_core::Host::new(
            conex_core::Registry::new(),
            Arc::new(conex_core::StaticPolicy::new(Vec::new())),
            Arc::new(conex_core::TargetPolicy::new()),
            Arc::new(conex_transport_http::TokioResolver),
            Arc::new(
                conex_transport_http::HttpConnector::new(
                    conex_transport_http::TlsTrustConfig::default(),
                    1024 * 1024,
                )
                .unwrap(),
            ),
            Arc::new(conex_host::credentials::EnvFileStore::new(Vec::new()).unwrap()),
            Arc::new(conex_host::audit_file::NullAudit),
            conex_core::HostLimits::default(),
        )
        .unwrap(),
    );
    Arc::new(Broker::new(
        host,
        BrokerDeps {
            content: None,
            session: None,
            operation: None,
            agents: None,
            catalog: None,
            host_origin: None,
            guest_principal: None,
            clients: None,
            remote: None,
            capabilities: registry.clone(),
            calls: registry
                .as_ref()
                .map(|registry| conex_host::capability::CallRouter::new(registry.clone())),
            state: None,
        },
    ))
}

async fn invoke(broker: &Broker, call: BrokerCall) -> Result<serde_json::Value, CallError> {
    broker.invoke(call).await
}

fn register_call(who: &Caller, generation: u64, endpoint: &str, contract: &str) -> BrokerCall {
    BrokerCall {
        caller: who.clone(),
        endpoint_id: String::new(),
        method: "module/register".into(),
        input: json!({
            "spaceId": "dev",
            "integrationId": "integration-a",
            "providerEndpointId": endpoint,
            "contractId": contract,
        }),
        deadline: deadline(),
        role: "service".into(),
        link_id: None,
        generation,
        max_frame_bytes: Limits::default().max_frame_bytes,
        request_id: String::new(),
        binding_id: None,
        space_id: String::new(),
        registration_id: String::new(),
    }
}

fn unregister_call(
    who: &Caller,
    generation: u64,
    endpoint: &str,
    registration_id: &str,
) -> BrokerCall {
    BrokerCall {
        caller: who.clone(),
        endpoint_id: String::new(),
        method: "module/unregister".into(),
        input: json!({
            "providerEndpointId": endpoint,
            "registrationId": registration_id,
        }),
        deadline: deadline(),
        role: "service".into(),
        link_id: None,
        generation,
        max_frame_bytes: Limits::default().max_frame_bytes,
        request_id: String::new(),
        binding_id: None,
        space_id: String::new(),
        registration_id: String::new(),
    }
}

fn list_call(who: &Caller, space: &str, max_frame_bytes: u32) -> BrokerCall {
    BrokerCall {
        caller: who.clone(),
        endpoint_id: String::new(),
        method: "module/list".into(),
        input: json!({ "spaceId": space }),
        deadline: deadline(),
        role: "service".into(),
        link_id: None,
        generation: 0,
        max_frame_bytes,
        request_id: String::new(),
        binding_id: None,
        space_id: String::new(),
        registration_id: String::new(),
    }
}

async fn list_modules(broker: &Broker, who: &Caller, space: &str) -> Vec<serde_json::Value> {
    invoke(broker, list_call(who, space, 1024 * 1024))
        .await
        .unwrap()["modules"]
        .as_array()
        .unwrap()
        .clone()
}

async fn register_ok(broker: &Broker, who: &Caller, generation: u64, endpoint: &str) -> String {
    invoke(
        broker,
        register_call(who, generation, endpoint, "conex.counter"),
    )
    .await
    .unwrap()["registrationId"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn module_methods_require_a_configured_registry() {
    let broker = broker(None);
    let error = invoke(
        &broker,
        register_call(&caller("node-a"), 11, "counter-ts", "conex.counter"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Unavailable));
}

#[tokio::test]
async fn unapproved_module_is_invisible_and_unregisterable() {
    let broker = broker(Some(registry()));
    // 未声明发布批准的 Node → forbidden，不泄漏。
    let error = invoke(
        &broker,
        register_call(&caller("node-x"), 11, "counter-any", "conex.counter"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));

    // 已批准 node-a 注册成功，registrationId 是 ULID。
    let result = invoke(
        &broker,
        register_call(&caller("node-a"), 11, "counter-ts", "conex.counter"),
    )
    .await
    .unwrap();
    let registration_id = result["registrationId"].as_str().unwrap().to_string();
    assert!(
        conex_proto::wire::is_ulid(&registration_id),
        "registrationId must be a ULID"
    );

    // 无权消费者看不到该模块。
    let modules = list_modules(&broker, &caller("mallory"), "dev").await;
    assert!(modules.is_empty());

    // 无权主体撤销 → forbidden（不泄漏注册是否存在）。
    let error = invoke(
        &broker,
        unregister_call(&caller("mallory"), 11, "counter-ts", &registration_id),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));
    // 他 Node 也不能撤销。
    let error = invoke(
        &broker,
        unregister_call(&caller("node-b"), 11, "counter-ts", &registration_id),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));
    // 注册仍在。
    assert_eq!(
        list_modules(&broker, &caller("alice"), "dev").await.len(),
        1
    );
}

#[tokio::test]
async fn bad_registration_does_not_kill_a_healthy_link() {
    let broker = broker(Some(registry()));
    let old_id = register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;

    // 新连接（代次 12）坏注册：未知契约。
    let error = invoke(
        &broker,
        register_call(&caller("node-a"), 12, "counter-extra", "conex.missing"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::BadRequest));
    // 新连接坏注册：未批准端点。
    let error = invoke(
        &broker,
        register_call(&caller("node-a"), 12, "counter-extra", "conex.counter"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));

    // 健康旧连接不受影响：module 仍活动，旧代次仍可撤销。
    let modules = list_modules(&broker, &caller("alice"), "dev").await;
    assert_eq!(modules[0]["registrationId"], old_id);
    let removed = invoke(
        &broker,
        unregister_call(&caller("node-a"), 11, "counter-ts", &old_id),
    )
    .await
    .unwrap();
    assert_eq!(removed["removed"], json!(true));
}

#[tokio::test]
async fn new_registration_takes_over_only_after_success() {
    let broker = broker(Some(registry()));
    let old_id = register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;

    // 新连接代次 12 先失败：不接管。
    invoke(
        &broker,
        register_call(&caller("node-a"), 12, "counter-extra", "conex.missing"),
    )
    .await
    .unwrap_err();

    // 同代次 11 重复注册同一端点 → conflict。
    let error = invoke(
        &broker,
        register_call(&caller("node-a"), 11, "counter-ts", "conex.counter"),
    )
    .await
    .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Conflict));

    // 代次 12 首个成功注册同一端点 → 原子激活，旧发布被替换。
    let new_id = register_ok(&broker, &caller("node-a"), 12, "counter-ts").await;
    assert_ne!(new_id, old_id);

    let modules = list_modules(&broker, &caller("alice"), "dev").await;
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0]["registrationId"], new_id);

    // 旧 registrationId 撤销 → stale_revision，且不删除新注册。
    let error = invoke(
        &broker,
        unregister_call(&caller("node-a"), 11, "counter-ts", &old_id),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::StaleRevision)
    );
    let modules = list_modules(&broker, &caller("alice"), "dev").await;
    assert_eq!(modules[0]["registrationId"], new_id);
}

#[tokio::test]
async fn old_generation_late_cleanup_does_not_delete_new_registration() {
    let registry = registry();
    let broker = broker(Some(registry.clone()));
    register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;
    let new_id = register_ok(&broker, &caller("node-a"), 12, "counter-ts").await;

    // 旧代次 11 迟到注册 → stale_revision，不覆盖新代次。
    let error = invoke(
        &broker,
        register_call(&caller("node-a"), 11, "counter-ts", "conex.counter"),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::StaleRevision)
    );

    // 旧代次 11 断线清理：不得删除新注册。
    registry.disconnect("node-a", 11);
    let modules = list_modules(&broker, &caller("alice"), "dev").await;
    assert_eq!(modules[0]["registrationId"], new_id);

    // 新代次 12 断线：注册随连接清理。
    registry.disconnect("node-a", 12);
    assert!(
        list_modules(&broker, &caller("alice"), "dev")
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn unauthorized_caller_gets_zero_forwarding_and_projection_is_limited() {
    let broker = broker(Some(registry()));
    register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;

    // 无消费批准的 mallory：零转发（列表为空）。
    assert!(
        list_modules(&broker, &caller("mallory"), "dev")
            .await
            .is_empty()
    );

    // bob 只被批准 counter/read：列表只投影该成员。
    let modules = list_modules(&broker, &caller("bob"), "dev").await;
    assert_eq!(modules[0]["methods"], json!(["counter/read"]));

    // alice 完整批准：整个契约。
    let modules = list_modules(&broker, &caller("alice"), "dev").await;
    let methods = modules[0]["methods"].as_array().unwrap();
    assert!(methods.iter().any(|name| name == "counter/read"));
    assert!(methods.iter().any(|name| name == "counter/add"));
}

#[tokio::test]
async fn other_space_modules_are_not_listed() {
    let broker = broker(Some(registry()));
    register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;
    // 其他 Space 列表为空。
    assert!(
        list_modules(&broker, &caller("alice"), "prod")
            .await
            .is_empty()
    );
    // 本 Space 仍有。
    assert_eq!(
        list_modules(&broker, &caller("alice"), "dev").await.len(),
        1
    );
}

#[tokio::test]
async fn list_frame_overflow_is_rejected_explicitly() {
    let broker = broker(Some(registry()));
    register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;
    // 帧预算小于编码结果：显式 payload_too_large，绝不静默截断。
    let error = invoke(&broker, list_call(&caller("alice"), "dev", 64))
        .await
        .unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::PayloadTooLarge)
    );
}

#[tokio::test]
async fn t03_wired_and_t04_unsupported_methods_fail_explicitly() {
    let broker = broker(Some(registry()));
    let call = BrokerCall {
        caller: caller("alice"),
        endpoint_id: String::new(),
        method: "module/invoke".into(),
        input: json!({}),
        deadline: deadline(),
        role: "service".into(),
        link_id: None,
        generation: 5,
        request_id: "01ARZ3NDEKTSV4RRFFQ69G5FB0".into(),
        binding_id: None,
        space_id: String::new(),
        registration_id: String::new(),
        max_frame_bytes: Limits::default().max_frame_bytes,
    };
    let error = invoke(&broker, call).await.unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::Forbidden),
        "module/invoke is wired (T03) and resolves the trusted registry first"
    );
    let call = BrokerCall {
        caller: caller("alice"),
        endpoint_id: String::new(),
        method: "module/cancel".into(),
        input: json!({}),
        deadline: deadline(),
        role: "service".into(),
        link_id: None,
        generation: 5,
        request_id: "01ARZ3NDEKTSV4RRFFQ69G5FB1".into(),
        binding_id: None,
        space_id: String::new(),
        registration_id: String::new(),
        max_frame_bytes: Limits::default().max_frame_bytes,
    };
    let error = invoke(&broker, call).await.unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::BadRequest),
        "module/cancel requires the original invoke requestId"
    );
    let call = BrokerCall {
        caller: caller("alice"),
        endpoint_id: String::new(),
        method: "state/subscribe".into(),
        input: json!({}),
        deadline: deadline(),
        role: "service".into(),
        link_id: None,
        generation: 0,
        request_id: String::new(),
        binding_id: None,
        space_id: String::new(),
        registration_id: String::new(),
        max_frame_bytes: Limits::default().max_frame_bytes,
    };
    let error = invoke(&broker, call).await.unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::UnsupportedCapability)
    );
}

#[test]
fn registry_validates_handler_declarations() {
    let registry = registry();
    // 部分声明：契约有 counter/read、counter/add 与 counter/value。
    let incomplete = HandlerDeclaration {
        methods: vec!["counter/read".into()],
        states: vec![],
    };
    let request = ModuleRequest {
        space_id: "dev".into(),
        integration_id: "integration-a".into(),
        provider_endpoint_id: "counter-ts".into(),
        contract_id: "conex.counter".into(),
    };
    let error = registry
        .register("node-a", 11, request.clone(), &incomplete)
        .unwrap_err();
    assert!(matches!(error, ModuleError::HandlerMissing(_)));
    // 完整声明成功。
    let handlers = registry.handler_declaration("conex.counter").unwrap();
    registry.register("node-a", 11, request, &handlers).unwrap();
}

#[test]
fn node_module_quota_is_bounded() {
    let contracts = Arc::new(ContractRegistry::new());
    contracts.register(counter_contract()).unwrap();
    let mut approvals = Vec::new();
    for index in 0..conex_host::capability::DEFAULT_MAX_MODULES {
        approvals.push(PublishApproval {
            node_id: "node-a".into(),
            space_id: "dev".into(),
            integration_id: "integration-a".into(),
            provider_endpoint_id: format!("counter-{index}"),
            contract_id: "conex.counter".into(),
        });
    }
    // 超限端点也必须已批准，quota 才是真正的拒绝原因。
    approvals.push(PublishApproval {
        node_id: "node-a".into(),
        space_id: "dev".into(),
        integration_id: "integration-a".into(),
        provider_endpoint_id: "counter-overflow".into(),
        contract_id: "conex.counter".into(),
    });
    let approvals = ApprovalSet::new(approvals, Vec::new());
    approvals.validate_contracts(&contracts).unwrap();
    approvals.validate_keys().unwrap();
    let registry = ModuleRegistry::new(contracts, Arc::new(approvals));
    let handlers = registry.handler_declaration("conex.counter").unwrap();
    for index in 0..conex_host::capability::DEFAULT_MAX_MODULES {
        registry
            .register(
                "node-a",
                11,
                ModuleRequest {
                    space_id: "dev".into(),
                    integration_id: "integration-a".into(),
                    provider_endpoint_id: format!("counter-{index}"),
                    contract_id: "conex.counter".into(),
                },
                &handlers,
            )
            .unwrap();
    }
    let error = registry
        .register(
            "node-a",
            11,
            ModuleRequest {
                space_id: "dev".into(),
                integration_id: "integration-a".into(),
                provider_endpoint_id: "counter-overflow".into(),
                contract_id: "conex.counter".into(),
            },
            &handlers,
        )
        .unwrap_err();
    assert!(matches!(error, ModuleError::QuotaExceeded(_)));
}

#[test]
fn sdk_shaped_frames_pass_the_client_boundary_and_forged_identity_is_rejected() {
    // SDK（crates/conex-sdk）构造的注册/列表帧必须通过中心 Client 边界
    // 校验：context 只含 plane，预算为正，input 完整；客户端不得携带身份。
    let register = r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","method":"module/register","params":{"context":{"plane":"broker"},"timeoutBudgetMs":8000,"input":{"spaceId":"dev","integrationId":"integration-a","providerEndpointId":"counter-ts","contractId":"conex.counter"}}}"#;
    decode_client_frame(register.as_bytes()).expect("register frame passes the client boundary");

    let list = r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FAW","method":"module/list","params":{"context":{"plane":"broker"},"timeoutBudgetMs":8000,"input":{"spaceId":"dev"}}}"#;
    decode_client_frame(list.as_bytes()).expect("list frame passes the client boundary");

    // 客户端自报身份 → bad_request，不因字段存在就信任。
    let forged = r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FAX","method":"module/register","params":{"context":{"plane":"broker","principalId":"alice"},"timeoutBudgetMs":8000,"input":{"spaceId":"dev","integrationId":"integration-a","providerEndpointId":"counter-ts","contractId":"conex.counter"}}}"#;
    let error = decode_client_frame(forged.as_bytes()).unwrap_err();
    assert_eq!(
        conex_proto::ErrorCode::try_from(error.rpc_code),
        Ok(conex_proto::ErrorCode::BadRequest)
    );
}

/// 与 ws_transport JSON 入站路径相同的边界解码（Boundary::Client）。
fn decode_client_frame(
    bytes: &[u8],
) -> Result<conex_proto::Message, conex_proto::wire::ProtocolError> {
    conex_proto::capability::decode_frame(bytes, conex_proto::capability::Boundary::Client)
}

fn p1_roots() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for name in ["content", "session", "operation"] {
        std::fs::create_dir_all(root.join(name)).unwrap();
    }
    dir
}

fn capability_toml(dir: &tempfile::TempDir) -> String {
    let root = dir.path().display();
    format!(
        r#"
listen = "127.0.0.1:0"
allow_loopback_http = true
audience = "host.local"
content_root = "{root}/content"
session_root = "{root}/session"
operation_root = "{root}/operation"

[[tokens]]
token_hash = "{}"
principal_id = "node-a"
tenant_id = "tenant-a"
audience = "host.local"
role = "service"

[capabilities]

[[capabilities.contracts]]
contract_id = "conex.counter"
[[capabilities.contracts.methods]]
name = "counter/read"
input_schema = "conex.CounterReadInput"
output_schema = "conex.CounterResult"
action = "counter.read"
execution = "read_only"
[[capabilities.contracts.methods]]
name = "counter/add"
input_schema = "conex.CounterAddInput"
output_schema = "conex.CounterResult"
action = "counter.add"
execution = "non_replayable"
[[capabilities.contracts.states]]
name = "counter/value"
value_schema = "conex.CounterStateValue"
action = "counter.value"

[[capabilities.publish]]
node_id = "node-a"
space_id = "dev"
integration_id = "integration-a"
provider_endpoint_id = "counter-ts"
contract_id = "conex.counter"

[[capabilities.consume]]
principal_id = "node-a"
space_id = "dev"
provider_endpoint_id = "counter-ts"
contract_id = "conex.counter"
"#,
        "0".repeat(64),
        root = root
    )
}

#[test]
fn static_approvals_validate_at_config_load() {
    // 合法装配：契约 + 批准装载通过，主机带 S1 能力注册表即可构建。
    let dir = p1_roots();
    let path = dir.path().join("host.toml");
    std::fs::write(&path, capability_toml(&dir)).unwrap();
    let config = conex_host::config::HostConfig::load(&path).unwrap();
    let built = conex_host::serve::build(&config).unwrap();
    assert!(built.broker.unwrap().deps().capabilities.is_some());

    // 消费批准引用未安装契约 → 装载拒绝（只改 consume 段的 contract_id）。
    let consume_block = "[[capabilities.consume]]\nprincipal_id = \"node-a\"\nspace_id = \"dev\"\nprovider_endpoint_id = \"counter-ts\"\ncontract_id = \"conex.counter\"";
    let bad = capability_toml(&dir).replace(
        consume_block,
        "[[capabilities.consume]]\nprincipal_id = \"node-a\"\nspace_id = \"dev\"\nprovider_endpoint_id = \"counter-ts\"\ncontract_id = \"conex.missing\"",
    );
    assert_ne!(bad, capability_toml(&dir), "consume block must be replaced");
    register_rejected(&bad);
}

#[test]
fn unknown_execution_class_is_rejected_at_config_load() {
    let dir = p1_roots();
    let good = capability_toml(&dir);
    let bad = good.replace("execution = \"read_only\"", "execution = \"big_impact\"");
    assert_ne!(bad, good, "fixture must contain the replaced line");
    register_rejected(&bad);
    // 也验证断言的装载文件用了精确路径。
    let bad_dup = good.replace(
        "execution = \"non_replayable\"",
        "execution = \"read_only\"",
    );
    assert_ne!(bad_dup, good);
}

fn register_rejected(toml: &str) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("host.toml");
    std::fs::write(&path, toml).unwrap();
    let config = conex_host::config::HostConfig::load(&path).unwrap();
    let error = match conex_host::serve::build(&config) {
        Ok(_) => panic!("config must be rejected at load"),
        Err(error) => error,
    };
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::Internal),
        "config load rejection is reported as a config error: {}",
        error.message()
    );
}

#[tokio::test]
async fn sdk_bootstrap_negotiates_authenticated_identity_and_enforces_lower_frame_limit() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let state = conex_host::ws_transport::WssState::new_with_capabilities(
        broker(Some(registry())),
        Limits::default(),
        caller("node-a"),
        vec!["agent/heartbeat".into()],
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = axum::Router::new()
        .route(
            "/wss",
            axum::routing::get(conex_host::ws_transport::ws_handler),
        )
        .with_state(Arc::new(state));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/wss"))
        .await
        .unwrap();
    let provides = [
        "module/invoke",
        "module/cancel",
        "state/subscribe",
        "state/unsubscribe",
    ];
    let hello = conex_proto::HelloRequest {
        profile_id: "conex-jsonrpc2-wss".into(),
        plane: conex_proto::Plane::Broker as i32,
        provides: provides.iter().map(|method| (*method).to_owned()).collect(),
        requires: vec!["module/list".into()],
    };
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0", "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "method": "conex/hello",
            "params": {"context": {"plane": "broker"}, "timeoutBudgetMs": 8000, "input": hello}
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reply: serde_json::Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    let hello: conex_proto::HelloResponse = serde_json::from_value(reply["result"].clone())
        .unwrap_or_else(|error| panic!("generated hello rejected {reply}: {error}"));
    assert!(conex_proto::wire::is_ulid(&hello.binding_id));
    let mut limits = hello.limits.unwrap();
    assert!(limits.max_subscriptions > 0 && limits.max_modules > 0);
    limits.max_frame_bytes = 1024;
    limits.max_inflight = 2;
    limits.max_queued_bytes = 4096;
    limits.timeout_ms = 1000;
    limits.max_subscriptions = 1;
    limits.max_modules = 1;
    let ready = conex_proto::ReadyRequest {
        negotiation_id: hello.binding_id.clone(),
        profile_id: hello.profile_id,
        plane: hello.plane,
        provides: provides.iter().map(|method| (*method).to_owned()).collect(),
        limits: Some(limits),
    };
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0", "id": "01ARZ3NDEKTSV4RRFFQ69G5FAW", "method": "conex/ready",
            "params": {"context": {"plane": "broker"}, "timeoutBudgetMs": 8000, "input": ready}
        })
        .to_string()
        .into(),
    ))
    .await
    .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reply: serde_json::Value = serde_json::from_str(reply.to_text().unwrap()).unwrap();
    let ready: conex_proto::ReadyResponse = serde_json::from_value(reply["result"].clone())
        .unwrap_or_else(|error| panic!("generated ready rejected {reply}: {error}"));
    assert_eq!(ready.limits, Some(limits));
    assert_eq!(ready.negotiation_id, hello.binding_id);
    let identity = ready.link_identity.unwrap();
    assert!(conex_proto::wire::is_ulid(&identity.link_id));
    assert_eq!(identity.peer_id, "node-a");
    assert_eq!(identity.tenant_id, "tenant-a");
    ws.send(Message::Text(" ".repeat(1025).into()))
        .await
        .unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(2), ws.next())
        .await
        .unwrap();
    match reply {
        Some(Ok(Message::Text(text))) => {
            let reply: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert!(
                reply["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("negotiated limit")
            );
        }
        // The WebSocket transport may reject the frame before application
        // dispatch; either way, oversized business input cannot be accepted.
        Some(Ok(Message::Close(_))) | None => {}
        Some(Err(tokio_tungstenite::tungstenite::Error::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        ))) => {}
        other => panic!("oversized frame was not rejected: {other:?}"),
    }
    server.abort();
}

#[tokio::test]
async fn provider_socket_errors_preserve_metadata_to_consumer_for_both_profiles() {
    use futures::SinkExt;
    use prost::Message as _;
    use tokio_tungstenite::tungstenite::Message;
    type Socket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn receive(socket: &mut Socket) -> serde_json::Value {
        use futures::StreamExt;
        use prost::Message as _;
        let reply = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let bytes = match reply {
            Message::Text(text) => text.as_bytes().to_vec(),
            Message::Binary(bytes) => {
                conex_proto::wire::encode_wire(&conex_proto::Message::decode(bytes).unwrap())
                    .unwrap()
            }
            other => panic!("unexpected frame {other:?}"),
        };
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn connect(url: &str, profile: &str) -> Socket {
        use futures::SinkExt;
        let (mut socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        socket.send(Message::Text(json!({
            "jsonrpc": "2.0", "id": "hello", "method": "conex/hello",
            "params": {"profileId": profile, "plane": "broker", "provides": [], "requires": []}
        }).to_string().into())).await.unwrap();
        let hello = receive(&mut socket).await;
        socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0", "id": "ready", "method": "conex/ready",
                    "params": {
                        "profileId": profile, "plane": "broker",
                        "negotiationId": hello["result"]["negotiationId"],
                        "provides": hello["result"]["provides"], "limits": hello["result"]["limits"]
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        assert!(receive(&mut socket).await.get("result").is_some());
        socket
    }

    for profile in ["conex-jsonrpc2-wss", "conex-protobuf-wss"] {
        let broker = broker(Some(registry()));
        let provider = Arc::new(conex_host::ws_transport::WssState::new(
            broker.clone(),
            Limits::default(),
            caller("node-a"),
        ));
        let consumer = Arc::new(conex_host::ws_transport::WssState::new(
            broker,
            Limits::default(),
            caller("alice"),
        ));
        let app = axum::Router::new()
            .route(
                "/provider",
                axum::routing::get(conex_host::ws_transport::ws_handler).with_state(provider),
            )
            .route(
                "/consumer",
                axum::routing::get(conex_host::ws_transport::ws_handler).with_state(consumer),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut provider = connect(&format!("ws://{addr}/provider"), profile).await;
        let mut consumer = connect(&format!("ws://{addr}/consumer"), "conex-jsonrpc2-wss").await;
        let register = json!({
            "jsonrpc": "2.0", "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV", "method": "module/register",
            "params": {"context": {"plane": "broker"}, "timeoutBudgetMs": 8000, "input": {
                "spaceId": "dev", "integrationId": "integration-a",
                "providerEndpointId": "counter-ts", "contractId": "conex.counter"
            }}
        });
        let encode = |value: &serde_json::Value| {
            if profile == "conex-protobuf-wss" {
                Message::Binary(
                    conex_proto::wire::decode_wire(value.to_string().as_bytes())
                        .unwrap()
                        .encode_to_vec()
                        .into(),
                )
            } else {
                Message::Text(value.to_string().into())
            }
        };
        provider.send(encode(&register)).await.unwrap();
        let registered = receive(&mut provider).await;
        let registration = registered["result"]["registrationId"]
            .as_str()
            .unwrap_or_else(|| panic!("registration failed: {registered}"));
        let mut invoke = json!({
            "jsonrpc": "2.0", "id": "01ARZ3NDEKTSV4RRFFQ69G5FAW", "method": "module/invoke",
            "params": {"context": {
                "plane": "broker", "providerEndpointId": "counter-ts",
                "spaceId": "dev", "registrationId": registration
            }, "timeoutBudgetMs": 8000, "input": {"method": "counter/add", "argument": {"delta": 3}}}
        });
        consumer
            .send(Message::Text(invoke.to_string().into()))
            .await
            .unwrap();
        let forwarded = receive(&mut provider).await;
        assert_eq!(forwarded["method"], "module/invoke");
        let expected = json!({
            "code": conex_proto::ErrorCode::BadRequest as i32, "message": "provider rejected before mutation",
            "data": {
                "code": "bad_request", "diagnosticId": "provider-diagnostic",
                "execution": "not_started", "retry": "never",
                "details": {"reason": "overflow"}
            }
        });
        provider
            .send(encode(&json!({
                "jsonrpc": "2.0", "id": forwarded["id"], "error": expected
            })))
            .await
            .unwrap();
        let received = receive(&mut consumer).await;
        assert_eq!(received["id"], "01ARZ3NDEKTSV4RRFFQ69G5FAW");
        assert_eq!(received["error"], expected, "{profile}: {received}");
        // A code/message alone cannot establish that a dispatched mutation
        // did not start. Preserve diagnostics but settle the outcome unknown.
        invoke["id"] = json!("01ARZ3NDEKTSV4RRFFQ69G5FAX");
        consumer
            .send(Message::Text(invoke.to_string().into()))
            .await
            .unwrap();
        let forwarded = receive(&mut provider).await;
        let mut incomplete = expected.clone();
        incomplete["data"]
            .as_object_mut()
            .unwrap()
            .remove("execution");
        incomplete["data"].as_object_mut().unwrap().remove("retry");
        provider
            .send(encode(&json!({
                "jsonrpc": "2.0", "id": forwarded["id"], "error": incomplete
            })))
            .await
            .unwrap();
        let received = receive(&mut consumer).await;
        assert_eq!(
            received["error"]["code"],
            conex_proto::ErrorCode::OutcomeUnknown as i32
        );
        assert_eq!(received["error"]["data"]["execution"], "unknown");
        assert_eq!(received["error"]["data"]["retry"], "never");
        assert_eq!(
            received["error"]["data"]["diagnosticId"],
            "provider-diagnostic"
        );
        assert_eq!(
            received["error"]["data"]["details"],
            expected["data"]["details"]
        );
        provider.close(None).await.unwrap();
        consumer.close(None).await.unwrap();
        server.abort();
    }
}

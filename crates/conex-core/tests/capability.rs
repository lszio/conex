//! S1 预安装契约与静态批准行为（docs/plans/2026-10-17-conex-s1.md T02）。
//!
//! 只覆盖行为边界：描述校验、注册表去重/查询、批准默认 deny、可见性投影、
//! 键唯一性与未安装契约引用。不检测源码文本。
use conex_core::capability::{
    ApprovalSet, CapabilityMethod, CapabilityState, ConsumeApproval, ContractDescription,
    ContractError, ContractRegistry, ExecutionClass, PublishApproval, is_valid_identifier,
    is_valid_method_name,
};

fn method(name: &str, execution: ExecutionClass) -> CapabilityMethod {
    CapabilityMethod {
        name: name.into(),
        input_schema: format!("conex.Test{name}Input"),
        output_schema: format!("conex.Test{name}Result"),
        action: "test.run".into(),
        execution,
    }
}

fn state(name: &str) -> CapabilityState {
    CapabilityState {
        name: name.into(),
        value_schema: format!("conex.Test{name}Value"),
        action: "test.watch".into(),
    }
}

/// 预安装 counter 类契约：一个 read_only 方法、一个 non_replayable 方法与
/// 一个状态源（T07 域实现接入同一形状）。
fn counter_contract() -> ContractDescription {
    ContractDescription {
        contract_id: "conex.counter".into(),
        methods: vec![
            method("counter/read", ExecutionClass::ReadOnly),
            method("counter/add", ExecutionClass::NonReplayable),
        ],
        states: vec![state("counter/value")],
    }
}

fn register(contract: ContractDescription) -> ContractRegistry {
    let registry = ContractRegistry::new();
    registry
        .register(contract)
        .expect("fixture contract is valid");
    registry
}

#[test]
fn valid_identifiers_and_method_names() {
    assert!(is_valid_identifier("conex.counter"));
    assert!(is_valid_identifier("node-a_1"));
    assert!(!is_valid_identifier(""));
    assert!(!is_valid_identifier("has space"));
    assert!(!is_valid_identifier("slash/name")); // identifier 不允许 /
    assert!(is_valid_method_name("counter/read"));
    assert!(!is_valid_method_name(""));
    assert!(!is_valid_method_name("bad name"));
}

#[test]
fn unknown_execution_classes_are_rejected_without_downgrade() {
    assert_eq!(
        ExecutionClass::parse("read_only"),
        Some(ExecutionClass::ReadOnly)
    );
    assert_eq!(
        ExecutionClass::parse("non_replayable"),
        Some(ExecutionClass::NonReplayable)
    );
    // 未知执行类别必须显式拒绝；不允许悄悄降级成 read_only。
    assert_eq!(ExecutionClass::parse("big_impact"), None);
    assert_eq!(ExecutionClass::parse(""), None);
    assert_eq!(ExecutionClass::parse("READ_ONLY"), None);
}

#[test]
fn validate_rejects_bad_and_duplicate_descriptions() {
    let mut empty_id = counter_contract();
    empty_id.contract_id = "".into();
    assert!(empty_id.validate().is_err());

    let mut bad_id = counter_contract();
    bad_id.contract_id = "not valid id!".into();
    assert!(bad_id.validate().is_err());

    let mut no_members = counter_contract();
    no_members.methods.clear();
    no_members.states.clear();
    assert!(no_members.validate().is_err());

    let mut duplicate_method = counter_contract();
    duplicate_method
        .methods
        .push(method("counter/read", ExecutionClass::ReadOnly));
    assert!(duplicate_method.validate().is_err());

    let mut duplicate_state = counter_contract();
    duplicate_state.states.push(state("counter/value"));
    assert!(duplicate_state.validate().is_err());

    let mut bad_method_name = counter_contract();
    bad_method_name.methods[0].name = "".into();
    assert!(bad_method_name.validate().is_err());

    let mut missing_schema = counter_contract();
    missing_schema.methods[0].output_schema = "".into();
    assert!(missing_schema.validate().is_err());

    let mut missing_action = counter_contract();
    missing_action.states[0].action = "".into();
    assert!(missing_action.validate().is_err());
}

#[test]
fn registry_rejects_duplicate_and_answers_installed() {
    let registry = register(counter_contract());
    // 第二次注册同一 contract id：可区分的 Duplicate 错误，不覆盖。
    assert_eq!(
        registry.register(counter_contract()),
        Err(ContractError::Duplicate("conex.counter".into()))
    );
    // 非法描述拒绝注册。
    let mut invalid = counter_contract();
    invalid.contract_id = "".into();
    assert!(matches!(
        registry.register(invalid),
        Err(ContractError::Invalid(_))
    ));
    let description = registry.get("conex.counter").expect("installed");
    assert_eq!(description.contract_id, "conex.counter");
    assert_eq!(registry.get("conex.unknown"), None);
    assert_eq!(registry.installed(), vec!["conex.counter".to_string()]);
}

#[test]
fn contract_answers_methods_and_states() {
    let description = register(counter_contract()).get("conex.counter").unwrap();
    assert!(description.method("counter/read").is_some());
    assert!(description.method("counter/add").is_some());
    assert_eq!(description.method("counter/missing"), None);
    assert!(description.state("counter/value").is_some());
    assert_eq!(description.method_names().len(), 2);
    assert_eq!(description.state_names(), vec!["counter/value".to_string()]);
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

#[test]
fn approval_lookup_is_exact_match_and_default_deny() {
    let approvals = ApprovalSet::new(
        vec![publish("node-a", "counter-ts")],
        vec![consume("alice", "counter-ts")],
    );
    // 精确命中。
    assert!(
        approvals
            .publish(
                "node-a",
                "dev",
                "integration-a",
                "counter-ts",
                "conex.counter"
            )
            .is_some()
    );
    assert!(
        approvals
            .consume("alice", "dev", "counter-ts", "conex.counter")
            .is_some()
    );
    // 任一维度不匹配即 None：旧 group/tenant/显示名/端点名/凭据引用都不
    // 授予 Space 权限——这里只有精确元组算数。
    assert!(
        approvals
            .publish(
                "node-b",
                "dev",
                "integration-a",
                "counter-ts",
                "conex.counter"
            )
            .is_none()
    );
    assert!(
        approvals
            .publish(
                "node-a",
                "prod",
                "integration-a",
                "counter-ts",
                "conex.counter"
            )
            .is_none()
    );
    assert!(
        approvals
            .publish(
                "node-a",
                "dev",
                "integration-b",
                "counter-ts",
                "conex.counter"
            )
            .is_none()
    );
    assert!(
        approvals
            .publish(
                "node-a",
                "dev",
                "integration-a",
                "counter-other",
                "conex.counter"
            )
            .is_none()
    );
    assert!(
        approvals
            .publish(
                "node-a",
                "dev",
                "integration-a",
                "counter-ts",
                "conex.other"
            )
            .is_none()
    );
    assert!(
        approvals
            .consume("bob", "dev", "counter-ts", "conex.counter")
            .is_none()
    );
    assert!(
        approvals
            .consume("alice", "dev", "counter-other", "conex.counter")
            .is_none()
    );
    // 其他维度（此处即凭据等价于 principal 之外的身份）不匹配 → 拒绝。
    assert!(
        approvals
            .consume("alice", "dev", "counter-ts", "conex.other")
            .is_none()
    );
}

#[test]
fn visible_projects_approval_members_and_never_widens() {
    let registry = register(counter_contract());
    let description = registry.get("conex.counter").unwrap();
    // 无消费批准 → 模块不可见。
    let empty = ApprovalSet::new(vec![publish("node-a", "counter-ts")], Vec::new());
    assert!(
        empty
            .visible("alice", "dev", "counter-ts", "conex.counter", &description)
            .is_none()
    );

    // 缺省 members = 整个契约。
    let full = ApprovalSet::new(
        vec![],
        vec![ConsumeApproval {
            principal_id: "alice".into(),
            space_id: "dev".into(),
            provider_endpoint_id: "counter-ts".into(),
            contract_id: "conex.counter".into(),
            methods: None,
            states: None,
        }],
    );
    let (methods, states) = full
        .visible("alice", "dev", "counter-ts", "conex.counter", &description)
        .expect("approved consumer sees the module");
    assert_eq!(
        methods,
        vec!["counter/read".to_string(), "counter/add".to_string()]
    );
    assert_eq!(states, vec!["counter/value".to_string()]);

    // 显式成员列表：只投影批准的方法/状态；契约外的成员被过滤，绝不加宽。
    let narrow = ApprovalSet::new(
        vec![],
        vec![ConsumeApproval {
            principal_id: "bob".into(),
            space_id: "dev".into(),
            provider_endpoint_id: "counter-ts".into(),
            contract_id: "conex.counter".into(),
            methods: Some(vec![
                "counter/read".into(),
                "counter/not-in-contract".into(),
            ]),
            states: Some(vec![]),
        }],
    );
    let (methods, states) = narrow
        .visible("bob", "dev", "counter-ts", "conex.counter", &description)
        .expect("approved consumer sees the module");
    assert_eq!(methods, vec!["counter/read".to_string()]);
    assert!(states.is_empty());
}

#[test]
fn validate_keys_rejects_duplicate_module_and_consume_keys() {
    // 同一 (spaceId, providerEndpointId) 被两个 Node 批准：配置装载必须拒绝。
    let duplicated_module = ApprovalSet::new(
        vec![
            publish("node-a", "counter-ts"),
            publish("node-b", "counter-ts"),
        ],
        Vec::new(),
    );
    assert!(duplicated_module.validate_keys().is_err());

    // 同一 (node, space, endpoint) 重复发布批准。
    let duplicated_publish = ApprovalSet::new(
        vec![
            publish("node-a", "counter-ts"),
            publish("node-a", "counter-ts"),
        ],
        Vec::new(),
    );
    assert!(duplicated_publish.validate_keys().is_err());

    // 同一消费者元组重复。
    let duplicated_consume = ApprovalSet::new(
        Vec::new(),
        vec![
            consume("alice", "counter-ts"),
            consume("alice", "counter-ts"),
        ],
    );
    assert!(duplicated_consume.validate_keys().is_err());

    // 合法表通过。
    let valid = ApprovalSet::new(
        vec![
            publish("node-a", "counter-ts"),
            publish("node-b", "counter-rust"),
        ],
        vec![
            consume("alice", "counter-ts"),
            consume("alice", "counter-rust"),
        ],
    );
    assert!(valid.validate_keys().is_ok());
}

#[test]
fn validate_contracts_rejects_unknown_contracts_and_members() {
    let registry = register(counter_contract());
    // 发布批准引用未安装契约。
    let bad_publish = ApprovalSet::new(
        vec![PublishApproval {
            contract_id: "conex.missing".into(),
            ..publish("node-a", "counter-ts")
        }],
        Vec::new(),
    );
    assert!(bad_publish.validate_contracts(&registry).is_err());

    // 消费批准引用未安装契约。
    let bad_consume = ApprovalSet::new(
        Vec::new(),
        vec![ConsumeApproval {
            contract_id: "conex.missing".into(),
            ..consume("alice", "counter-ts")
        }],
    );
    assert!(bad_consume.validate_contracts(&registry).is_err());

    // 消费批准显式成员不是契约成员。
    let bad_member = ApprovalSet::new(
        Vec::new(),
        vec![ConsumeApproval {
            methods: Some(vec!["counter/other".into()]),
            states: Some(vec!["counter/value".into()]),
            ..consume("alice", "counter-ts")
        }],
    );
    assert!(bad_member.validate_contracts(&registry).is_err());

    // 合格装载通过。
    let ok = ApprovalSet::new(
        vec![publish("node-a", "counter-ts")],
        vec![ConsumeApproval {
            methods: Some(vec!["counter/read".into()]),
            states: Some(vec!["counter/value".into()]),
            ..consume("alice", "counter-ts")
        }],
    );
    assert!(ok.validate_contracts(&registry).is_ok());
}

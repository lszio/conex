//! S1 双向调用路由（docs/plans/2026-10-17-conex-s1.md T03；
//! docs/contracts/s1-capabilities.md §7、A04–A07/A09/A11）。
//!
//! 测试层级（与 s1_registration.rs 相同）：`Broker::invoke` 真实宿主 API +
//! `CallRouter` 直接构造；提供端是内存 `RemoteLink`（mpsc 出站队列 +
//! `PendingRequests` 回复通道），观察转发/取消帧并确定性结算回复。既有
//! p1_* / T08 负责完整 socket/WSS 传输路径，本文件不重复搭 TLS/WSS 支架。
//!
//! 覆盖：身份盖章与伪造字段零转发、同 requestId 跨源隔离与同源冲突、
//! 排队/已发送取消与代次固定、non_replayable/read_only 断线与超时结算、
//! 迟到回复单次终结、路由表有界、HTTP binding 目录/调用/取消与 wire 门。
#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use axum::extract::ws::Message;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::Instant;

use conex_core::capability::{
    ApprovalSet, CapabilityMethod, CapabilityState, ConsumeApproval, ContractDescription,
    ContractRegistry, ExecutionClass, PublishApproval,
};
use conex_core::{CallResult, Caller, Limits};
use conex_host::broker::{Broker, BrokerCall, BrokerDeps};
use conex_host::capability::{CallRouter, ModuleRegistry};
use conex_host::remote::RemoteLink;
use conex_host::ws_transport::{AgentReply, PendingRequests};

const REQ_A: &str = "01ARZ3NDEKTSV4RRFFQ69G5FA0";
const REQ_B: &str = "01ARZ3NDEKTSV4RRFFQ69G5FA1";
const REQ_C: &str = "01ARZ3NDEKTSV4RRFFQ69G5FA2";
const REG_ULID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAW";

const SHORT_BUDGET: Duration = Duration::from_millis(60);
const LONG_BUDGET: Duration = Duration::from_secs(10);

// ---- fixtures -------------------------------------------------------------

fn caller(principal: &str) -> Caller {
    Caller {
        principal_id: principal.into(),
        tenant_id: "tenant-a".into(),
        actor_peer_id: "test".into(),
    }
}

fn deadline() -> Instant {
    Instant::now() + LONG_BUDGET
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

fn registry() -> Arc<ModuleRegistry> {
    let contracts = Arc::new(ContractRegistry::new());
    contracts
        .register(counter_contract())
        .expect("fixture contract");
    let approvals = ApprovalSet::new(
        vec![PublishApproval {
            node_id: "node-a".into(),
            space_id: "dev".into(),
            integration_id: "integration-a".into(),
            provider_endpoint_id: "counter-ts".into(),
            contract_id: "conex.counter".into(),
        }],
        vec![
            ConsumeApproval {
                methods: Some(vec!["counter/read".into()]),
                states: None,
                principal_id: "bob".into(),
                space_id: "dev".into(),
                provider_endpoint_id: "counter-ts".into(),
                contract_id: "conex.counter".into(),
            },
            ConsumeApproval {
                principal_id: "alice".into(),
                space_id: "dev".into(),
                provider_endpoint_id: "counter-ts".into(),
                contract_id: "conex.counter".into(),
                methods: None,
                states: None,
            },
        ],
    );
    approvals
        .validate_contracts(&contracts)
        .expect("fixture approvals");
    approvals.validate_keys().expect("fixture approvals");
    ModuleRegistry::new(contracts, Arc::new(approvals))
}

fn broker_with(registry: Arc<ModuleRegistry>, calls: Option<Arc<CallRouter>>) -> Arc<Broker> {
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
            capabilities: Some(registry),
            calls,
            state: None,
        },
    ))
}

async fn register_ok(broker: &Broker, who: &Caller, generation: u64, endpoint: &str) -> String {
    let call = BrokerCall {
        caller: who.clone(),
        endpoint_id: String::new(),
        method: "module/register".into(),
        input: json!({
            "spaceId": "dev",
            "integrationId": "integration-a",
            "providerEndpointId": endpoint,
            "contractId": "conex.counter",
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
    };
    broker.invoke(call).await.unwrap()["registrationId"]
        .as_str()
        .unwrap()
        .to_string()
}

#[allow(clippy::too_many_arguments)]
fn invoke_call(
    who: &Caller,
    generation: u64,
    request_id: &str,
    endpoint: &str,
    space_id: &str,
    registration_id: &str,
    input: Value,
    budget: Duration,
    binding_id: Option<&str>,
) -> BrokerCall {
    BrokerCall {
        caller: who.clone(),
        endpoint_id: endpoint.into(),
        method: "module/invoke".into(),
        input,
        deadline: Instant::now() + budget,
        role: "service".into(),
        link_id: None,
        generation,
        max_frame_bytes: Limits::default().max_frame_bytes,
        request_id: request_id.into(),
        binding_id: binding_id.map(str::to_string),
        space_id: space_id.into(),
        registration_id: registration_id.into(),
    }
}

fn read_invoke(
    who: &Caller,
    generation: u64,
    request_id: &str,
    registration_id: &str,
    budget: Duration,
    binding_id: Option<&str>,
) -> BrokerCall {
    invoke_call(
        who,
        generation,
        request_id,
        "counter-ts",
        "dev",
        registration_id,
        json!({ "method": "counter/read", "argument": {} }),
        budget,
        binding_id,
    )
}

fn add_invoke(
    who: &Caller,
    generation: u64,
    request_id: &str,
    registration_id: &str,
    budget: Duration,
    binding_id: Option<&str>,
) -> BrokerCall {
    invoke_call(
        who,
        generation,
        request_id,
        "counter-ts",
        "dev",
        registration_id,
        json!({ "method": "counter/add", "argument": { "delta": 1 } }),
        budget,
        binding_id,
    )
}

fn cancel_call(
    who: &Caller,
    generation: u64,
    target_request_id: &str,
    binding_id: Option<&str>,
) -> BrokerCall {
    BrokerCall {
        caller: who.clone(),
        endpoint_id: String::new(),
        method: "module/cancel".into(),
        input: json!({ "requestId": target_request_id }),
        deadline: deadline(),
        role: "service".into(),
        link_id: None,
        generation,
        max_frame_bytes: Limits::default().max_frame_bytes,
        request_id: String::new(),
        binding_id: binding_id.map(str::to_string),
        space_id: String::new(),
        registration_id: String::new(),
    }
}

fn list_call(who: &Caller, space: &str, binding_id: Option<&str>) -> BrokerCall {
    BrokerCall {
        caller: who.clone(),
        endpoint_id: String::new(),
        method: "module/list".into(),
        input: json!({ "spaceId": space }),
        deadline: deadline(),
        role: "service".into(),
        link_id: None,
        generation: 0,
        max_frame_bytes: Limits::default().max_frame_bytes,
        request_id: String::new(),
        binding_id: binding_id.map(str::to_string),
        space_id: String::new(),
        registration_id: String::new(),
    }
}

fn spawn_invoke(
    broker: Arc<Broker>,
    call: BrokerCall,
) -> tokio::task::JoinHandle<CallResult<Value>> {
    tokio::spawn(async move { broker.invoke(call).await })
}

async fn wait_until(mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "condition not reached before test deadline"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

async fn wait_pending(router: &CallRouter, expected: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while router.pending_len().await != expected {
        assert!(
            tokio::time::Instant::now() < deadline,
            "pending_len did not reach {expected}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// 标准装配：counter-ts 由 node-a 在代次 11 注册；alice/bob 为消费端。
struct Setup {
    broker: Arc<Broker>,
    router: Arc<CallRouter>,
    registration_id: String,
    provider: Provider,
}

async fn setup() -> Setup {
    let registry = registry();
    let router = CallRouter::new(registry.clone());
    let broker = broker_with(registry.clone(), Some(router.clone()));
    let provider = Provider::new(11);
    router.attach("node-a", 11, provider.link.clone()).await;
    let registration_id = register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;
    Setup {
        broker,
        router,
        registration_id,
        provider,
    }
}

// ---- 内存提供端链 --------------------------------------------------------

/// 提供端链：真实 `RemoteLink`（mpsc 出站队列 + `PendingRequests` 回复
/// 通道）。读者任务把文本帧收集进共享列表；测试用 `reply`/`drop_link`
/// 确定性结算转发调用。无实际 socket——帧内容与回复投递与真实 WSS 出站
/// 路径逐字节同构（`encode_wire` + `enqueue_message`）。
struct Provider {
    link: RemoteLink,
    generation: u64,
    pending: Arc<PendingRequests>,
    frames: Arc<Mutex<Vec<Value>>>,
    reader: tokio::task::JoinHandle<()>,
}

impl Provider {
    fn new(generation: u64) -> Self {
        let pending = Arc::new(PendingRequests::new(generation));
        let queued = Arc::new(AtomicUsize::new(0));
        let (tx, mut rx) = mpsc::channel(64);
        let link = RemoteLink::new(
            generation,
            tx,
            queued,
            pending.clone(),
            1024 * 1024,
            262_144,
        );
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let reader = tokio::spawn(async move {
            while let Some((message, _)) = rx.recv().await {
                if let Message::Text(text) = message
                    && let Ok(value) = serde_json::from_str::<Value>(&text)
                {
                    sink.lock().expect("provider frames poisoned").push(value);
                }
            }
        });
        Self {
            link,
            generation,
            pending,
            frames,
            reader,
        }
    }

    fn count(&self, method: &str) -> usize {
        self.frames
            .lock()
            .expect("provider frames poisoned")
            .iter()
            .filter(|frame| frame["method"] == method)
            .count()
    }

    async fn wait_invoke(&self) -> Value {
        wait_until(|| self.count("module/invoke") > 0).await;
        self.frames
            .lock()
            .expect("provider frames poisoned")
            .iter()
            .find(|frame| frame["method"] == "module/invoke")
            .cloned()
            .expect("invoke frame")
    }

    async fn wait_nth_invoke(&self, n: usize) -> Value {
        wait_until(|| self.count("module/invoke") > n).await;
        self.frames
            .lock()
            .expect("provider frames poisoned")
            .iter()
            .filter(|frame| frame["method"] == "module/invoke")
            .nth(n)
            .cloned()
            .expect("invoke frame")
    }

    async fn wait_cancel(&self) -> Value {
        wait_until(|| self.count("module/cancel") > 0).await;
        self.frames
            .lock()
            .expect("provider frames poisoned")
            .iter()
            .rev()
            .find(|frame| frame["method"] == "module/cancel")
            .cloned()
            .expect("cancel frame")
    }

    async fn frames(&self) -> Vec<Value> {
        self.frames
            .lock()
            .expect("provider frames poisoned")
            .clone()
    }

    async fn reply(&self, hop: &str, result: CallResult<AgentReply>) -> bool {
        self.pending.route(self.generation, hop, result).await
    }

    /// 提供端断线：未决 hop 全部收到连接级失败（与 ws_transport 断线清理同
    /// 样的 `Ok(Err(Unavailable))` 语义）。
    async fn drop_link(&self) {
        self.pending.fail_all().await;
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

// ---- tests ----------------------------------------------------------------

/// T03 步骤 2（A04）：转发帧只携带中心盖章身份；客户端自报的
/// space/registrationId 定位字段与活动记录不符时零转发拒绝。
#[tokio::test]
async fn forwarded_context_is_host_stamped_not_client_supplied() {
    let Setup {
        broker,
        registration_id,
        provider,
        ..
    } = setup().await;
    let task = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let frame = provider.wait_invoke().await;
    let hop = frame["id"].as_str().unwrap().to_string();
    // 转发上下文 = 认证 caller + 受信注册记录，与客户端输入无关。
    let context = &frame["params"]["context"];
    assert_eq!(context["principalId"], "alice");
    assert_eq!(context["tenantId"], "tenant-a");
    assert_eq!(context["spaceId"], "dev");
    assert_eq!(context["registrationId"], registration_id);
    assert_eq!(context["providerEndpointId"], "counter-ts");
    assert_eq!(frame["params"]["input"]["method"], "counter/read");

    provider
        .reply(
            &hop,
            Ok(AgentReply::Value(json!({ "count": 7, "revision": "0" }))),
        )
        .await;
    assert_eq!(task.await.unwrap().unwrap()["count"], 7);

    // 客户端自报 registrationId（不是活动值）→ stale_revision，零转发。
    let error = broker
        .invoke(read_invoke(
            &caller("alice"),
            21,
            REQ_B,
            REG_ULID,
            LONG_BUDGET,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::StaleRevision)
    );
    assert_eq!(error.wire().execution, "not_started");
    // 客户端自报 spaceId（无注册）→ forbidden，零转发。
    let error = broker
        .invoke(invoke_call(
            &caller("alice"),
            21,
            REQ_B,
            "counter-ts",
            "prod",
            &registration_id,
            json!({ "method": "counter/read", "argument": {} }),
            LONG_BUDGET,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));
    // 两个伪造尝试都没有产生额外业务帧。
    assert_eq!(provider.count("module/invoke"), 1);
}

/// T03 步骤 3（A04）：同一 requestId 在 `SourceKey::Link` 不同代次与
/// `SourceKey::Http` 不同 binding 之间独立解析，绝不串包。
#[tokio::test]
async fn same_request_id_across_sources_resolves_independently() {
    let Setup {
        broker,
        registration_id,
        provider,
        ..
    } = setup().await;
    // WSS：消费代次 21（alice）与 22（bob）同用 REQ_A。
    let a = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let b = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("bob"),
            22,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    wait_until(|| provider.count("module/invoke") >= 2).await;
    let frames = provider.frames().await;
    let invoke: Vec<Value> = frames
        .iter()
        .filter(|frame| frame["method"] == "module/invoke")
        .cloned()
        .collect();
    let alice_frame = invoke
        .iter()
        .find(|frame| frame["params"]["context"]["principalId"] == "alice")
        .expect("alice frame");
    let bob_frame = invoke
        .iter()
        .find(|frame| frame["params"]["context"]["principalId"] == "bob")
        .expect("bob frame");
    let hop_a = alice_frame["id"].as_str().unwrap();
    let hop_b = bob_frame["id"].as_str().unwrap();
    assert_ne!(hop_a, hop_b, "each forwarded invoke gets its own hop id");
    assert_eq!(
        alice_frame["params"]["context"]["registrationId"],
        registration_id
    );
    assert_eq!(
        bob_frame["params"]["context"]["registrationId"],
        registration_id
    );

    let reply_a = provider.reply(
        hop_a,
        Ok(AgentReply::Value(json!({"count": 1, "revision": "0"}))),
    );
    let reply_b = provider.reply(
        hop_b,
        Ok(AgentReply::Value(json!({"count": 22, "revision": "1"}))),
    );
    let (ra, rb) = tokio::join!(reply_a, reply_b);
    assert!(ra && rb, "both hops are registered");
    assert_eq!(a.await.unwrap().unwrap()["count"], 1);
    assert_eq!(b.await.unwrap().unwrap()["count"], 22);

    // HTTP：binding b1 与 b2 同用 REQ_B（同一 principal，顺序执行区分帧）。
    let call_c = invoke_call(
        &caller("alice"),
        0,
        REQ_B,
        "counter-ts",
        "dev",
        &registration_id,
        json!({ "method": "counter/read", "argument": {} }),
        LONG_BUDGET,
        Some("bnd-1"),
    );
    let call_d = add_invoke(
        &caller("alice"),
        0,
        REQ_B,
        &registration_id,
        LONG_BUDGET,
        Some("bnd-2"),
    );
    let c = spawn_invoke(broker.clone(), call_c);
    let frame_c = provider.wait_nth_invoke(2).await;
    let hop_c = frame_c["id"].as_str().unwrap().to_string();
    let d = spawn_invoke(broker.clone(), call_d);
    let frame_d = provider.wait_nth_invoke(3).await;
    let hop_d = frame_d["id"].as_str().unwrap().to_string();
    provider
        .reply(
            &hop_c,
            Ok(AgentReply::Value(json!({"count": 3, "revision": "1"}))),
        )
        .await;
    provider
        .reply(
            &hop_d,
            Ok(AgentReply::Value(json!({"count": 4, "revision": "2"}))),
        )
        .await;
    assert_eq!(c.await.unwrap().unwrap()["count"], 3);
    assert_eq!(d.await.unwrap().unwrap()["count"], 4);
}

/// T03 步骤 3（A04/§7.1）：同源并发重复 requestId 在转发前以 conflict 拒绝；
/// 调用终结后同一 requestId 可重用。
#[tokio::test]
async fn duplicate_request_id_in_same_source_conflicts_before_forward() {
    let Setup {
        broker,
        registration_id,
        provider,
        ..
    } = setup().await;
    let task = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let frame = provider.wait_invoke().await;
    let hop = frame["id"].as_str().unwrap().to_string();

    // 第一个调用已发送且记录在表：同源重复 ID → conflict/not_started。
    let error = broker
        .invoke(read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Conflict));
    assert_eq!(error.wire().execution, "not_started");
    assert_eq!(error.wire().retry, "never");
    assert_eq!(provider.count("module/invoke"), 1, "no second forward");

    provider
        .reply(
            &hop,
            Ok(AgentReply::Value(json!({ "count": 1, "revision": "0" }))),
        )
        .await;
    task.await.unwrap().unwrap();

    // 记录已终结：同一源、同一 requestId 可以再次调用。
    let again = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let frame = provider.wait_nth_invoke(1).await;
    let hop = frame["id"].as_str().unwrap().to_string();
    provider
        .reply(
            &hop,
            Ok(AgentReply::Value(json!({ "count": 2, "revision": "1" }))),
        )
        .await;
    assert_eq!(again.await.unwrap().unwrap()["count"], 2);
}

/// T03 步骤 5+7（A05）：排队调用（test_gate 屏障内）收到 cancel →
/// `{requested:true}`，invoke 永不发送，结算 cancelled/not_started。
#[tokio::test]
async fn queued_cancel_requests_and_never_sends() {
    let Setup {
        broker,
        router,
        registration_id,
        provider,
    } = setup().await;
    let (gate_tx, gate_rx) = oneshot::channel();
    *router.test_gate.lock().await = Some(gate_rx);
    let task = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    // 记录已插入、invoke 停在发送前屏障。
    wait_pending(&router, 1).await;

    let result = broker
        .invoke(cancel_call(&caller("alice"), 21, REQ_A, None))
        .await
        .unwrap();
    assert_eq!(result, json!({ "requested": true }));
    assert!(
        provider.frames().await.is_empty(),
        "queued cancel sends nothing"
    );

    // 放行：invoke 复查 cancelled 后丢弃，不发送。
    let _ = gate_tx.send(());
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Cancelled));
    assert_eq!(error.wire().execution, "not_started");
    assert_eq!(error.wire().retry, "never");
    assert_eq!(router.pending_len().await, 0);
    assert!(
        provider.frames().await.is_empty(),
        "never sent, not even control"
    );
}

/// T03 步骤 6（A05 SUPPORT）：排队期预算耗尽——请求在发出前已超预算，
/// 结算 timeout/not_started/safe，零业务帧，路由记录不残留
/// （契约 §7.2 “排队期间预算耗尽”一行；真实进程侧见 xtask s1_e2e A05）。
#[tokio::test]
async fn queued_budget_exhaustion_settles_not_started_and_never_sends() {
    let Setup {
        broker,
        router,
        registration_id,
        provider,
    } = setup().await;
    let mut call = read_invoke(
        &caller("alice"),
        21,
        REQ_C,
        &registration_id,
        LONG_BUDGET,
        None,
    );
    // 请求已排队到预算耗尽才到达路由器：越过发送边界前必须拒绝。
    call.deadline = Instant::now() - Duration::from_millis(10);
    let error = broker.invoke(call).await.unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Timeout));
    assert_eq!(error.wire().execution, "not_started");
    assert_eq!(error.wire().retry, "safe");
    assert!(
        provider.frames().await.is_empty(),
        "expired queued budget never sends, not even control"
    );
    assert_eq!(router.pending_len().await, 0);
}

/// T03 步骤 5+6（A05/A06）：已发送取消转发一条独立 module/cancel 控制帧，
/// 目标固定在原注册代次；新注册接管目录后不 redirect；原 invoke 仍由自己
/// 的回复结算。
#[tokio::test]
async fn sent_cancel_targets_original_registration_generation() {
    let Setup {
        broker,
        router,
        registration_id,
        provider,
    } = setup().await;
    let old_registration = registration_id.clone();
    let task = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let frame = provider.wait_invoke().await;
    let hop = frame["id"].as_str().unwrap().to_string();

    // 同 Node 新代次注册同一端点：目录切到代次 12。
    let provider12 = Provider::new(12);
    router.attach("node-a", 12, provider12.link.clone()).await;
    let new_registration = register_ok(&broker, &caller("node-a"), 12, "counter-ts").await;
    assert_ne!(new_registration, old_registration);

    let result = broker
        .invoke(cancel_call(&caller("alice"), 21, REQ_A, None))
        .await
        .unwrap();
    assert_eq!(result, json!({ "requested": true }));

    // 取消帧：独立控制 ID、指向原 hop、携带原注册代次的上下文，出现在
    // 代次 11 的链上；新代次链零帧（绝不改投新注册）。
    let cancel = provider.wait_cancel().await;
    let cancel_id = cancel["id"].as_str().unwrap();
    assert_ne!(cancel_id, hop, "cancel uses its own request id");
    assert!(conex_proto::wire::is_ulid(cancel_id));
    assert_eq!(cancel["method"], "module/cancel");
    assert_eq!(cancel["params"]["input"]["requestId"], hop);
    assert_eq!(
        cancel["params"]["context"]["registrationId"],
        old_registration
    );
    assert_eq!(
        cancel["params"]["context"]["providerEndpointId"],
        "counter-ts"
    );
    assert_eq!(cancel["params"]["context"]["principalId"], "alice");
    assert_eq!(provider.count("module/cancel"), 1);
    assert!(
        provider12.frames().await.is_empty(),
        "no redirect to new generation"
    );

    // 原 invoke 仍由自己的回复结算。
    provider
        .reply(
            &hop,
            Ok(AgentReply::Value(json!({ "count": 5, "revision": "0" }))),
        )
        .await;
    assert_eq!(task.await.unwrap().unwrap()["count"], 5);
    assert_eq!(router.pending_len().await, 0);
    // 记录已终结：再次取消不泄漏 → requested:false。
    let again = broker
        .invoke(cancel_call(&caller("alice"), 21, REQ_A, None))
        .await
        .unwrap();
    assert_eq!(again, json!({ "requested": false }));
}

/// T03 步骤 4+5（A07）：non_replayable 调用发送后提供端断线 →
/// outcome_unknown（-32012）/retry=never；不自动重发（帧数保持 1）。
#[tokio::test]
async fn non_replayable_link_loss_settles_outcome_unknown_exactly_once() {
    let Setup {
        router,
        registration_id,
        provider,
        broker,
    } = setup().await;
    let task = spawn_invoke(
        broker.clone(),
        add_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let frame = provider.wait_invoke().await;
    let hop = frame["id"].as_str().unwrap().to_string();

    provider.drop_link().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::OutcomeUnknown)
    );
    assert_eq!(
        error.wire().code,
        -32012,
        "outcome_unknown is the frozen wire code"
    );
    assert_eq!(error.wire().execution, "unknown");
    assert_eq!(error.wire().retry, "never");
    assert_eq!(provider.count("module/invoke"), 1, "no automatic re-send");
    assert_eq!(router.pending_len().await, 0);
    // 迟到的回复投不进去：hop 已结算，不会二次 resolve。
    assert!(
        !provider
            .reply(
                &hop,
                Ok(AgentReply::Value(json!({ "count": 1, "revision": "1" }))),
            )
            .await
    );
}

/// T03 步骤 4（A07）：read_only 调用断线 → 显式 unavailable/retry=safe；
/// 预算耗尽 → timeout/retry=safe；两者都不自动重放。
#[tokio::test]
async fn read_only_drop_settles_explicitly_with_safe_retry_never_replayed() {
    let Setup {
        registration_id,
        provider,
        broker,
        ..
    } = setup().await;
    // 失联：unavailable / unknown / safe。
    let task = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let _frame = provider.wait_invoke().await;
    provider.drop_link().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Unavailable));
    assert_eq!(error.wire().retry, "safe");
    assert_eq!(provider.count("module/invoke"), 1);

    // 预算耗尽：timeout / unknown / safe，同样不重放。
    let short = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_B,
            &registration_id,
            SHORT_BUDGET,
            None,
        ),
    );
    let error = tokio::time::timeout(Duration::from_secs(5), short)
        .await
        .expect("read_only invoke must settle at its deadline")
        .expect("invoke task must not panic")
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Timeout));
    assert_eq!(error.wire().retry, "safe");
    assert_eq!(
        provider.count("module/invoke"),
        2,
        "each attempt sends once"
    );
}

/// T03 步骤 5（A06）：发送后超时的 non_replayable 调用结算 outcome_unknown；
/// 迟到回复不二次 resolve；pending 归零。
#[tokio::test]
async fn late_reply_after_timeout_resolves_once_and_pending_returns_to_zero() {
    let Setup {
        router,
        registration_id,
        provider,
        broker,
    } = setup().await;
    let task = spawn_invoke(
        broker.clone(),
        add_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            SHORT_BUDGET,
            None,
        ),
    );
    let frame = provider.wait_invoke().await;
    let hop = frame["id"].as_str().unwrap().to_string();

    let error = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .expect("invoke must settle at its deadline")
        .expect("invoke task must not panic")
        .unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::OutcomeUnknown)
    );
    assert_eq!(error.wire().execution, "unknown");
    assert_eq!(error.wire().retry, "never");

    assert_eq!(router.pending_len().await, 0);
    assert!(
        !provider
            .reply(
                &hop,
                Ok(AgentReply::Value(json!({ "count": 9, "revision": "1" }))),
            )
            .await,
        "late reply must not resolve a settled invoke"
    );
    assert_eq!(provider.count("module/invoke"), 1);
}

/// T03 步骤 4（A12）：non_replayable 的畸形提供端输出结算 outcome_unknown，
/// 不是 bad_request/not_started——提供端已执行，结果只是不可信；read_only
/// 对照：明确 completed 失败。
#[tokio::test]
async fn malformed_provider_output_is_outcome_unknown_for_non_replayable() {
    let Setup {
        registration_id,
        provider,
        broker,
        ..
    } = setup().await;
    // 非规范 uint64 revision（前导零）→ schema 校验失败。
    let malformed = json!({ "count": 1, "revision": "007" });

    let task = spawn_invoke(
        broker.clone(),
        add_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let frame = provider.wait_invoke().await;
    let hop = frame["id"].as_str().unwrap().to_string();
    provider
        .reply(&hop, Ok(AgentReply::Value(malformed.clone())))
        .await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::OutcomeUnknown)
    );
    assert_eq!(error.wire().execution, "unknown");
    assert_eq!(error.wire().retry, "never");

    // 对照：read_only 畸形输出是确定的 completed 业务失败。
    let task = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_B,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let frame = provider.wait_nth_invoke(1).await;
    let hop = frame["id"].as_str().unwrap().to_string();
    provider.reply(&hop, Ok(AgentReply::Value(malformed))).await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.wire().execution, "completed");
    assert_eq!(error.wire().retry, "never");
    assert_ne!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::OutcomeUnknown)
    );
}

/// T03 步骤 3（§7.1 有界源关联，A09 控制预留）：路由表满 → invoke 在发送
/// 前 quota_exceeded/not_started；排队中被占的调用随后正常发送。
#[tokio::test]
async fn routing_table_capacity_returns_quota_exceeded_before_send() {
    let registry = registry();
    let router = CallRouter::with_max_pending(registry.clone(), 1);
    let broker = broker_with(registry.clone(), Some(router.clone()));
    let provider = Provider::new(11);
    router.attach("node-a", 11, provider.link.clone()).await;
    let registration_id = register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;

    let (gate_tx, gate_rx) = oneshot::channel();
    *router.test_gate.lock().await = Some(gate_rx);
    let first = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    wait_pending(&router, 1).await;

    let error = broker
        .invoke(read_invoke(
            &caller("alice"),
            21,
            REQ_B,
            &registration_id,
            LONG_BUDGET,
            None,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::QuotaExceeded)
    );
    assert_eq!(error.wire().execution, "not_started");
    assert_eq!(error.wire().retry, "never");
    // 饱和拒绝发生在发送前：此刻还没有任何业务帧。
    assert!(provider.frames().await.is_empty());

    let _ = gate_tx.send(());
    let frame = provider.wait_invoke().await;
    let hop = frame["id"].as_str().unwrap().to_string();
    provider
        .reply(
            &hop,
            Ok(AgentReply::Value(json!({ "count": 3, "revision": "0" }))),
        )
        .await;
    assert_eq!(first.await.unwrap().unwrap()["count"], 3);
    assert_eq!(router.pending_len().await, 0);
}

/// T03 步骤 3（A04）：Link 与 HTTP binding 两种源同 requestId 互不干扰、
/// 各回各的回复。
#[tokio::test]
async fn link_and_http_sources_with_same_request_id_do_not_cross() {
    let Setup {
        broker,
        registration_id,
        provider,
        ..
    } = setup().await;
    let wss = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_C,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let frame_wss = provider.wait_invoke().await;
    let hop_wss = frame_wss["id"].as_str().unwrap().to_string();

    // HTTP binding 与 WSS 使用同一 requestId：各自独立转发。
    let http = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            0,
            REQ_C,
            &registration_id,
            LONG_BUDGET,
            Some("bnd-1"),
        ),
    );
    let frame_http = provider.wait_nth_invoke(1).await;
    let hop_http = frame_http["id"].as_str().unwrap().to_string();
    assert_ne!(hop_wss, hop_http);

    // 每路回自己的回复；乱序结算也不交叉。
    provider
        .reply(
            &hop_http,
            Ok(AgentReply::Value(json!({ "count": 30, "revision": "1" }))),
        )
        .await;
    provider
        .reply(
            &hop_wss,
            Ok(AgentReply::Value(json!({ "count": 3, "revision": "0" }))),
        )
        .await;
    assert_eq!(http.await.unwrap().unwrap()["count"], 30);
    assert_eq!(wss.await.unwrap().unwrap()["count"], 3);
}

/// T03 步骤 6（A11）：HTTP binding 路径经 broker 支撑 module/list/invoke/
/// cancel；module/register 与 state/* 在 wire 门（validate_http_frame）被
/// 拒绝，先于任何业务分派。
#[tokio::test]
async fn http_surface_list_invoke_cancel_and_wire_rejections() {
    let Setup {
        broker,
        registration_id,
        provider,
        ..
    } = setup().await;
    // module/list：HTTP binding 主体（alice）看到授权模块。
    let modules = broker
        .invoke(list_call(&caller("alice"), "dev", Some("bnd-1")))
        .await
        .unwrap()["modules"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(modules.len(), 1);
    assert_eq!(modules[0]["providerEndpointId"], "counter-ts");

    // module/invoke + module/cancel：同一 binding 源定位原调用。
    let task = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            0,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            Some("bnd-1"),
        ),
    );
    let frame = provider.wait_invoke().await;
    let hop = frame["id"].as_str().unwrap().to_string();
    assert_eq!(frame["params"]["context"]["principalId"], "alice");
    let result = broker
        .invoke(cancel_call(&caller("alice"), 0, REQ_A, Some("bnd-1")))
        .await
        .unwrap();
    assert_eq!(result, json!({ "requested": true }));
    let cancel = provider.wait_cancel().await;
    assert_eq!(cancel["params"]["input"]["requestId"], hop);
    assert!(
        provider
            .reply(
                &hop,
                Ok(AgentReply::Value(json!({ "count": 4, "revision": "0" }))),
            )
            .await
    );
    assert_eq!(task.await.unwrap().unwrap()["count"], 4);
}

/// T03 步骤 6（A11）：`validate_http_frame` 与 http.rs 入口同一语义——HTTP
/// 只接受 list/invoke/cancel（且要求 bindingId）；register/state/subscribe
/// 在进入业务前以 unsupported_capability 拒绝。
#[test]
fn http_wire_gate_allows_consume_methods_and_rejects_provider_and_state() {
    // list/invoke/cancel 携带 bindingId → 放行。
    assert!(http_gate(
        r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FA2","method":"module/list","params":{"context":{"plane":"broker","bindingId":"bnd-1"},"timeoutBudgetMs":8000,"input":{"spaceId":"dev"}}}"#
    )
    .is_ok());
    assert!(http_gate(
        r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FA0","method":"module/invoke","params":{"context":{"providerEndpointId":"counter-ts","plane":"broker","spaceId":"dev","registrationId":"01ARZ3NDEKTSV4RRFFQ69G5FAW","bindingId":"bnd-1"},"timeoutBudgetMs":8000,"input":{"method":"counter/read","argument":{}}}}"#
    )
    .is_ok());
    assert!(http_gate(
        r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FA1","method":"module/cancel","params":{"context":{"providerEndpointId":"counter-ts","plane":"broker","spaceId":"dev","registrationId":"01ARZ3NDEKTSV4RRFFQ69G5FAW","bindingId":"bnd-1"},"timeoutBudgetMs":8000,"input":{"requestId":"01ARZ3NDEKTSV4RRFFQ69G5FA0"}}}"#
    )
    .is_ok());

    // module/register 与 state/subscribe：先于业务分派拒绝。
    let error = http_gate(
        r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FA3","method":"module/register","params":{"context":{"plane":"broker","bindingId":"bnd-1"},"timeoutBudgetMs":8000,"input":{"spaceId":"dev","integrationId":"integration-a","providerEndpointId":"counter-ts","contractId":"conex.counter"}}}"#,
    )
    .unwrap_err();
    assert_eq!(
        error.rpc_code,
        conex_proto::ErrorCode::UnsupportedCapability as i32
    );
    let error = http_gate(
        r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FA4","method":"state/subscribe","params":{"context":{"providerEndpointId":"counter-ts","plane":"broker","spaceId":"dev","registrationId":"01ARZ3NDEKTSV4RRFFQ69G5FAW","bindingId":"bnd-1"},"timeoutBudgetMs":8000,"input":{"subscriptionId":"01ARZ3NDEKTSV4RRFFQ69G5FA5","state":"counter/value","argument":{}}}}"#,
    )
    .unwrap_err();
    assert_eq!(
        error.rpc_code,
        conex_proto::ErrorCode::UnsupportedCapability as i32
    );

    // 允许的方法缺 bindingId → bad_request（HTTP 强制原 binding 归属）。
    let error = http_gate(
        r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FA2","method":"module/list","params":{"context":{"plane":"broker"},"timeoutBudgetMs":8000,"input":{"spaceId":"dev"}}}"#,
    )
    .unwrap_err();
    assert_eq!(error.rpc_code, conex_proto::ErrorCode::BadRequest as i32);
}

/// 与 http.rs 入口相同的 wire 门：解码（Boundary::Client）+ validate_http_frame。
fn http_gate(text: &str) -> Result<(), conex_proto::wire::ProtocolError> {
    let message = conex_proto::capability::decode_frame(
        text.as_bytes(),
        conex_proto::capability::Boundary::Client,
    )?;
    conex_proto::capability::validate_http_frame(&message)
}

/// T03 步骤 5（§7.3）：消费者断线——已发送调用转发取消控制帧；排队调用
/// 直接丢弃，两种情形都不产生额外业务帧。
#[tokio::test]
async fn cleanup_source_cancels_sent_and_drops_queued_without_business_frames() {
    let Setup {
        broker,
        router,
        registration_id,
        provider,
    } = setup().await;
    // 已发送：转发 module/cancel 控制帧到提供端。
    let task = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            21,
            REQ_A,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    let frame = provider.wait_invoke().await;
    let hop = frame["id"].as_str().unwrap().to_string();
    router.cleanup_source(21).await;
    assert_eq!(router.pending_len().await, 0);
    let cancel = provider.wait_cancel().await;
    assert_eq!(cancel["method"], "module/cancel");
    assert_eq!(cancel["params"]["input"]["requestId"], hop);
    assert_eq!(
        provider.count("module/invoke"),
        1,
        "sent call is not resent"
    );
    task.abort();

    // 排队未发送：直接丢弃，取消控制帧也没有（本地删除不是远端取消）。
    let (_gate_tx, gate_rx) = oneshot::channel();
    *router.test_gate.lock().await = Some(gate_rx);
    let queued = spawn_invoke(
        broker.clone(),
        read_invoke(
            &caller("alice"),
            22,
            REQ_B,
            &registration_id,
            LONG_BUDGET,
            None,
        ),
    );
    wait_pending(&router, 1).await;
    router.cleanup_source(22).await;
    assert_eq!(router.pending_len().await, 0);
    assert_eq!(provider.count("module/invoke"), 1, "queued call never sent");
    assert_eq!(
        provider.count("module/cancel"),
        1,
        "no cancel frame for queued"
    );
    queued.abort();
}

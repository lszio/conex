//! S1 状态中继（docs/plans/2026-10-17-conex-s1.md T04；
//! docs/contracts/s1-capabilities.md §8-§9、A02/A08-A10）。
//!
//! 测试层级（与 s1_calls.rs / s1_registration.rs 相同）：`Broker::invoke`
//! 真实宿主 API（state/subscribe、state/unsubscribe）+ `StateRelay` 直接
//! 构造（attach/detach/on_notification/response_enqueued/end_*）；提供端与
//! 消费者都是内存 `RemoteLink`（mpsc 出站队列 + `PendingRequests` 回复
//! 通道），观察转发/控制帧并确定性结算快照回复。既有 p1_* / T08 负责
//! 完整 socket/WSS 传输路径，本文件不重复搭 TLS/WSS 支架。
//!
//! 覆盖（T04 步骤 → 契约）：1 快照先于更新的 B2 屏障（A02）；2 opening
//! 失败/超时/取消/失联清理与迟到快照不复活（A08/A10）；3 代次/seq/
//! revision/sample/value 校验失序即 session_lost 终结（A10/A12）；4 退订
//! 与 state/end 各自恰好一次终结、成功取消与异常终止可区分（A08）；5 断线/
//! 提供端失联/换代/撤销/撤权清理（A08）；6 订阅预算与 opening 缓冲有界，
//! 显式拒绝/结束而非静默丢失，`subscriptions_len()` 每条清理路径归零。
#![forbid(unsafe_code)]

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::extract::ws::Message;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use conex_core::capability::{
    ApprovalSet, CapabilityMethod, CapabilityState, ConsumeApproval, ContractDescription,
    ContractRegistry, ExecutionClass, PublishApproval,
};
use conex_core::{CallResult, Caller, Limits};
use conex_host::broker::{Broker, BrokerCall, BrokerDeps};
use conex_host::capability::{ModuleRegistry, StateRelay};
use conex_host::remote::RemoteLink;
use conex_host::ws_transport::{AgentReply, PendingRequests};

// 消费者订阅 ID 与请求 ID（ULID 形状，契约 §2/Limits 校验）。
const SUB_0: &str = "01ARZ3NDEKTSV4RRFFQ69G5FD0";
const SUB_1: &str = "01ARZ3NDEKTSV4RRFFQ69G5FD1";
const SUB_2: &str = "01ARZ3NDEKTSV4RRFFQ69G5FD2";
const SUB_5: &str = "01ARZ3NDEKTSV4RRFFQ69G5FD5";
const REQ_0: &str = "01ARZ3NDEKTSV4RRFFQ69G5FC0";
const REQ_1: &str = "01ARZ3NDEKTSV4RRFFQ69G5FC1";
const REQ_2: &str = "01ARZ3NDEKTSV4RRFFQ69G5FC2";
const REQ_3: &str = "01ARZ3NDEKTSV4RRFFQ69G5FC3";
const REQ_4: &str = "01ARZ3NDEKTSV4RRFFQ69G5FC4";
const REQ_5: &str = "01ARZ3NDEKTSV4RRFFQ69G5FC5";

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

fn broker_with_state(registry: Arc<ModuleRegistry>, relay: Arc<StateRelay>) -> Arc<Broker> {
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
            calls: None,
            state: Some(relay),
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
fn subscribe_call(
    who: &Caller,
    generation: u64,
    request_id: &str,
    endpoint: &str,
    space_id: &str,
    registration_id: &str,
    state: &str,
    subscription_id: &str,
    argument: Value,
    budget: Duration,
) -> BrokerCall {
    BrokerCall {
        caller: who.clone(),
        endpoint_id: endpoint.into(),
        method: "state/subscribe".into(),
        input: json!({
            "subscriptionId": subscription_id,
            "state": state,
            "argument": argument,
        }),
        deadline: Instant::now() + budget,
        role: "service".into(),
        link_id: None,
        generation,
        max_frame_bytes: Limits::default().max_frame_bytes,
        request_id: request_id.into(),
        binding_id: None,
        space_id: space_id.into(),
        registration_id: registration_id.into(),
    }
}

fn unsubscribe_call(
    who: &Caller,
    generation: u64,
    request_id: &str,
    subscription_id: &str,
) -> BrokerCall {
    BrokerCall {
        caller: who.clone(),
        endpoint_id: String::new(),
        method: "state/unsubscribe".into(),
        input: json!({ "subscriptionId": subscription_id }),
        deadline: deadline(),
        role: "service".into(),
        link_id: None,
        generation,
        max_frame_bytes: Limits::default().max_frame_bytes,
        request_id: request_id.into(),
        binding_id: None,
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
    let wait_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !check() {
        assert!(
            tokio::time::Instant::now() < wait_deadline,
            "condition not reached before test deadline"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

/// 标准装配：counter-ts 由 node-a 在代次 11 注册；alice 是 gen 21 消费者，
/// bob 在需要处另建 gen 22 消费者。`consumer.queued` 供慢消费者测试注水。
struct Setup {
    broker: Arc<Broker>,
    relay: Arc<StateRelay>,
    registration_id: String,
    provider: TestLink,
    consumer: TestLink,
}

async fn setup() -> Setup {
    let registry = registry();
    let relay = StateRelay::new(registry.clone());
    let broker = broker_with_state(registry.clone(), relay.clone());
    let provider = TestLink::new(11);
    relay.attach("node-a", 11, provider.link.clone()).await;
    let consumer = TestLink::new(21);
    relay.attach_consumer(21, consumer.link.clone()).await;
    let registration_id = register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;
    Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    }
}

// ---- 内存链路 ------------------------------------------------------------

/// 一侧的无 socket 链：真实 `RemoteLink`（mpsc 出站队列 + `PendingRequests`
/// 回复通道）；接收端留在本结构内，观察 API 以 try_recv 排干把已入队文本
/// 帧移进共享累积列表——不再有读者任务竞态（发送方入队先于任何 await
/// 返回，排干即观察，0 帧断言仍然有意义）。测试用 `reply`/`drop_link`
/// 确定性结算转发请求；`queued` 注水模拟慢消费者。
struct TestLink {
    link: RemoteLink,
    generation: u64,
    pending: Arc<PendingRequests>,
    queued: Arc<AtomicUsize>,
    frames: Arc<Mutex<Vec<Value>>>,
    rx: Mutex<mpsc::Receiver<(Message, usize)>>,
}

impl TestLink {
    fn new(generation: u64) -> Self {
        let pending = Arc::new(PendingRequests::new(generation));
        let queued = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel(64);
        let link = RemoteLink::new(
            generation,
            tx,
            queued.clone(),
            pending.clone(),
            1024 * 1024,
            262_144,
        );
        Self {
            link,
            generation,
            pending,
            queued,
            frames: Arc::new(Mutex::new(Vec::new())),
            rx: Mutex::new(rx),
        }
    }

    /// 把已入队 mpsc 帧同步移进累积列表（不跨 await 持锁）。
    fn drain(&self) {
        let mut rx = self.rx.lock().expect("rx poisoned");
        loop {
            match rx.try_recv() {
                Ok((message, _)) => {
                    if let Message::Text(text) = message
                        && let Ok(value) = serde_json::from_str::<Value>(&text)
                    {
                        self.frames.lock().expect("frames poisoned").push(value);
                    }
                }
                Err(_) => break,
            }
        }
    }

    fn count(&self, method: &str) -> usize {
        self.drain();
        self.frames
            .lock()
            .expect("frames poisoned")
            .iter()
            .filter(|frame| frame["method"] == method)
            .count()
    }

    async fn frames(&self) -> Vec<Value> {
        // 排干为空时让出再排干（最多两轮）：覆盖发送方在同一 await 边界
        // 稍后入队的帧；已入队的帧无论调度顺序都被观察到。
        for _ in 0..2 {
            self.drain();
            tokio::task::yield_now().await;
        }
        self.drain();
        self.frames.lock().expect("frames poisoned").clone()
    }

    async fn method_frames(&self, method: &str) -> Vec<Value> {
        self.frames()
            .await
            .into_iter()
            .filter(|frame| frame["method"] == method)
            .collect()
    }

    async fn updates(&self) -> Vec<Value> {
        self.method_frames("state/update").await
    }

    async fn ends(&self) -> Vec<Value> {
        self.method_frames("state/end").await
    }

    async fn wait_subscribe(&self) -> (String, Value) {
        self.wait_subscribe_nth(0).await
    }

    async fn wait_subscribe_nth(&self, n: usize) -> (String, Value) {
        wait_until(|| self.count("state/subscribe") > n).await;
        let frame = self
            .frames()
            .await
            .into_iter()
            .filter(|frame| frame["method"] == "state/subscribe")
            .nth(n)
            .expect("subscribe frame");
        let hop = frame["id"].as_str().expect("hop id").to_string();
        (hop, frame)
    }

    async fn wait_unsubscribe(&self) {
        wait_until(|| self.count("state/unsubscribe") > 0).await;
    }

    async fn reply(&self, hop: &str, result: CallResult<AgentReply>) -> bool {
        self.pending.route(self.generation, hop, result).await
    }

    /// 提供端断线：未决 hop 全部收到连接级失败（与 ws_transport 断线清理
    /// 同样的 `Ok(Err(Unavailable))` 语义）。
    async fn drop_link(&self) {
        self.pending.fail_all().await;
    }
}

// ---- 帧构造 helpers ------------------------------------------------------

/// 提供端成功快照回复（registrationId 必须等于活动注册，契约 §8.1）。
fn snapshot_reply(registration_id: &str) -> Value {
    json!({
        "subscriptionId": "01ARZ3NDEKTSV4RRFFQ69G5FDF",
        "registrationId": registration_id,
        "snapshot": json!({
            "revision": "0",
            "value": { "count": 0 },
            "sampledAtMs": "0",
            "quality": "current",
        }),
    })
}

fn sample(revision: &str, count: i64, quality: &str) -> Value {
    json!({
        "revision": revision,
        "value": { "count": count },
        "sampledAtMs": "0",
        "quality": quality,
    })
}

/// 提供端 state/update 通知 input（契约 §11.2）。
fn update_input(hop: &str, seq: &str, revision: &str, count: i64, quality: &str) -> Value {
    json!({
        "subscriptionId": hop,
        "seq": seq,
        "sample": sample(revision, count, quality),
    })
}

/// 提供端 state/end 通知 input（§8.3 生成 Error JSON）。
fn end_input(hop: &str, code: &str, message: &str) -> Value {
    json!({
        "subscriptionId": hop,
        "error": json!({
            "code": code,
            "message": message,
            "diagnosticId": "",
            "execution": "completed",
            "retry": if code == "ERROR_CODE_FORBIDDEN" { "never" } else { "safe" },
        }),
    })
}

/// 提供端通知上下文：注册实例归属由 registrationId/代次校验。
fn provider_context(registration_id: &str) -> conex_proto::RequestContext {
    conex_proto::RequestContext {
        provider_endpoint_id: "counter-ts".into(),
        plane: conex_proto::Plane::Broker as i32,
        binding_id: None,
        principal_id: "node-a".into(),
        tenant_id: String::new(),
        space_id: "dev".into(),
        registration_id: registration_id.into(),
    }
}

/// 测试宿主屏障：订阅在转发上游后、等待快照前阻塞（B2 快照读取边界）。
async fn hold_gate(relay: &StateRelay) -> oneshot::Sender<()> {
    let (tx, rx) = oneshot::channel();
    *relay.test_gate.lock().await = Some(rx);
    tx
}

/// 标准建立：alice（gen 21）订阅 counter/value，快照回复 + response_enqueued
/// 屏障后返回活动订阅的 hop id。`nth` 是该消费者在本提供端链上的第几个
/// state/subscribe 帧（0 起），支持同一提供端链上多订阅的顺序结算。
async fn establish_active(
    broker: &Arc<Broker>,
    relay: &Arc<StateRelay>,
    provider: &TestLink,
    registration_id: &str,
    sub_id: &str,
    req_id: &str,
    nth: usize,
) -> String {
    let task = spawn_invoke(
        broker.clone(),
        subscribe_call(
            &caller("alice"),
            21,
            req_id,
            "counter-ts",
            "dev",
            registration_id,
            "counter/value",
            sub_id,
            json!({}),
            LONG_BUDGET,
        ),
    );
    let (hop, _) = provider.wait_subscribe_nth(nth).await;
    provider
        .reply(&hop, Ok(AgentReply::Value(snapshot_reply(registration_id))))
        .await;
    let result = task.await.unwrap().unwrap();
    assert_eq!(result["snapshot"]["revision"], "0");
    assert_eq!(result["subscriptionId"], sub_id);
    relay.response_enqueued(21, req_id).await;
    hop
}

// ---- tests ----------------------------------------------------------------

/// T04 步骤 1（A02）：gate 内提供端先发更新、快照响应尚未入队——更新进入
/// opening 缓冲，绝不先于快照到达消费者；`response_enqueued` 后按序冲刷
/// （seq 1,2,3），之后的活动更新（seq 4）继续投递。
#[tokio::test]
async fn snapshot_first_then_buffered_updates_in_order() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let gate = hold_gate(&relay).await;
    let task = spawn_invoke(
        broker.clone(),
        subscribe_call(
            &caller("alice"),
            21,
            REQ_0,
            "counter-ts",
            "dev",
            &registration_id,
            "counter/value",
            SUB_0,
            json!({}),
            LONG_BUDGET,
        ),
    );
    let (hop, frame) = provider.wait_subscribe().await;
    // 转发帧：hop subscriptionId、状态名，无客户端身份。
    assert_eq!(frame["params"]["input"]["subscriptionId"], hop);
    assert_eq!(frame["params"]["input"]["state"], "counter/value");
    // 快照读取边界并发更新：gate 内 3 个更新全部进入有界 opening 缓冲。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "2", "2", 2, "current"),
        )
        .await;
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "3", "3", 3, "current"),
        )
        .await;
    // 快照未入队：消费者一个更新都收不到（B2 屏障）。
    assert!(consumer.updates().await.is_empty());
    gate.send(()).unwrap();
    // 快照回复 → 唯一成功响应；仍无更新（response_enqueued 前）。
    provider
        .reply(
            &hop,
            Ok(AgentReply::Value(snapshot_reply(&registration_id))),
        )
        .await;
    let result = task.await.unwrap().unwrap();
    assert_eq!(result["snapshot"]["revision"], "0");
    assert_eq!(result["snapshot"]["value"], json!({ "count": 0 }));
    assert_eq!(result["subscriptionId"], SUB_0);
    assert!(consumer.updates().await.is_empty());
    // 成功响应已入队 → 冲刷缓冲更新，快照之后按序交付。
    relay.response_enqueued(21, REQ_0).await;
    let updates = consumer.updates().await;
    assert_eq!(updates.len(), 3);
    for (i, update) in updates.iter().enumerate() {
        let expected_seq = i + 1;
        assert_eq!(
            update["params"]["input"]["seq"],
            expected_seq.to_string(),
            "buffered updates must flush in seq order"
        );
        assert_eq!(update["params"]["input"]["subscriptionId"], SUB_0);
    }
    // 活动订阅继续：seq 4 直接投递。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "4", "4", 4, "current"),
        )
        .await;
    assert_eq!(consumer.updates().await.len(), 4);
    assert_eq!(relay.subscriptions_len().await, 1);
    relay.consumer_disconnected(21).await;
    assert_eq!(relay.subscriptions_len().await, 0);
}

/// T04 步骤 2（§8.1）：opening 超时 → 清理映射 + 上游 unsubscribe；迟到
/// 快照不复活订阅，也不产生任何消费者交付。
#[tokio::test]
async fn opening_timeout_cleans_up_and_late_snapshot_does_not_revive() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let task = spawn_invoke(
        broker.clone(),
        subscribe_call(
            &caller("alice"),
            21,
            REQ_0,
            "counter-ts",
            "dev",
            &registration_id,
            "counter/value",
            SUB_0,
            json!({}),
            Duration::from_millis(60),
        ),
    );
    let (hop, _) = provider.wait_subscribe().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Timeout));
    assert_eq!(error.wire().execution, "unknown");
    assert_eq!(error.wire().retry, "safe");
    provider.wait_unsubscribe().await;
    assert_eq!(relay.subscriptions_len().await, 0);
    assert!(consumer.updates().await.is_empty());
    // 迟到快照：pending 已被 cancel，route 返回 false；不复活。
    assert!(
        !provider
            .reply(
                &hop,
                Ok(AgentReply::Value(snapshot_reply(&registration_id)))
            )
            .await
    );
    assert_eq!(relay.subscriptions_len().await, 0);
    assert!(consumer.frames().await.is_empty());
}

/// T04 步骤 2（§8.1）：快照返回前 unsubscribe → 原 subscribe 终结为
/// cancelled/not_started，上游收到 state/unsubscribe，订阅不激活、迟到
/// 快照不复活。
#[tokio::test]
async fn opening_cancel_settles_cancelled_and_never_activates() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let gate = hold_gate(&relay).await;
    let task = spawn_invoke(
        broker.clone(),
        subscribe_call(
            &caller("alice"),
            21,
            REQ_0,
            "counter-ts",
            "dev",
            &registration_id,
            "counter/value",
            SUB_0,
            json!({}),
            LONG_BUDGET,
        ),
    );
    let (hop, _) = provider.wait_subscribe().await;
    // opening 期间取消：唯一 unsubscribe 响应。
    let removed = broker
        .invoke(unsubscribe_call(&caller("alice"), 21, REQ_1, SUB_0))
        .await
        .unwrap();
    assert_eq!(removed, json!({ "removed": true }));
    gate.send(()).unwrap();
    // 原 subscribe 以 cancelled/not_started 终结（契约 §8.1，非 unavailable）。
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Cancelled));
    assert_eq!(error.wire().execution, "not_started");
    assert_eq!(error.wire().retry, "never");
    provider.wait_unsubscribe().await;
    assert_eq!(relay.subscriptions_len().await, 0);
    assert!(consumer.frames().await.is_empty());
    // 迟到快照不得复活（pending 已由 cancel 移除）。
    assert!(
        !provider
            .reply(
                &hop,
                Ok(AgentReply::Value(snapshot_reply(&registration_id)))
            )
            .await
    );
    assert_eq!(relay.subscriptions_len().await, 0);
    assert!(consumer.frames().await.is_empty());
}

/// T04 步骤 2（§8.1/A08）：提供端连接在 opening 期失联 → subscribe 结算
/// unavailable/unknown/safe，清理映射并尽力向上游发送 unsubscribe。
#[tokio::test]
async fn opening_link_loss_settles_unavailable_and_releases() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let task = spawn_invoke(
        broker.clone(),
        subscribe_call(
            &caller("alice"),
            21,
            REQ_0,
            "counter-ts",
            "dev",
            &registration_id,
            "counter/value",
            SUB_0,
            json!({}),
            LONG_BUDGET,
        ),
    );
    let (_, _) = provider.wait_subscribe().await;
    provider.drop_link().await;
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Unavailable));
    assert_eq!(error.wire().execution, "unknown");
    assert_eq!(error.wire().retry, "safe");
    // establishment_failed 尽力发上游 unsubscribe（链接仍登记在 relay）。
    provider.wait_unsubscribe().await;
    assert_eq!(relay.subscriptions_len().await, 0);
    assert!(consumer.frames().await.is_empty());
}

/// T04 步骤 2（§8.1）：快照成功响应已决定但未入队时 unsubscribe —— 保留
/// 唯一成功响应，不激活订阅（response_enqueued 无对应映射，不送端），
/// 上游释放，之后无任何投递。
#[tokio::test]
async fn unsubscribe_after_snapshot_decided_keeps_single_response() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let gate = hold_gate(&relay).await;
    let task = spawn_invoke(
        broker.clone(),
        subscribe_call(
            &caller("alice"),
            21,
            REQ_0,
            "counter-ts",
            "dev",
            &registration_id,
            "counter/value",
            SUB_0,
            json!({}),
            LONG_BUDGET,
        ),
    );
    let (hop, _) = provider.wait_subscribe().await;
    gate.send(()).unwrap();
    // 快照回复 → 唯一成功响应已决定（尚未入队）。
    provider
        .reply(
            &hop,
            Ok(AgentReply::Value(snapshot_reply(&registration_id))),
        )
        .await;
    let result = task.await.unwrap().unwrap();
    assert_eq!(result["snapshot"]["revision"], "0");
    assert_eq!(result["subscriptionId"], SUB_0);
    // 入队前 unsubscribe：响应保留，但订阅不再激活。
    let removed = broker
        .invoke(unsubscribe_call(&caller("alice"), 21, REQ_1, SUB_0))
        .await
        .unwrap();
    assert_eq!(removed, json!({ "removed": true }));
    relay.response_enqueued(21, REQ_0).await;
    provider.wait_unsubscribe().await;
    assert_eq!(relay.subscriptions_len().await, 0);
    assert!(consumer.frames().await.is_empty());
    // 迟到的更新帧按不存在归属丢弃。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert!(consumer.frames().await.is_empty());
    assert_eq!(relay.subscriptions_len().await, 0);
}

/// T04 步骤 3（A10）：活动订阅的 seq 必须从 1 开始且连续；缺口/首帧非 1
/// 以 session_lost 终结该订阅，不合并、不静默丢弃。
#[tokio::test]
async fn active_seq_must_start_at_one_and_be_contiguous() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    // 首帧 seq=2（应从 1 开始）。
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "2", "1", 1, "current"),
        )
        .await;
    let ends = consumer.ends().await;
    assert_eq!(ends.len(), 1);
    assert_eq!(
        ends[0]["params"]["input"]["error"]["code"],
        "ERROR_CODE_SESSION_LOST"
    );
    assert_eq!(
        ends[0]["params"]["input"]["error"]["execution"],
        "completed"
    );
    assert_eq!(ends[0]["params"]["input"]["error"]["retry"], "safe");
    assert_eq!(ends[0]["params"]["input"]["subscriptionId"], SUB_0);
    assert_eq!(relay.subscriptions_len().await, 0);
    // 终结后继续发帧：不再投递。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert_eq!(consumer.updates().await.len(), 0);
    // 正常 seq1 后出现缺口 seq3。
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_1,
        REQ_1,
        1,
    )
    .await;
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert_eq!(consumer.updates().await.len(), 1);
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "3", "2", 2, "current"),
        )
        .await;
    assert_eq!(consumer.ends().await.len(), 2);
    assert_eq!(relay.subscriptions_len().await, 0);
}

/// T04 步骤 3（A10）：revision 严格递增；重复/倒退即 session_lost。
#[tokio::test]
async fn active_revision_must_increase_strictly() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert_eq!(consumer.updates().await.len(), 1);
    // 同 revision 重放：session_lost，不静默去重。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "2", "1", 2, "current"),
        )
        .await;
    let ends = consumer.ends().await;
    assert_eq!(ends.len(), 1);
    assert_eq!(
        ends[0]["params"]["input"]["error"]["code"],
        "ERROR_CODE_SESSION_LOST"
    );
    assert_eq!(relay.subscriptions_len().await, 0);
    assert_eq!(consumer.updates().await.len(), 1);
}

/// T04 步骤 3（A10）：sample 结构损坏或 quality 非法即以 session_lost 终结。
#[tokio::test]
async fn malformed_sample_or_bad_quality_ends_session() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    // sample 缺 revision。
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    let missing_revision = json!({
        "subscriptionId": hop,
        "seq": "1",
        "sample": json!({
            "value": { "count": 1 },
            "sampledAtMs": "0",
            "quality": "current",
        }),
    });
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &missing_revision,
        )
        .await;
    assert_eq!(consumer.ends().await.len(), 1);
    assert_eq!(relay.subscriptions_len().await, 0);
    // quality 非 current/stale。
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_1,
        REQ_1,
        1,
    )
    .await;
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "fresh"),
        )
        .await;
    let ends = consumer.ends().await;
    assert_eq!(ends.len(), 2);
    assert_eq!(
        ends[1]["params"]["input"]["error"]["code"],
        "ERROR_CODE_SESSION_LOST"
    );
    assert_eq!(relay.subscriptions_len().await, 0);
    assert_eq!(consumer.updates().await.len(), 0);
}

/// T04 步骤 3（A12）：sample.value 违反契约 schema 即以 session_lost 终结。
#[tokio::test]
async fn value_schema_violation_ends_session() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    let schema_violation = json!({
        "subscriptionId": hop,
        "seq": "1",
        "sample": json!({
            "revision": "1",
            "value": { "count": "not-an-int" },
            "sampledAtMs": "0",
            "quality": "current",
        }),
    });
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &schema_violation,
        )
        .await;
    let ends = consumer.ends().await;
    assert_eq!(ends.len(), 1);
    assert_eq!(
        ends[0]["params"]["input"]["error"]["code"],
        "ERROR_CODE_SESSION_LOST"
    );
    assert_eq!(relay.subscriptions_len().await, 0);
    assert!(consumer.updates().await.is_empty());
}

/// T04 步骤 3（A10）：无法可信归属的通知按不可信帧丢弃，不创建订阅：
/// 错误代次、伪造 subscriptionId、注册实例不符全都忽略，订阅存活。
#[tokio::test]
async fn untrusted_generation_or_forged_subscription_id_ignored() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    // 错误提供端代次（非 11）：丢弃，订阅存活。
    relay
        .on_notification(
            99,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert_eq!(relay.subscriptions_len().await, 1);
    assert!(consumer.frames().await.is_empty());
    // 伪造（未登记）subscriptionId：丢弃，不创建订阅。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(SUB_5, "1", "1", 1, "current"),
        )
        .await;
    assert_eq!(relay.subscriptions_len().await, 1);
    assert!(consumer.frames().await.is_empty());
    // 归属代次正确但注册实例不符：丢弃。
    let other_context = {
        let mut context = provider_context(&registration_id);
        context.registration_id = SUB_5.to_string();
        context
    };
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &other_context,
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert_eq!(relay.subscriptions_len().await, 1);
    assert!(consumer.frames().await.is_empty());
    // 可信更新照常投递。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    let updates = consumer.updates().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["params"]["input"]["seq"], "1");
    assert_eq!(relay.subscriptions_len().await, 1);
    relay.consumer_disconnected(21).await;
    assert_eq!(relay.subscriptions_len().await, 0);
}

/// T04 步骤 4（A08）：主动 unsubscribe 与提供端 state/end 各自恰好一次
/// 终结；成功取消（{removed:true}，无 end）与异常终止（end 带固定终止码）
/// 可区分；终结后不再投递更新。
#[tokio::test]
async fn unsubscribe_and_provider_end_each_deliver_exactly_one_terminal() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    // 提供端主动 end（slow_consumer 码）→ 恰好一个 end，无更新续发。
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_END,
            &provider_context(&registration_id),
            &end_input(&hop, "ERROR_CODE_SLOW_CONSUMER", "provider gave up"),
        )
        .await;
    let ends = consumer.ends().await;
    assert_eq!(ends.len(), 1);
    assert_eq!(
        ends[0]["params"]["input"]["error"]["code"],
        "ERROR_CODE_SLOW_CONSUMER"
    );
    assert_eq!(
        ends[0]["params"]["input"]["error"]["execution"],
        "completed"
    );
    assert_eq!(ends[0]["params"]["input"]["error"]["retry"], "safe");
    assert_eq!(ends[0]["params"]["input"]["subscriptionId"], SUB_0);
    assert_eq!(relay.subscriptions_len().await, 0);
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert!(consumer.updates().await.is_empty());
    // 主动 unsubscribe：{removed:true}，无 end（成功取消由响应确认）。
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_1,
        REQ_1,
        1,
    )
    .await;
    let ends_before = consumer.ends().await.len();
    let removed = broker
        .invoke(unsubscribe_call(&caller("alice"), 21, REQ_2, SUB_1))
        .await
        .unwrap();
    assert_eq!(removed, json!({ "removed": true }));
    provider.wait_unsubscribe().await;
    assert_eq!(
        consumer.ends().await.len(),
        ends_before,
        "unsubscribe 不送 end"
    );
    assert_eq!(relay.subscriptions_len().await, 0);
    // 未知/已结束 ID → {removed:false}。
    let removed = broker
        .invoke(unsubscribe_call(&caller("alice"), 21, REQ_3, SUB_1))
        .await
        .unwrap();
    assert_eq!(removed, json!({ "removed": false }));
    // 终结后更新不复活投递。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert!(consumer.updates().await.is_empty());
}

/// T04 步骤 5（A08）·消费者断线：释放全部订阅，host 不再投递（SDK 本地
/// 终结），预算可被后续连接复用。
#[tokio::test]
async fn consumer_disconnect_releases_all_subscriptions() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let _ = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    let _ = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_1,
        REQ_1,
        1,
    )
    .await;
    assert_eq!(relay.subscriptions_len().await, 2);
    relay.consumer_disconnected(21).await;
    assert_eq!(relay.subscriptions_len().await, 0);
    // 断开后更新帧按不存在归属丢弃（无消费者链可投递）。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop_of(&provider, 0).await, "1", "1", 1, "current"),
        )
        .await;
    assert_eq!(relay.subscriptions_len().await, 0);
    // 同代次重新接入后预算恢复：新订阅可建立。
    let new_consumer = TestLink::new(21);
    relay.attach_consumer(21, new_consumer.link.clone()).await;
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_2,
        REQ_2,
        2,
    )
    .await;
    assert_eq!(relay.subscriptions_len().await, 1);
    relay.consumer_disconnected(21).await;
    assert_eq!(relay.subscriptions_len().await, 0);
    let _ = hop;
    let _ = consumer;
}

/// 取测试中第 n 个订阅帧的 hop id（供断开后向旧 hop 发帧用）。
async fn hop_of(provider: &TestLink, n: usize) -> String {
    provider.wait_subscribe_nth(n).await.0
}

/// T04 步骤 5（A08）·提供端失联：detach 以 unavailable 终结其托管订阅并
/// 释放映射；后续更新不再投递。
#[tokio::test]
async fn provider_detach_ends_unavailable() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let hop = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    relay.detach("node-a", 11).await;
    let ends = consumer.ends().await;
    assert_eq!(ends.len(), 1);
    assert_eq!(
        ends[0]["params"]["input"]["error"]["code"],
        "ERROR_CODE_UNAVAILABLE"
    );
    assert_eq!(ends[0]["params"]["input"]["error"]["retry"], "safe");
    assert_eq!(relay.subscriptions_len().await, 0);
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert!(consumer.updates().await.is_empty());
}

/// T04 步骤 5（A08）·注册代次替换：首个新代次注册终结旧代次订阅
/// （stale_revision），对同代次/更新代次注册过滤，绝不误伤新订阅。
#[tokio::test]
async fn generation_replacement_ends_stale_and_spares_new() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let hop_old = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    // 新连接代次 12 原子激活：旧代次订阅以 stale_revision 终结。
    let provider_new = TestLink::new(12);
    relay.attach("node-a", 12, provider_new.link.clone()).await;
    let registration_new = register_ok(&broker, &caller("node-a"), 12, "counter-ts").await;
    let ends = consumer.ends().await;
    assert_eq!(ends.len(), 1);
    assert_eq!(
        ends[0]["params"]["input"]["error"]["code"],
        "ERROR_CODE_STALE_REVISION"
    );
    assert_eq!(ends[0]["params"]["input"]["error"]["retry"], "safe");
    assert_eq!(relay.subscriptions_len().await, 0);
    // 新注册建立订阅；同代次 end_stale_generation(12) 过滤 <12，无害。
    let hop_new = establish_active(
        &broker,
        &relay,
        &provider_new,
        &registration_new,
        SUB_1,
        REQ_1,
        0,
    )
    .await;
    relay.end_stale_generation("node-a", 12).await;
    assert_eq!(relay.subscriptions_len().await, 1);
    assert_eq!(consumer.ends().await.len(), 1, "同代次注册不终结新订阅");
    // 更新代次 13 才终结新订阅。
    relay.end_stale_generation("node-a", 13).await;
    assert_eq!(consumer.ends().await.len(), 2);
    assert_eq!(relay.subscriptions_len().await, 0);
    relay
        .on_notification(
            12,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_new),
            &update_input(&hop_new, "1", "1", 1, "current"),
        )
        .await;
    assert!(consumer.updates().await.is_empty());
    let _ = (hop_old, provider);
}

/// T04 步骤 5（A08）·模块撤销：end_registration 以 unavailable 终结该注册
/// 的全部订阅并释放。
#[tokio::test]
async fn registration_end_ends_subscriptions() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let _ = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    let _ = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_1,
        REQ_1,
        1,
    )
    .await;
    relay.end_registration(&registration_id).await;
    let ends = consumer.ends().await;
    assert_eq!(ends.len(), 2);
    for end in &ends {
        assert_eq!(
            end["params"]["input"]["error"]["code"],
            "ERROR_CODE_UNAVAILABLE"
        );
        assert_eq!(end["params"]["input"]["error"]["retry"], "safe");
    }
    assert_eq!(relay.subscriptions_len().await, 0);
}

/// T04 步骤 5（A08）·授权撤销：end_authorization 只终结该主体的订阅
/// （forbidden/retry=never），不影响其他主体；全部撤销后归零。
#[tokio::test]
async fn authorization_withdrawal_ends_forbidden_only_owner() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let _ = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    // bob 独立消费者（gen 22）。
    let bob = TestLink::new(22);
    relay.attach_consumer(22, bob.link.clone()).await;
    let task = spawn_invoke(
        broker.clone(),
        subscribe_call(
            &caller("bob"),
            22,
            REQ_5,
            "counter-ts",
            "dev",
            &registration_id,
            "counter/value",
            SUB_5,
            json!({}),
            LONG_BUDGET,
        ),
    );
    let (hop_bob, _) = provider.wait_subscribe_nth(1).await;
    provider
        .reply(
            &hop_bob,
            Ok(AgentReply::Value(snapshot_reply(&registration_id))),
        )
        .await;
    let result = task.await.unwrap().unwrap();
    assert_eq!(result["subscriptionId"], SUB_5);
    relay.response_enqueued(22, REQ_5).await;
    assert_eq!(relay.subscriptions_len().await, 2);
    // 撤销 alice：只终结 alice 的订阅，forbidden/retry=never。
    relay.end_authorization("alice").await;
    let alice_ends = consumer.ends().await;
    assert_eq!(alice_ends.len(), 1);
    assert_eq!(
        alice_ends[0]["params"]["input"]["error"]["code"],
        "ERROR_CODE_FORBIDDEN"
    );
    assert_eq!(alice_ends[0]["params"]["input"]["error"]["retry"], "never");
    assert_eq!(relay.subscriptions_len().await, 1);
    assert!(bob.updates().await.is_empty());
    // bob 订阅仍活动。
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop_bob, "1", "1", 1, "current"),
        )
        .await;
    assert_eq!(bob.updates().await.len(), 1);
    // 撤销 bob → 全部归零。
    relay.end_authorization("bob").await;
    assert_eq!(relay.subscriptions_len().await, 0);
}

/// T04 步骤 6（A09）：with_max 小订阅预算在建立前显式拒绝溢出
/// （quota_exceeded/not_started），不发提供端帧、不静默丢；退订释放预算；
/// 消费者断线后计数归零。
#[tokio::test]
async fn quota_rejects_overflow_and_cleanup_returns_to_zero() {
    let registry = registry();
    let relay = StateRelay::with_max(registry.clone(), 2, 100);
    let broker = broker_with_state(registry.clone(), relay.clone());
    let provider = TestLink::new(11);
    relay.attach("node-a", 11, provider.link.clone()).await;
    let consumer = TestLink::new(21);
    relay.attach_consumer(21, consumer.link.clone()).await;
    let registration_id = register_ok(&broker, &caller("node-a"), 11, "counter-ts").await;
    // 占满 2 个预算。
    let _ = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_0,
        REQ_0,
        0,
    )
    .await;
    let _ = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_1,
        REQ_1,
        1,
    )
    .await;
    assert_eq!(relay.subscriptions_len().await, 2);
    // 溢出显式拒绝：发送前 quota_exceeded/not_started，无新的提供端帧。
    let error = broker
        .invoke(subscribe_call(
            &caller("alice"),
            21,
            REQ_2,
            "counter-ts",
            "dev",
            &registration_id,
            "counter/value",
            SUB_2,
            json!({}),
            LONG_BUDGET,
        ))
        .await
        .unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::QuotaExceeded)
    );
    assert_eq!(error.wire().execution, "not_started");
    assert_eq!(error.wire().retry, "never");
    assert_eq!(provider.count("state/subscribe"), 2);
    assert_eq!(relay.subscriptions_len().await, 2);
    // 退订释放预算：新订阅可建立。
    broker
        .invoke(unsubscribe_call(&caller("alice"), 21, REQ_3, SUB_1))
        .await
        .unwrap();
    let _ = establish_active(
        &broker,
        &relay,
        &provider,
        &registration_id,
        SUB_2,
        REQ_4,
        2,
    )
    .await;
    assert_eq!(relay.subscriptions_len().await, 2);
    // 消费者断线：计数归零。
    relay.consumer_disconnected(21).await;
    assert_eq!(relay.subscriptions_len().await, 0);
    let _ = consumer;
}

/// T04 步骤 6（A09）：opening 窗口缓冲计入消费者队列预算；超限建立失败，
/// 以 slow_consumer/not_started 终结（而非静默丢帧或返成功后丢更新），
/// 上游观察者被释放，计数归零。
#[tokio::test]
async fn opening_buffer_overflow_fails_with_slow_consumer() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let gate = hold_gate(&relay).await;
    let task = spawn_invoke(
        broker.clone(),
        subscribe_call(
            &caller("alice"),
            21,
            REQ_0,
            "counter-ts",
            "dev",
            &registration_id,
            "counter/value",
            SUB_0,
            json!({}),
            LONG_BUDGET,
        ),
    );
    let (hop, _) = provider.wait_subscribe().await;
    // 消费者队列满到仅剩一帧：opening 缓冲超限。
    let watermark = consumer.link.state_data_watermark();
    consumer.queued.store(watermark - 1, Ordering::Release);
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    // 建立失败：subscribe 以 slow_consumer/not_started 终结。
    gate.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::SlowConsumer)
    );
    assert_eq!(error.wire().execution, "not_started");
    assert_eq!(error.wire().retry, "never");
    provider.wait_unsubscribe().await;
    assert_eq!(relay.subscriptions_len().await, 0);
    assert!(consumer.frames().await.is_empty(), "建立失败不投递任何帧");
}

/// T04 步骤 6（A09）·灰色窗口：快照成功响应已决定但未入队时缓冲超限——
/// 保留唯一成功响应，`response_enqueued` 在快照之后顺次交付 slow_consumer
/// end（保序，不静默丢窗口更新），计数归零。
#[tokio::test]
async fn grey_window_overflow_delivers_end_after_snapshot() {
    let Setup {
        broker,
        relay,
        registration_id,
        provider,
        consumer,
    } = setup().await;
    let gate = hold_gate(&relay).await;
    let task = spawn_invoke(
        broker.clone(),
        subscribe_call(
            &caller("alice"),
            21,
            REQ_0,
            "counter-ts",
            "dev",
            &registration_id,
            "counter/value",
            SUB_0,
            json!({}),
            LONG_BUDGET,
        ),
    );
    let (hop, _) = provider.wait_subscribe().await;
    gate.send(()).unwrap();
    // 成功响应已决定（subscribe 将返回 Ok），但尚未入队。
    provider
        .reply(
            &hop,
            Ok(AgentReply::Value(snapshot_reply(&registration_id))),
        )
        .await;
    let result = task.await.unwrap().unwrap();
    assert_eq!(result["snapshot"]["revision"], "0");
    // 灰色窗口内窗口更新超限：end 等 response_enqueued 后顺次交付。
    let watermark = consumer.link.state_data_watermark();
    consumer.queued.store(watermark - 1, Ordering::Release);
    relay
        .on_notification(
            11,
            conex_proto::capability::METHOD_STATE_UPDATE,
            &provider_context(&registration_id),
            &update_input(&hop, "1", "1", 1, "current"),
        )
        .await;
    assert!(
        consumer.updates().await.is_empty(),
        "更新不静默丢弃也不提前投递"
    );
    relay.response_enqueued(21, REQ_0).await;
    let ends = consumer.ends().await;
    assert_eq!(ends.len(), 1);
    assert_eq!(
        ends[0]["params"]["input"]["error"]["code"],
        "ERROR_CODE_SLOW_CONSUMER"
    );
    assert_eq!(
        ends[0]["params"]["input"]["error"]["execution"],
        "completed"
    );
    assert!(consumer.updates().await.is_empty());
    assert_eq!(relay.subscriptions_len().await, 0);
}

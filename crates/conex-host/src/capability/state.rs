//! S1 状态中继（docs/contracts/s1-capabilities.md §8-§9；docs/plans/
//! 2026-10-17-conex-s1.md T04）：`state/subscribe`、`state/unsubscribe`、
//! `state/update`、`state/end`。
//!
//! 规则：
//! - 订阅建立先分配 opening 记录与每跳 subscriptionId；上游成功快照经
//!   `response_enqueued`（快照响应已入消费者 socket 队列，B2 屏障）后才
//!   放行该订阅的 update。两跳都只在 opening 态持有有界缓存，建立期间
//!   超限即失败并清理上游订阅，绝不返回成功后静默丢弃窗口内更新。
//! - opening 失败/超时/取消/失联立即清理映射并发上游 `state/unsubscribe`；
//!   迟到快照/更新不得复活订阅。快照已入队与 unsubscribe 的竞态保留唯一
//!   成功响应，但不再激活订阅。
//! - 校验提供端代次、seq 从 1 连续、revision 严格递增、sample 结构与
//!   quality、sample.value 契约 schema；任何违例显式以 session_lost 结束
//!   该订阅，不合并或静默丢弃。
//! - 实现 unsubscribe、end 及断线/撤销/换代/撤权清理；成功取消由
//!   `unsubscribe` 响应确认，异常终止由 `state/end` 携带固定终止码：
//!   提供端失联/撤销 → unavailable、换代 → stale_revision、撤权 →
//!   forbidden、队列超限 → slow_consumer、序列失序 → session_lost。
//! - opening 缓冲与 socket 待发数据计入 maxQueuedBytes（数据水位 =
//!   max_queued - max_frame）；控制字节预留 = max_frame，控制空间打满时
//!   关闭该 Link 并终止其全部剩余订阅（契约 §9）。
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use conex_core::CallError;
use conex_core::capability::validate_state_value;
use conex_core::{CallResult, Caller};
use conex_proto;
use conex_proto::capability::{
    METHOD_STATE_END, METHOD_STATE_SUBSCRIBE, METHOD_STATE_UNSUBSCRIBE, METHOD_STATE_UPDATE,
    is_canonical_uint64, state_end_input, state_sample_from_json, state_update_input,
    validate_result,
};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::capability::ModuleRecord;
use crate::capability::ModuleRegistry;
use crate::capability::calls::{
    cancelled_not_started, conflict, control_message, forbidden, internal, not_started,
    remaining_budget_ms, stale_revision, stamped_context,
};
use crate::capability::registry::new_registration_id;
use crate::remote::RemoteLink;
use crate::ws_transport::AgentReply;

/// 契约 §9 建议的每 Link 同时 opening/active 的订阅数上限。
pub const DEFAULT_MAX_SUBSCRIPTIONS: usize = 64;

/// 状态中继自身的全局表闸。并发订阅本受每 Link 限额约束，这里是总闸，
/// 防失控增长（与 `CallRouter::DEFAULT_MAX_PENDING` 同思路）。
pub const DEFAULT_MAX_OPENINGS: usize = 8192;

/// 每个订阅恰好一次终结：成功响应或 `state/end` 通知（契约 §8.3）。
#[derive(Debug, Clone)]
struct Subscription {
    consumer_generation: u64,
    consumer_subscription_id: String,
    consumer_request_id: String,
    principal_id: String,
    target_node: String,
    target_generation: u64,
    /// 上游（提供端）转发上下文：中心盖章身份 + 注册实例。
    upstream_context: conex_proto::RequestContext,
    /// 下游（消费者）交付上下文：注册身份，不带任何端身份字段。
    delivery_context: conex_proto::RequestContext,
    contract_id: String,
    state_name: String,
    registration_id: String,
    /// 快照成功响应尚未入消费者队列（B2 屏障期间）。
    opening: bool,
    /// 消费者已 unsubscribe（opening 期间取消）。
    cancelled: bool,
    /// 灰色窗口溢出：响应已决定但未入队时缓冲超限，待 response_enqueued
    /// 在成功响应之后交付 slow_consumer end（保序，不静默丢帧）。
    pending_end: Option<conex_proto::Error>,
    /// 已接受的 revision（快照或最后更新）。
    last_revision: u64,
    /// 下一条应有 seq；快照后从 1 开始。
    next_seq: u64,
    /// opening 窗口的有界更新缓冲（seq/revision 验收推迟到快照已知）。
    buffer: Vec<BufferedUpdate>,
    buffered_bytes: usize,
}

#[derive(Debug, Clone)]
struct BufferedUpdate {
    seq: u64,
    revision: u64,
    message: conex_proto::Message,
}

#[derive(Clone)]
struct ConsumerState {
    /// 消费者交付链：快照响应之后的 update/end 走它的 socket 队列。
    link: RemoteLink,
    /// opening + active 订阅数（每 Link 限额）。
    active: usize,
    max_subscriptions: usize,
}

#[derive(Default)]
struct StateInner {
    /// hop subscriptionId（提供端可见）→ 订阅。
    by_hop: HashMap<String, Subscription>,
    /// (消费者代次, 消费者 subscriptionId) → hop id。
    by_consumer: HashMap<(u64, String), String>,
    /// (消费者代次, 消费者请求 requestId) → hop id（response_enqueued）。
    by_request: HashMap<(u64, String), String>,
    /// 提供端 hop 链：(node, 连接代次) → 链。
    links: HashMap<(String, u64), RemoteLink>,
    /// 消费者交付链：连接代次 → 链 + 限额记账。
    consumers: HashMap<u64, ConsumerState>,
    /// opening 期本方主动取消 pending（unsubscribe、缓冲溢出）时待 settle
    /// 的错误：subscribe future 从 oneshot 的 RecvError 醒来后消费，保证
    /// 建立期取消/溢出按契约终结（cancelled/not_started、slow_consumer/
    /// not_started），不被 pending 通道断开吞成 unavailable。hop id 全局
    /// 唯一，grey 窗口（成功响应已决定后溢出）遗留的条目惰性无害。
    settled: HashMap<String, CallError>,
}

pub struct StateRelay {
    registry: Arc<ModuleRegistry>,
    max_subscriptions: usize,
    max_openings: usize,
    inner: Mutex<StateInner>,
    /// 测试宿主屏障（docs/plans T04 步骤 6 前半）：订阅在转发上游后、等待
    /// 快照前等待该通知，使“快照读取边界并发更新”可确定复现。仅测试代码
    /// 设置，生产恒 None。不属于正式能力契约，也不进 SDK。
    #[doc(hidden)]
    pub test_gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl StateRelay {
    pub fn new(registry: Arc<ModuleRegistry>) -> Arc<Self> {
        Self::with_max(registry, DEFAULT_MAX_SUBSCRIPTIONS, DEFAULT_MAX_OPENINGS)
    }

    /// 测试宿主专用构造（docs/plans T04 步骤 6 有界性）：以限小订阅预算
    /// 确定性覆盖饱和拒绝。生产路径恒用 [`StateRelay::new`]。
    #[doc(hidden)]
    pub fn with_max(
        registry: Arc<ModuleRegistry>,
        max_subscriptions: usize,
        max_openings: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            registry,
            max_subscriptions,
            max_openings,
            inner: Mutex::new(StateInner::default()),
            test_gate: Mutex::new(None),
        })
    }

    /// 连接建立时登记提供端 hop 链（T04 与 T03 CallRouter 同源同点登记）。
    pub async fn attach(&self, node_id: &str, generation: u64, link: RemoteLink) {
        self.inner
            .lock()
            .await
            .links
            .insert((node_id.to_owned(), generation), link);
    }

    /// 连接建立时登记消费者交付链（同一连接可以既提供又消费）。
    pub async fn attach_consumer(&self, generation: u64, link: RemoteLink) {
        self.inner.lock().await.consumers.insert(
            generation,
            ConsumerState {
                link,
                active: 0,
                max_subscriptions: self.max_subscriptions,
            },
        );
    }

    pub(crate) async fn configure_consumer(&self, generation: u64, max_subscriptions: u32) {
        if let Some(consumer) = self.inner.lock().await.consumers.get_mut(&generation) {
            consumer.max_subscriptions = (max_subscriptions as usize).min(self.max_subscriptions);
        }
    }

    /// 提供端连接断开（代次精确）：移除 hop 链，并以 unavailable 终结该
    /// 代次托管的所有订阅（向各自消费者交付 end）。
    pub async fn detach(&self, node_id: &str, generation: u64) {
        let hops: Vec<String> = {
            let mut inner = self.inner.lock().await;
            inner.links.remove(&(node_id.to_owned(), generation));
            inner
                .by_hop
                .iter()
                .filter(|(_, sub)| {
                    sub.target_node == node_id && sub.target_generation == generation
                })
                .map(|(hop, _)| hop.clone())
                .collect()
        };
        for hop in hops {
            self.terminate(
                &hop,
                end_error(
                    conex_proto::ErrorCode::Unavailable,
                    "provider link closed; subscription terminated",
                ),
            )
            .await;
        }
    }

    /// 消费者连接断开：释放其全部订阅与交付链（SDK 本地终结，host 不再
    /// 投递）。提供端 hop 上的未决回复由各链路 own pending 清理。
    pub async fn consumer_disconnected(&self, generation: u64) {
        self.drop_consumer(generation).await;
    }

    /// 测试断言用：当前订阅记录数（有界性检查）。
    pub async fn subscriptions_len(&self) -> usize {
        self.inner.lock().await.by_hop.len()
    }

    /// 消费端 `state/subscribe`（契约 §8.1）。判定全部来自受信注册记录与
    /// 预安装契约；转发帧带中心盖章身份。
    #[allow(clippy::too_many_arguments)]
    pub async fn subscribe(
        &self,
        caller: &Caller,
        generation: u64,
        request_id: &str,
        provider_endpoint_id: &str,
        space_id: &str,
        registration_id: &str,
        state_name: &str,
        consumer_subscription_id: String,
        argument: Value,
        deadline: Instant,
    ) -> CallResult<Value> {
        // 1. 目标记录：受信源（同 T03，权限先于披露）。
        let record = self
            .registry
            .active_record(space_id, provider_endpoint_id)
            .ok_or_else(|| {
                forbidden("module is not registered or not visible to this principal")
            })?;
        let contract = self
            .registry
            .contracts()
            .get(&record.contract_id)
            .ok_or_else(|| internal("contract for active registration is not installed"))?;
        let visible = self
            .registry
            .approvals()
            .visible(
                &caller.principal_id,
                &record.space_id,
                &record.provider_endpoint_id,
                &record.contract_id,
                &contract,
            )
            .ok_or_else(|| {
                forbidden("consume approval is missing for this module or the caller")
            })?;
        if record.registration_id != registration_id {
            return Err(stale_revision(
                "registrationId is not the current registration",
            ));
        }
        // 2. 状态成员 + 消费者对该状态的授权投影。
        if contract.state(state_name).is_none() {
            return Err(not_started(
                conex_proto::ErrorCode::UnknownMethod,
                format!(
                    "contract {} has no state {state_name}",
                    contract.contract_id
                ),
            ));
        }
        if !visible.1.iter().any(|name| name == state_name) {
            return Err(forbidden(format!(
                "caller is not approved for contract state {state_name}"
            )));
        }
        // 3. 发送前预算（只约束建立，不给成功订阅设置寿命）。
        let remaining = remaining_budget_ms(deadline)?;
        // 4. 提供端链：固定到记录代次。
        let link = self
            .inner
            .lock()
            .await
            .links
            .get(&(record.node_id.clone(), record.generation))
            .cloned()
            .ok_or_else(|| {
                CallError::new(
                    conex_proto::ErrorCode::Unavailable,
                    "provider link is offline",
                )
                .with_execution("not_started")
                .with_retry("safe")
            })?;
        // 5. opening 记录 + 每跳 subscriptionId（有界）。
        let hop = new_registration_id();
        let upstream_context = stamped_context(caller, &record);
        let delivery_context = registration_context(&record);
        {
            let mut inner = self.inner.lock().await;
            if inner
                .by_consumer
                .contains_key(&(generation, consumer_subscription_id.clone()))
            {
                return Err(conflict("concurrent duplicate subscriptionId on this link"));
            }
            if inner
                .by_request
                .contains_key(&(generation, request_id.to_string()))
            {
                return Err(conflict(
                    "concurrent duplicate request id in the same source scope",
                ));
            }
            let consumer = inner.consumers.get(&generation).cloned().ok_or_else(|| {
                CallError::new(
                    conex_proto::ErrorCode::Unavailable,
                    "consumer link is not attached",
                )
                .with_execution("not_started")
                .with_retry("safe")
            })?;
            if consumer.active >= consumer.max_subscriptions {
                return Err(not_started(
                    conex_proto::ErrorCode::QuotaExceeded,
                    "subscription budget exceeded for this link",
                ));
            }
            if inner.by_hop.len() >= self.max_openings {
                return Err(not_started(
                    conex_proto::ErrorCode::QuotaExceeded,
                    "state relay is at capacity",
                ));
            }
            inner.by_hop.insert(
                hop.clone(),
                Subscription {
                    consumer_generation: generation,
                    consumer_subscription_id: consumer_subscription_id.clone(),
                    consumer_request_id: request_id.to_string(),
                    principal_id: caller.principal_id.clone(),
                    target_node: record.node_id.clone(),
                    target_generation: record.generation,
                    upstream_context,
                    delivery_context,
                    contract_id: record.contract_id.clone(),
                    state_name: state_name.to_string(),
                    registration_id: record.registration_id.clone(),
                    opening: true,
                    cancelled: false,
                    pending_end: None,
                    last_revision: 0,
                    next_seq: 1,
                    buffer: Vec::new(),
                    buffered_bytes: 0,
                },
            );
            inner
                .by_consumer
                .insert((generation, consumer_subscription_id), hop.clone());
            inner
                .by_request
                .insert((generation, request_id.to_string()), hop.clone());
            inner
                .consumers
                .get_mut(&generation)
                .expect("consumer attached above")
                .active += 1;
        }
        // 6. 转发 hop：先登记回复通道再发送；发送前复查取消（取消在插入
        //    记录后、发送前到达时，订阅已被移除 → 直接 cancelled）。
        let hop_registered = link.register_s1_hop(&hop).await;
        let receiver = {
            let mut inner = self.inner.lock().await;
            let Some(sub) = inner.by_hop.get_mut(&hop) else {
                return Err(cancelled_not_started());
            };
            if sub.cancelled {
                inner.remove_hop(&hop);
                return Err(cancelled_not_started());
            }
            let receiver = match hop_registered {
                Ok(receiver) => receiver,
                Err(error) => {
                    inner.remove_hop(&hop);
                    return Err(not_started(
                        error
                            .code_enum()
                            .unwrap_or(conex_proto::ErrorCode::BadRequest),
                        error.message(),
                    ));
                }
            };
            if let Err(error) = link.send_s1_frame(
                &hop,
                sub.upstream_context.clone(),
                METHOD_STATE_SUBSCRIBE,
                json!({
                    "subscriptionId": hop.clone(),
                    "state": state_name,
                    "argument": argument,
                }),
                remaining,
            ) {
                inner.remove_hop(&hop);
                link.cancel_hop(&hop).await;
                return Err(not_started(
                    error
                        .code_enum()
                        .unwrap_or(conex_proto::ErrorCode::BadRequest),
                    error.message(),
                ));
            }
            receiver
        };
        // 测试宿主屏障：快照读取边界（生产恒 None）。opening 窗口内到达的
        // 更新进入有界缓冲，未放行给消费者。
        if let Some(gate) = self.test_gate.lock().await.take() {
            let _ = gate.await;
        }
        // 7. 等待提供端快照（至剩余预算；预算只约束建立）。
        let outcome = tokio::time::timeout_at(deadline, receiver).await;
        match outcome {
            Ok(Ok(Ok(AgentReply::Value(value)))) => {
                // 提供端成功快照：结构 + 注册实例 + sample 契约校验。
                if let Err(error) = validate_result(METHOD_STATE_SUBSCRIBE, &value) {
                    let call = not_started(
                        conex_proto::ErrorCode::try_from(error.rpc_code)
                            .unwrap_or(conex_proto::ErrorCode::BadRequest),
                        format!("provider subscribe result is invalid: {}", error.message),
                    );
                    return Err(self.establishment_failed(&hop, call).await);
                }
                let map = value.as_object().expect("validated result");
                if map.get("registrationId").and_then(Value::as_str)
                    != Some(&record.registration_id)
                {
                    return Err(self
                        .establishment_failed(
                            &hop,
                            not_started(
                                conex_proto::ErrorCode::StaleRevision,
                                "provider snapshot belongs to a different registration",
                            ),
                        )
                        .await);
                }
                let snapshot = map.get("snapshot").cloned().unwrap_or(Value::Null);
                let revision = snapshot["revision"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .expect("validated sample revision");
                if let Err(error) = validate_state_value(&contract, state_name, &snapshot["value"])
                {
                    return Err(self
                        .establishment_failed(
                            &hop,
                            not_started(
                                conex_proto::ErrorCode::BadRequest,
                                format!(
                                    "provider snapshot value violates the contract: {}",
                                    error.message()
                                ),
                            ),
                        )
                        .await);
                }
                // 竞态复查：unsubscribe/激进溢出/断线清理可能已移除记录。
                let settled = {
                    let mut inner = self.inner.lock().await;
                    let Some(sub) = inner.by_hop.get(&hop) else {
                        return Err(cancelled_not_started());
                    };
                    if sub.pending_end.is_some() {
                        inner.remove_hop(&hop);
                        return Err(not_started(
                            conex_proto::ErrorCode::SlowConsumer,
                            "opening buffer exceeded the queue budget",
                        ));
                    }
                    if sub.cancelled {
                        inner.remove_hop(&hop);
                        return Err(cancelled_not_started());
                    }
                    let sub = inner.by_hop.get_mut(&hop).expect("present");
                    sub.last_revision = revision;
                    json!({
                        "subscriptionId": sub.consumer_subscription_id,
                        "registrationId": sub.registration_id,
                        "snapshot": snapshot,
                    })
                };
                Ok(settled)
            }
            Ok(Ok(Ok(AgentReply::Chunk(_)))) => Err(self
                .establishment_failed(
                    &hop,
                    CallError::new(
                        conex_proto::ErrorCode::Internal,
                        "unexpected binary reply for a state subscribe",
                    ),
                )
                .await),
            Ok(Ok(Err(error))) => {
                let error = if error.wire().execution.is_empty() {
                    CallError::new(
                        error
                            .code_enum()
                            .unwrap_or(conex_proto::ErrorCode::Internal),
                        error.message(),
                    )
                    .with_execution("unknown")
                    .with_retry("safe")
                } else {
                    error
                };
                Err(self.establishment_failed(&hop, error).await)
            }
            Ok(Err(_)) => {
                // pending 通道被本方断开：unsubscribe（opening 取消）或缓冲
                // 溢出已登记该 hop 的 settle 错误；没有登记则按链路失联处理。
                let error = {
                    let mut inner = self.inner.lock().await;
                    inner.settled.remove(&hop).unwrap_or_else(|| {
                        CallError::new(
                            conex_proto::ErrorCode::Unavailable,
                            "provider link closed during subscription establishment",
                        )
                        .with_execution("unknown")
                        .with_retry("safe")
                    })
                };
                Err(self.establishment_failed(&hop, error).await)
            }
            Err(_) => Err(self
                .establishment_failed(
                    &hop,
                    CallError::new(
                        conex_proto::ErrorCode::Timeout,
                        "subscription establishment timed out",
                    )
                    .with_execution("unknown")
                    .with_retry("safe"),
                )
                .await),
        }
    }

    /// 消费端 `state/unsubscribe`（契约 §8.3）：从活动表移除后停止交付并
    /// 尽力转发上游清理；未知/已结束的本地 ID 返回 `{removed:false}`。
    /// opening 期间取消会终结原 subscribe（cancelled/not_started），迟到
    /// 快照不再激活订阅；快照响应已入队则保留该响应。
    pub async fn unsubscribe(&self, generation: u64, subscription_id: &str) -> CallResult<Value> {
        let (hop, upstream_context, target_node, target_generation, opening, cancel_hop) = {
            let mut inner = self.inner.lock().await;
            let Some(hop) = inner
                .by_consumer
                .get(&(generation, subscription_id.to_string()))
                .cloned()
            else {
                return Ok(json!({ "removed": false }));
            };
            let Some(sub) = inner.by_hop.get(&hop).cloned() else {
                return Ok(json!({ "removed": false }));
            };
            let opening = sub.opening;
            inner.remove_hop(&hop);
            (
                hop,
                sub.upstream_context,
                sub.target_node,
                sub.target_generation,
                opening,
                sub.cancelled || sub.pending_end.is_some(),
            )
        };
        // 尽力释放上游观察者（固定注册代次；opening 中再取消 hop 使原
        // subscribe 立即以 cancelled 结算）。
        let upstream = {
            let inner = self.inner.lock().await;
            inner.links.get(&(target_node, target_generation)).cloned()
        };
        if let Some(link) = upstream {
            link.send_s1_control(control_message(
                &new_registration_id(),
                METHOD_STATE_UNSUBSCRIBE,
                &upstream_context,
                json!({ "subscriptionId": hop }),
                1,
            ))
            .await;
            if opening && !cancel_hop {
                // 契约 §8.1：opening 期取消把原 subscribe 终结为
                // cancelled/not_started；断开 pending 前登记 settle 错误。
                self.inner
                    .lock()
                    .await
                    .settled
                    .insert(hop.clone(), cancelled_not_started());
                link.cancel_hop(&hop).await;
            }
        }
        Ok(json!({ "removed": true }))
    }

    /// 快照成功响应已入消费者 socket 队列（transport 在 enqueue 成功后
    /// 调用）：此刻才放行该订阅的 update（两跳同规则的 B2 屏障）。灰色
    /// 窗口溢出的 end 也在此交付，保证 success → end 的因果顺序。
    pub async fn response_enqueued(&self, generation: u64, request_id: &str) {
        let (hop, action) = {
            let mut inner = self.inner.lock().await;
            let Some(hop) = inner
                .by_request
                .get(&(generation, request_id.to_string()))
                .cloned()
            else {
                return;
            };
            let Some(sub) = inner.by_hop.get_mut(&hop) else {
                return;
            };
            if !sub.opening || sub.cancelled {
                return;
            }
            sub.opening = false;
            let action = match sub.pending_end.take() {
                Some(error) => Action::End(error),
                None => Action::Flush,
            };
            (hop, action)
        };
        match action {
            Action::End(error) => self.terminate(&hop, error).await,
            Action::Flush => self.flush_opening(&hop).await,
        }
    }

    /// 提供端通知（state/update | state/end，来自 WSS 入站循环）。hop
    /// subscriptionId 无法归属或代次不符 → 按不可信帧丢弃，绝不据此创建
    /// 订阅（契约 §8.2）。
    pub async fn on_notification(
        &self,
        generation: u64,
        method: &str,
        context: &conex_proto::RequestContext,
        input: &Value,
    ) {
        let Some(map) = input.as_object() else { return };
        let Some(subscription_id) = map.get("subscriptionId").and_then(Value::as_str) else {
            return;
        };
        let sub_id = subscription_id.to_string();
        match method {
            METHOD_STATE_UPDATE => self.on_update(generation, &sub_id, context, map).await,
            METHOD_STATE_END => self.on_provider_end(generation, &sub_id, map).await,
            _ => {}
        }
    }

    async fn on_update(
        &self,
        generation: u64,
        sub_id: &str,
        context: &conex_proto::RequestContext,
        input: &Map<String, Value>,
    ) {
        // 归属：hop 存在、代次一致、注册实例一致、未终止。
        let owned = {
            let inner = self.inner.lock().await;
            inner.by_hop.get(sub_id).is_some_and(|sub| {
                sub.target_generation == generation
                    && sub.registration_id == context.registration_id
                    && !sub.cancelled
                    && sub.pending_end.is_none()
            })
        };
        if !owned {
            return;
        }
        // 形状：seq 规范 uint64、sample 结构与 quality。
        let Some(seq) = input
            .get("seq")
            .and_then(Value::as_str)
            .filter(|s| is_canonical_uint64(s))
            .and_then(|s| s.parse::<u64>().ok())
        else {
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "update seq is not a canonical uint64",
                ),
            )
            .await;
            return;
        };
        let Some(sample) = input.get("sample").cloned() else {
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "update is missing sample",
                ),
            )
            .await;
            return;
        };
        if state_sample_from_json(&sample).is_err() {
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "update sample violates the sample structure and quality rules",
                ),
            )
            .await;
            return;
        }
        let Some(revision) = sample["revision"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
        else {
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "update revision is not a canonical uint64",
                ),
            )
            .await;
            return;
        };
        // sample.value 必须符合契约状态 schema（A12）。
        let (contract_id, state_name) = {
            let inner = self.inner.lock().await;
            let Some(sub) = inner.by_hop.get(sub_id) else {
                return;
            };
            (sub.contract_id.clone(), sub.state_name.clone())
        };
        let Some(contract) = self.registry.contracts().get(&contract_id) else {
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "contract for the subscription is not installed",
                ),
            )
            .await;
            return;
        };
        if let Err(error) = validate_state_value(&contract, &state_name, &sample["value"]) {
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    format!(
                        "update value violates the contract schema: {}",
                        error.message()
                    ),
                ),
            )
            .await;
            return;
        }
        // 帧：构造完整交付通知并度量字节（计入 maxQueuedBytes）。
        let Ok(sample_proto) = state_sample_from_json(&sample) else {
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "update sample cannot be encoded",
                ),
            )
            .await;
            return;
        };
        // 交付链：独立锁只读 consumers（不持有 by_hop 借用）。
        let consumer_generation = {
            let inner = self.inner.lock().await;
            let Some(sub) = inner.by_hop.get(sub_id) else {
                return;
            };
            sub.consumer_generation
        };
        let link = {
            let inner = self.inner.lock().await;
            inner
                .consumers
                .get(&consumer_generation)
                .map(|consumer| consumer.link.clone())
        };
        let Some(link) = link else {
            let mut inner = self.inner.lock().await;
            inner.remove_hop(sub_id);
            return;
        };
        // 帧构造 + 顺序验收 + 缓冲/交付：单锁内所有记账都在 by_hop（避免
        // 跨字段借用）；opening 缓冲计入消费者队列预算（A02/A09）。
        let mut inner = self.inner.lock().await;
        // 预读消费者缓冲总字节（不可跨 by_hop 可变借用取用；并发下允许
        // 超一分帧后由 flush/terminate 兜底，缓冲仍整体有界）。
        let buffered = inner.consumer_buffered(consumer_generation);
        let Some(sub) = inner.by_hop.get_mut(sub_id) else {
            return;
        };
        if sub.target_generation != generation
            || sub.registration_id != context.registration_id
            || sub.cancelled
            || sub.pending_end.is_some()
        {
            return;
        }
        let Ok(update_input) =
            state_update_input(&sub.consumer_subscription_id, seq, &sample_proto)
        else {
            drop(inner);
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "update frame cannot be built",
                ),
            )
            .await;
            return;
        };
        let message = notification_message(
            METHOD_STATE_UPDATE,
            sub.delivery_context.clone(),
            update_input,
        );
        let bytes = conex_proto::wire::encode_wire(&message)
            .map(|b| b.len())
            .unwrap_or(usize::MAX);
        if bytes > link.max_frame() {
            drop(inner);
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "update frame exceeds maxFrameBytes",
                ),
            )
            .await;
            return;
        }
        if sub.opening {
            // opening 窗口：有界缓冲，seq/revision 验收推迟到快照已知后
            // （先快照后更新，契约 §8.2）；缓冲计入消费者队列预算。
            if link
                .queued_bytes()
                .saturating_add(buffered)
                .saturating_add(bytes)
                > link.state_data_watermark()
            {
                // 建立期间超限：失败并清理上游；若成功响应已入队，end 由
                // response_enqueued 在响应之后交付（不再静默丢窗口更新）。
                sub.pending_end = Some(end_error(
                    conex_proto::ErrorCode::SlowConsumer,
                    "opening buffer exceeded the queue budget",
                ));
                // 建立期溢出：subscribe 以 slow_consumer/not_started 终结
                // （与成功响应已入队后的 end 保序路径共用同一 pending_end）。
                inner.settled.insert(
                    sub_id.to_string(),
                    not_started(
                        conex_proto::ErrorCode::SlowConsumer,
                        "opening buffer exceeded the queue budget",
                    ),
                );
                let hop = sub_id.to_string();
                drop(inner);
                self.cancel_hop_upstream(&hop).await;
                return;
            }
            sub.buffer.push(BufferedUpdate {
                seq,
                revision,
                message,
            });
            sub.buffered_bytes += bytes;
            return;
        }
        // 活动订阅：seq 从 1 连续、revision 严格递增、uint64 禁止回绕。
        if seq != sub.next_seq {
            let expected = sub.next_seq;
            drop(inner);
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    format!("seq gap: expected {expected}, got {seq}"),
                ),
            )
            .await;
            return;
        }
        if seq == u64::MAX {
            drop(inner);
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "seq is about to overflow",
                ),
            )
            .await;
            return;
        }
        if revision <= sub.last_revision {
            drop(inner);
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "revision is not strictly increasing",
                ),
            )
            .await;
            return;
        }
        if revision == u64::MAX {
            drop(inner);
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SessionLost,
                    "revision is about to overflow",
                ),
            )
            .await;
            return;
        }
        sub.next_seq = seq + 1;
        sub.last_revision = revision;
        drop(inner);
        if !link.send_s1_state_data(&message) {
            // 数据空间满：该订阅以 slow_consumer 结束（不静默丢帧续发）。
            self.terminate(
                sub_id,
                end_error(
                    conex_proto::ErrorCode::SlowConsumer,
                    "consumer queue is full",
                ),
            )
            .await;
        }
    }

    async fn on_provider_end(&self, generation: u64, sub_id: &str, input: &Map<String, Value>) {
        // 提供端主动结束：end 的 error 已由 wire 校验；此处归属校验后原样
        // 转发给消费者（终止码固定，SDK 据此区分异常终止）。
        let owned = {
            let inner = self.inner.lock().await;
            inner.by_hop.get(sub_id).is_some_and(|sub| {
                sub.target_generation == generation && !sub.cancelled && sub.pending_end.is_none()
            })
        };
        if !owned {
            return;
        }
        let Some(error) = input.get("error") else {
            return;
        };
        let Ok(error) = conex_proto::capability::parse_state_end_error(error) else {
            return;
        };
        self.terminate(sub_id, error).await;
    }

    /// 新代次激活（首个成功注册）：旧代次上的订阅以 stale_revision 终结。
    /// 同一代次追加注册 record.generation 不变，过滤 `< new_generation`
    /// 天然无操作，绝不误伤新订阅（契约 §8.3 注册代次替换）。
    pub async fn end_stale_generation(&self, node_id: &str, new_generation: u64) {
        let hops: Vec<String> = {
            let inner = self.inner.lock().await;
            inner
                .by_hop
                .iter()
                .filter(|(_, sub)| {
                    sub.target_node == node_id && sub.target_generation < new_generation
                })
                .map(|(hop, _)| hop.clone())
                .collect()
        };
        for hop in hops {
            self.terminate(
                &hop,
                end_error(
                    conex_proto::ErrorCode::StaleRevision,
                    "registration generation was replaced",
                ),
            )
            .await;
        }
    }

    /// 模块撤销：该注册的订阅以 unavailable 终结（契约 §6.2 先禁路由再
    /// 结束订阅；§8.3 提供端主动撤销 → unavailable）。
    pub async fn end_registration(&self, registration_id: &str) {
        let hops: Vec<String> = {
            let inner = self.inner.lock().await;
            inner
                .by_hop
                .iter()
                .filter(|(_, sub)| sub.registration_id == registration_id)
                .map(|(hop, _)| hop.clone())
                .collect()
        };
        for hop in hops {
            self.terminate(
                &hop,
                end_error(
                    conex_proto::ErrorCode::Unavailable,
                    "module was unregistered",
                ),
            )
            .await;
        }
    }

    /// 授权撤销（契约 §8.3 → forbidden）：S1 静态批准下暂无运行时撤权
    /// 触发点，本入口供未来/测试宿主调用。消费者失去新订阅权限不影响
    /// 停止自己的在途请求，但现有订阅被显式终结。
    pub async fn end_authorization(&self, principal_id: &str) {
        let hops: Vec<String> = {
            let inner = self.inner.lock().await;
            inner
                .by_hop
                .iter()
                .filter(|(_, sub)| sub.principal_id == principal_id)
                .map(|(hop, _)| hop.clone())
                .collect()
        };
        for hop in hops {
            self.terminate(
                &hop,
                end_error(
                    conex_proto::ErrorCode::Forbidden,
                    "authorization was withdrawn",
                ),
            )
            .await;
        }
    }

    /// 建立失败/超时/断线：立即清理映射 + 尽力上游 unsubscribe + 取消
    /// hop。迟到快照/更新不得复活订阅。
    async fn establishment_failed(&self, hop: &str, error: CallError) -> CallError {
        let (upstream_context, upstream) = {
            let mut inner = self.inner.lock().await;
            let Some(sub) = inner.by_hop.get(hop) else {
                return error;
            };
            let upstream = inner
                .links
                .get(&(sub.target_node.clone(), sub.target_generation))
                .cloned();
            let upstream_context = sub.upstream_context.clone();
            inner.remove_hop(hop);
            (upstream_context, upstream)
        };
        if let Some(link) = upstream {
            link.send_s1_control(control_message(
                &new_registration_id(),
                METHOD_STATE_UNSUBSCRIBE,
                &upstream_context,
                json!({ "subscriptionId": hop }),
                1,
            ))
            .await;
            link.cancel_hop(hop).await;
        }
        error
    }

    /// 尽力释放上游观察者（供 opening 缓冲溢出等已决定路径复用）。
    async fn cancel_hop_upstream(&self, hop: &str) {
        let (upstream_context, upstream) = {
            let inner = self.inner.lock().await;
            let Some(sub) = inner.by_hop.get(hop) else {
                return;
            };
            let upstream = inner
                .links
                .get(&(sub.target_node.clone(), sub.target_generation))
                .cloned();
            (sub.upstream_context.clone(), upstream)
        };
        if let Some(link) = upstream {
            link.send_s1_control(control_message(
                &new_registration_id(),
                METHOD_STATE_UNSUBSCRIBE,
                &upstream_context,
                json!({ "subscriptionId": hop }),
                1,
            ))
            .await;
            link.cancel_hop(hop).await;
        }
    }

    /// response_enqueued 后的缓冲冲刷：从头验证 seq 连续（从 1）/revision
    /// 严格递增/uint64 回绕，任何违例或入队失败都显式终结，绝不续发。
    async fn flush_opening(&self, hop: &str) {
        let (consumer_generation, buffered, next_seq, last_revision) = {
            let mut inner = self.inner.lock().await;
            let Some(sub) = inner.by_hop.get_mut(hop) else {
                return;
            };
            let buffered = std::mem::take(&mut sub.buffer);
            sub.buffered_bytes = 0;
            (
                sub.consumer_generation,
                buffered,
                sub.next_seq,
                sub.last_revision,
            )
        };
        let link = {
            let inner = self.inner.lock().await;
            inner
                .consumers
                .get(&consumer_generation)
                .map(|consumer| consumer.link.clone())
        };
        let Some(link) = link else {
            let mut inner = self.inner.lock().await;
            inner.remove_hop(hop);
            return;
        };
        let mut expected_seq = next_seq;
        let mut last_rev = last_revision;
        for item in &buffered {
            if item.seq != expected_seq || item.seq == u64::MAX {
                self.terminate(
                    hop,
                    end_error(
                        conex_proto::ErrorCode::SessionLost,
                        format!("buffered updates violate seq continuity: expected {expected_seq}"),
                    ),
                )
                .await;
                return;
            }
            if item.revision <= last_rev || item.revision == u64::MAX {
                self.terminate(
                    hop,
                    end_error(
                        conex_proto::ErrorCode::SessionLost,
                        "buffered updates violate revision ordering",
                    ),
                )
                .await;
                return;
            }
            expected_seq = item.seq + 1;
            last_rev = item.revision;
            if !link.send_s1_state_data(&item.message) {
                self.terminate(
                    hop,
                    end_error(
                        conex_proto::ErrorCode::SlowConsumer,
                        "consumer queue is full after snapshot",
                    ),
                )
                .await;
                return;
            }
        }
        // 冲刷成功：推进接受基线（保持 opening 期间绕过了活动路径的验收
        // 一致；记录仍在 opening=false 状态）。
        let mut inner = self.inner.lock().await;
        if let Some(sub) = inner.by_hop.get_mut(hop) {
            sub.next_seq = expected_seq;
            sub.last_revision = last_rev;
        }
    }

    /// 终结一个订阅并向消费者交付 state/end（每个订阅恰好一次终结事件）。
    /// end 控制帧无法入队（控制空间满）时关闭该 Link 并终止该消费者的
    /// 全部剩余订阅（契约 §9）。
    async fn terminate(&self, hop: &str, error: conex_proto::Error) {
        let (sub, link) = {
            let mut inner = self.inner.lock().await;
            let Some(sub) = inner.by_hop.get(hop).cloned() else {
                return;
            };
            let Some(link) = inner
                .consumers
                .get(&sub.consumer_generation)
                .map(|consumer| consumer.link.clone())
            else {
                inner.remove_hop(hop);
                return;
            };
            (sub, link)
        };
        let Ok(end_input) = state_end_input(&sub.consumer_subscription_id, &error) else {
            // 固定终止码总是合法（parse_state_end_error 自检）；防御性兜底。
            let mut inner = self.inner.lock().await;
            inner.remove_hop(hop);
            return;
        };
        let message =
            notification_message(METHOD_STATE_END, sub.delivery_context.clone(), end_input);
        if !link.send_s1_state_control(&message) {
            link.close();
            self.drop_consumer(sub.consumer_generation).await;
            return;
        }
        {
            let mut inner = self.inner.lock().await;
            inner.remove_hop(hop);
        }
        // 释放上游观察者（尽力，固定注册代次；end 由中心生成的路径需要
        // 通知提供端停止，提供端自己发的 end 无副作用）。
        self.cancel_hop_upstream(hop).await;
    }

    async fn drop_consumer(&self, generation: u64) {
        let mut inner = self.inner.lock().await;
        inner.drop_consumer(generation);
    }
}

enum Action {
    End(conex_proto::Error),
    Flush,
}

impl StateInner {
    /// 移除一个订阅并恢复消费者 active 记账（索引一致性）。
    fn remove_hop(&mut self, hop: &str) -> Option<Subscription> {
        let sub = self.by_hop.remove(hop)?;
        self.by_consumer.remove(&(
            sub.consumer_generation,
            sub.consumer_subscription_id.clone(),
        ));
        self.by_request
            .remove(&(sub.consumer_generation, sub.consumer_request_id.clone()));
        if let Some(consumer) = self.consumers.get_mut(&sub.consumer_generation) {
            consumer.active = consumer.active.saturating_sub(1);
        }
        Some(sub)
    }

    /// 某消费者的 opening 缓冲总字节（订阅表内求和：所有记账都在 by_hop，
    /// 单一锁下一致；每 Link ≤ max_subscriptions 项，代价可忽略）。
    fn consumer_buffered(&self, generation: u64) -> usize {
        self.by_hop
            .values()
            .filter(|sub| sub.consumer_generation == generation)
            .map(|sub| sub.buffered_bytes)
            .sum()
    }

    fn drop_consumer(&mut self, generation: u64) {
        let hops: Vec<String> = self
            .by_hop
            .iter()
            .filter(|(_, sub)| sub.consumer_generation == generation)
            .map(|(hop, _)| hop.clone())
            .collect();
        for hop in hops {
            self.remove_hop(&hop);
        }
        self.consumers.remove(&generation);
    }
}

/// 下游交付上下文：注册身份（提供端/space/registration），不带任何端
/// 身份字段——消费者收到的通知不携带中心盖章身份。
fn registration_context(record: &ModuleRecord) -> conex_proto::RequestContext {
    conex_proto::RequestContext {
        provider_endpoint_id: record.provider_endpoint_id.clone(),
        plane: conex_proto::Plane::Broker as i32,
        binding_id: None,
        principal_id: String::new(),
        tenant_id: String::new(),
        space_id: record.space_id.clone(),
        registration_id: record.registration_id.clone(),
    }
}

/// state/end 通知帧（无 id，无预算；契约 §5.1 通知不返回 JSON-RPC 响应）。
fn notification_message(
    method: &str,
    context: conex_proto::RequestContext,
    input: Value,
) -> conex_proto::Message {
    conex_proto::Message {
        body: Some(conex_proto::message::Body::Notification(
            conex_proto::Notification {
                method: method.to_owned(),
                params: Some(conex_proto::CallParams {
                    context: Some(context),
                    timeout_budget_ms: 0,
                    input: serde_json::from_value(input).ok(),
                }),
            },
        )),
    }
}

/// 固定终止码（契约 §8.3）：forbidden → retry=never，其余 → retry=safe；
/// execution=completed 表示订阅已终结，不表示业务副作用完成。
fn end_error(code: conex_proto::ErrorCode, message: impl Into<String>) -> conex_proto::Error {
    conex_proto::Error {
        code: code as i32,
        message: message.into(),
        diagnostic_id: String::new(),
        execution: "completed".into(),
        retry: if code == conex_proto::ErrorCode::Forbidden {
            "never"
        } else {
            "safe"
        }
        .into(),
        details: None,
    }
}

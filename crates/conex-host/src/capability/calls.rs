//! S1 调用路由（docs/contracts/s1-capabilities.md §7；docs/plans/
//! 2026-10-17-conex-s1.md T03）：`module/invoke` 与 `module/cancel`。
//!
//! 规则：
//! - 目标解析只来自受信注册记录与预安装契约；客户端上下文的
//!   providerEndpointId/spaceId/registrationId 只作定位，转发帧携带的身份
//!   （principalId/tenantId）、Space、注册实例、输入/输出与执行类别一律取
//!   记录/认证值，客户端身份字段在 wire 边界已被拒绝（Boundary::Client）。
//! - 有界源关联表：`(源 Link 代次或 HTTP binding, 发起方 requestId) → (目标
//!   注册代次, 每跳转发 ID)`，转发剩余预算；同一 requestId 不同源互不串包，
//!   同源并发重复 ID 在转发前以 conflict 拒绝。
//! - 发送前拒绝 → not_started；越过发送边界后的超时/断线/发送不确定/畸形
//!   输出：non_replayable 结算 outcome_unknown（retry=never），read_only
//!   保持显式 timeout/unavailable（retry=safe），永不自动重放。
//! - cancel 返回 `{requested}`，原 invoke 保持自己的终结结果；排队取消不
//!   执行；已发送取消向固定注册代次传播（不按当前目录重新解析，绝不改投
//!   新注册）；原请求最多一个终态，迟到回复不二次 resolve。
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Arc;

use conex_core::CallError;
use conex_core::capability::{ExecutionClass, validate_method_input, validate_method_output};
use conex_core::{CallResult, Caller};
use conex_proto;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::capability::ModuleRegistry;
use crate::capability::registry::new_registration_id;
use crate::remote::RemoteLink;
use crate::ws_transport::AgentReply;

/// 源关联表的全局上限。并发调用本身受每个 Link/HTTP binding 的 maxInflight
/// 约束，这里是路由表自身的总闸，防失控增长。
// ponytail: single global cap 4096; per-source caps would need max_inflight
// plumbing from the transports, add when a deployment runs >4k concurrent
// S1 invokes across all sources at once.
pub const DEFAULT_MAX_PENDING: usize = 4096;

/// 源标识（契约 §7.1）：同一 requestId 在不同源之间绝不串包。WSS 源用连接
/// 代次（全局唯一、不复用），HTTP 源用 bindingId（每 hello 唯一）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SourceKey {
    Link {
        generation: u64,
        request_id: String,
    },
    Http {
        binding_id: String,
        request_id: String,
    },
}

impl SourceKey {
    pub fn link(generation: u64, request_id: impl Into<String>) -> Self {
        SourceKey::Link {
            generation,
            request_id: request_id.into(),
        }
    }

    pub fn http(binding_id: impl Into<String>, request_id: impl Into<String>) -> Self {
        SourceKey::Http {
            binding_id: binding_id.into(),
            request_id: request_id.into(),
        }
    }
}

#[derive(Debug, Clone)]
struct PendingInvoke {
    /// 每跳转发 ID（提供端看到的 JSON-RPC id）。
    hop_request_id: String,
    /// 取消帧转发目标：注册代次固定的 (node, generation)，不随目录变化。
    target_node: String,
    target_generation: u64,
    /// 原 invoke 的中心盖章上下文；取消帧复用同一上下文（SDK 校验匹配）。
    context: conex_proto::RequestContext,
    deadline: Instant,
    /// 是否已越过发送边界（enqueue 成功）。未发送的取消直接终结原调用。
    sent: bool,
    /// 排队取消标记：invoke 在发送前复查；为 true 则丢弃不发送。
    cancelled: bool,
}

pub struct CallRouter {
    registry: Arc<ModuleRegistry>,
    max_pending: usize,
    /// 提供端 Link 表：(node, 连接代次) → 链。只有注册记录中的代次能收到
    /// 转发/取消；旧代次链保留到连接关闭。
    links: Mutex<HashMap<(String, u64), RemoteLink>>,
    /// 源关联表（有界）。
    pending: Mutex<HashMap<SourceKey, PendingInvoke>>,
    /// 测试宿主屏障（docs/plans T03 步骤 7 后半）：仅测试代码设置，生产恒
    /// None。invoke 在插入关联记录后、发送前等待该通知，使“排队取消不执行”
    /// 可确定复现。不属于正式能力契约，也不进 SDK。
    #[doc(hidden)]
    pub test_gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl CallRouter {
    pub fn new(registry: Arc<ModuleRegistry>) -> Arc<Self> {
        Self::with_max_pending(registry, DEFAULT_MAX_PENDING)
    }

    /// 测试宿主专用构造（docs/plans T03 步骤 7 后半有界性）：以限小路由表
    /// 确定性覆盖饱和拒绝。生产路径恒用 [`CallRouter::new`]。
    #[doc(hidden)]
    pub fn with_max_pending(registry: Arc<ModuleRegistry>, max_pending: usize) -> Arc<Self> {
        Arc::new(Self {
            registry,
            max_pending,
            links: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            test_gate: Mutex::new(None),
        })
    }

    /// 连接建立时登记提供端链。S1 提供端是任意完成认证的 WSS 连接；代次由
    /// 连接分配，注册记录以它作 fence。
    pub async fn attach(&self, node_id: &str, generation: u64, link: RemoteLink) {
        self.links
            .lock()
            .await
            .insert((node_id.to_owned(), generation), link);
    }

    /// 连接关闭时移除提供端链（代次精确匹配，绝不误删新代次）。
    pub async fn detach(&self, node_id: &str, generation: u64) {
        self.links
            .lock()
            .await
            .remove(&(node_id.to_owned(), generation));
    }

    /// 测试断言用：当前源关联记录数（有界性检查）。
    pub async fn pending_len(&self) -> usize {
        self.pending.lock().await.len()
    }

    /// 消费端 `module/invoke`（契约 §7.1）。全部判决来自受信记录与契约；
    /// 转发帧带中心盖章的认证身份，提供端只看到该上下文。
    #[allow(clippy::too_many_arguments)]
    pub async fn invoke(
        &self,
        caller: &Caller,
        source: SourceKey,
        provider_endpoint_id: &str,
        space_id: &str,
        registration_id: &str,
        input: Value,
        deadline: Instant,
    ) -> CallResult<Value> {
        // 1. 目标记录：受信源。键由客户端定位，但 Space/注册/契约/代次
        //    全部取自记录，客户端不能自选身份。
        let record = self
            .registry
            .active_record(space_id, provider_endpoint_id)
            .ok_or_else(|| {
                forbidden("module is not registered or not visible to this principal")
            })?;
        // 2. 权限先于披露（契约 §6.3）：无消费批准 → forbidden，不泄漏
        //    注册是否存在/是否过期。
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
        // 3. 注册实例匹配；不符 → stale_revision，不自动切到新提供者。
        if record.registration_id != registration_id {
            return Err(stale_revision(
                "registrationId is not the current registration",
            ));
        }
        // 4. 输入形状与方法成员（typed 校验共用 core 入口）。
        let map = input.as_object().ok_or_else(|| {
            not_started(
                conex_proto::ErrorCode::BadRequest,
                "invoke input must be an object",
            )
        })?;
        let method = map.get("method").and_then(Value::as_str).ok_or_else(|| {
            not_started(
                conex_proto::ErrorCode::BadRequest,
                "invoke input requires method",
            )
        })?;
        let method = method.to_string();
        let argument = map.get("argument").cloned().unwrap_or(Value::Null);
        let member = contract.method(&method).ok_or_else(|| {
            not_started(
                conex_proto::ErrorCode::UnknownMethod,
                format!("contract {} has no method {method}", contract.contract_id),
            )
        })?;
        if !visible.0.iter().any(|name| name == &method) {
            return Err(forbidden(format!(
                "caller is not approved for contract method {method}"
            )));
        }
        validate_method_input(&contract, &method, &argument).map_err(|error| {
            not_started(
                error
                    .code_enum()
                    .unwrap_or(conex_proto::ErrorCode::BadRequest),
                error.message(),
            )
        })?;
        // 5. 提供端链：固定到记录代次。
        let link = self
            .links
            .lock()
            .await
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
        // 6. 发送前预算：耗尽 → timeout/not_started/safe（排队期预算耗尽）。
        let remaining = remaining_budget_ms(deadline)?;
        let execution = member.execution;
        // 7. 源关联（有界）＋ 每跳转发 ID。先插记录再发送，让取消能拦截。
        let hop = new_registration_id();
        {
            let mut pending = self.pending.lock().await;
            if pending.contains_key(&source) {
                return Err(conflict(
                    "concurrent duplicate request id in the same source scope",
                ));
            }
            if pending.len() >= self.max_pending {
                return Err(not_started(
                    conex_proto::ErrorCode::QuotaExceeded,
                    "call routing table is at capacity",
                ));
            }
            pending.insert(
                source.clone(),
                PendingInvoke {
                    hop_request_id: hop.clone(),
                    target_node: record.node_id.clone(),
                    target_generation: record.generation,
                    context: stamped_context(caller, &record),
                    deadline,
                    sent: false,
                    cancelled: false,
                },
            );
        }
        // 测试宿主屏障：记录已插入、发送前（生产恒 None）。取消在此窗口
        // 到达时置 cancelled，invoke 在发送前复查后丢弃而不发送。
        if let Some(gate) = self.test_gate.lock().await.take() {
            let _ = gate.await;
        }
        // 发送边界：cancelled 复查 + 入队 + 标记 sent 在同一临界区，取消
        // 只能在其前后介入——之前置 cancelled 则此处丢弃（排队取消不
        // 执行）；之后看到 sent=true 才转发真正取消帧。
        let hop_registered = link.register_s1_hop(&hop).await;
        let receiver = {
            let mut pending = self.pending.lock().await;
            let Some(record) = pending.get_mut(&source) else {
                // 记录已被断线清理消费：原调用终结，不再发送。
                link.cancel_hop(&hop).await;
                return Err(cancelled_not_started());
            };
            if record.cancelled {
                pending.remove(&source);
                link.cancel_hop(&hop).await;
                return Err(cancelled_not_started());
            }
            let receiver = match hop_registered {
                Ok(receiver) => receiver,
                Err(error) => {
                    pending.remove(&source);
                    return Err(not_started(
                        error
                            .code_enum()
                            .unwrap_or(conex_proto::ErrorCode::BadRequest),
                        error.message(),
                    ));
                }
            };
            // 同步入队：与取消的 sent 读取原子。
            if let Err(error) = link.send_s1_frame(
                &hop,
                record.context.clone(),
                "module/invoke",
                json!({ "method": method, "argument": argument }),
                remaining,
            ) {
                pending.remove(&source);
                link.cancel_hop(&hop).await;
                return Err(not_started(
                    error
                        .code_enum()
                        .unwrap_or(conex_proto::ErrorCode::BadRequest),
                    error.message(),
                ));
            }
            record.sent = true;
            receiver
        };
        // 8. 等待确定结果（至剩余预算）。发送成功 ≠ 已执行：越过边界后的
        //    超时/断线对 non_replayable 保守结算 outcome_unknown。
        let outcome = tokio::time::timeout_at(deadline, receiver).await;
        {
            let mut pending = self.pending.lock().await;
            pending.remove(&source);
        }
        match outcome {
            Ok(Ok(Ok(AgentReply::Value(value)))) => {
                if let Err(error) = validate_method_output(&contract, &method, &value) {
                    if execution == ExecutionClass::NonReplayable {
                        return Err(unknown_outcome(format!(
                            "provider returned a result that does not match the contract output ({})",
                            error.message()
                        )));
                    }
                    return Err(CallError::new(
                        error
                            .code_enum()
                            .unwrap_or(conex_proto::ErrorCode::BadRequest),
                        error.message(),
                    )
                    .with_execution("completed")
                    .with_retry("never"));
                }
                Ok(value)
            }
            Ok(Ok(Ok(AgentReply::Chunk(_)))) => Err(CallError::new(
                conex_proto::ErrorCode::Internal,
                "unexpected binary reply for an S1 module call",
            )),
            Ok(Ok(Err(error))) => Err(settle_forward_error(execution, error)),
            Ok(Err(_)) => Err(settle_link_lost(execution)),
            Err(_) => {
                // 尽力移除 hop：迟到回复不得重新 resolve 已终结调用。
                link.cancel_hop(&hop).await;
                Err(settle_timeout(execution))
            }
        }
    }

    /// 消费端 `module/cancel`（契约 §7.3）：返回 `{requested}`，表示中心
    /// 已受理停止请求；原 invoke 保持自己的终结结果。无活动记录 → false。
    pub async fn cancel(&self, source: SourceKey, deadline: Instant) -> CallResult<Value> {
        let record = {
            let mut pending = self.pending.lock().await;
            let Some(record) = pending.get(&source) else {
                return Ok(json!({ "requested": false }));
            };
            if !record.sent {
                // 排队取消：不执行，invoke 在发送前复查后自我移除。
                pending.get_mut(&source).expect("present").cancelled = true;
                return Ok(json!({ "requested": true }));
            }
            record.clone()
        };
        // 已发送：向固定注册代次传播，绝不按当前目录重新解析（撤销后不
        // 会误发新注册）。控制帧独立 requestId，不占用原 invoke 的 ID。
        if let Some(link) = self
            .links
            .lock()
            .await
            .get(&(record.target_node.clone(), record.target_generation))
            .cloned()
        {
            let budget = remaining_budget_ms(deadline).unwrap_or(1);
            link.send_s1_control(control_message(
                &new_registration_id(),
                "module/cancel",
                &record.context,
                json!({ "requestId": record.hop_request_id }),
                budget,
            ))
            .await;
        }
        Ok(json!({ "requested": true }))
    }

    /// 消费者连接断线（源 = 该连接代次）：释放其全部源关联记录，并对已
    /// 发送的调用尽力转发取消到固定代次（契约 §7.3）。本地记录删除不是
    /// 远端取消：停止请求必须经过显式转发帧。排队（未发送）的调用直接
    /// 丢弃，不产生任何业务帧。调用方任务已被 transports 中止，原调用的
    /// 迟到回复由 `PendingRequests::route` 丢弃，不会二次 resolve。
    pub async fn cleanup_source(&self, generation: u64) {
        let gone: Vec<PendingInvoke> = {
            let mut pending = self.pending.lock().await;
            let keys: Vec<SourceKey> = pending
                .keys()
                .filter(
                    |key| matches!(key, SourceKey::Link { generation: g, .. } if *g == generation),
                )
                .cloned()
                .collect();
            keys.into_iter()
                .filter_map(|key| pending.remove(&key))
                .collect()
        };
        for record in gone {
            if !record.sent {
                continue;
            }
            if let Some(link) = self
                .links
                .lock()
                .await
                .get(&(record.target_node.clone(), record.target_generation))
                .cloned()
            {
                let budget = u32::try_from(
                    record
                        .deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis(),
                )
                .unwrap_or(u32::MAX)
                .max(1);
                link.send_s1_control(control_message(
                    &new_registration_id(),
                    "module/cancel",
                    &record.context,
                    json!({ "requestId": record.hop_request_id }),
                    budget,
                ))
                .await;
            }
        }
    }
}

/// 中心盖章的转发上下文：身份与注册全部来自受信源。T04 状态中继复用。
pub(crate) fn stamped_context(
    caller: &Caller,
    record: &crate::capability::ModuleRecord,
) -> conex_proto::RequestContext {
    conex_proto::RequestContext {
        provider_endpoint_id: record.provider_endpoint_id.clone(),
        plane: conex_proto::Plane::Broker as i32,
        binding_id: None,
        principal_id: caller.principal_id.clone(),
        tenant_id: caller.tenant_id.clone(),
        space_id: record.space_id.clone(),
        registration_id: record.registration_id.clone(),
    }
}

pub(crate) fn control_message(
    request_id: &str,
    method: &str,
    context: &conex_proto::RequestContext,
    input: Value,
    timeout_budget_ms: u32,
) -> conex_proto::Message {
    conex_proto::Message {
        body: Some(conex_proto::message::Body::Request(conex_proto::Request {
            request_id: request_id.to_owned(),
            method: method.to_owned(),
            params: Some(conex_proto::CallParams {
                context: Some(context.clone()),
                timeout_budget_ms,
                input: serde_json::from_value(input).ok(),
            }),
        })),
    }
}

/// 发送前剩余预算；耗尽 → timeout/not_started/safe（契约 §7.2 排队期预算）。
pub(crate) fn remaining_budget_ms(deadline: Instant) -> CallResult<u32> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(CallError::new(
            conex_proto::ErrorCode::Timeout,
            "request budget exhausted before send",
        )
        .with_execution("not_started")
        .with_retry("safe"));
    }
    Ok(remaining.as_millis().clamp(1, u128::from(u32::MAX)) as u32)
}

/// 提供端失败回复/断线错误的结算：Unavailable/Timeout 是连接级不确定性
/// （可能已发送、可能已执行），non_replayable 保守 outcome_unknown。
fn settle_forward_error(execution: ExecutionClass, error: CallError) -> CallError {
    let connection_level = matches!(
        error.code_enum(),
        Some(conex_proto::ErrorCode::Unavailable) | Some(conex_proto::ErrorCode::Timeout)
    );
    if execution == ExecutionClass::NonReplayable {
        if connection_level || error.wire().execution.is_empty() || error.wire().retry.is_empty() {
            let mut wire = error.into_wire();
            wire.code = conex_proto::ErrorCode::OutcomeUnknown as i32;
            wire.execution = "unknown".into();
            wire.retry = "never".into();
            return CallError::from_wire(wire);
        }
        // Preserve the provider's validated execution evidence, including
        // an explicit rejection before mutation; never infer completed here.
        return error;
    }
    // read_only：显式 timeout/unavailable，未知结果标 safe（允许重试），
    // 但 SDK 不自动重放。
    if error.wire().execution.is_empty() {
        enrich(error, "unknown", "safe")
    } else {
        error
    }
}

fn settle_link_lost(execution: ExecutionClass) -> CallError {
    if execution == ExecutionClass::NonReplayable {
        unknown_outcome("provider link closed before a confirmed result")
    } else {
        CallError::new(
            conex_proto::ErrorCode::Unavailable,
            "provider link closed before a confirmed result",
        )
        .with_execution("unknown")
        .with_retry("safe")
    }
}

fn settle_timeout(execution: ExecutionClass) -> CallError {
    if execution == ExecutionClass::NonReplayable {
        unknown_outcome("invoke exceeded the budget after send; outcome unknown, no retry")
    } else {
        CallError::new(
            conex_proto::ErrorCode::Timeout,
            "read_only invoke exceeded the budget after send",
        )
        .with_execution("unknown")
        .with_retry("safe")
    }
}

fn enrich(error: CallError, execution: &str, retry: &str) -> CallError {
    let mut wire = error.into_wire();
    if wire.execution.is_empty() {
        wire.execution = execution.to_owned();
    }
    if wire.retry.is_empty() {
        wire.retry = retry.to_owned();
    }
    CallError::from_wire(wire)
}

pub(crate) fn not_started(code: conex_proto::ErrorCode, message: impl Into<String>) -> CallError {
    CallError::new(code, message)
        .with_execution("not_started")
        .with_retry("never")
}

pub(crate) fn cancelled_not_started() -> CallError {
    CallError::new(
        conex_proto::ErrorCode::Cancelled,
        "invoke was cancelled before it was sent",
    )
    .with_execution("not_started")
    .with_retry("never")
}

fn unknown_outcome(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::OutcomeUnknown, message)
        .with_execution("unknown")
        .with_retry("never")
}

pub(crate) fn forbidden(message: impl Into<String>) -> CallError {
    not_started(conex_proto::ErrorCode::Forbidden, message)
}

pub(crate) fn conflict(message: impl Into<String>) -> CallError {
    not_started(conex_proto::ErrorCode::Conflict, message)
}

pub(crate) fn stale_revision(message: impl Into<String>) -> CallError {
    not_started(conex_proto::ErrorCode::StaleRevision, message)
}

pub(crate) fn internal(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::Internal, message)
}

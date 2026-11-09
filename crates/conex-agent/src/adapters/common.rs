//! 适配器共享骨架。omp 与 codex 的 `conex.agent/1` 契约面里 13 个方法是对
//! 同一节点 SQLite 的同一投影语义（11 读 + conversations/create + steer 显式
//! 拒绝），此前在两个适配器里各手写一份互为复制；本模块收敛为一份，经
//! [`AgentBackend`] 特征读取共享数据，并注入四个真正的后端差异点（submit/
//! cancel/respond/subagents-cancel 的进程会话语义）。会话 actor、帧协议与
//! 能力表仍是各适配器私有——那部分不假装可统一。
use crate::node_config::ApprovedWorkspace;
use conex_agent_domain::{AuthorizedScope, Store};
use conex_proto as p;
use conex_proto::{
    AgentConversationCreateInput, AgentConversationGetInput, AgentConversationHistoryInput,
    AgentConversationListInput, AgentEndpointListInput, AgentEventReadInput,
    AgentExecutionCancelInput, AgentExecutionGetInput, AgentExecutionSteerInput,
    AgentInteractionListInput, AgentInteractionRespondInput, AgentMessageReadInput,
    AgentRelationshipListInput, AgentSnapshotInput, AgentSubagentCancelInput,
    AgentSubagentListInput, AgentSubmitInput, AgentWorkspaceListInput,
};
use conex_sdk::{Contract, Error, Execution as SdkExecution, Method};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::oneshot;

/// 两个适配器共用同一领域契约 id；区分端点的是 `adapter` 字段。
pub(crate) const CONTRACT_ID: &str = "conex.agent";

pub(crate) type Reply<T> = oneshot::Sender<Result<T, String>>;

/// 由种子确定性派生合法 ULID（26 字符 Crockford base32，首字符 ≤'7'）。
/// 端点/Workspace 的稳定身份跨节点进程重启保留；不是熵池随机数。
pub(crate) fn stable_ulid(seed: &str) -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let digest = Sha256::digest(seed.as_bytes());
    let mut value = u128::from_le_bytes(digest[..16].try_into().expect("16 bytes")) << 2;
    let mut out = [b'0'; 26];
    for slot in out.iter_mut().rev() {
        *slot = ALPHABET[(value & 0x1f) as usize];
        value >>= 5;
    }
    String::from_utf8(out.to_vec()).expect("crockford alphabet is ASCII")
}

pub(crate) use conex_proto::now_ms;

pub(crate) fn store_error(error: conex_agent_domain::StoreError) -> Error {
    use conex_agent_domain::StoreError as E;
    let (code, execution, retry) = match &error {
        E::InvalidArgument(_) => (p::ErrorCode::BadRequest, "not_started", "never"),
        E::NotFound { .. } => (p::ErrorCode::UnknownProvider, "not_started", "never"),
        E::ConversationBusy { .. } | E::IdempotencyMismatch { .. } | E::Conflict(_) => {
            (p::ErrorCode::Conflict, "not_started", "never")
        }
        E::StaleRevision { .. } => (p::ErrorCode::StaleRevision, "not_started", "never"),
        E::PayloadTooLarge { .. } => (p::ErrorCode::PayloadTooLarge, "not_started", "never"),
        E::QuotaExceeded(_) => (p::ErrorCode::QuotaExceeded, "not_started", "never"),
        E::OutcomeUnknown(_) => (p::ErrorCode::OutcomeUnknown, "unknown", "never"),
        E::ResumeUnavailable { .. } => (p::ErrorCode::ResumeUnavailable, "not_started", "safe"),
        E::NodeMismatch { .. } | E::Db(_) | E::Closed => {
            (p::ErrorCode::Internal, "not_started", "never")
        }
    };
    Error::new(code, error.to_string(), execution, retry)
}

pub(crate) fn unavailable(message: impl std::fmt::Display) -> Error {
    Error::new(
        p::ErrorCode::Unavailable,
        message.to_string(),
        "not_started",
        "never",
    )
}

pub(crate) fn unknown_outcome(message: impl std::fmt::Display) -> Error {
    Error::new(
        p::ErrorCode::OutcomeUnknown,
        message.to_string(),
        "unknown",
        "never",
    )
}

pub(crate) fn proto_json<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|error| {
        Error::new(
            p::ErrorCode::Internal,
            error.to_string(),
            "not_started",
            "never",
        )
    })
}

pub(crate) async fn probe_version(binary: &str) -> String {
    let attempt = tokio::time::timeout(
        Duration::from_secs(10),
        Command::new(binary).arg("--version").output(),
    )
    .await;
    match attempt {
        Ok(Ok(output)) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        }
        _ => "unknown".into(),
    }
}

/// 后端适配器的领域面：共享契约需要的节点数据 + 四个后端特有操作。
/// 实现者（omp/codex）保留自己的会话 actor 与帧协议；本特征只约束
/// 契约面统一依赖的形状。
pub(crate) trait AgentBackend: Send + Sync + 'static {
    /// 领域存储（节点 SQLite，唯一权威）。
    fn store(&self) -> &Store;
    fn node_id(&self) -> &str;
    /// 管理员批准的 workspace 清单（rootRef 键 → 真实根目录，路径不出节点）。
    fn workspace_entries(&self) -> &[ApprovedWorkspace];

    /// workspace_id（稳定 ULID，由 rootRef 键派生）→ 真实根目录；
    /// 未知 id 显式 BadRequest，不落到后端进程才失败。
    fn workspace_root(&self, workspace_id: &str) -> Result<PathBuf, Error> {
        self.workspace_entries()
            .iter()
            .find(|workspace| {
                stable_ulid(&format!("workspace:{}:{}", self.node_id(), workspace.id))
                    == workspace_id
            })
            .map(|workspace| workspace.root.clone())
            .ok_or_else(|| {
                Error::new(
                    p::ErrorCode::BadRequest,
                    "unknown workspaceId for this endpoint",
                    "not_started",
                    "never",
                )
            })
    }

    /// 提交执行：admit → dispatching → 单次 dispatch；进程写入失败转
    /// unknown，不自动重发（契约 §5）。
    fn submit(
        &self,
        scope: AuthorizedScope,
        input: AgentSubmitInput,
    ) -> impl Future<Output = Result<Value, Error>> + Send;
    /// 取消执行：request_cancel 后按后端语义中断进程会话。
    fn cancel_execution(
        &self,
        scope: AuthorizedScope,
        input: AgentExecutionCancelInput,
    ) -> impl Future<Output = Result<Value, Error>> + Send;
    /// 交互答复：CAS 一次发送；发送失败转 unknown（契约 §7）。
    fn respond(
        &self,
        scope: AuthorizedScope,
        input: AgentInteractionRespondInput,
    ) -> impl Future<Output = Result<Value, Error>> + Send;
    /// 取消子代理；不支持的后端按能力边界拒绝。
    fn cancel_subagent(
        &self,
        scope: AuthorizedScope,
        input: AgentSubagentCancelInput,
    ) -> impl Future<Output = Result<Value, Error>> + Send;
}

/// 注册表方法清单（application.rs 校验用）。名字与 agent_contract 的键由
/// 两个适配器的测试强制一致；schema 不进注册表，运行时以契约为准。
pub(crate) fn agent_registry_contracts() -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let names: BTreeSet<String> = [
        "agent/endpoints/list",
        "agent/workspaces/list",
        "agent/conversations/list",
        "agent/conversations/get",
        "agent/conversations/history",
        "agent/messages/read",
        "agent/executions/get",
        "agent/interactions/list",
        "agent/relationships/list",
        "agent/subagents/list",
        "events/snapshot",
        "events/read",
        "agent/conversations/create",
        "agent/executions/submit",
        "agent/executions/cancel",
        "agent/interactions/respond",
        "agent/subagents/cancel",
        "agent/executions/steer",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    Ok(BTreeMap::from([(CONTRACT_ID.to_string(), names)]))
}

/// `conex.agent/1` 契约：操作名即方法名，schema 为生成消息名（契约 §4）。
/// 13 个共享方法在此实现一次；submit/cancel/respond/subagents-cancel 由
/// [`AgentBackend`] 实现者提供。
pub(crate) fn agent_contract<B: AgentBackend>(backend: Arc<B>) -> Result<Arc<Contract>, Error> {
    let mut methods = BTreeMap::new();
    let read = |input: &str, output: &str| Method::new(input, output, SdkExecution::ReadOnly);
    let write = |input: &str, output: &str| Method::new(input, output, SdkExecution::NonReplayable);
    macro_rules! read_method {
        ($name:expr, $input:ty, $output:expr, $body:expr) => {
            methods.insert(
                $name.to_string(),
                read(
                    concat!("conex.", stringify!($input)),
                    concat!("conex.", $output),
                )
                .with_handler({
                    let captured = backend.clone();
                    move |context, input: $input| {
                        let backend = captured.clone();
                        let body = $body;
                        async move {
                            let scope = AuthorizedScope::new(context.caller.principal_id.clone())
                                .map_err(store_error)?;
                            body(backend.clone(), scope, input).await
                        }
                    }
                }),
            );
        };
    }
    macro_rules! write_method {
        ($name:expr, $input:ty, $output:expr, $body:expr) => {
            methods.insert(
                $name.to_string(),
                write(
                    concat!("conex.", stringify!($input)),
                    concat!("conex.", $output),
                )
                .with_handler({
                    let captured = backend.clone();
                    move |context, input: $input| {
                        let backend = captured.clone();
                        let body = $body;
                        async move {
                            let scope = AuthorizedScope::new(context.caller.principal_id.clone())
                                .map_err(store_error)?;
                            body(backend.clone(), scope, input).await
                        }
                    }
                }),
            );
        };
    }
    read_method!(
        "agent/endpoints/list",
        AgentEndpointListInput,
        "AgentEndpointListResult",
        |backend: Arc<B>, _scope, input: AgentEndpointListInput| async move {
            let _ = &input;
            // 手工投影：pbjson 省略默认值而 schema 要求显式布尔（experimental、
            // capabilities 的 false 位），这里逐字段输出保证完整。
            let items = backend
                .store()
                .list_endpoints()
                .await
                .map_err(store_error)?
                .into_iter()
                .map(|endpoint| {
                    let capabilities = endpoint.capabilities.unwrap_or_default();
                    // proto3 JSON：可缺席字段在 None 时必须省略，不能写显式 null
                    //（TS 域校验按生成 Schema 拒绝 null；生成 serde 的 proto_json
                    // 即此语义，这里手工拼接保持同一形状）。
                    let mut item = json!({
                        "id": endpoint.id,
                        "nodeId": endpoint.node_id,
                        "moduleInstanceId": endpoint.module_instance_id,
                        "generation": endpoint.generation.to_string(),
                        "adapter": endpoint.adapter,
                        "backendVersion": endpoint.backend_version,
                        "protocolVersion": endpoint.protocol_version,
                        "experimental": endpoint.experimental,
                        "capabilities": {
                            "createConversation": capabilities.create_conversation,
                            "continueConversation": capabilities.continue_conversation,
                            "readHistory": capabilities.read_history,
                            "streamMessages": capabilities.stream_messages,
                            "cancelExecution": capabilities.cancel_execution,
                            "approval": capabilities.approval,
                            "userInput": capabilities.user_input,
                            "listSubagents": capabilities.list_subagents,
                            "cancelSubagent": capabilities.cancel_subagent,
                            "steerExecution": capabilities.steer_execution,
                            "attachExternal": capabilities.attach_external,
                            "recoverActive": capabilities.recover_active,
                        },
                        "availability": endpoint.availability,
                        "catalogScope": endpoint.catalog_scope,
                    });
                    if let Some(limits) = &endpoint.limits {
                        item["limits"] = json!({
                            "maxPageSize": limits.max_page_size,
                            "maxSubmitBytes": limits.max_submit_bytes,
                            "maxMessageReadBytes": limits.max_message_read_bytes,
                        });
                    }
                    if let Some(name) = &endpoint.display_name {
                        item["displayName"] = json!(name);
                    }
                    item
                })
                .collect::<Vec<_>>();
            Ok(json!({ "items": items }))
        }
    );
    read_method!(
        "agent/workspaces/list",
        AgentWorkspaceListInput,
        "AgentWorkspaceListResult",
        |backend: Arc<B>, _scope, input: AgentWorkspaceListInput| async move {
            let workspaces = backend
                .workspace_entries()
                .iter()
                .map(|workspace| p::Workspace {
                    id: stable_ulid(&format!("workspace:{}:{}", backend.node_id(), workspace.id)),
                    node_id: backend.node_id().to_string(),
                    display_name: workspace.display_name.clone(),
                    root_ref: workspace.id.clone(),
                })
                .collect();
            let _ = &input;
            proto_json(&p::AgentWorkspaceListResult {
                items: workspaces,
                next_page_token: None,
            })
        }
    );
    read_method!(
        "agent/conversations/list",
        AgentConversationListInput,
        "AgentConversationListResult",
        |backend: Arc<B>, _scope, input: AgentConversationListInput| async move {
            let mut conversations = backend
                .store()
                .list_conversations(&input.endpoint_id, input.workspace_id.as_deref())
                .await
                .map_err(store_error)?;
            // catalogScope=node-managed：只列本节点登记会话；单页上限 100。
            conversations.truncate(input.limit.unwrap_or(50).clamp(1, 100) as usize);
            proto_json(&p::AgentConversationListResult {
                items: conversations,
                next_page_token: None,
            })
        }
    );
    read_method!(
        "agent/conversations/get",
        AgentConversationGetInput,
        "AgentConversationGetResult",
        |backend: Arc<B>, _scope, input: AgentConversationGetInput| async move {
            let conversation = backend
                .store()
                .read_conversation(&input.conversation_id)
                .await
                .map_err(store_error)?;
            proto_json(&p::AgentConversationGetResult {
                conversation: Some(conversation),
            })
        }
    );
    read_method!(
        "agent/conversations/history",
        AgentConversationHistoryInput,
        "AgentConversationHistoryResult",
        |backend: Arc<B>, _scope, input: AgentConversationHistoryInput| async move {
            let (messages, has_more) = backend
                .store()
                .read_history(
                    &input.conversation_id,
                    input.before_message_id.as_deref(),
                    input.limit.unwrap_or(50).clamp(1, 100),
                )
                .await
                .map_err(store_error)?;
            // 消息表是权威持久投影；历史翻页不跨保留边界，无缺口。
            proto_json(&p::AgentConversationHistoryResult {
                messages,
                has_more,
                quality: "current".into(),
                gaps: Vec::new(),
            })
        }
    );
    read_method!(
        "agent/messages/read",
        AgentMessageReadInput,
        "AgentMessageReadResult",
        |backend: Arc<B>, scope, input: AgentMessageReadInput| async move {
            let result = backend
                .store()
                .read_message(
                    &scope,
                    &input.conversation_id,
                    &input.message_id,
                    input.part_index,
                    input.offset_bytes,
                    input.max_bytes.unwrap_or(4096).min(65536),
                )
                .await
                .map_err(store_error)?;
            proto_json(&result)
        }
    );
    read_method!(
        "agent/executions/get",
        AgentExecutionGetInput,
        "AgentExecutionGetResult",
        |backend: Arc<B>, _scope, input: AgentExecutionGetInput| async move {
            let execution = backend
                .store()
                .read_execution(&input.execution_id)
                .await
                .map_err(store_error)?;
            proto_json(&p::AgentExecutionGetResult {
                execution: Some(execution),
            })
        }
    );
    read_method!(
        "agent/interactions/list",
        AgentInteractionListInput,
        "AgentInteractionListResult",
        |backend: Arc<B>, _scope, input: AgentInteractionListInput| async move {
            let items = backend
                .store()
                .list_interactions(
                    &input.conversation_id,
                    input.execution_id.as_deref(),
                    input.limit.unwrap_or(50).clamp(1, 100),
                )
                .await
                .map_err(store_error)?;
            proto_json(&p::AgentInteractionListResult {
                items,
                next_page_token: None,
            })
        }
    );
    read_method!(
        "agent/relationships/list",
        AgentRelationshipListInput,
        "AgentRelationshipListResult",
        |backend: Arc<B>, _scope, input: AgentRelationshipListInput| async move {
            let items = backend
                .store()
                .list_relationships(
                    &input.conversation_id,
                    input.limit.unwrap_or(50).clamp(1, 100),
                )
                .await
                .map_err(store_error)?;
            proto_json(&p::AgentRelationshipListResult {
                items,
                next_page_token: None,
            })
        }
    );
    read_method!(
        "agent/subagents/list",
        AgentSubagentListInput,
        "AgentSubagentListResult",
        |backend: Arc<B>, _scope, input: AgentSubagentListInput| async move {
            let (items, quality) = backend
                .store()
                .list_subagents(&input.execution_id, input.limit.unwrap_or(50).clamp(1, 100))
                .await
                .map_err(store_error)?;
            proto_json(&p::AgentSubagentListResult {
                items,
                next_page_token: None,
                quality,
            })
        }
    );
    read_method!(
        "events/snapshot",
        AgentSnapshotInput,
        "AgentSnapshotResult",
        |backend: Arc<B>, scope, input: AgentSnapshotInput| async move {
            let snapshot = backend
                .store()
                .snapshot(&scope, &input.conversation_id)
                .await
                .map_err(store_error)?;
            proto_json(&p::AgentSnapshotResult {
                snapshot: Some(snapshot),
            })
        }
    );
    read_method!(
        "events/read",
        AgentEventReadInput,
        "AgentEventReadResult",
        |backend: Arc<B>, scope, input: AgentEventReadInput| async move {
            let result = backend
                .store()
                .read_events(&scope, &input.cursor.expect("cursor required"), input.limit)
                .await
                .map_err(store_error)?;
            proto_json(&result)
        }
    );
    write_method!(
        "agent/conversations/create",
        AgentConversationCreateInput,
        "AgentConversationCreateResult",
        |backend: Arc<B>, scope, input: AgentConversationCreateInput| async move {
            // workspace 归属校验（ULID 由 rootRef 键派生）：未知 workspace
            // 在创建时就失败，不推迟到首次受控执行。
            backend.workspace_root(&input.workspace_id)?;
            // create 只建节点侧空会话；backend mapping 在首次受控执行后登记。
            let conversation = backend
                .store()
                .create_conversation(
                    &scope,
                    &input.endpoint_id,
                    &input.workspace_id,
                    &input.idempotency_key,
                    input.title.as_deref(),
                    now_ms(),
                )
                .await
                .map_err(store_error)?;
            proto_json(&p::AgentConversationCreateResult {
                conversation: Some(conversation),
            })
        }
    );
    write_method!(
        "agent/executions/submit",
        AgentSubmitInput,
        "AgentSubmitResult",
        |backend: Arc<B>, scope, input: AgentSubmitInput| async move {
            backend.submit(scope, input).await
        }
    );
    write_method!(
        "agent/executions/cancel",
        AgentExecutionCancelInput,
        "AgentExecutionCancelResult",
        |backend: Arc<B>, scope, input: AgentExecutionCancelInput| async move {
            backend.cancel_execution(scope, input).await
        }
    );
    // steer 已进契约以便语义化错误；能力开关由各适配器按实测证据置位，
    // 未实测时调用返回 unsupported_capability 而不是静默成功。
    write_method!(
        "agent/executions/steer",
        AgentExecutionSteerInput,
        "AgentExecutionSteerResult",
        |_backend: Arc<B>, _scope, input: AgentExecutionSteerInput| async move {
            let _ = input;
            let refused: Result<Value, Error> = Err(Error::new(
                p::ErrorCode::UnsupportedCapability,
                "steer_execution is not enabled for this endpoint",
                "not_started",
                "never",
            ));
            refused
        }
    );
    write_method!(
        "agent/interactions/respond",
        AgentInteractionRespondInput,
        "AgentInteractionRespondResult",
        |backend: Arc<B>, scope, input: AgentInteractionRespondInput| async move {
            backend.respond(scope, input).await
        }
    );
    write_method!(
        "agent/subagents/cancel",
        AgentSubagentCancelInput,
        "AgentSubagentCancelResult",
        |backend: Arc<B>, scope, input: AgentSubagentCancelInput| async move {
            backend.cancel_subagent(scope, input).await
        }
    );
    Ok(Arc::new(Contract::new(
        CONTRACT_ID,
        methods,
        BTreeMap::new(),
    )?))
}

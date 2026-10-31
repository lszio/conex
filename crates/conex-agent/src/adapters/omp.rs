//! OMP RPC 适配器（docs/plans/2026-10-31-conex-mvp-p3.md）。
//!
//! 协议权威参考：本机安装包 `@oh-my-pi/pi-coding-agent` 的
//! `src/modes/rpc/{rpc-types,rpc-frame}.ts`（实现时实测 18.14.0；P0 证据页另记
//! 18.13.2 行为）。OMP RPC 是 stdin/stdout 上的自定义 JSONL，不是 JSON-RPC2；
//! 物理帧 1 MiB、协议 v2 分片重组上限 64 MiB（分片 data 为标准 base64）。
//!
//! 不变量：一个活动会话一个受控进程；领域状态机以节点 SQLite 为唯一权威；
//! 发送前必须持久化 dispatching；abort 回执不是终态，prompt_result+settled 才是；
//! 交互过期不自动批准——confirm 走 cancelled:false（显式 deny），select/input
//! 无法无歧义拒绝时 abort 并等真实静止（契约 §7、P0 证据 §3）。
use crate::node_config::OmpAdapterConfig;
use crate::node_types::RuntimeFactory;
use conex_agent_domain::{
    Admission, AuthorizedScope, BackendObservation, BackendResponseOutcome, Store, TerminalState,
};
use conex_proto as p;
use conex_proto::{
    AgentConversationCreateInput, AgentConversationGetInput, AgentConversationHistoryInput,
    AgentConversationListInput, AgentEndpointListInput, AgentEventReadInput,
    AgentExecutionCancelInput, AgentExecutionGetInput, AgentExecutionSteerInput,
    AgentInteractionListInput, AgentInteractionRespondInput, AgentMessageReadInput,
    AgentRelationshipListInput, AgentSnapshotInput, AgentSubagentCancelInput,
    AgentSubagentListInput, AgentSubmitInput, AgentWorkspaceListInput,
};
use conex_sdk::{Contract, Error, Execution as SdkExecution, Method, StartedModule};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc, oneshot, watch};

pub const ADAPTER: &str = "omp";
pub const CONTRACT_ID: &str = "conex.agent";
const PROTOCOL_VERSION: u32 = 2;
const MAX_PHYSICAL_FRAME: usize = 1024 * 1024;
const MAX_REASSEMBLED: usize = 64 * 1024 * 1024;
/// 契约 §7：后端没有截止时间时节点上限 10 分钟；过期不自动批准。
const NODE_INTERACTION_TTL_MS: u64 = 10 * 60 * 1000;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

type Reply<T> = oneshot::Sender<Result<T, String>>;

/// 由种子确定性派生合法 ULID（26 字符 Crockford base32，首字符 ≤'7'）。
/// 端点/Workspace 的稳定身份跨节点进程重启保留；不是熵池随机数。
pub fn stable_ulid(seed: &str) -> String {
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

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn store_error(error: conex_agent_domain::StoreError) -> Error {
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

fn unavailable(message: impl std::fmt::Display) -> Error {
    Error::new(
        p::ErrorCode::Unavailable,
        message.to_string(),
        "not_started",
        "never",
    )
}

fn unknown_outcome(message: impl std::fmt::Display) -> Error {
    Error::new(
        p::ErrorCode::OutcomeUnknown,
        message.to_string(),
        "unknown",
        "never",
    )
}

fn proto_json<T: serde::Serialize>(value: &T) -> Result<Value, Error> {
    serde_json::to_value(value).map_err(|error| {
        Error::new(
            p::ErrorCode::Internal,
            error.to_string(),
            "not_started",
            "never",
        )
    })
}

/// 应用包可见的端点能力（契约 §3）。steer 未实测一律 false，live 验收取得
/// 证据后才允许置位；不预先谎报。
fn omp_capabilities() -> p::AgentCapabilities {
    p::AgentCapabilities {
        create_conversation: true,
        continue_conversation: true,
        read_history: true,
        stream_messages: true,
        cancel_execution: true,
        approval: true,
        user_input: true,
        list_subagents: true,
        cancel_subagent: true,
        steer_execution: false,
        attach_external: false,
        recover_active: true,
    }
}

// ---------------------------------------------------------------------------
// 后端（模块 service）
// ---------------------------------------------------------------------------

pub struct OmpBackend {
    config: OmpAdapterConfig,
    store: Store,
    node_id: String,
    backend_version: String,
    sessions: Mutex<BTreeMap<String, Arc<SessionHandle>>>,
    data_dir: PathBuf,
}

impl OmpBackend {
    fn workspace_root(&self, workspace_id: &str) -> Result<PathBuf, Error> {
        self.config
            .workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
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

    async fn ensure_session(
        &self,
        conversation_id: &str,
        workspace_id: &str,
    ) -> Result<Arc<SessionHandle>, Error> {
        let root = self.workspace_root(workspace_id)?;
        let mut sessions = self.sessions.lock().await;
        if let Some(handle) = sessions.get(conversation_id) {
            if *handle.done.borrow() {
                sessions.remove(conversation_id);
            } else {
                return Ok(handle.clone());
            }
        }
        if sessions.len() >= self.config.max_concurrent_sessions {
            return Err(unavailable(
                "omp concurrent session limit reached; active sessions are not preempted",
            ));
        }
        let handle = spawn_session(
            &self.config.backend_binary,
            &self.data_dir,
            conversation_id,
            &root,
            self.store.clone(),
            self.node_id.clone(),
        )
        .await?;
        sessions.insert(conversation_id.to_string(), handle.clone());
        Ok(handle)
    }

    async fn session_for_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<Arc<SessionHandle>>, Error> {
        let execution = self
            .store
            .read_execution(execution_id)
            .await
            .map_err(store_error)?;
        let sessions = self.sessions.lock().await;
        Ok(sessions.get(&execution.conversation_id).cloned())
    }

    async fn stop_all(&self, deadline: tokio::time::Instant) {
        let sessions: Vec<Arc<SessionHandle>> =
            self.sessions.lock().await.values().cloned().collect();
        for session in sessions {
            let _ = session.stop(deadline).await;
        }
    }
}

/// RuntimeFactory：模块启动时登记端点/Workspace 实体并构建契约。
pub fn factory(config: OmpAdapterConfig) -> RuntimeFactory {
    Arc::new(move |runtime, _context| {
        let config = config.clone();
        Box::pin(async move {
            let backend_version = probe_version(&config.backend_binary).await;
            let endpoint_id = stable_ulid(&format!(
                "endpoint:{}:{}",
                runtime.node_epoch, runtime.module_key
            ));
            let backend = Arc::new(OmpBackend {
                config: config.clone(),
                store: runtime.store.clone(),
                node_id: runtime.node_id.clone(),
                backend_version,
                sessions: Mutex::new(BTreeMap::new()),
                data_dir: runtime.data_dir.clone(),
            });
            let mut workspaces = Vec::new();
            for workspace in &config.workspaces {
                workspaces.push(p::Workspace {
                    id: stable_ulid(&format!("workspace:{}:{}", backend.node_id, workspace.id)),
                    node_id: backend.node_id.clone(),
                    display_name: workspace.display_name.clone(),
                    root_ref: workspace.id.clone(),
                });
            }
            backend
                .store
                .put_endpoint(p::AgentAdapterEndpoint {
                    id: endpoint_id,
                    node_id: backend.node_id.clone(),
                    module_instance_id: runtime.resource_id.clone(),
                    generation: runtime.generation,
                    adapter: ADAPTER.into(),
                    backend_version: backend.backend_version.clone(),
                    protocol_version: PROTOCOL_VERSION.to_string(),
                    experimental: false,
                    capabilities: Some(omp_capabilities()),
                    availability: "ready".into(),
                    catalog_scope: "node-managed".into(),
                    limits: Some(p::AgentLimits {
                        max_page_size: 100,
                        max_submit_bytes: 65536,
                        max_message_read_bytes: 65536,
                    }),
                    display_name: Some("OMP".into()),
                })
                .await
                .map_err(|error| crate::node::failure(&error.to_string()))?;
            for workspace in workspaces {
                backend
                    .store
                    .put_workspace(workspace)
                    .await
                    .map_err(|error| crate::node::failure(&error.to_string()))?;
            }
            let mut started = StartedModule::new(agent_contract(backend.clone())?);
            started.service = Some(backend.clone());
            started.cleanup = Some(Arc::new(move |deadline| {
                let backend = backend.clone();
                Box::pin(async move {
                    backend.stop_all(deadline).await;
                    Ok(())
                })
            }));
            Ok(started)
        })
    })
}

async fn probe_version(binary: &str) -> String {
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

/// `conex.agent/1` 契约：操作名即方法名，schema 为生成消息名（契约 §4）。
fn agent_contract(backend: Arc<OmpBackend>) -> Result<Arc<Contract>, Error> {
    let mut methods = BTreeMap::new();
    let read = |input: &str, output: &str| Method::new(input, output, SdkExecution::ReadOnly);
    let write = |input: &str, output: &str| Method::new(input, output, SdkExecution::NonReplayable);
    macro_rules! read_method {
        ($name:expr, $input:ty, $input_name:expr, $output_name:expr, $body:expr) => {
            methods.insert(
                $name.to_string(),
                read($input_name, $output_name).with_handler({
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
        ($name:expr, $input:ty, $input_name:expr, $output_name:expr, $body:expr) => {
            methods.insert(
                $name.to_string(),
                write($input_name, $output_name).with_handler({
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
        "AgentEndpointListInput",
        "AgentEndpointListResult",
        |backend: Arc<OmpBackend>, _scope, input: AgentEndpointListInput| async move {
            let _ = &input;
            proto_json(&p::AgentEndpointListResult {
                items: backend.store.list_endpoints().await.map_err(store_error)?,
                next_page_token: None,
            })
        }
    );
    read_method!(
        "agent/workspaces/list",
        AgentWorkspaceListInput,
        "AgentWorkspaceListInput",
        "AgentWorkspaceListResult",
        |backend: Arc<OmpBackend>, _scope, input: AgentWorkspaceListInput| async move {
            let workspaces = backend
                .config
                .workspaces
                .iter()
                .map(|workspace| p::Workspace {
                    id: stable_ulid(&format!("workspace:{}:{}", backend.node_id, workspace.id)),
                    node_id: backend.node_id.clone(),
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
        "AgentConversationListInput",
        "AgentConversationListResult",
        |backend: Arc<OmpBackend>, _scope, input: AgentConversationListInput| async move {
            let mut conversations = backend
                .store
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
        "AgentConversationGetInput",
        "AgentConversationGetResult",
        |backend: Arc<OmpBackend>, _scope, input: AgentConversationGetInput| async move {
            let conversation = backend
                .store
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
        "AgentConversationHistoryInput",
        "AgentConversationHistoryResult",
        |backend: Arc<OmpBackend>, _scope, input: AgentConversationHistoryInput| async move {
            let (messages, has_more) = backend
                .store
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
        "AgentMessageReadInput",
        "AgentMessageReadResult",
        |backend: Arc<OmpBackend>, scope, input: AgentMessageReadInput| async move {
            let result = backend
                .store
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
        "AgentExecutionGetInput",
        "AgentExecutionGetResult",
        |backend: Arc<OmpBackend>, _scope, input: AgentExecutionGetInput| async move {
            let execution = backend
                .store
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
        "AgentInteractionListInput",
        "AgentInteractionListResult",
        |backend: Arc<OmpBackend>, _scope, input: AgentInteractionListInput| async move {
            let items = backend
                .store
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
        "AgentRelationshipListInput",
        "AgentRelationshipListResult",
        |backend: Arc<OmpBackend>, _scope, input: AgentRelationshipListInput| async move {
            let items = backend
                .store
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
        "AgentSubagentListInput",
        "AgentSubagentListResult",
        |backend: Arc<OmpBackend>, _scope, input: AgentSubagentListInput| async move {
            let (items, quality) = backend
                .store
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
        "AgentSnapshotInput",
        "AgentSnapshotResult",
        |backend: Arc<OmpBackend>, scope, input: AgentSnapshotInput| async move {
            let snapshot = backend
                .store
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
        "AgentEventReadInput",
        "AgentEventReadResult",
        |backend: Arc<OmpBackend>, scope, input: AgentEventReadInput| async move {
            let result = backend
                .store
                .read_events(&scope, &input.cursor.expect("cursor required"), input.limit)
                .await
                .map_err(store_error)?;
            proto_json(&result)
        }
    );
    write_method!(
        "agent/conversations/create",
        AgentConversationCreateInput,
        "AgentConversationCreateInput",
        "AgentConversationCreateResult",
        |backend: Arc<OmpBackend>, scope, input: AgentConversationCreateInput| async move {
            // create 只建节点侧空会话；backend mapping 在首次受控执行后登记。
            let conversation = backend
                .store
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
        "AgentSubmitInput",
        "AgentSubmitResult",
        |backend: Arc<OmpBackend>, scope, input: AgentSubmitInput| async move {
            submit(&backend, &scope, input).await
        }
    );
    write_method!(
        "agent/executions/cancel",
        AgentExecutionCancelInput,
        "AgentExecutionCancelInput",
        "AgentExecutionCancelResult",
        |backend: Arc<OmpBackend>, scope, input: AgentExecutionCancelInput| async move {
            let target = backend
                .store
                .request_cancel(
                    &scope,
                    &input.execution_id,
                    input.expected_generation,
                    now_ms(),
                )
                .await
                .map_err(store_error)?;
            if target.requested
                && let Some(session) = backend.session_for_execution(&input.execution_id).await?
            {
                session.abort(&input.execution_id).await;
            }
            proto_json(&p::AgentExecutionCancelResult {
                execution_id: target.execution_id,
                requested: target.requested,
                state: target.state,
            })
        }
    );
    // steer 已进契约以便语义化错误；能力开关为 false（未取得 live 证据），
    // 调用返回 unsupported_capability 而不是静默成功。
    write_method!(
        "agent/executions/steer",
        AgentExecutionSteerInput,
        "AgentExecutionSteerInput",
        "AgentExecutionSteerResult",
        |_backend: Arc<OmpBackend>, _scope, input: AgentExecutionSteerInput| async move {
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
        "AgentInteractionRespondInput",
        "AgentInteractionRespondResult",
        |backend: Arc<OmpBackend>, scope, input: AgentInteractionRespondInput| async move {
            respond(&backend, &scope, input).await
        }
    );
    write_method!(
        "agent/subagents/cancel",
        AgentSubagentCancelInput,
        "AgentSubagentCancelInput",
        "AgentSubagentCancelResult",
        |backend: Arc<OmpBackend>, scope, input: AgentSubagentCancelInput| async move {
            let target = backend
                .store
                .request_cancel(
                    &scope,
                    &input.execution_id,
                    input.expected_generation,
                    now_ms(),
                )
                .await
                .map_err(store_error)?;
            if target.requested
                && let Some(session) = backend.session_for_execution(&input.execution_id).await?
            {
                let execution = backend
                    .store
                    .read_execution(&input.execution_id)
                    .await
                    .map_err(store_error)?;
                if let Some(backend_ref) = execution.backend_ref.as_deref() {
                    session.cancel_subagent(backend_ref).await;
                }
            }
            proto_json(&p::AgentExecutionCancelResult {
                execution_id: target.execution_id,
                requested: target.requested,
                state: target.state,
            })
            .map(|mut value| {
                value["__kind"] = json!("subagent");
                value
            })
        }
    );
    Ok(Arc::new(Contract::new(
        CONTRACT_ID,
        methods,
        BTreeMap::new(),
    )?))
}

/// 注册表方法清单（application.rs 校验用）。名字与 agent_contract 的键由
/// 下方测试强制一致；schema 不进注册表，运行时以契约为准。
pub fn registry_contracts() -> Result<BTreeMap<String, BTreeSet<String>>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_backend(store: Store) -> Arc<OmpBackend> {
        Arc::new(OmpBackend {
            config: OmpAdapterConfig {
                backend_binary: "omp".into(),
                max_concurrent_sessions: 2,
                workspaces: Vec::new(),
            },
            store,
            node_id: "01ARZ3NDEKTSV4RRFFQ69G5FAX".into(),
            backend_version: "test".into(),
            sessions: Mutex::new(BTreeMap::new()),
            data_dir: std::env::temp_dir(),
        })
    }

    #[tokio::test]
    async fn registry_names_match_installed_contract() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("db"), "node", Default::default())
            .await
            .unwrap();
        let contract = agent_contract(test_backend(store)).unwrap();
        let installed: BTreeSet<String> = contract.methods.keys().cloned().collect();
        let registered = registry_contracts().unwrap()[CONTRACT_ID].clone();
        assert_eq!(installed, registered);
        // 每个方法都必须落在权限类映射内，否则永远不可调用。
        for name in &installed {
            assert!(
                super::super::agent_permission_class(name).is_some(),
                "method {name} has no permission class"
            );
        }
    }

    #[test]
    fn stable_ulid_matches_repo_validation() {
        for seed in ["a", "endpoint:x:y", "workspace:n:main"] {
            let id = stable_ulid(seed);
            assert!(conex_proto::wire::is_ulid(&id), "{id} for {seed}");
        }
        assert_ne!(stable_ulid("a"), stable_ulid("b"));
    }
}

/// 提交 → 准入 → 会话进程 → dispatching → 单次 prompt（契约 §5）。
async fn submit(
    backend: &Arc<OmpBackend>,
    scope: &AuthorizedScope,
    input: AgentSubmitInput,
) -> Result<Value, Error> {
    let conversation = backend
        .store
        .read_conversation(&input.conversation_id)
        .await
        .map_err(store_error)?;
    let session = backend
        .ensure_session(&conversation.id, &conversation.workspace_id)
        .await?;
    let now = now_ms();
    match backend
        .store
        .admit(
            scope,
            &AgentSubmitInput {
                conversation_id: input.conversation_id.clone(),
                idempotency_key: input.idempotency_key.clone(),
                content: input.content.clone(),
            },
            now,
        )
        .await
        .map_err(store_error)?
    {
        Admission::Existing { execution } => proto_json(&p::AgentSubmitResult {
            execution: Some(execution),
        }),
        Admission::Created {
            execution,
            dispatch_token,
        } => {
            let permit = backend
                .store
                .mark_dispatching(
                    &execution.id,
                    execution.generation,
                    dispatch_token,
                    now_ms(),
                )
                .await
                .map_err(store_error)?;
            // permit 消费即发送证据；进程写入失败转 unknown，不自动重发。
            session
                .dispatch(
                    execution.id.clone(),
                    execution.generation,
                    input.content.clone(),
                )
                .await
                .map_err(unknown_outcome)?;
            let _ = permit;
            Ok(proto_json(&p::AgentSubmitResult {
                execution: Some(execution),
            })?)
        }
    }
}

/// 交互答复：CAS 一次发送；发送失败转 unknown（契约 §7）。
async fn respond(
    backend: &Arc<OmpBackend>,
    scope: &AuthorizedScope,
    input: AgentInteractionRespondInput,
) -> Result<Value, Error> {
    let permit = backend
        .store
        .begin_response(scope, &input, now_ms())
        .await
        .map_err(store_error)?;
    let decision_kind = input
        .decision
        .as_ref()
        .map(|decision| decision.kind.clone())
        .unwrap_or_default();
    let located = session_for_interaction(backend, permit.interaction_id()).await;
    let Some((session, omp_request_id, kind)) = located else {
        backend
            .store
            .finish_response(
                permit,
                BackendResponseOutcome::Invalidated {
                    reason: "backend request not tracked".into(),
                },
                now_ms(),
            )
            .await
            .map_err(store_error)?;
        return Err(unknown_outcome("backend request not tracked"));
    };
    if kind != decision_kind {
        backend
            .store
            .finish_response(
                permit,
                BackendResponseOutcome::Invalidated {
                    reason: "decision kind does not match backend request".into(),
                },
                now_ms(),
            )
            .await
            .map_err(store_error)?;
        return Err(Error::new(
            p::ErrorCode::BadRequest,
            "decision kind does not match backend request",
            "not_started",
            "never",
        ));
    }
    let response = match decision_kind.as_str() {
        "selection" => {
            json!({"value": input.decision.as_ref().and_then(|d| d.choice_ids.first().cloned())})
        }
        "approval" => {
            json!({"confirmed": input.decision.as_ref().and_then(|d| d.choice_id.as_deref()) == Some("allow")})
        }
        "input" => json!({"value": input.decision.as_ref().and_then(|d| d.text.clone())}),
        other => {
            let reason = format!("unknown decision kind {other}");
            backend
                .store
                .finish_response(
                    permit,
                    BackendResponseOutcome::Invalidated {
                        reason: reason.clone(),
                    },
                    now_ms(),
                )
                .await
                .map_err(store_error)?;
            return Err(Error::new(
                p::ErrorCode::BadRequest,
                reason,
                "not_started",
                "never",
            ));
        }
    };
    match session.respond_ui(&omp_request_id, response).await {
        Ok(()) => {
            let resolved = backend
                .store
                .finish_response(permit, BackendResponseOutcome::Confirmed, now_ms())
                .await
                .map_err(store_error)?;
            Ok(proto_json(&p::AgentInteractionRespondResult {
                interaction: Some(resolved),
            })?)
        }
        Err(error) => {
            backend
                .store
                .finish_response(
                    permit,
                    BackendResponseOutcome::Unknown {
                        reason: error.clone(),
                    },
                    now_ms(),
                )
                .await
                .map_err(store_error)?;
            Err(unknown_outcome(format!(
                "omp response send failed: {error}"
            )))
        }
    }
}

// ---------------------------------------------------------------------------
// 会话进程 actor
// ---------------------------------------------------------------------------

struct SessionHandle {
    commands: mpsc::Sender<SessionCommand>,
    done: watch::Receiver<bool>,
}

enum SessionCommand {
    Dispatch {
        execution_id: String,
        generation: u64,
        content: String,
        reply: Reply<()>,
    },
    Abort {
        execution_id: String,
    },
    RespondUi {
        omp_request_id: String,
        response: Value,
        reply: Reply<()>,
    },
    /// 定位持有该交互的 OMP 请求 id 与类型（respond 的发送边界）。
    LocateInteraction {
        interaction_id: String,
        reply: oneshot::Sender<Option<(String, String)>>,
    },
    CancelSubagent {
        subagent_id: String,
        reply: Reply<()>,
    },
    Stop {
        deadline: tokio::time::Instant,
        reply: Reply<()>,
    },
}

impl SessionHandle {
    async fn dispatch(
        &self,
        execution_id: String,
        generation: u64,
        content: String,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::Dispatch {
                execution_id,
                generation,
                content,
                reply: tx,
            })
            .await
            .map_err(|_| "omp session stopped".to_string())?;
        tokio::time::timeout(COMMAND_TIMEOUT, rx)
            .await
            .map_err(|_| "omp dispatch timeout".to_string())?
            .map_err(|_| "omp session dropped request".to_string())?
    }

    async fn abort(&self, execution_id: &str) {
        let _ = self
            .commands
            .send(SessionCommand::Abort {
                execution_id: execution_id.to_string(),
            })
            .await;
    }

    async fn respond_ui(&self, omp_request_id: &str, response: Value) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::RespondUi {
                omp_request_id: omp_request_id.to_string(),
                response,
                reply: tx,
            })
            .await
            .map_err(|_| "omp session stopped".to_string())?;
        tokio::time::timeout(COMMAND_TIMEOUT, rx)
            .await
            .map_err(|_| "omp response timeout".to_string())?
            .map_err(|_| "omp session dropped request".to_string())?
    }

    async fn cancel_subagent(&self, subagent_id: &str) {
        let (tx, _rx) = oneshot::channel();
        let _ = self
            .commands
            .send(SessionCommand::CancelSubagent {
                subagent_id: subagent_id.to_string(),
                reply: tx,
            })
            .await;
    }

    async fn locate_interaction(&self, interaction_id: &str) -> Option<(String, String)> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::LocateInteraction {
                interaction_id: interaction_id.to_string(),
                reply: tx,
            })
            .await
            .ok()?;
        tokio::time::timeout(COMMAND_TIMEOUT, rx).await.ok()?.ok()?
    }

    async fn stop(&self, deadline: tokio::time::Instant) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::Stop {
                deadline,
                reply: tx,
            })
            .await
            .map_err(|_| "omp session already stopped".to_string())?;
        let _ = rx.await;
        Ok(())
    }
}

/// 按交互 id 反查会话（respond 的发送边界在会话 actor 手里）。
async fn session_for_interaction(
    backend: &OmpBackend,
    interaction_id: &str,
) -> Option<(Arc<SessionHandle>, String, String)> {
    let sessions = backend.sessions.lock().await;
    for session in sessions.values() {
        if let Some((omp_request_id, kind)) = session.locate_interaction(interaction_id).await {
            return Some((session.clone(), omp_request_id, kind));
        }
    }
    None
}

struct ActiveExecution {
    execution_id: String,
    generation: u64,
    prompt_id: String,
    /// OMP messageId -> 节点 Message ID（契约 §8 显式映射）。
    messages: BTreeMap<String, String>,
    /// OMP messageId -> 已累计 text 字节数（end 时核对完整性）。
    streamed_bytes: BTreeMap<String, usize>,
    /// prompt_result 已到但 settled 未到时暂存状态。
    pending_status: Option<String>,
}

struct UiPending {
    interaction_id: String,
    kind: String,
    expires_at_ms: u64,
}

#[derive(Default)]
struct ChunkState {
    count: usize,
    next_index: usize,
    parts: Vec<String>,
    bytes: usize,
}

enum SessionEvent {
    Line(String),
    StdoutClosed,
}

struct SessionActor {
    conversation_id: String,
    session_dir: PathBuf,
    store: Store,
    node_id: String,
    stdin: tokio::process::ChildStdin,
    events: mpsc::Receiver<SessionEvent>,
    commands: mpsc::Receiver<SessionCommand>,
    exited: watch::Receiver<bool>,
    pending: BTreeMap<String, Reply<Value>>,
    next_request_id: u64,
    omp_session_id: Option<String>,
    queued_dispatch: Option<(String, u64, String, Reply<()>)>,
    active: Option<ActiveExecution>,
    ui_pending: BTreeMap<String, UiPending>,
    chunks: BTreeMap<String, ChunkState>,
    sub_executions: BTreeMap<String, String>,
}

impl SessionActor {
    fn request_id(&mut self) -> String {
        self.next_request_id += 1;
        format!("c{:016x}", self.next_request_id)
    }

    async fn write_line(&mut self, line: &str) -> Result<(), String> {
        if let Err(error) = self.stdin.write_all(line.as_bytes()).await {
            return Err(format!("omp stdin write: {error}"));
        }
        if let Err(error) = self.stdin.write_all(b"\n").await {
            return Err(format!("omp stdin write: {error}"));
        }
        self.stdin
            .flush()
            .await
            .map_err(|error| format!("omp stdin write: {error}"))
    }

    /// 发送命令并等待同 id 的 response 帧。
    async fn request(&mut self, mut command: Value) -> Result<Value, String> {
        let id = self.request_id();
        command["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id.clone(), tx);
        if let Err(error) = self.write_line(&command.to_string()).await {
            self.pending.remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(COMMAND_TIMEOUT, rx).await {
            Ok(Ok(result)) => result.map_err(|error| format!("omp refused: {error}")),
            Ok(Err(_)) => Err("omp session dropped request".into()),
            Err(_) => {
                self.pending.remove(&id);
                Err("omp command timeout".into())
            }
        }
    }

    async fn bootstrap(&mut self) -> Result<(), String> {
        let ready = self.next_frame().await?;
        if ready.get("type").and_then(Value::as_str) != Some("ready") {
            return Err("first omp frame was not ready".into());
        }
        let response = self
            .request(json!({"type":"negotiate_protocol","protocolVersion":PROTOCOL_VERSION}))
            .await?;
        if response.get("success").and_then(Value::as_bool) != Some(true) {
            return Err(format!("protocol negotiation refused: {response}"));
        }
        for command in [
            json!({"type":"set_event_filter","events":null,"messageUpdates":"delta"}),
            json!({"type":"set_subagent_subscription","level":"events"}),
        ] {
            let response = self.request(command).await?;
            if response.get("success").and_then(Value::as_bool) != Some(true) {
                return Err(format!("omp config command refused: {response}"));
            }
        }
        let open = self
            .request(json!({
                "type":"open_session",
                "sessionDir": self.session_dir.display().to_string()
            }))
            .await?;
        let session_id = open
            .get("data")
            .and_then(|data| data.get("sessionId"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("open_session without sessionId: {open}"))?
            .to_string();
        self.omp_session_id = Some(session_id);
        self.store
            .record_backend_mapping(
                ADAPTER,
                &self.node_id,
                self.omp_session_id.as_deref().expect("set"),
                &self.conversation_id,
            )
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    /// 从事件流取下一逻辑帧（含 v2 分片重组）。
    async fn next_frame(&mut self) -> Result<Value, String> {
        loop {
            match self.events.recv().await {
                Some(SessionEvent::Line(line)) => {
                    if line.len() > MAX_PHYSICAL_FRAME {
                        return Err("omp physical frame exceeds 1 MiB".into());
                    }
                    let frame: Value = serde_json::from_str(line.trim())
                        .map_err(|error| format!("omp frame decode: {error}"))?;
                    if frame.get("type").and_then(Value::as_str) == Some("rpc_chunk") {
                        if let Some(reassembled) = self.accept_chunk(&frame)? {
                            return serde_json::from_str(&reassembled)
                                .map_err(|error| format!("omp reassembled frame: {error}"));
                        }
                        continue;
                    }
                    return Ok(frame);
                }
                Some(SessionEvent::StdoutClosed) | None => {
                    return Err("omp stdout closed".into());
                }
            }
        }
    }

    fn accept_chunk(&mut self, frame: &Value) -> Result<Option<String>, String> {
        use base64::Engine as _;
        let chunk_id = frame
            .get("chunkId")
            .and_then(Value::as_str)
            .ok_or("rpc_chunk without chunkId")?
            .to_string();
        let index = frame
            .get("index")
            .and_then(Value::as_u64)
            .ok_or("rpc_chunk without index")? as usize;
        let count = frame
            .get("count")
            .and_then(Value::as_u64)
            .ok_or("rpc_chunk without count")? as usize;
        let data = frame
            .get("data")
            .and_then(Value::as_str)
            .ok_or("rpc_chunk without data")?;
        if count == 0 {
            return Err("rpc_chunk count out of range".into());
        }
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| "rpc_chunk data is not canonical base64".to_string())?;
        let state = self.chunks.entry(chunk_id.clone()).or_default();
        if state.count == 0 {
            state.count = count;
            state.next_index = 0;
        }
        if state.count != count || index != state.next_index {
            self.chunks.remove(&chunk_id);
            return Err("rpc_chunk sequence mismatch".into());
        }
        state.bytes += decoded.len();
        if state.bytes > MAX_REASSEMBLED {
            self.chunks.remove(&chunk_id);
            return Err("reassembled frame exceeds 64 MiB".into());
        }
        let text =
            String::from_utf8(decoded).map_err(|_| "reassembled frame is not UTF-8".to_string())?;
        state.parts.push(text);
        state.next_index += 1;
        if state.next_index == state.count {
            let state = self.chunks.remove(&chunk_id);
            return Ok(state.map(|state| state.parts.concat()));
        }
        Ok(None)
    }

    async fn run(mut self, done: watch::Sender<bool>) {
        // 启动失败：排队中的 dispatch 以 backend_unreachable 终止（证明未发送）。
        let boot = self.bootstrap().await;
        let mut boot_error = None;
        match boot {
            Ok(()) => {}
            Err(error) => {
                boot_error = Some(error);
            }
        }
        if let Some((execution_id, generation, content, reply)) = self.queued_dispatch.take() {
            match &boot_error {
                Some(error) => {
                    let _ = reply.send(Err(error.clone()));
                    let _ = self
                        .store
                        .apply_backend(
                            &execution_id,
                            generation,
                            BackendObservation::Quiescent {
                                state: TerminalState::Failed(format!(
                                    "backend_unreachable: {error}"
                                )),
                            },
                            now_ms(),
                        )
                        .await;
                }
                None => {
                    self.dispatch(execution_id, generation, content, reply)
                        .await;
                }
            }
        }
        let mut closed = false;
        loop {
            let expiry = self.next_expiry();
            let expiry_sleep = async {
                match expiry {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => std::future::pending::<()>().await,
                }
            };
            tokio::select! {
                event = self.events.recv() => {
                    match event {
                        Some(SessionEvent::Line(line)) => {
                            if line.len() > MAX_PHYSICAL_FRAME {
                                eprintln!("conex-agent omp: physical frame exceeds 1 MiB; closing session");
                                break;
                            }
                            let parsed = serde_json::from_str::<Value>(line.trim());
                            match parsed {
                                Ok(frame) if frame.get("type").and_then(Value::as_str) == Some("rpc_chunk") => {
                                    if let Err(error) = self.accept_chunk(&frame) {
                                        eprintln!("conex-agent omp: chunk error: {error}");
                                    } else if let Ok(Some(reassembled)) = self.take_complete_chunk() {
                                        match serde_json::from_str::<Value>(&reassembled) {
                                            Ok(frame) => self.handle_frame(frame).await,
                                            Err(error) => eprintln!("conex-agent omp: reassembled frame: {error}"),
                                        }
                                    }
                                }
                                Ok(frame) => self.handle_frame(frame).await,
                                Err(error) => eprintln!("conex-agent omp: frame decode: {error}"),
                            }
                        }
                        Some(SessionEvent::StdoutClosed) | None => {
                            closed = true;
                            break;
                        }
                    }
                }
                command = self.commands.recv() => {
                    match command {
                        Some(SessionCommand::Stop { deadline, reply }) => {
                            let _ = reply.send(Ok(()));
                            self.shutdown(deadline).await;
                            return;
                        }
                        Some(SessionCommand::Dispatch { execution_id, generation, content, reply }) => {
                            self.dispatch(execution_id, generation, content, reply).await;
                        }
                        Some(SessionCommand::Abort { execution_id }) => {
                            if self.active.as_ref().is_some_and(|active| active.execution_id == execution_id) {
                                let _ = self.write_line(&json!({"type":"abort"}).to_string()).await;
                            }
                        }
                        Some(SessionCommand::RespondUi { omp_request_id, response, reply }) => {
                            let mut payload = response;
                            payload["type"] = json!("extension_ui_response");
                            payload["id"] = json!(omp_request_id);
                            let _ = reply.send(self.write_line(&payload.to_string()).await);
                        }
                        Some(SessionCommand::LocateInteraction { interaction_id, reply }) => {
                            let hit = self.ui_pending.iter().find(|(_, pending)| {
                                pending.interaction_id == interaction_id
                            });
                            let _ = reply.send(hit.map(|(id, pending)| {
                                (id.clone(), pending.kind.clone())
                            }));
                        }
                        Some(SessionCommand::CancelSubagent { subagent_id, reply }) => {
                            let result = self
                                .write_line(&json!({"type":"cancel_subagent","subagentId":subagent_id}).to_string())
                                .await;
                            let _ = reply.send(result);
                        }
                        None => break,
                    }
                }
                _ = self.exited.changed(), if !closed => {
                    if *self.exited.borrow() {
                        closed = true;
                        break;
                    }
                }
                _ = expiry_sleep => {
                    self.expire_due().await;
                }
            }
        }
        if closed {
            self.process_exit().await;
        }
        let _ = done.send_replace(true);
    }

    fn next_expiry(&self) -> Option<tokio::time::Instant> {
        self.ui_pending
            .values()
            .map(|pending| {
                let delta = pending.expires_at_ms.saturating_sub(now_ms());
                tokio::time::Instant::now() + std::time::Duration::from_millis(delta)
            })
            .min()
    }

    async fn expire_due(&mut self) {
        let now = now_ms();
        let due: Vec<(String, UiPending)> = self
            .ui_pending
            .iter()
            .filter(|(_, pending)| pending.expires_at_ms <= now)
            .map(|(id, pending)| {
                (
                    id.clone(),
                    UiPending {
                        interaction_id: pending.interaction_id.clone(),
                        kind: pending.kind.clone(),
                        expires_at_ms: pending.expires_at_ms,
                    },
                )
            })
            .collect();
        for (request_id, pending) in due {
            self.ui_pending.remove(&request_id);
            // confirm 的 cancelled 在 OMP 侧等于显式 false（deny）；select/input
            // 的取消语义不确定（P0：可能落到默认选择），只能 abort 等真实静止。
            if pending.kind == "approval" {
                let _ = self
                    .write_line(
                        &json!({"type":"extension_ui_response","id":request_id,"cancelled":true})
                            .to_string(),
                    )
                    .await;
            } else if let Some(active) = self.active.as_ref() {
                let _execution_id = active.execution_id.clone();
                if let Err(error) = self.write_line(&json!({"type":"abort"}).to_string()).await {
                    eprintln!("conex-agent omp: expiry abort failed: {error}");
                }
            }
        }
    }

    fn take_complete_chunk(&mut self) -> Result<Option<String>, String> {
        let Some(chunk_id) = self
            .chunks
            .iter()
            .find(|(_, state)| state.next_index == state.count && state.count > 0)
            .map(|(id, _)| id.clone())
        else {
            return Ok(None);
        };
        let state = self.chunks.remove(&chunk_id);
        Ok(state.map(|state| state.parts.concat()))
    }

    async fn dispatch(
        &mut self,
        execution_id: String,
        generation: u64,
        content: String,
        reply: Reply<()>,
    ) {
        if self.active.is_some() {
            let _ = reply.send(Err("conversation already has an active execution".into()));
            return;
        }
        let Some(session_id) = self.omp_session_id.clone() else {
            self.queued_dispatch = Some((execution_id, generation, content, reply));
            return;
        };
        let prompt_id = self.request_id();
        if let Err(error) = self
            .write_line(&json!({"type":"prompt","message":content,"id":prompt_id}).to_string())
            .await
        {
            let _ = reply.send(Err(error.clone()));
            let _ = self
                .store
                .apply_backend(
                    &execution_id,
                    generation,
                    BackendObservation::OutcomeUnknown {
                        reason: format!("prompt send failed: {error}"),
                    },
                    now_ms(),
                )
                .await;
            return;
        }
        let _ = reply.send(Ok(()));
        let backend_ref = session_id.clone();
        self.active = Some(ActiveExecution {
            execution_id: execution_id.clone(),
            generation,
            prompt_id,
            messages: BTreeMap::new(),
            streamed_bytes: BTreeMap::new(),
            pending_status: None,
        });
        let _ = self
            .store
            .apply_backend(
                &execution_id,
                generation,
                BackendObservation::Started {
                    backend_ref: Some(backend_ref),
                },
                now_ms(),
            )
            .await;
    }

    async fn handle_frame(&mut self, frame: Value) {
        let kind = frame
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        match kind.as_str() {
            "response" => self.handle_response(frame).await,
            "message_start" => self.handle_message_start(frame).await,
            "message_update" => self.handle_message_update(frame).await,
            "message_end" => self.handle_message_end(frame).await,
            "prompt_result" => self.handle_prompt_result(frame).await,
            "session_settled" => self.handle_settled().await,
            "extension_ui_request" => self.handle_ui_request(frame).await,
            "subagent_lifecycle" => self.handle_subagent_lifecycle(frame).await,
            _ => {}
        }
    }

    async fn handle_response(&mut self, frame: Value) {
        let Some(id) = frame.get("id").and_then(Value::as_str).map(str::to_string) else {
            return;
        };
        if let Some(active) = self.active.as_ref()
            && id == active.prompt_id
        {
            if frame.get("success").and_then(Value::as_bool) == Some(true) {
                return; // Started 已应用；流事件随后。
            }
            let error = frame
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("prompt refused")
                .to_string();
            let (execution_id, generation) = (active.execution_id.clone(), active.generation);
            self.active = None;
            let _ = self
                .store
                .apply_backend(
                    &execution_id,
                    generation,
                    BackendObservation::Quiescent {
                        state: TerminalState::Failed(format!("backend_refused: {error}")),
                    },
                    now_ms(),
                )
                .await;
            return;
        }
        if let Some(reply) = self.pending.remove(&id) {
            let _ = if frame.get("success").and_then(Value::as_bool) == Some(true) {
                reply.send(Ok(frame.get("data").cloned().unwrap_or(Value::Null)))
            } else {
                reply.send(Err(frame
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("refused")
                    .to_string()))
            };
        }
    }

    async fn handle_message_start(&mut self, frame: Value) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let Some(message_id) = frame.get("messageId").and_then(Value::as_str) else {
            return;
        };
        let role = frame
            .get("message")
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            .unwrap_or("assistant")
            .to_string();
        let result = self
            .store
            .apply_backend(
                &active.execution_id,
                active.generation,
                BackendObservation::MessageStarted {
                    role,
                    backend_key: Some(message_id.to_string()),
                },
                now_ms(),
            )
            .await;
        if let Ok(result) = result
            && let Some(node_id) = result.message_id
        {
            let active = self.active.as_mut().expect("checked");
            active.messages.insert(message_id.to_string(), node_id);
            active.streamed_bytes.insert(message_id.to_string(), 0);
        }
    }

    async fn handle_message_update(&mut self, frame: Value) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let Some(omp_message) = frame.get("messageId").and_then(Value::as_str) else {
            return;
        };
        let Some(node_message) = active.messages.get(omp_message).cloned() else {
            return;
        };
        let Some(event) = frame.get("assistantMessageEvent") else {
            return;
        };
        let (execution_id, generation) = (active.execution_id.clone(), active.generation);
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "text_delta" => {
                let delta = event.get("delta").and_then(Value::as_str).unwrap_or("");
                if delta.is_empty() {
                    return;
                }
                if let Some(entry) = self
                    .active
                    .as_mut()
                    .expect("checked")
                    .streamed_bytes
                    .get_mut(omp_message)
                {
                    *entry += delta.len();
                }
                let _ = self
                    .store
                    .apply_backend(
                        &execution_id,
                        generation,
                        BackendObservation::TextAppended {
                            message_id: node_message,
                            text: delta.to_string(),
                        },
                        now_ms(),
                    )
                    .await;
            }
            "toolcall_start" | "toolcall_end" => {
                let name = event
                    .get("block")
                    .and_then(|block| block.get("toolCall"))
                    .and_then(|tool| tool.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let phase = if event_type_is_start(event) {
                    "started"
                } else {
                    "completed"
                };
                let _ = self
                    .store
                    .apply_backend(
                        &execution_id,
                        generation,
                        BackendObservation::ToolPartAppended {
                            message_id: node_message,
                            name,
                            phase: phase.to_string(),
                            summary: String::new(),
                        },
                        now_ms(),
                    )
                    .await;
            }
            _ => {}
        }
    }

    async fn handle_message_end(&mut self, frame: Value) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let Some(omp_message) = frame.get("messageId").and_then(Value::as_str) else {
            return;
        };
        let Some(node_message) = active.messages.get(omp_message).cloned() else {
            return;
        };
        // *_end 是权威快照：累计流字节 < 权威文本长度 => 无法完整表示。
        let authoritative = frame
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .map(str::len)
                    .sum::<usize>()
            })
            .unwrap_or(0);
        let streamed = active.streamed_bytes.get(omp_message).copied().unwrap_or(0);
        let (execution_id, generation) = (active.execution_id.clone(), active.generation);
        let _ = self
            .store
            .apply_backend(
                &execution_id,
                generation,
                BackendObservation::MessageCompleted {
                    message_id: node_message,
                    incomplete: authoritative > streamed,
                },
                now_ms(),
            )
            .await;
    }

    async fn handle_prompt_result(&mut self, frame: Value) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        if frame.get("id").and_then(Value::as_str) != Some(active.prompt_id.as_str()) {
            return;
        }
        let status = frame
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("error")
            .to_string();
        let settled = frame
            .get("sessionSettled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if settled {
            let (execution_id, generation) = (active.execution_id.clone(), active.generation);
            self.active = None;
            self.apply_terminal(&execution_id, generation, &status)
                .await;
        } else {
            self.active.as_mut().expect("active checked").pending_status = Some(status);
        }
    }

    async fn handle_settled(&mut self) {
        let Some(status) = self
            .active
            .as_ref()
            .and_then(|active| active.pending_status.clone())
        else {
            return;
        };
        let (execution_id, generation) = (
            self.active
                .as_ref()
                .expect("active checked")
                .execution_id
                .clone(),
            self.active.as_ref().expect("active checked").generation,
        );
        self.active = None;
        self.apply_terminal(&execution_id, generation, &status)
            .await;
    }

    async fn apply_terminal(&self, execution_id: &str, generation: u64, status: &str) {
        let state = match status {
            "completed" => TerminalState::Succeeded,
            "aborted" => TerminalState::Cancelled("backend_aborted".into()),
            other => TerminalState::Failed(format!("backend_{other}")),
        };
        let _ = self
            .store
            .apply_backend(
                execution_id,
                generation,
                BackendObservation::Quiescent { state },
                now_ms(),
            )
            .await;
    }

    async fn handle_ui_request(&mut self, frame: Value) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let Some(request_id) = frame.get("id").and_then(Value::as_str) else {
            return;
        };
        let method = frame.get("method").and_then(Value::as_str).unwrap_or("");
        let (kind, choices, prompt) = match method {
            "select" => {
                let options = frame
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(Value::as_str)
                            .map(|option| {
                                (option.to_string(), option.to_string(), "select".to_string())
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                (
                    "selection",
                    options,
                    frame
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                )
            }
            "confirm" => (
                "approval",
                vec![
                    (
                        "allow".to_string(),
                        "allow".to_string(),
                        "allow_once".to_string(),
                    ),
                    ("deny".to_string(), "deny".to_string(), "deny".to_string()),
                ],
                frame
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ),
            "input" => (
                "input",
                Vec::new(),
                frame
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ),
            // 未知 server request：显式拒绝，不无限等待、不默认批准（契约 §7）。
            other => {
                let _ = self
                    .write_line(
                        &json!({"type":"extension_ui_response","id":request_id,"cancelled":true})
                            .to_string(),
                    )
                    .await;
                eprintln!("conex-agent omp: ui request {other} refused");
                return;
            }
        };
        if kind == "selection" && choices.is_empty() {
            let _ = self
                .write_line(
                    &json!({"type":"extension_ui_response","id":request_id,"cancelled":true})
                        .to_string(),
                )
                .await;
            return;
        }
        let timeout_ms = frame
            .get("timeout")
            .and_then(Value::as_u64)
            .filter(|timeout| *timeout > 1_000)
            .map(|timeout| timeout.saturating_sub(1_000).min(NODE_INTERACTION_TTL_MS))
            .unwrap_or(NODE_INTERACTION_TTL_MS);
        let expires_at_ms = now_ms().saturating_add(timeout_ms);
        let result = self
            .store
            .apply_backend(
                &active.execution_id,
                active.generation,
                BackendObservation::InteractionOpened {
                    kind: kind.to_string(),
                    prompt,
                    choices,
                    expires_at_ms: Some(expires_at_ms),
                },
                now_ms(),
            )
            .await;
        if let Ok(result) = result
            && let Some(interaction_id) = result.interaction_id
        {
            self.ui_pending.insert(
                request_id.to_string(),
                UiPending {
                    interaction_id,
                    kind: kind.to_string(),
                    expires_at_ms,
                },
            );
        }
    }

    async fn handle_subagent_lifecycle(&mut self, frame: Value) {
        let Some(payload) = frame.get("payload") else {
            return;
        };
        let Some(subagent_id) = payload.get("id").and_then(Value::as_str) else {
            return;
        };
        let status = payload
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("started");
        let backend_ref = format!("omp-subagent:{subagent_id}");
        let conversation = self.conversation_id.clone();
        match status {
            "started" => {
                let Some(active) = self.active.as_ref() else {
                    return;
                };
                let parent = active.execution_id.clone();
                let execution = match self
                    .store
                    .admit_backend_subexecution(&conversation, &parent, &backend_ref, now_ms())
                    .await
                {
                    Ok(execution) => execution,
                    Err(error) => {
                        eprintln!("conex-agent omp: subagent admission failed: {error}");
                        return;
                    }
                };
                self.sub_executions
                    .insert(backend_ref.clone(), execution.id.clone());
                let relationship = p::Relationship {
                    id: stable_ulid(&format!("rel:{conversation}:{subagent_id}")),
                    from: parent,
                    to: backend_ref,
                    kind: "subagent".into(),
                    source: "backend".into(),
                    quality: "current".into(),
                    observed_at_ms: now_ms(),
                };
                let _ = self
                    .store
                    .record_relationship(&conversation, relationship)
                    .await;
            }
            "completed" | "failed" | "aborted" => {
                let Some(execution_id) = self.sub_executions.get(backend_ref.as_str()).cloned()
                else {
                    return;
                };
                let state = match status {
                    "completed" => TerminalState::Succeeded,
                    "aborted" => TerminalState::Cancelled("subagent_aborted".into()),
                    _ => TerminalState::Failed("subagent_failed".into()),
                };
                let generation = self
                    .store
                    .read_execution(&execution_id)
                    .await
                    .map(|execution| execution.generation)
                    .unwrap_or(0);
                let _ = self
                    .store
                    .apply_backend(
                        &execution_id,
                        generation,
                        BackendObservation::Quiescent { state },
                        now_ms(),
                    )
                    .await;
                // 终态关系保留历史质量；from 引用不变。
                if let Some(from) = self
                    .store
                    .read_execution(&execution_id)
                    .await
                    .ok()
                    .map(|execution| execution.parent_execution_id)
                    .unwrap_or(None)
                {
                    let relationship = p::Relationship {
                        id: stable_ulid(&format!("rel:{conversation}:{subagent_id}")),
                        from,
                        to: backend_ref,
                        kind: "subagent".into(),
                        source: "backend".into(),
                        quality: "historical".into(),
                        observed_at_ms: now_ms(),
                    };
                    let _ = self
                        .store
                        .record_relationship(&conversation, relationship)
                        .await;
                }
            }
            _ => {}
        }
    }

    async fn process_exit(&mut self) {
        // 自有进程确认退出：活跃执行 Quiescent(backend_terminated) 清槽（契约 §5）。
        if let Some(active) = self.active.take() {
            let _ = self
                .store
                .apply_backend(
                    &active.execution_id,
                    active.generation,
                    BackendObservation::Quiescent {
                        state: TerminalState::Failed("backend_terminated".into()),
                    },
                    now_ms(),
                )
                .await;
        }
        self.pending.clear();
    }

    async fn shutdown(&mut self, deadline: tokio::time::Instant) {
        if let Some(active) = self.active.take() {
            let _ = self
                .store
                .apply_backend(
                    &active.execution_id,
                    active.generation,
                    BackendObservation::OutcomeUnknown {
                        reason: "adapter stopped".into(),
                    },
                    now_ms(),
                )
                .await;
        }
        let _ = self.stdin.shutdown().await;
        let _ = tokio::time::timeout_at(deadline, self.exited.changed()).await;
    }
}

fn event_type_is_start(event: &Value) -> bool {
    event.get("type").and_then(Value::as_str) == Some("toolcall_start")
}

/// 会话进程 spawn 与身份登记。同会话残留的上一代进程经身份核对后终止
/// （自有进程；不误杀复用 PID 的无关进程），避免双写同一 session 目录。
async fn spawn_session(
    omp_binary: &str,
    data_dir: &std::path::Path,
    conversation_id: &str,
    workspace_root: &std::path::Path,
    store: Store,
    node_id: String,
) -> Result<Arc<SessionHandle>, Error> {
    let session_dir = data_dir.join("omp-sessions").join(conversation_id);
    crate::process::ensure_private_dir(&session_dir)
        .map_err(|error| unavailable(format!("omp session dir: {error}")))?;
    terminate_previous_omp(data_dir, conversation_id).await;
    let mut child = Command::new(omp_binary)
        .args([
            "--mode",
            "rpc",
            "--session-dir",
            &session_dir.display().to_string(),
            "--cwd",
            &workspace_root.display().to_string(),
            "--no-extensions",
            "--no-skills",
            "--no-rules",
            "--no-context-files",
            "--no-lsp",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| unavailable(format!("omp spawn: {error}")))?;
    let pid = child.id().unwrap_or(0);
    // 身份文件供下次 spawn 前核对残留进程（自有进程才终止，不误杀复用 PID）。
    let identity = if pid != 0 {
        let (start_ticks, boot_id) = crate::process::process_identity(pid).unwrap_or_default();
        json!({"pid": pid, "start_ticks": start_ticks, "boot_id": boot_id})
    } else {
        json!({"pid": 0})
    };
    let _ = std::fs::write(
        data_dir
            .join("omp-sessions")
            .join(conversation_id)
            .join(".conex-identity.json"),
        identity.to_string(),
    );
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    let (event_tx, event_rx) = mpsc::channel::<SessionEvent>(256);
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await
            && event_tx.send(SessionEvent::Line(line)).await.is_ok()
        {}
        let _ = event_tx.send(SessionEvent::StdoutClosed).await;
    });
    let (exited_tx, exited_rx) = watch::channel(false);
    tokio::spawn(async move {
        let mut child = child;
        let _ = child.wait().await;
        let _ = exited_tx.send_replace(true);
    });
    let (command_tx, command_rx) = mpsc::channel::<SessionCommand>(64);
    let (done_tx, done_rx) = watch::channel(false);
    let actor = SessionActor {
        conversation_id: conversation_id.to_string(),
        session_dir,
        store,
        node_id,
        stdin,
        events: event_rx,
        commands: command_rx,
        exited: exited_rx,
        pending: BTreeMap::new(),
        next_request_id: 0,
        omp_session_id: None,
        queued_dispatch: None,
        active: None,
        ui_pending: BTreeMap::new(),
        chunks: BTreeMap::new(),
        sub_executions: BTreeMap::new(),
    };
    let handle = Arc::new(SessionHandle {
        commands: command_tx,
        done: done_rx,
    });
    tokio::spawn(async move {
        actor.run(done_tx).await;
    });
    Ok(handle)
}

/// 自有 OMP 进程的崩溃残留：身份核对一致才终止（不误杀复用 PID 的进程）。
async fn terminate_previous_omp(data_dir: &std::path::Path, conversation_id: &str) {
    use sysinfo::System;
    let identity_path = data_dir
        .join("omp-sessions")
        .join(conversation_id)
        .join(".conex-identity.json");
    let Ok(text) = std::fs::read_to_string(&identity_path) else {
        return;
    };
    let Ok(identity) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    let (Some(pid), start, boot) = (
        identity.get("pid").and_then(Value::as_u64),
        identity
            .get("start_ticks")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        identity
            .get("boot_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ) else {
        return;
    };
    if pid == 0 {
        return;
    }
    let target = sysinfo::Pid::from_u32(u32::try_from(pid).unwrap_or(0));
    let mut system = System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target]), true);
    if let Some(process) = system.process(target)
        && crate::process::process_identity(target.as_u32())
            .is_ok_and(|(ticks, boot_now)| ticks == start && boot_now == boot)
    {
        process.kill();
    }
    let _ = std::fs::remove_file(&identity_path);
}

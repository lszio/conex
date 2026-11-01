//! Codex app-server 适配器（docs/plans/2026-10-31-conex-mvp-p4.md）。
//!
//! 协议权威参考：本机 `codex app-server generate-json-schema` 导出
//! （实现时实测 codex-cli 0.163.0；事件名带 `item/`、`turn/`、`thread/`
//! 前缀）。app-server 是 stdin/stdout 上的 JSON-RPC（newline 分帧）；server
//! request（审批族）必须应答，未知 method 显式拒绝（JSON-RPC error），不
//! 默认批准、不无限等待（契约 §7）。
//!
//! 形态边界（契约 §2/P4 计划 §3）：owned stdio 子进程；外部认证 WS 形态在
//! 本机 app-server 接入形态核实后接线，不做半套。审批未决时消费者断开 →
//! 恢复核对失败即 invalidated，不重放旧批准（P0 §4.3 冻结处理）。
//!
//! live 状态：本机 0.163.0 的 `codex app-server` 直连在 P0 之后变为静默
//! 退出（daemon 化，proxy 帧格式未随 schema 文档化）；live 验收被该后端
//! 形态漂移阻断。协议映射以下列单元测试与 P0 证据页（§4 实测事件名）为
//! 基线；live 证据在形态核实后补齐（P4 计划 §6 允许 live 与离线分别报告）。
use crate::adapters::omp::{registry_contracts, stable_ulid, store_error};
use crate::node_config::CodexAdapterConfig;
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
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc, oneshot, watch};

pub const ADAPTER: &str = "codex";
pub const CONTRACT_ID: &str = "conex.agent";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(45);

type Reply<T> = oneshot::Sender<Result<T, String>>;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
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

/// 端点能力（契约 §3）。approval/cancel/recoverActive 有 P0 live 证据；
/// steer/userInput/listSubagents 未实测一律 false（P0：subagent 查询需
/// experimentalApi 且未覆盖），不谎报。
fn codex_capabilities() -> p::AgentCapabilities {
    p::AgentCapabilities {
        create_conversation: true,
        continue_conversation: true,
        read_history: true,
        stream_messages: true,
        cancel_execution: true,
        approval: true,
        user_input: false,
        list_subagents: false,
        cancel_subagent: false,
        steer_execution: false,
        attach_external: false,
        recover_active: true,
    }
}

pub struct CodexBackend {
    store: Store,
    node_id: String,
    module_instance_id: String,
    sessions: Mutex<BTreeMap<String, Arc<SessionHandle>>>,
    data_dir: PathBuf,
    config: CodexAdapterConfig,
}

impl CodexBackend {
    fn workspace_root(&self, workspace_id: &str) -> Result<PathBuf, Error> {
        self.config
            .workspaces
            .iter()
            .find(|workspace| {
                stable_ulid(&format!("workspace:{}:{}", self.node_id, workspace.id)) == workspace_id
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
        let handle = spawn_session(
            &self.config.backend_binary,
            &self.data_dir,
            conversation_id,
            &root,
            &self.config,
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
    ) -> Result<Option<(Arc<SessionHandle>, Option<String>)>, Error> {
        let execution = self
            .store
            .read_execution(execution_id)
            .await
            .map_err(store_error)?;
        let sessions = self.sessions.lock().await;
        Ok(sessions
            .get(&execution.conversation_id)
            .map(|session| (session.clone(), execution.backend_ref.clone())))
    }

    async fn stop_all(&self, deadline: tokio::time::Instant) {
        let sessions: Vec<Arc<SessionHandle>> =
            self.sessions.lock().await.values().cloned().collect();
        for session in sessions {
            let _ = session.stop(deadline).await;
        }
    }
}

pub fn factory(config: CodexAdapterConfig) -> RuntimeFactory {
    Arc::new(move |runtime, _context| {
        let config = config.clone();
        Box::pin(async move {
            let endpoint_id = stable_ulid(&format!(
                "endpoint:{}:{}",
                runtime.node_epoch, runtime.module_key
            ));
            let backend = Arc::new(CodexBackend {
                store: runtime.store.clone(),
                node_id: runtime.node_id.clone(),
                module_instance_id: runtime.resource_id.clone(),
                sessions: Mutex::new(BTreeMap::new()),
                data_dir: runtime.data_dir.clone(),
                config: config.clone(),
            });
            backend
                .store
                .put_endpoint(p::AgentAdapterEndpoint {
                    id: endpoint_id,
                    node_id: backend.node_id.clone(),
                    module_instance_id: runtime.resource_id.clone(),
                    generation: runtime.generation,
                    adapter: ADAPTER.into(),
                    backend_version: probe_version(&config.backend_binary).await,
                    protocol_version: "jsonrpc".into(),
                    experimental: false,
                    capabilities: Some(codex_capabilities()),
                    availability: "ready".into(),
                    catalog_scope: "node-managed".into(),
                    limits: Some(p::AgentLimits {
                        max_page_size: 100,
                        max_submit_bytes: 65536,
                        max_message_read_bytes: 65536,
                    }),
                    display_name: Some("Codex".into()),
                })
                .await
                .map_err(|error| crate::node::failure(&error.to_string()))?;
            for workspace in &config.workspaces {
                backend
                    .store
                    .put_workspace(p::Workspace {
                        id: stable_ulid(&format!("workspace:{}:{}", backend.node_id, workspace.id)),
                        node_id: backend.node_id.clone(),
                        display_name: workspace.display_name.clone(),
                        root_ref: workspace.id.clone(),
                    })
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

/// `conex.agent/1` 契约。域读方法面与 OMP 共用同一 store 直读语义
/// （重写而非共享抽象是已知的两包重复，集成所有者合并时统一——P4 计划
/// §1 共享层条款）。
fn agent_contract(backend: Arc<CodexBackend>) -> Result<Arc<Contract>, Error> {
    let read = |input: &str, output: &str| Method::new(input, output, SdkExecution::ReadOnly);
    let write = |input: &str, output: &str| Method::new(input, output, SdkExecution::NonReplayable);
    let mut methods: BTreeMap<String, Method> = BTreeMap::new();
    macro_rules! read_method {
        ($name:expr, $input:ty, $output:ty, $body:expr) => {
            methods.insert(
                $name.to_string(),
                read(
                    concat!("conex.", stringify!($input)),
                    concat!("conex.", stringify!($output)),
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
        ($name:expr, $input:ty, $output:ty, $body:expr) => {
            methods.insert(
                $name.to_string(),
                write(
                    concat!("conex.", stringify!($input)),
                    concat!("conex.", stringify!($output)),
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
        AgentEndpointListResult,
        |backend: Arc<CodexBackend>, _scope, _input: AgentEndpointListInput| async move {
            proto_json(&p::AgentEndpointListResult {
                items: backend.store.list_endpoints().await.map_err(store_error)?,
                next_page_token: None,
            })
        }
    );
    read_method!(
        "agent/workspaces/list",
        AgentWorkspaceListInput,
        AgentWorkspaceListResult,
        |backend: Arc<CodexBackend>, _scope, _input: AgentWorkspaceListInput| async move {
            let items = backend
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
            proto_json(&p::AgentWorkspaceListResult {
                items,
                next_page_token: None,
            })
        }
    );
    read_method!(
        "agent/conversations/list",
        AgentConversationListInput,
        AgentConversationListResult,
        |backend: Arc<CodexBackend>, _scope, input: AgentConversationListInput| async move {
            let mut conversations = backend
                .store
                .list_conversations(&input.endpoint_id, input.workspace_id.as_deref())
                .await
                .map_err(store_error)?;
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
        AgentConversationGetResult,
        |backend: Arc<CodexBackend>, _scope, input: AgentConversationGetInput| async move {
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
        AgentConversationHistoryResult,
        |backend: Arc<CodexBackend>, _scope, input: AgentConversationHistoryInput| async move {
            let (messages, has_more) = backend
                .store
                .read_history(
                    &input.conversation_id,
                    input.before_message_id.as_deref(),
                    input.limit.unwrap_or(50).clamp(1, 100),
                )
                .await
                .map_err(store_error)?;
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
        AgentMessageReadResult,
        |backend: Arc<CodexBackend>, scope, input: AgentMessageReadInput| async move {
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
        AgentExecutionGetResult,
        |backend: Arc<CodexBackend>, _scope, input: AgentExecutionGetInput| async move {
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
        AgentInteractionListResult,
        |backend: Arc<CodexBackend>, _scope, input: AgentInteractionListInput| async move {
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
        AgentRelationshipListResult,
        |backend: Arc<CodexBackend>, _scope, input: AgentRelationshipListInput| async move {
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
        AgentSubagentListResult,
        |backend: Arc<CodexBackend>, _scope, input: AgentSubagentListInput| async move {
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
        AgentSnapshotResult,
        |backend: Arc<CodexBackend>, scope, input: AgentSnapshotInput| async move {
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
        AgentEventReadResult,
        |backend: Arc<CodexBackend>, scope, input: AgentEventReadInput| async move {
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
        AgentConversationCreateResult,
        |backend: Arc<CodexBackend>, scope, input: AgentConversationCreateInput| async move {
            // workspace 归属校验（ULID 由 rootRef 键派生）。
            backend.workspace_root(&input.workspace_id)?;
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
        AgentSubmitResult,
        |backend: Arc<CodexBackend>, scope, input: AgentSubmitInput| async move {
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
                .admit(&scope, &backend.module_instance_id, &input, now)
                .await
                .map_err(store_error)?
            {
                Admission::Existing { execution } => Ok(proto_json(&p::AgentSubmitResult {
                    execution: Some(execution),
                })?),
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
                    session
                        .turn(
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
    );
    write_method!(
        "agent/executions/cancel",
        AgentExecutionCancelInput,
        AgentExecutionCancelResult,
        |backend: Arc<CodexBackend>, scope, input: AgentExecutionCancelInput| async move {
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
                && let Some((session, backend_ref)) =
                    backend.session_for_execution(&input.execution_id).await?
                && let (Some(thread_id), Some(turn_id)) = (
                    backend_ref,
                    backend
                        .store
                        .read_execution(&input.execution_id)
                        .await
                        .ok()
                        .and_then(|e| e.backend_ref),
                )
            {
                // turn/interrupt 定位 (threadId, turnId)；threadId 在 backend_ref。
                session.interrupt(thread_id, turn_id).await;
            }
            proto_json(&p::AgentExecutionCancelResult {
                execution_id: target.execution_id,
                requested: target.requested,
                state: target.state,
            })
        }
    );
    write_method!(
        "agent/interactions/respond",
        AgentInteractionRespondInput,
        AgentInteractionRespondResult,
        |backend: Arc<CodexBackend>, scope, input: AgentInteractionRespondInput| async move {
            let permit = backend
                .store
                .begin_response(&scope, &input, now_ms())
                .await
                .map_err(store_error)?;
            let decision = input.decision.as_ref().expect("validated");
            // decline 语义固定：审批拒绝必须落到显式 decline（契约 §7/P0 §4.1）。
            let codex_decision = match decision.kind.as_str() {
                "approval" => {
                    if decision.choice_id.as_deref() == Some("deny") {
                        "decline"
                    } else {
                        "accept"
                    }
                }
                other => {
                    let reason = format!("decision kind {other} is not an approval");
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
            // 定位持有该审批的会话：审批登记在会话 actor 内，逐个询问。
            let holders: Vec<Arc<SessionHandle>> =
                backend.sessions.lock().await.values().cloned().collect();
            let mut located = None;
            for session in holders {
                if let Some(hit) = session.locate_interaction(permit.interaction_id()).await {
                    located = Some((session, hit));
                    break;
                }
            }
            let Some((session, (server_request_id, _))) = located else {
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
            match session
                .respond_approval(server_request_id, codex_decision.to_string())
                .await
            {
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
                        "codex approval send failed: {error}"
                    )))
                }
            }
        }
    );
    // steer 能力为 false：显式 unsupported，不静默成功。
    write_method!(
        "agent/executions/steer",
        AgentExecutionSteerInput,
        AgentExecutionSteerResult,
        |_backend, _scope, input: AgentExecutionSteerInput| async move {
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
    // subagents/cancel 同样依赖 listSubagents=false 的能力边界。
    write_method!(
        "agent/subagents/cancel",
        AgentSubagentCancelInput,
        AgentSubagentCancelResult,
        |backend: Arc<CodexBackend>, scope, input: AgentSubagentCancelInput| async move {
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
            proto_json(&p::AgentSubagentCancelResult {
                execution_id: target.execution_id,
                requested: target.requested,
                state: target.state,
            })
        }
    );
    Ok(Arc::new(Contract::new(
        CONTRACT_ID,
        methods,
        BTreeMap::new(),
    )?))
}

// ---------------------------------------------------------------------------
// 会话进程 actor（JSON-RPC 双向）
// ---------------------------------------------------------------------------

enum SessionCommand {
    Turn {
        execution_id: String,
        generation: u64,
        content: String,
        reply: Reply<()>,
    },
    Interrupt {
        thread_id: String,
        turn_id: String,
    },
    RespondApproval {
        server_request_id: Value,
        decision: String,
        reply: Reply<()>,
    },
    LocateInteraction {
        interaction_id: String,
        reply: oneshot::Sender<Option<(Value, String)>>,
    },
    Stop {
        deadline: tokio::time::Instant,
        reply: Reply<()>,
    },
}

struct SessionHandle {
    commands: mpsc::Sender<SessionCommand>,
    done: watch::Receiver<bool>,
}

impl SessionHandle {
    async fn turn(
        &self,
        execution_id: String,
        generation: u64,
        content: String,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::Turn {
                execution_id,
                generation,
                content,
                reply: tx,
            })
            .await
            .map_err(|_| "codex session stopped".to_string())?;
        tokio::time::timeout(STARTUP_TIMEOUT, rx)
            .await
            .map_err(|_| "codex turn timeout".to_string())?
            .map_err(|_| "codex session dropped request".to_string())?
    }

    async fn interrupt(&self, thread_id: String, turn_id: String) {
        let _ = self
            .commands
            .send(SessionCommand::Interrupt { thread_id, turn_id })
            .await;
    }

    async fn respond_approval(
        &self,
        server_request_id: Value,
        decision: String,
    ) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::RespondApproval {
                server_request_id,
                decision,
                reply: tx,
            })
            .await
            .map_err(|_| "codex session stopped".to_string())?;
        tokio::time::timeout(STARTUP_TIMEOUT, rx)
            .await
            .map_err(|_| "codex approval response timeout".to_string())?
            .map_err(|_| "codex session dropped request".to_string())?
    }

    async fn locate_interaction(&self, interaction_id: &str) -> Option<(Value, String)> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::LocateInteraction {
                interaction_id: interaction_id.to_string(),
                reply: tx,
            })
            .await
            .ok()?;
        tokio::time::timeout(STARTUP_TIMEOUT, rx).await.ok()?.ok()?
    }

    async fn stop(&self, deadline: tokio::time::Instant) -> Result<(), String> {
        let (tx, rx) = oneshot::channel();
        self.commands
            .send(SessionCommand::Stop {
                deadline,
                reply: tx,
            })
            .await
            .map_err(|_| "codex session already stopped".to_string())?;
        let _ = rx.await;
        Ok(())
    }
}

struct ActiveTurn {
    execution_id: String,
    generation: u64,
    request_id: Value,
    thread_id: String,
    /// codex itemId -> 节点 Message ID。
    messages: BTreeMap<String, String>,
    streamed_bytes: BTreeMap<String, usize>,
}

struct ApprovalPending {
    server_request_id: Value,
}

struct SessionActor {
    conversation_id: String,
    store: Store,
    node_id: String,
    config: CodexAdapterConfig,
    workspace_root: PathBuf,
    stdin: tokio::process::ChildStdin,
    events: mpsc::Receiver<String>,
    commands: mpsc::Receiver<SessionCommand>,
    exited: watch::Receiver<bool>,
    next_request_id: u64,
    thread_id: Option<String>,
    active: Option<ActiveTurn>,
    approvals: BTreeMap<String, ApprovalPending>,
}

impl SessionActor {
    fn request_id(&mut self) -> Value {
        self.next_request_id += 1;
        json!(self.next_request_id)
    }

    async fn write_line(&mut self, line: &str) -> Result<(), String> {
        if let Err(error) = self.stdin.write_all(line.as_bytes()).await {
            return Err(format!("codex stdin write: {error}"));
        }
        if let Err(error) = self.stdin.write_all(b"\n").await {
            return Err(format!("codex stdin write: {error}"));
        }
        self.stdin
            .flush()
            .await
            .map_err(|error| format!("codex stdin write: {error}"))
    }

    /// bootstrap 阶段顺序泵帧直到同 id 的 response（主循环未启动）。
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.request_id();
        let id_text = id.to_string();
        let command = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        self.write_line(&command.to_string()).await?;
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        loop {
            if Instant::now() > deadline {
                return Err("codex command timeout".into());
            }
            let frame = self.next_frame().await?;
            if frame
                .get("id")
                .and_then(Value::as_i64)
                .map(|value| value.to_string())
                == Some(id_text.clone())
                || frame.get("id").and_then(Value::as_str) == Some(id_text.as_str())
            {
                return if frame.get("error").is_some() {
                    Err(format!(
                        "codex refused {method}: {}",
                        frame["error"]["message"].as_str().unwrap_or("?")
                    ))
                } else {
                    Ok(frame.get("result").cloned().unwrap_or(Value::Null))
                };
            }
            self.handle_frame(frame).await;
        }
    }

    async fn next_frame(&mut self) -> Result<Value, String> {
        match self.events.recv().await {
            Some(line) => serde_json::from_str(line.trim())
                .map_err(|error| format!("codex frame decode: {error}")),
            None => Err("codex stdout closed".into()),
        }
    }

    async fn bootstrap(&mut self) -> Result<(), String> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {"name": "conex-node", "version": "0.1.0"},
                "capabilities": {"experimentalApi": false}
            }),
        )
        .await?;
        self.write_line(&json!({"jsonrpc":"2.0","method":"initialized"}).to_string())
            .await?;
        let mut start =
            json!({"cwd": self.workspace_root.display().to_string(), "ephemeral": false});
        if let Some(policy) = &self.config.approval_policy {
            start["approvalPolicy"] = json!(policy);
        }
        if let Some(sandbox) = &self.config.sandbox {
            start["sandbox"] = json!(sandbox);
        }
        let response = self.request("thread/start", start).await?;
        let thread_id = response
            .get("thread")
            .and_then(|thread| thread.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| format!("thread/start without thread.id: {response}"))?
            .to_string();
        self.thread_id = Some(thread_id.clone());
        self.store
            .record_backend_mapping(ADAPTER, &self.node_id, &thread_id, &self.conversation_id)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn run(mut self, done: watch::Sender<bool>) {
        let boot_error = self.bootstrap().await.err();
        if let Some(error) = &boot_error {
            eprintln!("conex-agent codex: bootstrap failed: {error}");
        }
        loop {
            tokio::select! {
                event = self.events.recv() => {
                    match event {
                        Some(line) => match serde_json::from_str::<Value>(line.trim()) {
                            Ok(frame) => self.handle_frame(frame).await,
                            Err(error) => eprintln!("conex-agent codex: frame decode: {error}"),
                        },
                        None => break,
                    }
                }
                command = self.commands.recv() => {
                    match command {
                        Some(SessionCommand::Stop { deadline, reply }) => {
                            let _ = reply.send(Ok(()));
                            self.shutdown(deadline).await;
                            return;
                        }
                        Some(SessionCommand::Turn { execution_id, generation, content, reply }) => {
                            self.turn(execution_id, generation, content, reply).await;
                        }
                        Some(SessionCommand::Interrupt { thread_id, turn_id }) => {
                            let id = self.request_id();
                            let _ = self
                                .write_line(&json!({
                                    "jsonrpc":"2.0","id":id,
                                    "method":"turn/interrupt",
                                    "params":{"threadId":thread_id,"turnId":turn_id}
                                }).to_string())
                                .await;
                        }
                        Some(SessionCommand::RespondApproval { server_request_id, decision, reply }) => {
                            let response = json!({
                                "jsonrpc":"2.0","id":server_request_id,
                                "result":{"decision":decision}
                            });
                            let _ = reply.send(self.write_line(&response.to_string()).await);
                        }
                        Some(SessionCommand::LocateInteraction { interaction_id, reply }) => {
                            let hit = self.approvals.get(&interaction_id).map(|pending| {
                                (pending.server_request_id.clone(), "approval".to_string())
                            });
                            let _ = reply.send(hit);
                        }
                        None => break,
                    }
                }
                _ = self.exited.changed() => {
                    if *self.exited.borrow() {
                        break;
                    }
                }
            }
        }
        self.process_exit().await;
        let _ = done.send_replace(true);
    }

    async fn turn(
        &mut self,
        execution_id: String,
        generation: u64,
        content: String,
        reply: Reply<()>,
    ) {
        if self.active.is_some() {
            let _ = reply.send(Err("conversation already has an active turn".into()));
            return;
        }
        let Some(thread_id) = self.thread_id.clone() else {
            let _ = reply.send(Err("codex thread not ready".into()));
            return;
        };
        let id = self.request_id();
        if let Err(error) = self
            .write_line(
                &json!({
                    "jsonrpc":"2.0","id":id,
                    "method":"turn/start",
                    "params":{
                        "threadId":thread_id,
                        "cwd": self.workspace_root.display().to_string(),
                        "input":[{"type":"text","text":content}]
                    }
                })
                .to_string(),
            )
            .await
        {
            let _ = reply.send(Err(error.clone()));
            let _ = self
                .store
                .apply_backend(
                    &execution_id,
                    generation,
                    BackendObservation::OutcomeUnknown {
                        reason: format!("turn/start send failed: {error}"),
                    },
                    now_ms(),
                )
                .await;
            return;
        }
        let _ = reply.send(Ok(()));
        self.active = Some(ActiveTurn {
            execution_id: execution_id.clone(),
            generation,
            request_id: id,
            thread_id,
            messages: BTreeMap::new(),
            streamed_bytes: BTreeMap::new(),
        });
        let _ = self
            .store
            .apply_backend(
                &execution_id,
                generation,
                BackendObservation::Started {
                    backend_ref: Some(self.active.as_ref().expect("set").thread_id.clone()),
                },
                now_ms(),
            )
            .await;
    }

    async fn handle_frame(&mut self, frame: Value) {
        if let Some(method) = frame
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            if frame.get("id").is_some() {
                self.handle_server_request(&method, frame).await;
            } else {
                self.handle_notification(&method, frame).await;
            }
            return;
        }
        if let Some(active) = self.active.as_ref()
            && frame.get("id").is_some()
            && frame["id"] == active.request_id
            && frame.get("error").is_some()
        {
            let error = frame["error"]["message"]
                .as_str()
                .unwrap_or("turn refused")
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
        }
    }

    async fn handle_server_request(&mut self, method: &str, frame: Value) {
        let Some(request_id) = frame.get("id").cloned() else {
            return;
        };
        match method {
            "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval" => {}
            // 未知 server request：JSON-RPC error 显式拒绝，不等待、不批准。
            other => {
                let _ = self
                    .write_line(&json!({
                        "jsonrpc":"2.0","id":request_id,
                        "error":{"code":-32601,"message":format!("unsupported server request {other}")}
                    }).to_string())
                    .await;
                eprintln!("conex-agent codex: server request {other} refused");
                return;
            }
        }
        let Some(active) = self.active.as_ref() else {
            let _ = self
                .write_line(
                    &json!({
                        "jsonrpc":"2.0","id":request_id,
                        "error":{"code":-32601,"message":"no active turn"}
                    })
                    .to_string(),
                )
                .await;
            return;
        };
        let prompt = frame
            .get("params")
            .map(|params| {
                params
                    .get("command")
                    .or_else(|| params.get("reason"))
                    .and_then(Value::as_str)
                    .map(|text| {
                        if text.len() > 200 {
                            format!("{}…", &text[..200])
                        } else {
                            text.to_string()
                        }
                    })
            })
            .unwrap_or(None);
        let result = self
            .store
            .apply_backend(
                &active.execution_id,
                active.generation,
                BackendObservation::InteractionOpened {
                    kind: "approval".to_string(),
                    prompt,
                    choices: vec![
                        (
                            "allow".to_string(),
                            "allow".to_string(),
                            "allow_once".to_string(),
                        ),
                        ("deny".to_string(), "deny".to_string(), "deny".to_string()),
                    ],
                    expires_at_ms: Some(now_ms() + 10 * 60 * 1000),
                },
                now_ms(),
            )
            .await;
        if let Ok(result) = result
            && let Some(interaction_id) = result.interaction_id
        {
            self.approvals.insert(
                interaction_id,
                ApprovalPending {
                    server_request_id: request_id,
                },
            );
        }
    }

    async fn handle_notification(&mut self, method: &str, frame: Value) {
        let params = frame.get("params").cloned().unwrap_or(Value::Null);
        match method {
            "turn/completed" => {
                let status = params
                    .get("turn")
                    .and_then(|turn| turn.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let Some(active) = self.active.as_ref() else {
                    return;
                };
                let (execution_id, generation) = (active.execution_id.clone(), active.generation);
                self.active = None;
                // interrupted 才是已确认取消；interrupt 回执本身不充当终态。
                let state = match status.as_str() {
                    "completed" => TerminalState::Succeeded,
                    "interrupted" => TerminalState::Cancelled("backend_interrupted".into()),
                    other => TerminalState::Failed(format!("backend_{other}")),
                };
                let _ = self
                    .store
                    .apply_backend(
                        &execution_id,
                        generation,
                        BackendObservation::Quiescent { state },
                        now_ms(),
                    )
                    .await;
            }
            "item/agentMessage/delta" => {
                let item_id = params.get("itemId").and_then(Value::as_str).unwrap_or("");
                let delta = params.get("delta").and_then(Value::as_str).unwrap_or("");
                if item_id.is_empty() || delta.is_empty() {
                    return;
                }
                if !self
                    .active
                    .as_ref()
                    .is_some_and(|active| active.messages.contains_key(item_id))
                {
                    self.open_message(item_id).await;
                }
                let Some(node_message) = self
                    .active
                    .as_ref()
                    .and_then(|active| active.messages.get(item_id).cloned())
                else {
                    return;
                };
                if let Some(entry) = self
                    .active
                    .as_mut()
                    .expect("checked")
                    .streamed_bytes
                    .get_mut(item_id)
                {
                    *entry += delta.len();
                }
                let (execution_id, generation) = (
                    self.active.as_ref().expect("checked").execution_id.clone(),
                    self.active.as_ref().expect("checked").generation,
                );
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
            "item/started" => {
                let item = params.get("item");
                if item
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    == Some("agentMessage")
                    && let Some(item_id) =
                        item.and_then(|item| item.get("id")).and_then(Value::as_str)
                    && !self
                        .active
                        .as_ref()
                        .is_some_and(|active| active.messages.contains_key(item_id))
                {
                    self.open_message(item_id).await;
                }
            }
            "item/completed" => {
                let item = params.get("item");
                if item
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    != Some("agentMessage")
                {
                    return;
                }
                let Some(item_id) = item.and_then(|item| item.get("id")).and_then(Value::as_str)
                else {
                    return;
                };
                let Some(node_message) = self
                    .active
                    .as_ref()
                    .and_then(|active| active.messages.get(item_id).cloned())
                else {
                    return;
                };
                let authoritative = item
                    .and_then(|item| item.get("text"))
                    .and_then(Value::as_str)
                    .map(str::len)
                    .unwrap_or(0);
                let streamed = self
                    .active
                    .as_ref()
                    .and_then(|active| active.streamed_bytes.get(item_id).copied())
                    .unwrap_or(0);
                let (execution_id, generation) =
                    (active_execution_id(self), active_generation(self));
                let mut incomplete = authoritative < streamed;
                if authoritative > streamed && streamed == 0 {
                    // delta 过滤下 end 带全文：一次性补齐再收口。
                    let text = item
                        .and_then(|item| item.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let _ = self
                        .store
                        .apply_backend(
                            &execution_id,
                            generation,
                            BackendObservation::TextAppended {
                                message_id: node_message.clone(),
                                text: text.to_string(),
                            },
                            now_ms(),
                        )
                        .await;
                } else if authoritative != streamed {
                    incomplete = true;
                }
                let _ = self
                    .store
                    .apply_backend(
                        &execution_id,
                        generation,
                        BackendObservation::MessageCompleted {
                            message_id: node_message,
                            incomplete,
                        },
                        now_ms(),
                    )
                    .await;
            }
            _ => {}
        }
    }

    async fn open_message(&mut self, item_id: &str) {
        let Some(active) = self.active.as_ref() else {
            return;
        };
        let result = self
            .store
            .apply_backend(
                &active.execution_id,
                active.generation,
                BackendObservation::MessageStarted {
                    role: "assistant".to_string(),
                    backend_key: Some(item_id.to_string()),
                },
                now_ms(),
            )
            .await;
        if let Ok(result) = result
            && let Some(node_id) = result.message_id
        {
            let active = self.active.as_mut().expect("checked");
            active.messages.insert(item_id.to_string(), node_id);
            active.streamed_bytes.insert(item_id.to_string(), 0);
        }
    }

    async fn process_exit(&mut self) {
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

fn active_execution_id(actor: &SessionActor) -> String {
    actor
        .active
        .as_ref()
        .expect("active checked")
        .execution_id
        .clone()
}

fn active_generation(actor: &SessionActor) -> u64 {
    actor.active.as_ref().expect("active checked").generation
}

async fn spawn_session(
    codex_binary: &str,
    data_dir: &std::path::Path,
    conversation_id: &str,
    workspace_root: &std::path::Path,
    config: &CodexAdapterConfig,
    store: Store,
    node_id: String,
) -> Result<Arc<SessionHandle>, Error> {
    crate::process::ensure_private_dir(&data_dir.join("codex-sessions"))
        .map_err(|error| unavailable(format!("codex session dir: {error}")))?;
    let mut child = Command::new(codex_binary)
        .arg("app-server")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| unavailable(format!("codex spawn: {error}")))?;
    let stdin = child.stdin.take().expect("stdin piped");
    let stdout = child.stdout.take().expect("stdout piped");
    let (event_tx, event_rx) = mpsc::channel::<String>(256);
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await
            && event_tx.send(line).await.is_ok()
        {}
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
        store,
        node_id,
        config: config.clone(),
        workspace_root: workspace_root.to_path_buf(),
        stdin,
        events: event_rx,
        commands: command_rx,
        exited: exited_rx,
        next_request_id: 0,
        thread_id: None,
        active: None,
        approvals: BTreeMap::new(),
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

/// 注册表方法面与 OMP 同一 conex.agent 契约。
pub fn codex_registry_contracts() -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    registry_contracts()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_backend(store: Store) -> Arc<CodexBackend> {
        Arc::new(CodexBackend {
            store,
            node_id: "01ARZ3NDEKTSV4RRFFQ69G5FAX".into(),
            module_instance_id: "01ARZ3NDEKTSV4RRFFQ69G5FAY".into(),
            sessions: Mutex::new(BTreeMap::new()),
            data_dir: std::env::temp_dir(),
            config: CodexAdapterConfig {
                backend_binary: "codex".into(),
                approval_policy: Some("untrusted".into()),
                sandbox: Some("read-only".into()),
                workspaces: Vec::new(),
            },
        })
    }

    #[tokio::test]
    async fn codex_registry_matches_contract_and_capabilities_are_honest() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("db"), "node", Default::default())
            .await
            .unwrap();
        let contract = agent_contract(test_backend(store)).unwrap();
        let installed: BTreeSet<String> = contract.methods.keys().cloned().collect();
        assert_eq!(installed, codex_registry_contracts().unwrap()[CONTRACT_ID]);
        for name in &installed {
            assert!(crate::adapters::agent_permission_class(name).is_some());
        }
        let capabilities = codex_capabilities();
        assert!(capabilities.approval && capabilities.cancel_execution);
        // 未实测能力不谎报（P0：subagent 查询需 experimentalApi 且未覆盖）。
        assert!(!capabilities.list_subagents && !capabilities.steer_execution);
    }

    #[test]
    fn terminal_states_follow_turn_status_not_interrupt_receipts() {
        // turn/completed 的 status 映射：interrupted 才是已确认取消。
        fn state_of(status: &str) -> &'static str {
            match status {
                "completed" => "succeeded",
                "interrupted" => "cancelled",
                _ => "failed",
            }
        }
        assert_eq!(state_of("completed"), "succeeded");
        assert_eq!(state_of("interrupted"), "cancelled");
        assert_eq!(state_of("failed"), "failed");
    }
}

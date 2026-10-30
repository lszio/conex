//! 领域类型：不可克隆凭证、后端观察、错误与限额（agent-domain 契约 §5-§7、
//! event-recovery 契约 §4-§5）。记录结构使用 conex_proto 生成类型，
//! 本模块不重复建模。

use conex_proto::Execution;

/// 本进程新准入产生、不可克隆的发送凭证（契约 §5 第 3-4 步）。
/// 不上 wire、不从数据库恢复；无 Clone 实现即不可克隆，私有字段阻止外部构造。
/// mark_dispatching 以值消费 token，并用数据库 accepted→dispatching CAS
/// 防止跨连接重复发送。
pub struct DispatchToken {
    execution_id: String,
    generation: u64,
    _secret: (),
}

impl DispatchToken {
    pub(crate) fn new(execution_id: String, generation: u64) -> Self {
        Self {
            execution_id,
            generation,
            _secret: (),
        }
    }
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl std::fmt::Debug for DispatchToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchToken")
            .field("execution_id", &self.execution_id)
            .finish_non_exhaustive()
    }
}

/// mark_dispatching 提交后的一次性发送许可；数据库 CAS 是真正的单次闸门，
/// 该对象只作为发送前证据存在（不可克隆）。
pub struct DispatchPermit {
    execution_id: String,
    generation: u64,
    _secret: (),
}

impl DispatchPermit {
    pub(crate) fn new(execution_id: String, generation: u64) -> Self {
        Self {
            execution_id,
            generation,
            _secret: (),
        }
    }
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl std::fmt::Debug for DispatchPermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchPermit")
            .field("execution_id", &self.execution_id)
            .finish_non_exhaustive()
    }
}

/// 一次性答复许可：begin_response 成功 CAS 后返回，finish_response 以值消费。
pub struct ResponsePermit {
    interaction_id: String,
    decision_digest: String,
    responder_id: String,
    _secret: (),
}

impl ResponsePermit {
    pub(crate) fn new(
        interaction_id: String,
        decision_digest: String,
        responder_id: String,
    ) -> Self {
        Self {
            interaction_id,
            decision_digest,
            responder_id,
            _secret: (),
        }
    }
    pub fn interaction_id(&self) -> &str {
        &self.interaction_id
    }
    pub(crate) fn decision_digest(&self) -> &str {
        &self.decision_digest
    }
    pub(crate) fn responder_id(&self) -> &str {
        &self.responder_id
    }
}

impl std::fmt::Debug for ResponsePermit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResponsePermit")
            .field("interaction_id", &self.interaction_id)
            .finish_non_exhaustive()
    }
}

/// 已授权调用范围。授权判定发生在 P2 Authorizer；存储只接收已验证主体，
/// 并把它记入幂等键 (principalId,nodeId,endpointId,operation,key)。
#[derive(Clone, Debug)]
pub struct AuthorizedScope {
    pub principal_id: String,
}

impl AuthorizedScope {
    pub fn new(principal_id: impl Into<String>) -> Result<Self, StoreError> {
        let principal_id = principal_id.into();
        if principal_id.is_empty() {
            return Err(StoreError::InvalidArgument(
                "principal_id must not be empty".into(),
            ));
        }
        Ok(Self { principal_id })
    }
}

/// admit 结果：Existing 永远拿不到可发送 permit（契约 §5）。
#[derive(Debug)]
pub enum Admission {
    Created {
        execution: Execution,
        /// 仅新准入产生；不可克隆、不上 wire、不跨重启。
        dispatch_token: DispatchToken,
    },
    Existing {
        execution: Execution,
    },
}

impl Admission {
    pub fn execution(&self) -> &Execution {
        match self {
            Admission::Created { execution, .. } | Admission::Existing { execution } => execution,
        }
    }
}

/// finish_response 的后端答复结果（契约 §7）。
#[derive(Clone, Debug)]
pub enum BackendResponseOutcome {
    /// 后端确认已接收答复。
    Confirmed,
    /// 已发送但结果不明；交互转 unknown，严禁再次回答，只能核对。
    Unknown { reason: String },
    /// 明确不再有效（例如恢复核对失败）。
    Invalidated { reason: String },
}

/// 终态与原因（Quiescent 触发；契约 §5/§6）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalState {
    Succeeded,
    Failed(String),
    Cancelled(String),
}

impl TerminalState {
    pub fn as_str(&self) -> &'static str {
        match self {
            TerminalState::Succeeded => "succeeded",
            TerminalState::Failed(_) => "failed",
            TerminalState::Cancelled(_) => "cancelled",
        }
    }
    pub fn reason(&self) -> Option<&str> {
        match self {
            TerminalState::Succeeded => None,
            TerminalState::Failed(r) | TerminalState::Cancelled(r) => Some(r),
        }
    }
}

/// 适配器依据真实协议产生的领域观察（契约 §4/§5；不是任意后台 JSON）。
#[derive(Clone, Debug)]
pub enum BackendObservation {
    /// 后端已接受执行；backend_ref 仅受限节点记录。
    Started { backend_ref: Option<String> },
    /// 后端开始一条消息；store 分配节点 Message ID 并在 ApplyResult 返回。
    MessageStarted { role: String },
    /// 追加文本到已知节点消息的第一个 text part；store 按 UTF-8 边界拆分
    /// 为 <=4096 字节的 message.delta 事件。
    TextAppended { message_id: String, text: String },
    /// 消息结束；incomplete 表示后端明确无法完整表示。
    MessageCompleted {
        message_id: String,
        incomplete: bool,
    },
    /// 后端请求人工交互；store 分配节点 InteractionRequest ID。
    InteractionOpened {
        kind: String,
        prompt: Option<String>,
        choices: Vec<(String, String, String)>,
        expires_at_ms: Option<u64>,
    },
    /// 后端活动静止并给出终态；包含取消确认与拥有原实例的确认退出。
    Quiescent { state: TerminalState },
    /// 发送结果不明；执行转 unknown 并保持占槽。
    OutcomeUnknown { reason: String },
}

#[derive(Clone, Debug, Default)]
pub struct ApplyResult {
    pub message_id: Option<String>,
    pub interaction_id: Option<String>,
    pub execution_state: Option<String>,
    /// 迟到/冲突观察被诊断保留而不覆盖终态时为 false。
    pub applied: bool,
    pub events_appended: u64,
}

/// 恢复核对目标：只读，不携带 DispatchPermit（契约 §5）。
#[derive(Clone, Debug)]
pub struct ReconcileTarget {
    pub execution_id: String,
    pub conversation_id: String,
    pub generation: u64,
    pub state: String,
    pub backend_ref: Option<String>,
}

#[derive(Clone, Debug)]
pub struct CancelTarget {
    pub execution_id: String,
    pub requested: bool,
    pub state: String,
}

#[derive(Clone, Debug, Default)]
pub struct PruneResult {
    pub events_deleted: u64,
    pub bytes_deleted: u64,
}

/// 自有资源核对状态（private-runtime 契约 §1/§6；P2.2 计划 §3）。
///
/// created/unknown 属于未核对，仍占其运行约束；released 是已证实退出或
/// 完成受控接管后的明确终态；active 只在控制通道确认仍在运行时写入。
/// 核对不能仅凭 PID 数字，不误杀复用 PID 的其他进程。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceState {
    /// 已登记所有权，创建/启动尚未确认受控运行（如 boot/启动中途崩溃）。
    Created,
    /// 控制通道确认仍受控运行。
    Active,
    /// 崩溃/核对前状态不明；不能自动并行重建冲突实例。
    Unknown,
    /// 已证实退出或完成受控接管。
    Released,
}

impl ResourceState {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResourceState::Created => "created",
            ResourceState::Active => "active",
            ResourceState::Unknown => "unknown",
            ResourceState::Released => "released",
        }
    }

    pub(crate) fn parse_label(value: &str) -> Option<Self> {
        match value {
            "created" => Some(ResourceState::Created),
            "active" => Some(ResourceState::Active),
            "unknown" => Some(ResourceState::Unknown),
            "released" => Some(ResourceState::Released),
            _ => None,
        }
    }
}

/// 自有资源所有权记录（private-runtime §1/§6；计划 P2.2 表字段）。
/// control_identity 是不透明控制通道身份（节点受限存储），禁止投影到中心。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnedResource {
    pub id: String,
    pub module_key: String,
    pub generation: u64,
    pub owner_epoch: String,
    pub control_kind: String,
    pub control_identity: Vec<u8>,
    pub state: ResourceState,
}

/// 领域错误（契约 §9 机器错误；reason 字符串在 P2/P3 边界映射 ErrorCode，
/// 这里保持机器可识别的枚举）。
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("{entity} {id} not found")]
    NotFound { entity: &'static str, id: String },
    #[error("conversation busy, active execution {active_execution_id}")]
    ConversationBusy { active_execution_id: String },
    #[error("idempotency mismatch with existing {existing_id}")]
    IdempotencyMismatch { existing_id: String },
    #[error("quota exceeded: {0}")]
    QuotaExceeded(&'static str),
    #[error("stale revision: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("conflict: {0}")]
    Conflict(&'static str),
    #[error("payload too large: limit {limit}")]
    PayloadTooLarge { limit: usize },
    #[error(
        "resume unavailable for {stream_id} epoch {epoch}: resync required (min {min}, head {head})"
    )]
    ResumeUnavailable {
        stream_id: String,
        epoch: String,
        min: u64,
        head: u64,
    },
    #[error("outcome unknown: {0}")]
    OutcomeUnknown(String),
    #[error("store belongs to node {expected}, not {actual}")]
    NodeMismatch { expected: String, actual: String },
    #[error("database error: {0}")]
    Db(String),
    #[error("store closed or queue full")]
    Closed,
}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        match &error {
            rusqlite::Error::SqliteFailure(e, msg)
                if e.code == rusqlite::ErrorCode::DatabaseBusy
                    || e.code == rusqlite::ErrorCode::DatabaseLocked =>
            {
                StoreError::Db(format!("database busy: {msg:?}"))
            }
            _ => StoreError::Db(error.to_string()),
        }
    }
}

/// 首版规范限额（event-recovery 契约 §5；实现可调低，不得调高于 wire 预算）。
#[derive(Clone, Debug)]
pub struct StoreLimits {
    pub max_active_executions: usize,
    pub max_undecided_interactions: usize,
    pub max_active_messages: usize,
    pub max_event_bytes: usize,
    pub max_event_page_bytes: usize,
    pub max_event_page: usize,
    pub max_delta_bytes: usize,
    pub max_interaction_bytes: usize,
    pub max_message_meta_bytes: usize,
    pub max_execution_meta_bytes: usize,
    pub max_conversation_bytes: usize,
    pub max_parts: usize,
    pub max_tool_summary_bytes: usize,
    pub max_submit_bytes: usize,
    pub snapshot_terminal_executions: usize,
    pub snapshot_resolved_interactions: usize,
    pub snapshot_messages: usize,
    pub snapshot_relationships: usize,
    pub snapshot_budget_bytes: usize,
    pub retention_ms: u64,
    pub max_stream_event_bytes: u64,
    pub max_total_event_bytes: u64,
    pub dedup_ttl_ms: u64,
    pub max_dedup_records: usize,
    pub default_interaction_ttl_ms: u64,
    pub max_dispatch_queue: usize,
}

impl Default for StoreLimits {
    fn default() -> Self {
        Self {
            max_active_executions: 16,
            max_undecided_interactions: 16,
            max_active_messages: 32,
            max_event_bytes: 8192,
            max_event_page_bytes: 196608,
            max_event_page: 32,
            max_delta_bytes: 4096,
            max_interaction_bytes: 4096,
            max_message_meta_bytes: 2048,
            max_execution_meta_bytes: 1024,
            max_conversation_bytes: 2048,
            max_parts: 8,
            max_tool_summary_bytes: 512,
            max_submit_bytes: 65536,
            snapshot_terminal_executions: 20,
            snapshot_resolved_interactions: 20,
            snapshot_messages: 100,
            snapshot_relationships: 100,
            snapshot_budget_bytes: 149504,
            retention_ms: 24 * 60 * 60 * 1000,
            max_stream_event_bytes: 64 * 1024 * 1024,
            max_total_event_bytes: 512 * 1024 * 1024,
            dedup_ttl_ms: 7 * 24 * 60 * 60 * 1000,
            max_dedup_records: 100000,
            default_interaction_ttl_ms: 10 * 60 * 1000,
            max_dispatch_queue: 64,
        }
    }
}

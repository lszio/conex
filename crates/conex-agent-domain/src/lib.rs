//! Conex agent domain state: admission, dispatch boundary, approval CAS,
//! persistent events and recovery on a real node SQLite store.
//!
//! Contracts: docs/contracts/agent-domain.md, docs/contracts/event-recovery.md.
//! No Host/Adapter dependency: this crate is the single behavioral source for
//! P1 (plan docs/plans/2026-10-29-conex-mvp-p1.md).
#![forbid(unsafe_code)]

mod model;
mod projection;
mod store;
mod ulid;

pub use model::{
    Admission, AuthorizedScope, BackendObservation, BackendResponseOutcome, CancelTarget,
    DispatchPermit, DispatchToken, OwnedResource, PruneResult, ReconcileTarget, ResourceState,
    ResponsePermit, StoreError, StoreLimits, TerminalState,
};
pub use store::Store;

/// 节点侧业务 ID 生成（契约 §2：ID 为 ULID，节点分配）。进程内单调。
pub fn new_ulid() -> String {
    static GEN: std::sync::LazyLock<crate::ulid::IdGen> =
        std::sync::LazyLock::new(crate::ulid::IdGen::new);
    GEN.mint()
}

/// Re-exported protocol records so callers depend on one domain surface.
pub use conex_proto::{
    AgentAdapterEndpoint, AgentCapabilities, AgentConversationListResult, AgentEvent,
    AgentEventCursor, AgentEventReadResult, AgentLimits, AgentMessage, AgentMessageDelta,
    AgentSnapshot, Conversation, Execution, InteractionChoice, InteractionRequest, Relationship,
    Workspace,
};

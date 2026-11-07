//! Bounded audit port: reserve capacity before sensitive work, finish once.
use crate::types::CallResult;

#[derive(Debug, Clone)]
pub struct AuditStart {
    pub event_id: String,
    pub trace_id: String,
    pub principal_id: String,
    pub tenant_id: String,
    pub actor_peer_id: String,
    pub endpoint_id: String,
    pub method: String,
    pub resource_scope: String,
    pub plane: i32,
    pub policy_version: u64,
    pub phase: String,
    /// S1 模块消费主体（contracts/s1-module-principals.md §3）；None = 连接主体自身。
    pub consumer_module: Option<String>,
    /// WSS 链路代次（绑定图/审计用，不参与授权）；非链路路径为 0。
    pub generation: u64,
}

#[derive(Debug, Clone)]
pub struct AuditEnd {
    pub phase: String,
    pub outcome: String,
    pub error_code: Option<i32>,
    pub execution: String,
    pub latency_ms: u64,
}

pub trait AuditReservation: Send {
    fn finish(self: Box<Self>, end: AuditEnd) -> CallResult<()>;
}

pub trait AuditSink: Send + Sync {
    /// Reserve capacity for one audit record. Failing to reserve must reject the
    /// sensitive call before it produces side effects.
    fn reserve(&self, start: &AuditStart) -> CallResult<Box<dyn AuditReservation>>;
}

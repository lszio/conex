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

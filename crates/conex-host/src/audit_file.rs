//! Bounded file audit sink with capacity reservation and 0600 permissions.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use conex_core::{AuditEnd, AuditReservation, AuditSink, AuditStart, CallError, CallResult};
use conex_proto;
use serde_json::json;

/// Audit sink that does nothing; used when no audit file is configured.
pub struct NullAudit;

struct NullReservation;

impl AuditReservation for NullReservation {
    fn finish(self: Box<Self>, _end: AuditEnd) -> CallResult<()> {
        Ok(())
    }
}

impl AuditSink for NullAudit {
    fn reserve(&self, _start: &AuditStart) -> CallResult<Box<dyn AuditReservation>> {
        Ok(Box::new(NullReservation))
    }
}

pub struct FileAuditSink {
    file: Arc<Mutex<File>>,
    pending: Arc<AtomicUsize>,
    capacity: usize,
}

impl FileAuditSink {
    pub fn open(path: &Path, capacity: usize) -> CallResult<FileAuditSink> {
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(path).map_err(|error| {
            CallError::new(
                conex_proto::ErrorCode::Internal,
                format!("open audit file: {error}"),
            )
        })?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            pending: Arc::new(AtomicUsize::new(0)),
            capacity,
        })
    }
}

impl AuditSink for FileAuditSink {
    fn reserve(&self, start: &AuditStart) -> CallResult<Box<dyn AuditReservation>> {
        if self.pending.load(Ordering::SeqCst) >= self.capacity {
            return Err(CallError::new(
                conex_proto::ErrorCode::Unavailable,
                "audit capacity is exhausted",
            ));
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FileReservation {
            file: self.file.clone(),
            pending: self.pending.clone(),
            start: start.clone(),
            done: AtomicBool::new(false),
        }))
    }
}

struct FileReservation {
    file: Arc<Mutex<File>>,
    pending: Arc<AtomicUsize>,
    start: AuditStart,
    /// Set before the capacity is returned, so Drop never double-decrements.
    done: AtomicBool,
}

impl AuditReservation for FileReservation {
    fn finish(self: Box<Self>, end: AuditEnd) -> CallResult<()> {
        // One line carries who/what (the reserved start) together with the
        // outcome; before the P1 audit sink this file only held the End
        // summary, so it could account capacity but not answer "who did
        // what" (contracts/s1-module-principals.md §3).
        let start = &self.start;
        let record = json!({
            "eventId": start.event_id,
            "traceId": start.trace_id,
            "principalId": start.principal_id,
            "tenantId": start.tenant_id,
            "actorPeerId": start.actor_peer_id,
            "endpointId": start.endpoint_id,
            "method": start.method,
            "resourceScope": start.resource_scope,
            "plane": start.plane,
            "policyVersion": start.policy_version,
            "consumerModule": start.consumer_module,
            "generation": start.generation,
            "phase": end.phase,
            "outcome": end.outcome,
            "errorCode": end.error_code,
            "execution": end.execution,
            "latencyMs": end.latency_ms,
        });
        let mut file = self.file.lock().expect("audit file lock poisoned");
        writeln!(file, "{record}").map_err(|error| {
            CallError::new(
                conex_proto::ErrorCode::Internal,
                format!("write audit record: {error}"),
            )
        })?;
        self.done.store(true, Ordering::SeqCst);
        self.pending.fetch_sub(1, Ordering::SeqCst);
        Ok(())
    }
}

impl Drop for FileReservation {
    fn drop(&mut self) {
        // 未 finish 的预留(错误早退、panic、finish 写失败)必须归还容量,
        // 否则 pending 只增不减,容量耗尽后所有敏感调用被 Unavailable 拒绝。
        // Drop 只回滚计数,不写文件。
        if !self.done.swap(true, Ordering::SeqCst) {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(phase: &str) -> AuditStart {
        AuditStart {
            event_id: "e".into(),
            trace_id: "t".into(),
            principal_id: "p".into(),
            tenant_id: "t".into(),
            actor_peer_id: "a".into(),
            endpoint_id: "e".into(),
            method: "m".into(),
            resource_scope: "r".into(),
            plane: 1,
            policy_version: 1,
            phase: phase.into(),
            consumer_module: None,
            generation: 0,
        }
    }

    #[test]
    fn writes_records_owner_only_and_bounds_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let sink = FileAuditSink::open(&path, 2).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let first = sink.reserve(&start("authorize")).unwrap();
        let _second = sink.reserve(&start("execute")).unwrap();
        assert!(
            sink.reserve(&start("audit_finish")).is_err(),
            "capacity must reject"
        );
        first
            .finish(AuditEnd {
                phase: "authorize".into(),
                outcome: "allow".into(),
                error_code: None,
                execution: "completed".into(),
                latency_ms: 1,
            })
            .unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("\"phase\":\"authorize\""));
        // The reserved start lands in the same line: the record answers
        // "who did what", not just the outcome summary.
        assert!(content.contains("\"principalId\":\"p\""));
        assert!(content.contains("\"method\":\"m\""));
        assert!(content.contains("\"consumerModule\":null"));
        assert!(content.contains("\"generation\":0"));
    }

    #[test]
    fn drop_without_finish_releases_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.log");
        let sink = FileAuditSink::open(&path, 1).unwrap();
        let reservation = sink.reserve(&start("authorize")).unwrap();
        // Drop without finish (error early-return, panic, failed write in
        // finish) must hand the capacity slot back, otherwise pending only
        // grows and every sensitive call ends up rejected with Unavailable.
        drop(reservation);
        assert!(
            sink.reserve(&start("execute")).is_ok(),
            "dropped reservation must release its capacity slot"
        );
        // Drop only rolls back the counter; it must not write a record.
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 0);
    }
}

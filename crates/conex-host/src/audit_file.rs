//! Bounded file audit sink with capacity reservation and 0600 permissions.
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use conex_core::{AuditEnd, AuditReservation, AuditSink, AuditStart, CallError, CallResult};
use conex_proto::v1;
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
            CallError::new(v1::ErrorCode::Internal, format!("open audit file: {error}"))
        })?;
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            pending: Arc::new(AtomicUsize::new(0)),
            capacity,
        })
    }
}

impl AuditSink for FileAuditSink {
    fn reserve(&self, _start: &AuditStart) -> CallResult<Box<dyn AuditReservation>> {
        if self.pending.load(Ordering::SeqCst) >= self.capacity {
            return Err(CallError::new(
                v1::ErrorCode::Unavailable,
                "audit capacity is exhausted",
            ));
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FileReservation {
            file: self.file.clone(),
            pending: self.pending.clone(),
        }))
    }
}

struct FileReservation {
    file: Arc<Mutex<File>>,
    pending: Arc<AtomicUsize>,
}

impl AuditReservation for FileReservation {
    fn finish(self: Box<Self>, end: AuditEnd) -> CallResult<()> {
        let record = json!({
            "phase": end.phase,
            "outcome": end.outcome,
            "errorCode": end.error_code,
            "execution": end.execution,
            "latencyMs": end.latency_ms,
        });
        let mut file = self.file.lock().expect("audit file lock poisoned");
        writeln!(file, "{record}").map_err(|error| {
            CallError::new(
                v1::ErrorCode::Internal,
                format!("write audit record: {error}"),
            )
        })?;
        self.pending.fetch_sub(1, Ordering::SeqCst);
        Ok(())
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
    }
}

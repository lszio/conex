//! Operation execution class, dedup key, persistent record store.
//!
//! Implements design §7.3:
//!   execution class → retry derivation is fixed and total.
//!   dedup key = (tenant, principal, providerEndpoint, space?, resource, method, operationId).
//!   same key + different param_digest → conflict.
//!   response lost: deduplicated → operation/get returns the persisted result;
//!                  non_replayable → unknown, no auto-retry.
//!
//! The record store is local persistent (gitignored). On crash recovery the
//! in-memory state is rebuilt by scanning `<root>/<key>.json` and
//! `<root>/<key>.result.json`.
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::CallError;
use conex_proto::v1;

/// 24h default TTL for dedup records (design §7.3).
pub const DEFAULT_TTL_HOURS: u64 = 24;

/// Execution class — the four categories from design §7.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionClass {
    ReadOnly,
    Idempotent,
    Deduplicated,
    NonReplayable,
}

impl ExecutionClass {
    pub fn retry(&self) -> RetryPolicy {
        match self {
            ExecutionClass::ReadOnly => RetryPolicy::Safe,
            ExecutionClass::Idempotent => RetryPolicy::Safe,
            ExecutionClass::Deduplicated => RetryPolicy::WithOperationId,
            ExecutionClass::NonReplayable => RetryPolicy::Never,
        }
    }

    pub fn execution_state(&self, sent_to_upstream: bool) -> ExecutionState {
        match (self, sent_to_upstream) {
            (ExecutionClass::NonReplayable, true) => ExecutionState::Unknown,
            _ => ExecutionState::Completed,
        }
    }
}

/// Retry policy — derived from execution class, NOT runtime-inferred from
/// HTTP status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetryPolicy {
    Never,
    Safe,
    WithOperationId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionState {
    NotStarted,
    Completed,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DedupKey {
    pub tenant_id: String,
    pub principal_id: String,
    pub provider_endpoint_id: String,
    pub space_id: Option<String>,
    pub resource_id: String,
    pub method: String,
    pub operation_id: String,
}

impl DedupKey {
    pub fn sanitize(&self) -> String {
        let mut out = String::with_capacity(self.tenant_id.len() + 64);
        out.push_str(&sanitize_component(&self.tenant_id));
        out.push('_');
        out.push_str(&sanitize_component(&self.principal_id));
        out.push('_');
        out.push_str(&sanitize_component(&self.provider_endpoint_id));
        out.push('_');
        out.push_str(
            &self
                .space_id
                .as_deref()
                .map(sanitize_component)
                .unwrap_or_else(|| "_".to_string()),
        );
        out.push('_');
        out.push_str(&sanitize_component(&self.resource_id));
        out.push('_');
        out.push_str(&sanitize_component(&self.method));
        out.push('_');
        out.push_str(&sanitize_component(&self.operation_id));
        out
    }
}
fn sanitize_component(s: &str) -> String {
    // Slashes separate on-disk directory parts; convert them to `_` so the
    // on-disk filename for each component is a single segment.
    s.chars()
        .map(|c| match c {
            '/' | '\\' => '_',
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => c,
            _ => '_',
        })
        .collect()
}

/// Validate a key component to prevent path traversal in the on-disk path.
/// Slashes are allowed (resource_id naturally contains "/"), but the
/// resulting filename component is sanitized by `DedupKey::sanitize`.
fn ensure_safe_path_component(name: &str) -> Result<(), OperationError> {
    if name.is_empty() || name.contains("..") || name.contains('\\') {
        return Err(OperationError::PathTraversal(name.to_string()));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    Accepted,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationRecord {
    pub schema_version: u32,
    pub key: DedupKey,
    pub state: OperationState,
    pub execution_class: ExecutionClass,
    pub param_digest: String,
    pub accepted_at_ms: u64,
    pub settled_at_ms: Option<u64>,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettledResult {
    pub success: Option<serde_json::Value>,
    pub failure: Option<v1::Error>,
    pub execution: ExecutionState,
}

#[derive(Debug, Error)]
pub enum OperationError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("dedup key conflict: same key with different param_digest")]
    Conflict,
    #[error("unknown key")]
    Unknown,
    #[error("expired")]
    Expired,
    #[error("path traversal blocked: {0}")]
    PathTraversal(String),
}

impl OperationError {
    pub fn to_call_error(&self) -> CallError {
        use conex_proto::v1::ErrorCode;
        match self {
            OperationError::Conflict => CallError::new(ErrorCode::Conflict, "operationId conflict"),
            OperationError::Unknown => {
                CallError::new(ErrorCode::UnknownMethod, "operation not found")
            }
            OperationError::Expired => {
                CallError::new(ErrorCode::OutcomeUnknown, "operation expired")
            }
            OperationError::Io(_) | OperationError::Json(_) => {
                CallError::new(ErrorCode::Internal, format!("operation store: {self}"))
            }
            OperationError::PathTraversal(_) => {
                CallError::new(ErrorCode::BadRequest, "path traversal")
            }
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Default)]
struct State {
    records: HashMap<String, OperationRecord>,
    results: HashMap<String, SettledResult>,
}

pub struct OperationStore {
    root: PathBuf,
    state: Mutex<State>,
    ttl_ms: u64,
}

impl std::fmt::Debug for OperationStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationStore")
            .field("root", &self.root)
            .finish()
    }
}

impl OperationStore {
    pub fn open(root: &Path) -> Result<Self, OperationError> {
        fs::create_dir_all(root)?;
        let mut state = State::default();
        for entry in fs::read_dir(root)?.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if name.ends_with(".result.json") || !name.ends_with(".json") {
                continue;
            }
            let id = name.trim_end_matches(".json").to_string();
            let text = fs::read_to_string(&path)?;
            let record: OperationRecord = serde_json::from_str(&text)?;
            let result_path = root.join(format!("{id}.result.json"));
            let result = if result_path.exists() {
                let text = fs::read_to_string(&result_path)?;
                Some(serde_json::from_str::<SettledResult>(&text)?)
            } else {
                None
            };
            state.records.insert(id.clone(), record);
            if let Some(r) = result {
                state.results.insert(id, r);
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            state: Mutex::new(state),
            ttl_ms: DEFAULT_TTL_HOURS * 60 * 60 * 1000,
        })
    }

    pub fn with_ttl_hours(mut self, hours: u64) -> Self {
        self.ttl_ms = hours * 60 * 60 * 1000;
        self
    }

    fn persist_record(&self, id: &str, record: &OperationRecord) -> Result<(), OperationError> {
        let path = self.root.join(format!("{id}.json"));
        let tmp = self.root.join(format!("{id}.json.tmp"));
        let json = serde_json::to_string_pretty(record)?;
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.flush()?;
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn persist_result(&self, id: &str, result: &SettledResult) -> Result<(), OperationError> {
        let path = self.root.join(format!("{id}.result.json"));
        let tmp = self.root.join(format!("{id}.result.json.tmp"));
        let json = serde_json::to_string_pretty(result)?;
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.flush()?;
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn accept(
        &self,
        key: DedupKey,
        execution_class: ExecutionClass,
        param_digest: String,
    ) -> Result<OperationRecord, OperationError> {
        ensure_safe_path_component(&key.tenant_id)?;
        ensure_safe_path_component(&key.principal_id)?;
        ensure_safe_path_component(&key.provider_endpoint_id)?;
        ensure_safe_path_component(&key.resource_id)?;
        ensure_safe_path_component(&key.method)?;
        ensure_safe_path_component(&key.operation_id)?;
        let id = key.sanitize();
        let now = now_ms();
        let expires_at_ms = now.saturating_add(self.ttl_ms);
        let mut guard = self.state.lock().expect("operation state poisoned");
        if let Some(existing) = guard.records.get(&id).cloned() {
            if existing.param_digest != param_digest {
                return Err(OperationError::Conflict);
            }
            return Ok(existing);
        }
        let record = OperationRecord {
            schema_version: 1,
            key: key.clone(),
            state: OperationState::Accepted,
            execution_class,
            param_digest,
            accepted_at_ms: now,
            settled_at_ms: None,
            expires_at_ms,
        };
        self.persist_record(&id, &record)?;
        guard.records.insert(id, record.clone());
        Ok(record)
    }

    pub fn mark_running(&self, key: &DedupKey) -> Result<OperationRecord, OperationError> {
        let id = key.sanitize();
        let mut guard = self.state.lock().expect("operation state poisoned");
        let record = guard
            .records
            .get(&id)
            .cloned()
            .ok_or(OperationError::Unknown)?;
        let now = now_ms();
        if now > record.expires_at_ms {
            return Err(OperationError::Expired);
        }
        let mut updated = record.clone();
        updated.state = OperationState::Running;
        self.persist_record(&id, &updated)?;
        guard.records.insert(id.clone(), updated.clone());
        Ok(updated)
    }

    pub fn settle_success(
        &self,
        key: &DedupKey,
        success: serde_json::Value,
    ) -> Result<OperationRecord, OperationError> {
        let id = key.sanitize();
        let mut guard = self.state.lock().expect("operation state poisoned");
        let record = guard
            .records
            .get(&id)
            .cloned()
            .ok_or(OperationError::Unknown)?;
        let now = now_ms();
        if now > record.expires_at_ms {
            return Err(OperationError::Expired);
        }
        let mut updated = record.clone();
        updated.state = OperationState::Succeeded;
        updated.settled_at_ms = Some(now);
        let execution = record.execution_class.execution_state(false);
        let result = SettledResult {
            success: Some(success),
            failure: None,
            execution,
        };
        self.persist_record(&id, &updated)?;
        self.persist_result(&id, &result)?;
        guard.records.insert(id.clone(), updated.clone());
        guard.results.insert(id, result);
        Ok(updated)
    }

    pub fn settle_failure(
        &self,
        key: &DedupKey,
        failure: v1::Error,
    ) -> Result<OperationRecord, OperationError> {
        let id = key.sanitize();
        let mut guard = self.state.lock().expect("operation state poisoned");
        let record = guard
            .records
            .get(&id)
            .cloned()
            .ok_or(OperationError::Unknown)?;
        let now = now_ms();
        if now > record.expires_at_ms {
            return Err(OperationError::Expired);
        }
        let mut updated = record.clone();
        updated.state = OperationState::Failed;
        updated.settled_at_ms = Some(now);
        let result = SettledResult {
            success: None,
            failure: Some(failure),
            execution: ExecutionState::Completed,
        };
        self.persist_record(&id, &updated)?;
        self.persist_result(&id, &result)?;
        guard.records.insert(id.clone(), updated.clone());
        guard.results.insert(id, result);
        Ok(updated)
    }

    pub fn settle_unknown(&self, key: &DedupKey) -> Result<OperationRecord, OperationError> {
        let id = key.sanitize();
        let mut guard = self.state.lock().expect("operation state poisoned");
        let record = guard
            .records
            .get(&id)
            .cloned()
            .ok_or(OperationError::Unknown)?;
        let now = now_ms();
        if now > record.expires_at_ms {
            return Err(OperationError::Expired);
        }
        let mut updated = record.clone();
        updated.state = OperationState::Unknown;
        updated.settled_at_ms = Some(now);
        let result = SettledResult {
            success: None,
            failure: None,
            execution: ExecutionState::Unknown,
        };
        self.persist_record(&id, &updated)?;
        self.persist_result(&id, &result)?;
        guard.records.insert(id.clone(), updated.clone());
        guard.results.insert(id, result);
        Ok(updated)
    }

    pub fn cancel(&self, key: &DedupKey) -> Result<OperationState, OperationError> {
        let id = key.sanitize();
        let mut guard = self.state.lock().expect("operation state poisoned");
        let mut record = guard
            .records
            .get(&id)
            .cloned()
            .ok_or(OperationError::Unknown)?;
        let now = now_ms();
        if now > record.expires_at_ms {
            return Err(OperationError::Expired);
        }
        let result = guard.results.get(&id).cloned();
        match record.state {
            OperationState::Accepted | OperationState::Running => {
                record.state = OperationState::Cancelled;
                record.settled_at_ms = Some(now);
                let next = SettledResult {
                    success: None,
                    failure: None,
                    execution: ExecutionState::Completed,
                };
                self.persist_record(&id, &record)?;
                self.persist_result(&id, &next)?;
                guard.results.insert(id.clone(), next);
                guard.records.insert(id.clone(), record.clone());
                Ok(record.state)
            }
            other => {
                // Settled (succeeded/failed/cancelled/unknown): record is
                // terminal; return its state so the caller sees the truth.
                let _ = result;
                Ok(other)
            }
        }
    }

    pub fn get(
        &self,
        key: &DedupKey,
    ) -> Result<(OperationRecord, Option<SettledResult>), OperationError> {
        let id = key.sanitize();
        let guard = self.state.lock().expect("operation state poisoned");
        let record = guard
            .records
            .get(&id)
            .cloned()
            .ok_or(OperationError::Unknown)?;
        let now = now_ms();
        if now > record.expires_at_ms {
            return Err(OperationError::Expired);
        }
        let result = guard.results.get(&id).cloned();
        Ok((record, result))
    }

    pub fn collect_garbage(&self) -> Result<usize, OperationError> {
        let now = now_ms();
        let mut guard = self.state.lock().expect("operation state poisoned");
        let mut dropped = 0;
        let expired: Vec<String> = guard
            .records
            .iter()
            .filter(|(_, r)| r.expires_at_ms <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            let path = self.root.join(format!("{id}.json"));
            let result_path = self.root.join(format!("{id}.result.json"));
            fs::remove_file(&path).ok();
            fs::remove_file(&result_path).ok();
            guard.records.remove(&id);
            guard.results.remove(&id);
            dropped += 1;
        }
        Ok(dropped)
    }
}

/// Convenience: serialize an Error to a stable JSON string for failure persistence.
pub fn serialize_failure(error: &v1::Error) -> String {
    serde_json::to_string(error).unwrap_or_default()
}

/// Read a persisted record back from disk (used by crash-recovery tests).
pub fn read_record_file(
    path: &Path,
) -> Result<(OperationRecord, Option<SettledResult>), OperationError> {
    let mut text = String::new();
    fs::File::open(path)?.read_to_string(&mut text)?;
    let record: OperationRecord = serde_json::from_str(&text)?;
    let result_path = path.with_extension("result.json");
    let result = if result_path.exists() {
        let mut text = String::new();
        fs::File::open(&result_path)?.read_to_string(&mut text)?;
        Some(serde_json::from_str::<SettledResult>(&text)?)
    } else {
        None
    };
    Ok((record, result))
}

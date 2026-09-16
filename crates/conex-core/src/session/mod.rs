//! Session/attachment state machine (design §7.1, §7.2).
//!
//! A Session binds (principalId, tenantId, providerEndpointId, plane,
//! workspacePeerId?, humanPeerId?, recovery) and is identified by a sessionId.
//! An Attachment represents a single peer+role's participation; each
//! attachment carries a monotonic `epoch` that fences old Links on resume.
//!
//! The demo slice persists sessions + attachments under one directory and
//! rebuilds in-memory state on open. P1-10 will wire this into the broker.
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

/// Default lease per attachment: 120 s (design §7.1).
pub const DEFAULT_ATTACHMENT_LEASE_MS: u64 = 120_000;

/// Recovery levels exposed by `session/open`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryLevel {
    None,
    InProcess,
    Persistent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBinding {
    pub principal_id: String,
    pub tenant_id: String,
    pub provider_endpoint_id: String,
    pub plane: v1::Plane,
    pub workspace_peer_id: Option<String>,
    pub human_peer_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentRecord {
    pub attachment_id: String,
    pub peer_role: String,
    pub epoch: u64,
    pub lease_until_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub binding: SessionBinding,
    pub recovery: RecoveryLevel,
    pub created_at_ms: u64,
    pub attachments: Vec<AttachmentRecord>,
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("attachment not found: {0}")]
    AttachmentNotFound(String),
    #[error("epoch mismatch: expected {expected}, actual {actual}")]
    EpochMismatch { expected: u64, actual: u64 },
    #[error("binding mismatch")]
    BindingMismatch,
    #[error("lease expired")]
    LeaseExpired,
    #[error("recovery {requested:?} not supported (granted {granted:?})")]
    UnsupportedRecovery {
        requested: RecoveryLevel,
        granted: RecoveryLevel,
    },
    #[error("path traversal blocked: {0}")]
    PathTraversal(String),
}

impl SessionError {
    pub fn code(&self) -> i32 {
        use v1::ErrorCode as E;
        match self {
            SessionError::NotFound(_) => E::UnknownProvider as i32,
            SessionError::AttachmentNotFound(_) => E::UnknownProvider as i32,
            SessionError::EpochMismatch { .. } => E::ResumeUnavailable as i32,
            SessionError::BindingMismatch => E::Unauthorized as i32,
            SessionError::LeaseExpired => E::SessionLost as i32,
            SessionError::UnsupportedRecovery { .. } => E::UnsupportedCapability as i32,
            SessionError::PathTraversal(_) => E::BadRequest as i32,
            _ => E::Internal as i32,
        }
    }

    pub fn to_call_error(&self) -> CallError {
        use conex_proto::v1::ErrorCode;
        match self {
            SessionError::NotFound(_) => {
                CallError::new(ErrorCode::UnknownProvider, "session not found")
            }
            SessionError::AttachmentNotFound(_) => {
                CallError::new(ErrorCode::UnknownProvider, "attachment not found")
            }
            SessionError::EpochMismatch { .. } => {
                CallError::new(ErrorCode::ResumeUnavailable, "epoch mismatch")
            }
            SessionError::BindingMismatch => {
                CallError::new(ErrorCode::Unauthorized, "session binding mismatch")
            }
            SessionError::LeaseExpired => {
                CallError::new(ErrorCode::SessionLost, "attachment lease expired")
            }
            SessionError::UnsupportedRecovery { .. } => CallError::new(
                ErrorCode::UnsupportedCapability,
                "recovery level unsupported",
            ),
            SessionError::PathTraversal(_) => {
                CallError::new(ErrorCode::BadRequest, "path traversal")
            }
            SessionError::Io(_) | SessionError::Json(_) => {
                CallError::new(ErrorCode::Internal, format!("session store: {self}"))
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

fn sanitize_id(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => c,
            _ => '_',
        })
        .collect()
}

fn ensure_safe(name: &str) -> Result<(), SessionError> {
    if name.is_empty() || name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(SessionError::PathTraversal(name.to_string()));
    }
    Ok(())
}

#[derive(Default)]
struct State {
    sessions: HashMap<String, SessionRecord>,
}

pub struct SessionStore {
    root: PathBuf,
    state: Mutex<State>,
    lease_ms: u64,
}

impl std::fmt::Debug for SessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionStore")
            .field("root", &self.root)
            .finish()
    }
}

impl SessionStore {
    pub fn open(root: &Path) -> Result<Self, SessionError> {
        fs::create_dir_all(root)?;
        let mut state = State::default();
        for entry in fs::read_dir(root)?.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name.ends_with(".json") {
                continue;
            }
            let id = name.trim_end_matches(".json").to_string();
            let text = fs::read_to_string(&path)?;
            let record: SessionRecord = serde_json::from_str(&text)?;
            state.sessions.insert(id, record);
        }
        Ok(Self {
            root: root.to_path_buf(),
            state: Mutex::new(state),
            lease_ms: DEFAULT_ATTACHMENT_LEASE_MS,
        })
    }

    pub fn with_attachment_lease_ms(mut self, ms: u64) -> Self {
        self.lease_ms = ms;
        self
    }

    fn persist(&self, record: &SessionRecord) -> Result<(), SessionError> {
        let path = self
            .root
            .join(format!("{}.json", sanitize_id(&record.session_id)));
        let tmp = self
            .root
            .join(format!("{}.json.tmp", sanitize_id(&record.session_id)));
        let json = serde_json::to_string_pretty(record)?;
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.flush()?;
        }
        fs::rename(&tmp, &path)?;
        Ok(())
    }

    fn drop_persisted(&self, session_id: &str) {
        let path = self.root.join(format!("{}.json", sanitize_id(session_id)));
        let _ = fs::remove_file(&path);
    }

    /// Open a new session. Returns `(session_id, granted_recovery)`. P1
    /// only grants `RecoveryLevel::InProcess`; persistent would require
    /// extra wiring (out of P1 demo scope).
    pub fn open_session(
        &self,
        binding: SessionBinding,
        requested: RecoveryLevel,
        attachment_id: &str,
        peer_role: &str,
    ) -> Result<(String, RecoveryLevel), SessionError> {
        ensure_safe(&binding.principal_id)?;
        ensure_safe(&binding.tenant_id)?;
        ensure_safe(&binding.provider_endpoint_id)?;
        ensure_safe(attachment_id)?;
        ensure_safe(peer_role)?;
        let granted = match requested {
            RecoveryLevel::Persistent => RecoveryLevel::InProcess, // downgrade in P1
            other => other,
        };
        let session_id = format!(
            "ses-{}-{}-{}",
            sanitize_id(&binding.tenant_id),
            sanitize_id(&binding.principal_id),
            now_ms()
        );
        let attachment = AttachmentRecord {
            attachment_id: attachment_id.to_string(),
            peer_role: peer_role.to_string(),
            epoch: 1,
            lease_until_ms: now_ms().saturating_add(self.lease_ms),
        };
        let record = SessionRecord {
            session_id: session_id.clone(),
            binding,
            recovery: granted,
            created_at_ms: now_ms(),
            attachments: vec![attachment],
        };
        self.persist(&record)?;
        self.with_state(|s| {
            s.sessions.insert(session_id.clone(), record);
        });
        if requested == RecoveryLevel::Persistent {
            // We downgrade; surface that explicitly.
            return Err(SessionError::UnsupportedRecovery { requested, granted });
        }
        Ok((session_id, granted))
    }

    /// Resume an attachment by fencing older epochs.
    pub fn resume(
        &self,
        session_id: &str,
        attachment_id: &str,
        expected_epoch: u64,
        current_binding: &SessionBinding,
    ) -> Result<u64, SessionError> {
        let mut new_epoch = 0u64;
        self.with_state(|s| {
            let record = s
                .sessions
                .get(session_id)
                .cloned()
                .ok_or_else(|| SessionError::NotFound(session_id.to_string()))?;
            if record.binding != *current_binding {
                return Err(SessionError::BindingMismatch);
            }
            let attach = record
                .attachments
                .iter()
                .find(|a| a.attachment_id == attachment_id)
                .cloned()
                .ok_or_else(|| SessionError::AttachmentNotFound(attachment_id.to_string()))?;
            if now_ms() > attach.lease_until_ms {
                return Err(SessionError::LeaseExpired);
            }
            if attach.epoch != expected_epoch {
                return Err(SessionError::EpochMismatch {
                    expected: expected_epoch,
                    actual: attach.epoch,
                });
            }
            new_epoch = attach.epoch + 1;
            let record = s.sessions.get_mut(session_id).unwrap();
            let attach = record
                .attachments
                .iter_mut()
                .find(|a| a.attachment_id == attachment_id)
                .unwrap();
            attach.epoch = new_epoch;
            attach.lease_until_ms = now_ms().saturating_add(self.lease_ms);
            Ok(())
        })?;
        let persisted = self.with_state(|s| s.sessions.get(session_id).cloned().unwrap());
        self.persist(&persisted)?;
        Ok(new_epoch)
    }

    /// Renew a single attachment's lease without advancing the epoch.
    pub fn renew(
        &self,
        session_id: &str,
        attachment_id: &str,
        expected_epoch: u64,
    ) -> Result<u64, SessionError> {
        let new_lease = now_ms().saturating_add(self.lease_ms);
        self.with_state(|s| {
            let record = s
                .sessions
                .get_mut(session_id)
                .ok_or_else(|| SessionError::NotFound(session_id.to_string()))?;
            let attach = record
                .attachments
                .iter_mut()
                .find(|a| a.attachment_id == attachment_id)
                .ok_or_else(|| SessionError::AttachmentNotFound(attachment_id.to_string()))?;
            if attach.epoch != expected_epoch {
                return Err(SessionError::EpochMismatch {
                    expected: expected_epoch,
                    actual: attach.epoch,
                });
            }
            attach.lease_until_ms = new_lease;
            Ok(())
        })?;
        let persisted = self.with_state(|s| s.sessions.get(session_id).cloned().unwrap());
        self.persist(&persisted)?;
        Ok(new_lease)
    }

    /// Close an attachment; on the final attachment the session is dropped.
    pub fn close(
        &self,
        session_id: &str,
        attachment_id: &str,
        expected_epoch: u64,
    ) -> Result<u64, SessionError> {
        let final_epoch = self.with_state(|s| {
            let record = s
                .sessions
                .get_mut(session_id)
                .ok_or_else(|| SessionError::NotFound(session_id.to_string()))?;
            let pos = record
                .attachments
                .iter()
                .position(|a| a.attachment_id == attachment_id)
                .ok_or_else(|| SessionError::AttachmentNotFound(attachment_id.to_string()))?;
            if record.attachments[pos].epoch != expected_epoch {
                return Err(SessionError::EpochMismatch {
                    expected: expected_epoch,
                    actual: record.attachments[pos].epoch,
                });
            }
            let final_epoch = record.attachments[pos].epoch;
            record.attachments.remove(pos);
            Ok(final_epoch)
        })?;
        if self.with_state(|s| {
            s.sessions
                .get(session_id)
                .map(|r| r.attachments.is_empty())
                .unwrap_or(true)
        }) {
            self.with_state(|s| {
                s.sessions.remove(session_id);
            });
            self.drop_persisted(session_id);
        } else {
            let persisted = self.with_state(|s| s.sessions.get(session_id).cloned().unwrap());
            self.persist(&persisted)?;
        }
        Ok(final_epoch)
    }

    pub fn get(&self, session_id: &str) -> Result<SessionRecord, SessionError> {
        self.with_state(|s| {
            s.sessions
                .get(session_id)
                .cloned()
                .ok_or_else(|| SessionError::NotFound(session_id.to_string()))
        })
    }

    /// Drop sessions whose attachments have all expired.
    pub fn collect_garbage(&self) -> Result<usize, SessionError> {
        let now = now_ms();
        let mut to_drop = Vec::new();
        self.with_state(|s| {
            for (id, record) in &s.sessions {
                if record.attachments.is_empty()
                    || record.attachments.iter().all(|a| a.lease_until_ms <= now)
                {
                    to_drop.push(id.clone());
                }
            }
        });
        let dropped = to_drop.len();
        for id in &to_drop {
            self.with_state(|s| {
                s.sessions.remove(id);
            });
            self.drop_persisted(id);
        }
        Ok(dropped)
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut guard = self.state.lock().expect("session state poisoned");
        f(&mut guard)
    }
}

/// Convenience: read a persisted session record back from disk (for crash-recovery tests).
pub fn read_session_file(path: &Path) -> Result<SessionRecord, SessionError> {
    let mut text = String::new();
    fs::File::open(path)?.read_to_string(&mut text)?;
    Ok(serde_json::from_str(&text)?)
}

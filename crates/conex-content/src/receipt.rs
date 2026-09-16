//! Persisted state JSON envelopes (uploads, pins, commits).
//!
//! Each `.json` file is written atomically (write to `.tmp` then rename). The
//! store reconstructs in-memory state by scanning `*.json` on open, which is
//! what makes the crash-recovery test cases reproducible.
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::ContentError;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadState {
    pub upload_id: String,
    pub format_version: u32,
    pub chunk_size: u32,
    pub declared_size_bytes: u64,
    pub declared_root_kind: String, // "raw" | "manifest"
    pub declared_root_cid: String,
    pub received_chunks: Vec<u32>,
    pub started_at_ms: u64,
    pub lease_until_ms: u64,
}

impl UploadState {
    pub fn new(
        upload_id: String,
        format_version: u32,
        chunk_size: u32,
        declared_size_bytes: u64,
        declared_root_kind: String,
        declared_root_cid: String,
        lease_ms: u64,
    ) -> Self {
        let now = now_ms();
        Self {
            upload_id,
            format_version,
            chunk_size,
            declared_size_bytes,
            declared_root_kind,
            declared_root_cid,
            received_chunks: Vec::new(),
            started_at_ms: now,
            lease_until_ms: now.saturating_add(lease_ms),
        }
    }

    pub fn is_expired(&self) -> bool {
        now_ms() >= self.lease_until_ms
    }

    pub fn last_chunk_index(&self) -> Option<u32> {
        self.received_chunks.iter().copied().max()
    }

    pub fn insert_chunk(&mut self, index: u32) -> Result<(), ContentError> {
        if self.received_chunks.contains(&index) {
            return Err(ContentError::DuplicateChunk(index));
        }
        self.received_chunks.push(index);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitRecord {
    pub commit_id: String,
    pub root_cid: String,
    pub root_kind: String,
    pub committed_bytes: u64,
    pub receipt_id: String,
    pub committed_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PinRecord {
    pub pin_id: String,
    pub root_cid: String,
    pub expires_at_ms: u64,
    pub created_at_ms: u64,
}

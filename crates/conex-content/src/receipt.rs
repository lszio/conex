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

/// One received chunk: its index and the CID the server recomputed from the bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceivedChunk {
    pub index: u32,
    pub cid: String,
}

/// Logical ownership of uploaded/committed content (plan M1.2). Physical
/// blocks are deduplicated by CID, but authorization is never merged across
/// owners: every upload, commit and pin carries its own owner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Owner {
    pub principal_id: String,
    pub tenant_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UploadState {
    pub upload_id: String,
    pub format_version: u32,
    pub chunk_size: u32,
    pub declared_size_bytes: u64,
    pub declared_root_kind: String, // "raw" | "manifest"
    pub declared_root_cid: String,
    pub received_chunks: Vec<ReceivedChunk>,
    pub started_at_ms: u64,
    pub lease_until_ms: u64,
    /// `None` on records written before M1.2: unbound staging is never
    /// claimed automatically and must be reaped once expired.
    #[serde(default)]
    pub owner: Option<Owner>,
    #[serde(default)]
    pub resource_id: Option<String>,
    /// Running total of verified chunk bytes, enforced against
    /// `declared_size_bytes` at the write path.
    #[serde(default)]
    pub received_bytes: u64,
}

impl UploadState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        upload_id: String,
        format_version: u32,
        chunk_size: u32,
        declared_size_bytes: u64,
        declared_root_kind: String,
        declared_root_cid: String,
        lease_ms: u64,
        owner: Owner,
        resource_id: String,
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
            owner: Some(owner),
            resource_id: Some(resource_id),
            received_bytes: 0,
        }
    }

    pub fn is_expired(&self) -> bool {
        now_ms() >= self.lease_until_ms
    }

    pub fn last_chunk_index(&self) -> Option<u32> {
        self.received_chunks.iter().map(|chunk| chunk.index).max()
    }

    pub fn insert_chunk(&mut self, index: u32, cid: &str) -> Result<(), ContentError> {
        if self
            .received_chunks
            .iter()
            .any(|chunk| chunk.index == index)
        {
            return Err(ContentError::DuplicateChunk(index));
        }
        self.received_chunks.push(ReceivedChunk {
            index,
            cid: cid.to_string(),
        });
        Ok(())
    }

    /// Leaf CIDs in ascending chunk order. Requires a contiguous `0..n` index
    /// range: commit must reference the blocks this upload actually received.
    pub fn leaves_in_order(&self) -> Result<Vec<String>, ContentError> {
        let mut chunks = self.received_chunks.clone();
        chunks.sort_by_key(|chunk| chunk.index);
        for (position, chunk) in chunks.iter().enumerate() {
            if chunk.index != position as u32 {
                return Err(ContentError::MissingChunkIndex(position as u32));
            }
        }
        Ok(chunks.into_iter().map(|chunk| chunk.cid).collect())
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
    /// Owning principal/tenant; `None` for pre-M1.2 records, which stay
    /// unreadable through `blob/get` until an administrator binds them.
    #[serde(default)]
    pub owner: Option<Owner>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PinRecord {
    pub pin_id: String,
    pub root_cid: String,
    pub expires_at_ms: u64,
    pub created_at_ms: u64,
    #[serde(default)]
    pub owner: Option<Owner>,
}

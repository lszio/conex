//! Content façade: ties together the local block store with the upload/commit
//! lifecycle. This is the public surface that P1-07 (blob stream) consumes;
//! P1-10 verification drives it from the broker test harness.
use std::sync::Arc;

use bytes::Bytes;

use crate::error::ContentError;
use crate::receipt::{CommitRecord, PinRecord, UploadState, now_ms};
use crate::store::{LocalBlockStore, deterministic_id};

/// Public façade returned by `ContentStore::open`.
pub struct ContentStore {
    inner: Arc<LocalBlockStore>,
    /// Default lease for new uploads in milliseconds.
    default_lease_ms: u64,
}

impl std::fmt::Debug for ContentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentStore").finish_non_exhaustive()
    }
}

impl ContentStore {
    pub fn open(root: &std::path::Path, default_lease_ms: u64) -> Result<Self, ContentError> {
        let cr = crate::store::ContentRoot::open(root)?;
        let inner = Arc::new(LocalBlockStore::open(cr)?);
        Ok(Self {
            inner,
            default_lease_ms,
        })
    }

    pub fn block_store(&self) -> &LocalBlockStore {
        &self.inner
    }

    /// Begin a new upload; the returned `Upload` handles the rest. The caller
    /// identity comes from the authenticated broker context, never the wire.
    #[allow(clippy::too_many_arguments)]
    pub fn begin_upload(
        &self,
        format_version: u32,
        chunk_size: u32,
        declared_size_bytes: u64,
        declared_root_kind: &str,
        declared_root_cid: &str,
        lease_ms: Option<u64>,
        owner: crate::receipt::Owner,
        resource_id: &str,
    ) -> Result<Upload, ContentError> {
        let lease_ms = lease_ms.unwrap_or(self.default_lease_ms);
        // Resume an active staging upload with the same content identity so a
        // reconnecting consumer continues from its received chunks instead of
        // restarting (design §5.5; wire: blob/put returns alreadyHaveChunkCids).
        // Resume-by-root never crosses owners (plan M1.2).
        if let Some(existing) = self.inner.find_active_upload_by_root(
            &owner,
            format_version,
            chunk_size,
            declared_size_bytes,
            declared_root_cid,
        ) {
            return Ok(Upload {
                store: self.inner.clone(),
                state: existing,
            });
        }
        let upload_id = deterministic_id("upl", now_ms(), 0);
        let state = UploadState::new(
            upload_id.clone(),
            format_version,
            chunk_size,
            declared_size_bytes,
            declared_root_kind.to_string(),
            declared_root_cid.to_string(),
            lease_ms,
            owner,
            resource_id.to_string(),
        );
        self.inner.record_upload(state.clone())?;
        Ok(Upload {
            store: self.inner.clone(),
            state,
        })
    }

    /// Resume by upload id; the stored owner must match the caller. Unbound
    /// (pre-M1.2) staging is never claimed (plan M1.2).
    pub fn resume_upload(
        &self,
        upload_id: &str,
        owner: &crate::receipt::Owner,
    ) -> Result<Upload, ContentError> {
        let state = self.inner.get_upload(upload_id)?;
        if state.owner.as_ref() != Some(owner) {
            return Err(ContentError::Forbidden);
        }
        Ok(Upload {
            store: self.inner.clone(),
            state,
        })
    }

    pub fn cancel_upload(
        &self,
        upload_id: &str,
        owner: &crate::receipt::Owner,
    ) -> Result<(), ContentError> {
        let state = self.inner.get_upload(upload_id)?;
        if state.owner.as_ref() != Some(owner) {
            return Err(ContentError::Forbidden);
        }
        let path = self
            .inner
            .content_root()
            .path()
            .join("staging")
            .join(upload_id);
        let _ = std::fs::remove_dir_all(&path);
        self.inner.with_refs(|r| {
            r.uploads.remove(upload_id);
            Ok(())
        })
    }

    /// Commit an upload (`BlobCommitRequest`): the declared root and kind must
    /// match the ones recorded at `blob/put`; the store then re-derives and
    /// verifies everything from the upload's own received chunks.
    pub fn commit(
        &self,
        upload_id: &str,
        declared_root_cid: &str,
        declared_root_kind: &str,
        owner: &crate::receipt::Owner,
    ) -> Result<CommitRecord, ContentError> {
        let upload = self.inner.get_upload(upload_id)?;
        if upload.owner.as_ref() != Some(owner) {
            return Err(ContentError::Forbidden);
        }
        if upload.declared_root_cid != declared_root_cid
            || upload.declared_root_kind != declared_root_kind
        {
            return Err(ContentError::DeclaredRootMismatch {
                declared: declared_root_cid.to_string(),
                recomputed: upload.declared_root_cid,
            });
        }
        self.inner.commit(upload_id)
    }

    pub fn pin(
        &self,
        root_cid: &str,
        pin_id: &str,
        expires_at_ms: u64,
        owner: &crate::receipt::Owner,
    ) -> Result<PinRecord, ContentError> {
        self.inner.pin(root_cid, pin_id, expires_at_ms, owner)
    }

    pub fn unpin(&self, pin_id: &str, owner: &crate::receipt::Owner) -> Result<(), ContentError> {
        self.inner.unpin(pin_id, owner)
    }

    pub fn has_block(&self, cid: &str) -> Result<bool, ContentError> {
        self.inner.has_block(cid)
    }

    /// `blob/have`: presence is only reported for the caller's own content;
    /// foreign CIDs read as absent so nothing leaks (plan M1.2).
    pub fn has(
        &self,
        cids: &[String],
        owner: &crate::receipt::Owner,
    ) -> Result<Vec<bool>, ContentError> {
        cids.iter()
            .map(
                |cid| Ok(self.inner.has_block(cid)? && self.inner.chunk_known_to_owner(cid, owner)),
            )
            .collect()
    }

    /// `blob/get`: a bare valid CID is not a credential — the chunk must sit
    /// in the reachable set of a root this caller owns (plan M1.2).
    pub fn get(&self, cid: &str, owner: &crate::receipt::Owner) -> Result<Bytes, ContentError> {
        if !self.inner.chunk_owned_by(cid, owner) {
            return Err(ContentError::Forbidden);
        }
        self.inner.read_block(cid)
    }

    pub fn collect_garbage(&self) -> Result<usize, ContentError> {
        self.inner.collect_garbage()
    }

    pub fn purge_expired_uploads(&self) -> usize {
        self.inner.purge_expired_uploads()
    }

    pub fn has_commit(&self, root_cid: &str) -> bool {
        self.inner.has_commit(root_cid)
    }

    pub fn get_commit(&self, root_cid: &str) -> Option<CommitRecord> {
        self.inner.get_commit(root_cid)
    }
}

/// Handle to an in-progress upload; calls `put_chunk` for each block.
pub struct Upload {
    store: Arc<LocalBlockStore>,
    state: UploadState,
}

impl Upload {
    pub fn upload_id(&self) -> &str {
        &self.state.upload_id
    }

    pub fn state(&self) -> &UploadState {
        &self.state
    }

    /// Verify and store a single chunk. Returns the recomputed CID on success.
    pub fn put_chunk(&self, index: u32, bytes: &[u8]) -> Result<String, ContentError> {
        if self.state.is_expired() {
            return Err(ContentError::UploadExpired);
        }
        if bytes.len() > self.state.chunk_size as usize {
            return Err(ContentError::ChunkOutOfRange {
                index,
                max: self.state.chunk_size,
            });
        }
        let computed = conex_proto::cid::cid_for_raw(bytes);
        self.store.put_block(&computed, bytes)?;
        self.store
            .record_chunk(&self.state.upload_id, index, &computed, bytes.len())?;
        Ok(computed)
    }

    /// The root CID declared at `blob/put`; `commit` re-derives it from the
    /// received chunks and must reproduce exactly this value.
    pub fn declared_root(&self) -> &str {
        &self.state.declared_root_cid
    }
}

//! Block storage for the share scene.
//!
//! The share scene keeps metadata in memory and bytes on disk, addressed by
//! CID. This module is the whole of that boundary: it knows how a staged
//! upload turns into a committed root, how a byte range maps onto blocks, and
//! when bytes may be freed. Nothing above it needs to know a layout exists.
//!
//! Three decisions are worth naming, because each one is a place where the
//! obvious alternative leaks:
//!
//! * **A second commit of the same root is merged, not duplicated.** Content
//!   has one address, so two people offering the same bytes create two items
//!   and one root. A blind retract would delete the bytes the other item still
//!   serves; `hold_root`/`release_root` count live offers instead, and only the
//!   last one frees blocks.
//! * **The share root is never garbage collected.** A block written by an
//!   in-flight upload has no refcount yet, so `collect_garbage` reads it as
//!   garbage. Reclamation is by hold and by lease, which is the only thing that
//!   knows an offer is still live.
//! * **A restart reclaims everything.** The scene is disposable — a
//!   disconnected owner's bytes stop being readable immediately — so a
//!   reconnected process with no live holders treats what it finds as
//!   leftover. Staging is *not* reclaimed here: an interrupted upload is still
//!   resumable and expires on its own lease.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use conex_content::{ContentError, ContentStore};
use conex_proto::cid::content_cid;

/// Fixed chunk size for the share scene. Not negotiated: the CID geometry, the
/// store's staging and the browser's chunking all assume one value, and a
/// second one would be a second code path nobody tests.
pub const SHARE_CHUNK_SIZE: usize = conex_proto::cid::CHUNK_SIZE;

/// One finished staged upload, reduced to what the scene stores.
pub struct ShareRoot {
    pub root_cid: String,
    pub size_bytes: u64,
}

/// One block's part of a byte range: which file, and which bytes of it.
#[derive(Debug, Clone)]
pub struct RangeSlice {
    pub path: std::path::PathBuf,
    /// Offset within the block to start reading at.
    pub skip: usize,
    /// How many bytes of the block the range covers.
    pub take: usize,
}

/// A byte range resolved onto the blocks it touches.
#[derive(Debug, Clone)]
pub struct RangePlan {
    pub total: u64,
    /// First byte of the range, clamped into the item.
    pub start: u64,
    /// Last byte of the range, clamped into the item.
    pub end: u64,
    pub slices: Vec<RangeSlice>,
}

/// Progress of a staged upload, for a client that reconnected.
#[derive(Debug, Clone)]
pub struct StagedUpload {
    pub upload_id: String,
    pub size_bytes: u64,
    /// Chunks the store already holds. The client resumes from exactly here.
    pub received_count: u64,
    pub received_bytes: u64,
}

/// The root CID the store will derive for a payload of this size.
///
/// Computed before anything is uploaded, because the staged upload is bound to
/// a *content*, and that binding is what makes a retry resume instead of
/// starting over.
pub fn root_cid_for(bytes: &[u8]) -> String {
    content_cid(bytes, SHARE_CHUNK_SIZE)
}

/// CID of one chunk, for a caller that already has the bytes.
///
/// Exposed because a test has to declare what a client would declare, and the
/// alternative is a test-local reimplementation of the one rule.
#[doc(hidden)]
pub fn share_chunk_cid_for_test(bytes: &[u8]) -> String {
    conex_proto::cid::cid_for_raw(bytes)
}

/// `true` when the payload is small enough to be a single raw block, which is
/// the store's own rule for choosing the root kind.
pub fn is_raw_root(size_bytes: u64) -> bool {
    size_bytes <= SHARE_CHUNK_SIZE as u64
}

fn root_kind(size_bytes: u64) -> &'static str {
    if is_raw_root(size_bytes) {
        "raw"
    } else {
        "manifest"
    }
}

/// Block backend for one share scene.
#[derive(Clone)]
pub struct ShareBlocks {
    store: Arc<ContentStore>,
    lease_ms: u64,
}

impl std::fmt::Debug for ShareBlocks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShareBlocks").finish_non_exhaustive()
    }
}

impl ShareBlocks {
    /// Open (creating if needed) a share root that is separate from any other
    /// store on the host. Sharing a root with the blob channel would let one
    /// scene's garbage collection delete the other's in-flight blocks.
    pub fn open(root: &Path, lease_ms: u64) -> Result<Self, ContentError> {
        let store = ContentStore::open(root, lease_ms)?;
        Ok(Self {
            store: Arc::new(store),
            lease_ms,
        })
    }

    pub fn lease_ms(&self) -> u64 {
        self.lease_ms
    }

    /// Start (or rejoin) the staged upload for this content and owner.
    ///
    /// The leaf CIDs the client declared are what bind the staging directory to
    /// a *content*: a second attempt at the same content from the same owner
    /// finds the existing staging and reports how far it got, instead of
    /// starting over. That is also why resume needs no client-side state —
    /// re-declaring the same file re-finds the same upload.
    ///
    /// A client that declares the wrong number of leaves for its size is
    /// refused here, before anything is stored.
    pub fn begin(
        &self,
        owner: &conex_content::Owner,
        size_bytes: u64,
        leaf_cids: &[String],
    ) -> Result<StagedUpload, ContentError> {
        let chunk_size = SHARE_CHUNK_SIZE as u32;
        let expected_leaves = if size_bytes == 0 {
            0
        } else {
            size_bytes.div_ceil(chunk_size as u64) as usize
        };
        if leaf_cids.len() != expected_leaves {
            return Err(ContentError::MissingChunks(vec![format!(
                "{size_bytes} bytes at {chunk_size} need {expected_leaves} chunks, {} declared",
                leaf_cids.len()
            )]));
        }
        // Derived here, not by the client, so the manifest rule has exactly one
        // implementation and the browser only ever hashes its own chunks.
        let root_cid = conex_proto::cid::content_cid_for_parts(chunk_size, size_bytes, leaf_cids)
            .map_err(|error| ContentError::InvalidCid(format!("{error:?}")))?;
        let upload = self.store.begin_upload(
            conex_proto::cid::MANIFEST_FORMAT_VERSION,
            chunk_size,
            size_bytes,
            root_kind(size_bytes),
            &root_cid,
            Some(self.lease_ms),
            owner.clone(),
            "share",
            leaf_cids,
        )?;
        Ok(self.staged(&upload))
    }

    /// Bind a staged upload to its owner. An upload id is not an access token
    /// either: only its owner may resume, chunk, commit or cancel it.
    pub fn staged_by_id(
        &self,
        owner: &conex_content::Owner,
        upload_id: &str,
    ) -> Result<StagedUpload, ContentError> {
        let upload = self.store.resume_upload(upload_id, owner)?;
        Ok(self.staged(&upload))
    }

    /// Accept one chunk, in order.
    ///
    /// The chunk must be the next one the store expects: a single writer that
    /// always appends is what turns "where do I resume" into one number. The
    /// bytes are checked against the CID the client declared for this index
    /// *before* the store is touched, so a bad chunk is refused without
    /// persisting a block or advancing the receipt.
    ///
    /// A chunk that arrives again after a retry is the content-addressed no-op
    /// it looks like: the CID is recomputed from the bytes, and identical bytes
    /// are the same block.
    ///
    /// The CID it is checked against comes from the receipt, not from the
    /// request: the host derived the root from that list, so a client cannot
    /// relax the check by declaring something else on the wire.
    pub fn put_chunk(
        &self,
        owner: &conex_content::Owner,
        upload_id: &str,
        index: usize,
        bytes: &[u8],
    ) -> Result<u64, ContentError> {
        let upload = self.store.resume_upload(upload_id, owner)?;
        let expected_index = upload.state().received_chunks.len();
        if index != expected_index {
            return Err(ContentError::MissingChunkIndex(expected_index as u32));
        }
        // The CID the client declared for this index is the authority on what
        // these bytes were supposed to be, so a mismatch is refused here rather
        // than discovered as a root mismatch at commit.
        let declared = upload
            .state()
            .declared_chunk_cid(expected_index)
            .ok_or(ContentError::MissingChunks(vec![format!(
                "chunk {expected_index} was never declared"
            )]))?
            .to_string();
        let recomputed = conex_proto::cid::cid_for_raw(bytes);
        if recomputed != declared {
            return Err(ContentError::BadChunk {
                declared,
                computed: recomputed,
            });
        }
        upload.put_chunk(index as u32, bytes)?;
        // `state()` is the snapshot the handle was built from, so it still
        // reports the count from before this chunk. Adding one is what makes
        // the number the client resumes from correct: reporting the pre-insert
        // count would send it back one chunk and re-send what just landed.
        Ok(expected_index as u64 + 1)
    }

    /// Take a whole payload and store it, for a host that already has the
    /// bytes in hand (the whole-body upload route).
    ///
    /// The host derives the root itself, which is exactly what a client would
    /// compute: one function decides the address, so a whole-body upload and a
    /// chunked upload of the same file land on the same root.
    pub fn ingest(
        &self,
        owner: &conex_content::Owner,
        bytes: &[u8],
    ) -> Result<ShareRoot, ContentError> {
        // The host declares its leaves exactly as a client would, so the
        // whole-body route and the chunked route end up on one code path.
        let (leaves, size) = conex_proto::cid::leaf_cids_for(bytes, SHARE_CHUNK_SIZE);
        let staged = self.begin(owner, size, &leaves)?;
        for (index, chunk) in bytes.chunks(SHARE_CHUNK_SIZE).enumerate() {
            self.put_chunk(owner, &staged.upload_id, index, chunk)?;
        }
        self.commit(owner, &staged.upload_id)
    }

    /// Finish a staged upload and adopt its root.
    ///
    /// A repeat of an already-committed root is merged: the staging directory
    /// is discarded and the caller holds the committed root instead, so two
    /// offers of the same bytes stay two offers.
    pub fn commit(
        &self,
        owner: &conex_content::Owner,
        upload_id: &str,
    ) -> Result<ShareRoot, ContentError> {
        let staged = self.staged_by_id(owner, upload_id)?;
        let root_cid = self
            .store
            .resume_upload(upload_id, owner)?
            .declared_root()
            .to_string();
        if self.store.has_commit(&root_cid) {
            self.cancel(owner, upload_id)?;
            self.store.block_store().hold_root(&root_cid)?;
        } else {
            self.store
                .commit(upload_id, &root_cid, root_kind(staged.size_bytes), owner)?;
            self.store.block_store().hold_root(&root_cid)?;
        }
        Ok(ShareRoot {
            root_cid,
            size_bytes: staged.size_bytes,
        })
    }

    /// Abandon a staged upload. The bytes already written stay in the content
    /// store until their lease expires, so an abandoned upload never leaves a
    /// half-written item readable.
    pub fn cancel(
        &self,
        owner: &conex_content::Owner,
        upload_id: &str,
    ) -> Result<(), ContentError> {
        self.store.cancel_upload(upload_id, owner)
    }

    /// Drop one live offer of a root, freeing its blocks when the last offer
    /// goes.
    pub fn release(&self, root_cid: &str) {
        let _ = self.store.block_store().release_root(root_cid);
    }

    /// Reclaim every committed root. Called once at startup, when no offer
    /// survives a restart.
    pub fn reclaim_all(&self) -> usize {
        self.store.block_store().reclaim_all_committed()
    }

    /// Reap staged uploads whose lease expired. Returns how many were dropped.
    pub fn purge_expired(&self) -> usize {
        self.store.purge_expired_uploads()
    }

    /// Read a byte range of committed content.
    ///
    /// The range maps onto a contiguous run of leaves: offset picks the first
    /// block, and only the blocks the range actually touches are read. A range
    /// that runs past the end is clamped to it rather than answered short, so
    /// a caller never mistakes a clamp for a truncated file.
    pub fn read_range(
        &self,
        root_cid: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, ContentError> {
        let size = self.size_of(root_cid)?;
        let start = offset.min(size);
        let end = start.saturating_add(length).min(size);
        if end <= start {
            return Ok(Vec::new());
        }
        let store = self.store.block_store();
        let leaves = store.leaves_of(root_cid)?;
        let chunk = SHARE_CHUNK_SIZE as u64;
        let first = (start / chunk) as usize;
        let last = ((end - 1) / chunk) as usize;
        if first >= leaves.len() {
            return Err(ContentError::UnknownBlock(format!(
                "root {root_cid} has {} leaves, none covers byte {start}",
                leaves.len()
            )));
        }
        let last = last.min(leaves.len() - 1);
        let mut out = Vec::with_capacity((end - start) as usize);
        for leaf in &leaves[first..=last] {
            out.extend_from_slice(&store.read_block(leaf)?);
        }
        // The first block may start mid-chunk and the last may end mid-chunk,
        // so the trim is by the range, not by whole blocks.
        let trim_start = usize::try_from(start - first as u64 * chunk).unwrap_or(out.len());
        let trim_end = usize::try_from(trim_start as u64 + (end - start))
            .ok()
            .filter(|end| *end <= out.len())
            .ok_or_else(|| {
                ContentError::UnknownBlock(format!(
                    "root {root_cid} is short of the bytes it declares ({size})"
                ))
            })?;
        let trim_start = trim_start.min(out.len());
        Ok(out[trim_start..trim_end].to_vec())
    }

    /// Where the bytes of a range live, block by block.
    ///
    /// A response streams from this rather than from a buffer: the alternative
    /// reads the whole item into memory before the first byte moves, which for
    /// a large download is exactly the cost the scene is trying not to pay.
    /// Only the block boundaries are computed here — the reads themselves
    /// happen on the caller's async path.
    pub fn range_plan(
        &self,
        root_cid: &str,
        offset: u64,
        length: u64,
    ) -> Result<RangePlan, ContentError> {
        let total = self.size_of(root_cid)?;
        let start = offset.min(total);
        // The last byte that exists is `total - 1`. A range that runs past the
        // end is clamped rather than requested, so the `Content-Range` a route
        // advertises never names a byte the item does not have. An off-by-one
        // here would tell a browser a seek target exists when it does not.
        let end = total
            .saturating_sub(1)
            .min(start.saturating_add(length).saturating_sub(1));
        let mut slices = Vec::new();
        if length > 0 && end >= start {
            let leaves = self.store.block_store().leaves_of(root_cid)?;
            let chunk = SHARE_CHUNK_SIZE as u64;
            let first = (start / chunk) as usize;
            // `end` is an inclusive index, so the byte it names is in block
            // `end / chunk`. Writing it as `(end - 1) / chunk` underflows for a
            // range that ends at byte zero, which a preview of a one-byte file
            // does.
            let last = (end / chunk) as usize;
            if first >= leaves.len() {
                return Err(ContentError::UnknownBlock(format!(
                    "root {root_cid} has {} leaves, none covers byte {start}",
                    leaves.len()
                )));
            }
            let last = last.min(leaves.len() - 1);
            for (offset, block_index) in (first..=last).enumerate() {
                let block_first = (first + offset) as u64 * chunk;
                // The item's own last byte bounds a final short block, so the
                // last block's nominal length is `total - block_first` rather
                // than a full chunk.
                let block_last = (block_first + chunk - 1).min(total - 1);
                // Where the range enters and leaves this block.
                let enter = block_first.max(start);
                let leave = block_last.min(end);
                let path = self
                    .store
                    .block_store()
                    .block_path(&leaves[block_index])
                    .ok_or_else(|| {
                        ContentError::UnknownBlock(format!(
                            "block {} is missing from disk",
                            leaves[block_index]
                        ))
                    })?;
                slices.push(RangeSlice {
                    path,
                    skip: (enter - block_first) as usize,
                    take: (leave - enter + 1) as usize,
                });
            }
        }
        Ok(RangePlan {
            total,
            start,
            end,
            slices,
        })
    }

    /// Committed size of a root, or an error when it was never committed.
    pub fn size_of(&self, root_cid: &str) -> Result<u64, ContentError> {
        let store = self.store.block_store();
        if !store.has_commit(root_cid) {
            return Err(ContentError::UnknownCommit(root_cid.to_string()));
        }
        Ok(store
            .get_commit(root_cid)
            .map(|commit| commit.committed_bytes)
            .unwrap_or(0))
    }

    fn staged(&self, upload: &conex_content::Upload) -> StagedUpload {
        StagedUpload {
            upload_id: upload.upload_id().to_string(),
            size_bytes: upload.state().declared_size_bytes,
            received_count: upload.state().received_chunks.len() as u64,
            received_bytes: upload.state().received_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner() -> conex_content::Owner {
        conex_content::Owner {
            principal_id: "principal".into(),
            tenant_id: "tenant".into(),
        }
    }

    fn blocks(tmp: &tempfile::TempDir) -> ShareBlocks {
        ShareBlocks::open(tmp.path(), 60_000).expect("open share blocks")
    }

    fn payload(chunks: usize) -> Vec<u8> {
        (0..(chunks * SHARE_CHUNK_SIZE + 17))
            .map(|i| (i % 251) as u8)
            .collect()
    }

    /// Declare a payload the way a client would: hash every chunk up front, then
    /// send them.
    fn leaves_of(bytes: &[u8]) -> Vec<String> {
        bytes
            .chunks(SHARE_CHUNK_SIZE)
            .map(conex_proto::cid::cid_for_raw)
            .collect()
    }

    fn begin_like_a_client(
        blocks: &ShareBlocks,
        owner: &conex_content::Owner,
        bytes: &[u8],
    ) -> crate::share_blocks::StagedUpload {
        blocks
            .begin(owner, bytes.len() as u64, &leaves_of(bytes))
            .expect("begin")
    }

    /// Send chunks `[from, to)`, the way a client that reconnected does.
    fn send_like_a_client(
        blocks: &ShareBlocks,
        owner: &conex_content::Owner,
        upload_id: &str,
        bytes: &[u8],
        from: usize,
    ) -> u64 {
        send_upto(
            blocks,
            owner,
            upload_id,
            bytes,
            from,
            bytes.chunks(SHARE_CHUNK_SIZE).len(),
        )
    }

    /// Send chunks `[from, to)`.
    fn send_upto(
        blocks: &ShareBlocks,
        owner: &conex_content::Owner,
        upload_id: &str,
        bytes: &[u8],
        from: usize,
        to: usize,
    ) -> u64 {
        let mut received = 0;
        for (index, chunk) in bytes
            .chunks(SHARE_CHUNK_SIZE)
            .enumerate()
            .take(to)
            .skip(from)
        {
            received = blocks
                .put_chunk(owner, upload_id, index, chunk)
                .expect("chunk");
        }
        received
    }

    #[test]
    fn a_range_read_returns_exactly_the_asked_bytes() {
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        let bytes = payload(3);
        let staged = begin_like_a_client(&blocks, &owner(), &bytes);
        assert_eq!(staged.received_count, 0);
        let sent = send_like_a_client(&blocks, &owner(), &staged.upload_id, &bytes, 0);
        assert_eq!(
            sent, 4,
            "the host counts the chunks it holds, not the ones just sent"
        );
        let committed = blocks.commit(&owner(), &staged.upload_id).unwrap();
        assert_eq!(committed.size_bytes, bytes.len() as u64);
        let root = committed.root_cid.clone();
        assert_eq!(blocks.size_of(&root).unwrap(), bytes.len() as u64);

        // Head: stops inside the second block.
        let head = blocks
            .read_range(&root, 0, SHARE_CHUNK_SIZE as u64 + 40)
            .unwrap();
        assert_eq!(head, bytes[..SHARE_CHUNK_SIZE + 40]);
        // Middle: starts and ends inside blocks.
        let middle = blocks
            .read_range(&root, SHARE_CHUNK_SIZE as u64 + 40, 30)
            .unwrap();
        assert_eq!(middle, bytes[SHARE_CHUNK_SIZE + 40..SHARE_CHUNK_SIZE + 70]);
        // Tail: past the end clamps instead of answering short.
        let tail = blocks
            .read_range(&root, bytes.len() as u64 - 5, 1000)
            .unwrap();
        assert_eq!(tail, bytes[bytes.len() - 5..]);
        assert_eq!(blocks.read_range(&root, 0, 0).unwrap(), Vec::<u8>::new());
        assert_eq!(
            blocks
                .read_range(&root, bytes.len() as u64 + 10, 5)
                .unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn a_range_plan_points_at_exactly_the_blocks_a_range_touches() {
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        let bytes = payload(4);
        let staged = begin_like_a_client(&blocks, &owner(), &bytes);
        send_like_a_client(&blocks, &owner(), &staged.upload_id, &bytes, 0);
        let root = blocks.commit(&owner(), &staged.upload_id).unwrap().root_cid;
        let chunk = SHARE_CHUNK_SIZE as u64;

        // A head that stops inside the second block: the first block is read
        // whole, the second only up to the end of the range.
        let plan = blocks.range_plan(&root, 0, chunk + 40).unwrap();
        assert_eq!(plan.total, bytes.len() as u64);
        assert_eq!(plan.slices.len(), 2);
        assert_eq!(plan.slices[0].skip, 0);
        assert_eq!(plan.slices[0].take, chunk as usize);
        assert_eq!(plan.slices[1].skip, 0);
        assert_eq!(plan.slices[1].take, 40);

        // A middle range that starts and ends inside blocks.
        let plan = blocks.range_plan(&root, chunk + 10, 30).unwrap();
        assert_eq!(plan.slices.len(), 1);
        assert_eq!(plan.slices[0].skip, 10);
        assert_eq!(plan.slices[0].take, 30);

        // A range that straddles a boundary reads both blocks, each trimmed.
        let plan = blocks.range_plan(&root, chunk - 5, 10).unwrap();
        assert_eq!(plan.slices.len(), 2);
        assert_eq!(plan.slices[0].skip, chunk as usize - 5);
        assert_eq!(plan.slices[0].take, 5);
        assert_eq!(plan.slices[1].skip, 0);
        assert_eq!(plan.slices[1].take, 5);

        // Past the end clamps rather than answering short.
        let plan = blocks
            .range_plan(&root, bytes.len() as u64 - 3, 100)
            .unwrap();
        assert_eq!(plan.start, bytes.len() as u64 - 3);
        assert_eq!(plan.end, bytes.len() as u64 - 1);
        // Three bytes clamped out of a hundred asked for.
        assert_eq!(plan.slices.iter().map(|s| s.take).sum::<usize>(), 3);
        // A range wholly past the end has no blocks to read.
        let plan = blocks.range_plan(&root, bytes.len() as u64 + 9, 5).unwrap();
        assert!(plan.slices.is_empty());
        // Every slice names a block that is really on disk.
        for slice in blocks.range_plan(&root, 0, chunk * 3).unwrap().slices {
            assert!(slice.path.exists(), "{} must exist", slice.path.display());
        }
    }

    /// Every range of a small item, checked against the bytes themselves.
    ///
    /// The individual cases above each pin one boundary; this walks all of them
    /// so a boundary that is right in one direction cannot be wrong in the
    /// other. Off-by-ones in a range map do not fail loudly, they hand a browser
    /// a byte it did not ask for.
    #[test]
    fn every_range_of_a_small_item_reads_back_the_bytes_it_names() {
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        // One full block plus a short tail, so the sweep crosses a block
        // boundary and ends inside the short one.
        let bytes: Vec<u8> = (0..(SHARE_CHUNK_SIZE + 101))
            .map(|i| (i % 251) as u8)
            .collect();
        let size = bytes.len() as u64;
        let staged = begin_like_a_client(&blocks, &owner(), &bytes);
        send_like_a_client(&blocks, &owner(), &staged.upload_id, &bytes, 0);
        let root = blocks.commit(&owner(), &staged.upload_id).unwrap().root_cid;
        // A strided sweep, with every boundary of interest called out. Walking
        // every offset of a multi-block item is minutes of file reads for the
        // same coverage: what matters is 0, 1, each block edge, the item's own
        // end, and past it, and the stride catches anything the edges do not.
        let chunk = SHARE_CHUNK_SIZE as u64;
        let mut starts: Vec<u64> = (0..size).step_by(997).collect();
        starts.extend([0, 1, chunk - 1, chunk, chunk + 1, size - 2, size - 1]);
        let lengths = [1u64, 2, 7, size, size + 5, chunk - 1, chunk, chunk + 1];
        for start in starts {
            for length in lengths {
                let plan = blocks.range_plan(&root, start, length).unwrap();
                let served: usize = plan.slices.iter().map(|slice| slice.take).sum();
                let expected = (size - plan.start).min(length);
                assert_eq!(
                    served as u64, expected,
                    "range {start}+{length} must serve exactly what exists"
                );
                // The bytes the slices name are the bytes the range asked for.
                let mut read = Vec::new();
                for slice in &plan.slices {
                    let block = std::fs::read(&slice.path).expect("block");
                    read.extend_from_slice(&block[slice.skip..slice.skip + slice.take]);
                }
                assert_eq!(
                    read,
                    bytes[plan.start as usize..(plan.start as usize + expected as usize)],
                    "range {start}+{length} must read the bytes it names"
                );
            }
        }
    }

    #[test]
    fn a_chunk_out_of_order_is_refused_before_the_store_is_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        let bytes = payload(3);
        let staged = begin_like_a_client(&blocks, &owner(), &bytes);
        // Skipping the first chunk is refused, not stored and retried later.
        assert!(matches!(
            blocks.put_chunk(&owner(), &staged.upload_id, 1, &bytes[SHARE_CHUNK_SIZE..]),
            Err(ContentError::MissingChunkIndex(0))
        ));
        assert_eq!(
            blocks
                .staged_by_id(&owner(), &staged.upload_id)
                .unwrap()
                .received_count,
            0,
            "the refused chunk did not advance the upload"
        );
    }

    #[test]
    fn an_upload_id_is_not_an_access_token() {
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        let bytes = payload(1);
        let staged = begin_like_a_client(&blocks, &owner(), &bytes);
        // A different principal is a different owner, not a different name for
        // the same one, so it cannot resume, chunk, commit or cancel.
        let stranger = conex_content::Owner {
            principal_id: "someone-else".into(),
            tenant_id: "tenant".into(),
        };
        assert!(matches!(
            blocks.staged_by_id(&stranger, &staged.upload_id),
            Err(ContentError::Forbidden)
        ));
        assert!(matches!(
            blocks.put_chunk(&stranger, &staged.upload_id, 0, &bytes),
            Err(ContentError::Forbidden)
        ));
        assert!(matches!(
            blocks.commit(&stranger, &staged.upload_id),
            Err(ContentError::Forbidden)
        ));
        assert!(matches!(
            blocks.cancel(&stranger, &staged.upload_id),
            Err(ContentError::Forbidden)
        ));
        assert!(blocks.staged_by_id(&owner(), &staged.upload_id).is_ok());
    }

    #[test]
    fn the_same_content_offered_twice_is_one_root_that_survives_one_release() {
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        let bytes = payload(2);
        let root = root_cid_for(&bytes);
        let offer = || {
            let staged = begin_like_a_client(&blocks, &owner(), &bytes);
            send_like_a_client(&blocks, &owner(), &staged.upload_id, &bytes, 0);
            blocks.commit(&owner(), &staged.upload_id).unwrap();
        };
        offer();
        offer();
        // Two offers of identical bytes are two holds of one root, so
        // withdrawing one leaves the other readable.
        blocks.release(&root);
        assert_eq!(
            blocks.read_range(&root, 0, 16).unwrap(),
            bytes[..16],
            "the root still serves the bytes after one offer was withdrawn"
        );
        blocks.release(&root);
        assert!(
            blocks.size_of(&root).is_err(),
            "the last release frees the root"
        );
    }

    #[test]
    fn re_declaring_the_same_content_rejoins_the_same_upload() {
        // The whole resume story in one test: the client remembers nothing, so
        // re-declaring what it was always going to send has to find the same
        // staging and report how far it got.
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        let bytes = payload(4);
        let first = begin_like_a_client(&blocks, &owner(), &bytes);
        send_like_a_client(&blocks, &owner(), &first.upload_id, &bytes, 0);

        let second = begin_like_a_client(&blocks, &owner(), &bytes);
        assert_eq!(
            second.upload_id, first.upload_id,
            "the same content rejoins the same staging"
        );
        assert_eq!(
            second.received_count,
            bytes.chunks(SHARE_CHUNK_SIZE).len() as u64,
            "and reports how far it got"
        );
    }

    #[test]
    fn an_interrupted_upload_resumes_from_the_chunk_that_was_lost() {
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        let bytes = payload(5);
        let upload_id = begin_like_a_client(&blocks, &owner(), &bytes).upload_id;
        // Send three chunks, then "lose the connection".
        send_upto(&blocks, &owner(), &upload_id, &bytes, 0, 3);
        let before = blocks.staged_by_id(&owner(), &upload_id).unwrap();
        assert_eq!(before.received_count, 3);
        assert_eq!(before.received_bytes, 3 * SHARE_CHUNK_SIZE as u64);

        // Resume: re-declare, skip what the host already has, send the rest.
        let resumed = begin_like_a_client(&blocks, &owner(), &bytes);
        let sent = send_like_a_client(
            &blocks,
            &owner(),
            &resumed.upload_id,
            &bytes,
            resumed.received_count as usize,
        );
        // The host reports the total it holds, so a resume that sends the tail
        // reports the whole number.
        assert_eq!(sent, 6, "all six chunks are now held");
        let committed = blocks.commit(&owner(), &resumed.upload_id).unwrap();
        assert_eq!(
            blocks.read_range(&committed.root_cid, 0, 8).unwrap(),
            bytes[..8]
        );
    }

    #[test]
    fn a_chunk_with_the_wrong_bytes_is_refused_before_the_store_is_touched() {
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        let bytes = payload(3);
        let upload_id = begin_like_a_client(&blocks, &owner(), &bytes).upload_id;
        // The right length, the wrong bytes: the declared CID still says what
        // this position was supposed to hold, so the mismatch is refused here
        // rather than surfacing as a root mismatch after every byte was sent.
        let wrong = vec![0u8; SHARE_CHUNK_SIZE];
        assert!(matches!(
            blocks.put_chunk(&owner(), &upload_id, 0, &wrong),
            Err(ContentError::BadChunk { .. })
        ));
        assert_eq!(
            blocks
                .staged_by_id(&owner(), &upload_id)
                .unwrap()
                .received_count,
            0,
            "the refused chunk advanced nothing"
        );
        // The correct bytes still work, so a refusal is recoverable.
        assert!(
            blocks
                .put_chunk(&owner(), &upload_id, 0, &bytes[..SHARE_CHUNK_SIZE])
                .is_ok()
        );
    }

    #[test]
    fn a_declaration_with_the_wrong_chunk_count_is_refused_up_front() {
        let tmp = tempfile::tempdir().unwrap();
        let blocks = blocks(&tmp);
        let bytes = payload(3);
        let leaves = leaves_of(&bytes);
        // One leaf short: the size and the declaration disagree, so nothing is
        // staged and there is nothing to resume.
        let short = leaves[..leaves.len() - 1].to_vec();
        assert!(matches!(
            blocks.begin(&owner(), bytes.len() as u64, &short),
            Err(ContentError::MissingChunks(_))
        ));
        // A leaf that is not a CID at all is refused by the same door.
        assert!(
            blocks
                .begin(&owner(), bytes.len() as u64, &["nope".to_string()])
                .is_err()
        );
        // The honest count goes through.
        assert!(blocks.begin(&owner(), bytes.len() as u64, &leaves).is_ok());
    }

    #[test]
    fn a_restart_reclaims_committed_roots_and_keeps_staging() {
        let tmp = tempfile::tempdir().unwrap();
        let bytes = payload(2);
        let upload_id;
        {
            let blocks = blocks(&tmp);
            let staged = begin_like_a_client(&blocks, &owner(), &bytes);
            upload_id = staged.upload_id.clone();
            blocks
                .put_chunk(&owner(), &upload_id, 0, &bytes[..SHARE_CHUNK_SIZE])
                .unwrap();
            drop(blocks);
        }
        let reopened = blocks(&tmp);
        // Staging survives the restart, so the upload is still resumable from
        // where it stopped rather than restarting.
        let staged = reopened.staged_by_id(&owner(), &upload_id).unwrap();
        assert_eq!(staged.received_count, 1);
        let root = blocks_and_commit(&reopened, &owner(), &upload_id, &bytes);
        // A restart with no live offers reclaims what survived it.
        let reopened = blocks(&tmp);
        assert!(reopened.size_of(&root).is_ok());
        assert!(reopened.reclaim_all() > 0);
        assert!(reopened.size_of(&root).is_err());
    }

    fn blocks_and_commit(
        blocks: &ShareBlocks,
        owner: &conex_content::Owner,
        upload_id: &str,
        bytes: &[u8],
    ) -> String {
        let already = blocks
            .staged_by_id(owner, upload_id)
            .unwrap()
            .received_count as usize;
        send_like_a_client(blocks, owner, upload_id, bytes, already);
        blocks.commit(owner, upload_id).unwrap().root_cid
    }
}

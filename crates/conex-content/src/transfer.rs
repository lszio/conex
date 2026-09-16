//! Blob transfer layer (P1-07).
//!
//! Bridges wire/inline payloads to the persistent block store. For the
//! demo slice this layer talks directly to `ContentStore`; once P1-04
//! lands it will hook the same logic onto stream frames (`StreamFrame`
//! carries `bytes` for each chunk index).
//!
//! Inline policy (design §5.6):
//! - Blocks ≤ inline_threshold_bytes may be carried inline with the metadata;
//!   blocks > inline_threshold_bytes must be uploaded as chunks.
//! - Default inline threshold: 64 KiB.
//! - Memory cap: the writer buffers at most one chunk at a time. The reader
//!   reconstructs one chunk at a time and writes through `std::io::Write`
//!   so total memory stays bounded regardless of total content size.
use std::io::Write;
use std::path::Path;

use crate::error::ContentError;
use crate::store::{ContentRoot, LocalBlockStore};
use bytes::Bytes;
use conex_proto::cid::{cid_for_raw, content_cid_for_parts, leaf_cids_for};
use conex_proto::v1;

/// Default inline threshold (设计 §5.6).
pub const DEFAULT_INLINE_THRESHOLD_BYTES: u32 = 65_536;
/// Maximum bytes allowed in a single decoded blob content (1 GiB cap).
pub const MAX_BLOB_BYTES: u64 = 1 << 30;

/// Outcome of a `decode_content` call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedBlob {
    pub root_cid: String,
    pub size_bytes: u64,
}

/// Reconstruct a blob's content by walking leaves in chunk-index order.
///
/// `leaves` MUST be ordered by ascending chunk index; the root is recomputed
/// from them with the canonical rule (`conex_proto::cid::content_cid_for_parts`,
/// which also binds `chunk_size` and the total length). Any deviation returns
/// `DeclaredRootMismatch` and the caller MUST treat the upload as aborted.
pub fn decode_content(
    store: &LocalBlockStore,
    declared_root_cid: &str,
    chunk_size: u32,
    leaves: &[String],
    total_bytes: u64,
    mut out: impl Write,
) -> Result<DecodedBlob, ContentError> {
    if leaves.is_empty() {
        return Err(ContentError::MissingChunks(vec![
            declared_root_cid.to_string(),
        ]));
    }
    if total_bytes > MAX_BLOB_BYTES {
        return Err(ContentError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("blob size {total_bytes} exceeds MAX_BLOB_BYTES {MAX_BLOB_BYTES}"),
        )));
    }
    let mut written = 0u64;
    for leaf in leaves {
        let block = store.read_block(leaf)?;
        if block.len() > chunk_size as usize {
            return Err(ContentError::ChunkOutOfRange {
                index: 0,
                max: chunk_size,
            });
        }
        let computed = cid_for_raw(&block);
        if &computed != leaf {
            return Err(ContentError::BadChunk {
                declared: leaf.clone(),
                computed,
            });
        }
        out.write_all(&block).map_err(ContentError::Io)?;
        written = written.saturating_add(block.len() as u64);
    }
    let recomputed = content_cid_for_parts(chunk_size, total_bytes, leaves)
        .map_err(|error| ContentError::Manifest(error.message))?;
    if recomputed != declared_root_cid {
        return Err(ContentError::DeclaredRootMismatch {
            declared: declared_root_cid.to_string(),
            recomputed,
        });
    }
    if written != total_bytes {
        return Err(ContentError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("decoded {written} bytes but total_bytes declared {total_bytes}"),
        )));
    }
    Ok(DecodedBlob {
        root_cid: declared_root_cid.to_string(),
        size_bytes: total_bytes,
    })
}

/// Compute the leaves for a payload using P1 broker chunking. Returns
/// `(leaf_cids, total_bytes)` with leaves in chunk-index order; a payload that
/// fits in one chunk has exactly one leaf (empty content is the empty block).
/// The canonical rule lives in `conex_proto::cid`; this is a convenience view.
pub fn chunk_payload(bytes: &[u8], chunk_size: u32) -> (Vec<String>, u64) {
    if chunk_size == 0 || bytes.len() <= chunk_size as usize {
        return (vec![cid_for_raw(bytes)], bytes.len() as u64);
    }
    leaf_cids_for(bytes, chunk_size as usize)
}

/// Return `true` if a payload with the given `total_bytes` may be carried
/// inline (single-block case AND total_bytes within the inline threshold).
pub fn is_inline_eligible(total_bytes: u64, chunk_size: u32, inline_threshold_bytes: u32) -> bool {
    total_bytes <= chunk_size as u64 && total_bytes <= inline_threshold_bytes as u64
}

/// Inline byte representation for a single-block blob: the raw payload
/// bytes themselves. The receiver verifies `cid_for_raw(bytes) ==
/// declared_root_cid` after decoding.
pub fn inline_payload(bytes: &[u8]) -> Bytes {
    Bytes::copy_from_slice(bytes)
}

/// Build the `v1::BlobRef` for a committed root CID.
pub fn blob_ref_for(
    root_cid: &str,
    size_bytes: u64,
    provider_id: &str,
    plane: &str,
    resource_id: &str,
) -> v1::BlobRef {
    v1::BlobRef {
        cid: root_cid.to_string(),
        size_bytes: size_bytes.to_string(),
        mime: "application/octet-stream".to_string(),
        access: Some(v1::BlobAccess {
            provider_id: provider_id.to_string(),
            plane: plane.to_string(),
            space_id: None,
            resource_id: resource_id.to_string(),
        }),
    }
}

/// Locate the content root directory a given `ContentRoot` was built from.
pub fn content_root_path(root: &ContentRoot) -> &Path {
    root.path()
}

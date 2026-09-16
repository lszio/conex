//! CIDv1 content addressing (design S5.3). P0 fixes raw + SHA-256.
use cid::Cid;
use multihash::Multihash;
use sha2::{Digest, Sha256};

use crate::wire::ProtocolError;

/// multicodec: raw
pub const RAW_CODEC: u64 = 0x55;
/// multihash: sha2-256
pub const SHA2_256: u64 = 0x12;

/// CIDv1 / raw / SHA-256, lowercase base32 text.
pub fn cid_for_raw(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hash = Multihash::<64>::wrap(SHA2_256, &digest).expect("sha2-256 digest fits in 64 bytes");
    Cid::new_v1(RAW_CODEC, hash).to_string()
}

/// Parse and normalize a CID text form. Rejects unsupported codec/multihash and
/// trailing bytes; accepted text bases are normalized to binary for comparison.
pub fn parse_cid(text: &str) -> Result<Cid, ProtocolError> {
    let cid = Cid::try_from(text)
        .map_err(|e| ProtocolError::bad_request(None, format!("invalid CID: {e}")))?;
    if cid.codec() != RAW_CODEC {
        return Err(ProtocolError::bad_request(
            None,
            format!("unsupported content codec {}", cid.codec()),
        ));
    }
    if cid.hash().code() != SHA2_256 {
        return Err(ProtocolError::bad_request(
            None,
            format!("unsupported multihash code {}", cid.hash().code()),
        ));
    }
    if cid.hash().size() != 32 {
        return Err(ProtocolError::bad_request(
            None,
            format!(
                "sha2-256 digest must be 32 bytes, got {}",
                cid.hash().size()
            ),
        ));
    }
    Ok(cid)
}
/// Default broker chunk size: 256 KiB (design §5.4).
pub const CHUNK_SIZE: usize = 262_144;

/// Canonical manifest bytes prefix. See docs/contracts/p1-chunking.md §3.
/// A manifest for N chunks is the prefix followed by each chunk's CID text in
/// ascending chunk-index order, concatenated without separators.
pub const MANIFEST_PREFIX: &str = "manifest/v1/";

/// Split bytes into fixed-size chunks (last chunk may be shorter).
pub fn chunk(bytes: &[u8], chunk_size: usize) -> impl Iterator<Item = &[u8]> {
    assert!(chunk_size > 0, "chunk_size must be > 0");
    bytes.chunks(chunk_size)
}

/// Build the canonical manifest bytes for chunked content.
/// Format: `<MANIFEST_PREFIX>` followed by the leaf CIDs concatenated in order.
pub fn manifest_bytes_for_leaves<I, S>(leaves: I) -> Vec<u8>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out: Vec<u8> = MANIFEST_PREFIX.as_bytes().to_vec();
    for cid in leaves {
        out.extend_from_slice(cid.as_ref().as_bytes());
    }
    out
}

/// Unified content addressing function (设计 §5.3；P0 固定；P1 沿用).
///
/// - Single-block content (len <= chunk_size): CIDv1 raw/SHA-256 of the bytes.
/// - Multi-block content (len > chunk_size): CIDv1 raw/SHA-256 of the canonical
///   manifest bytes `manifest/v1||leaf0||leaf1||...` where leaf_i is the
///   CIDv1 raw/SHA-256 of the i-th fixed-size chunk (last chunk may be shorter).
pub fn content_cid(bytes: &[u8], chunk_size: usize) -> String {
    if bytes.len() <= chunk_size {
        return cid_for_raw(bytes);
    }
    let leaves = chunk(bytes, chunk_size).map(cid_for_raw);
    cid_for_raw(&manifest_bytes_for_leaves(leaves))
}

//! CIDv1 content addressing (design §5.3/§5.4).
//!
//! P0 fixes `raw + SHA-256`. P1 adds the canonical chunked manifest: multi-block
//! content is addressed by the CID of a `ChunkManifest` (chunking.proto) encoded
//! with the deterministic rule frozen in `docs/contracts/p1-chunking.md` §3.
//! This module is the single implementation of that rule; nothing else may
//! rebuild manifest bytes.
use cid::Cid;
use multihash::Multihash;
use prost::Message;
use sha2::{Digest, Sha256};

use crate::v1::{ChunkEntry, ChunkManifest, ManifestEntries};
use crate::wire::ProtocolError;

/// multicodec: raw
pub const RAW_CODEC: u64 = 0x55;
/// multihash: sha2-256
pub const SHA2_256: u64 = 0x12;
/// Content format version for P1 manifests (`ChunkManifest.format_version`).
pub const MANIFEST_FORMAT_VERSION: u32 = 1;
/// Maximum entries per manifest level (`ManifestEntries`, chunking.proto).
pub const MANIFEST_FANOUT: usize = 1024;
/// Default broker chunk size: 256 KiB (design §5.4).
pub const CHUNK_SIZE: usize = 262_144;

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

/// Split bytes into fixed-size chunks (last chunk may be shorter).
pub fn chunk(bytes: &[u8], chunk_size: usize) -> impl Iterator<Item = &[u8]> {
    assert!(chunk_size > 0, "chunk_size must be > 0");
    bytes.chunks(chunk_size)
}

/// One manifest object: its CID and the exact bytes that produce it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestNode {
    pub cid: String,
    pub bytes: Vec<u8>,
}

/// Canonical manifest tree for one chunked content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestTree {
    /// Root CID; equals the CID of the last entry in `manifests`.
    pub root_cid: String,
    /// Leaf manifests first, then each parent level, root manifest last.
    pub manifests: Vec<ManifestNode>,
    pub leaf_cids: Vec<String>,
}

/// Number of leaves for a content length (always ≥ 2 for callers of
/// `manifest_tree`, since single-block content uses the raw CID).
fn leaf_count(content_length: u64, chunk_size: u32) -> u64 {
    content_length.div_ceil(chunk_size as u64)
}

/// Logical length of leaf `index` under the canonical fixed-size chunking rule.
pub fn leaf_length(content_length: u64, chunk_size: u32, index: usize) -> u64 {
    let start = index as u64 * chunk_size as u64;
    (content_length - start).min(chunk_size as u64)
}

fn leaf_manifest_bytes(
    chunk_size: u32,
    content_length: u64,
    leaves: &[String],
    base: usize,
) -> Vec<u8> {
    let entries = leaves
        .iter()
        .enumerate()
        .map(|(offset, cid)| ChunkEntry {
            chunk_cid: cid.clone(),
            chunk_length: leaf_length(content_length, chunk_size, base + offset).to_string(),
        })
        .collect();
    ChunkManifest {
        format_version: MANIFEST_FORMAT_VERSION,
        chunk_size,
        content_length: content_length.to_string(),
        root: Some(ManifestEntries {
            leaves: entries,
            child_manifest_cids: Vec::new(),
        }),
    }
    .encode_to_vec()
}

fn parent_manifest_bytes(chunk_size: u32, content_length: u64, children: &[String]) -> Vec<u8> {
    ChunkManifest {
        format_version: MANIFEST_FORMAT_VERSION,
        chunk_size,
        content_length: content_length.to_string(),
        root: Some(ManifestEntries {
            leaves: Vec::new(),
            child_manifest_cids: children.to_vec(),
        }),
    }
    .encode_to_vec()
}

fn invalid(message: impl Into<String>) -> ProtocolError {
    ProtocolError::bad_request(None, message)
}

/// Build the canonical manifest tree for chunked content.
///
/// The leaf list must be the full, ascending `chunkIndex` order; every leaf CID
/// must be a CIDv1 raw/SHA-256 text form. Manifest bytes are produced from the
/// generated `ChunkManifest` types, so Rust and TypeScript encode identical
/// bytes (see `conformance/vectors/p1/chunking.json`).
pub fn manifest_tree(
    chunk_size: u32,
    content_length: u64,
    leaf_cids: &[String],
) -> Result<ManifestTree, ProtocolError> {
    manifest_tree_with_fanout(chunk_size, content_length, leaf_cids, MANIFEST_FANOUT)
}

/// Same rule with an explicit per-level fanout. The wire format fixes 1024
/// (`MANIFEST_FANOUT`); the parameter exists so the canonical rule can be tested
/// at deeper levels without materializing a 1 MiB leaf list.
fn manifest_tree_with_fanout(
    chunk_size: u32,
    content_length: u64,
    leaf_cids: &[String],
    fanout: usize,
) -> Result<ManifestTree, ProtocolError> {
    if fanout == 0 {
        return Err(invalid("manifest fanout must be > 0"));
    }
    if chunk_size == 0 {
        return Err(invalid("chunk_size must be > 0"));
    }
    if content_length <= chunk_size as u64 {
        return Err(invalid(format!(
            "content_length {content_length} fits in one chunk ({chunk_size}); single-block content uses the raw CID"
        )));
    }
    let expected = leaf_count(content_length, chunk_size);
    if leaf_cids.len() as u64 != expected {
        return Err(invalid(format!(
            "expected {expected} leaves for {content_length} bytes at chunk_size {chunk_size}, got {}",
            leaf_cids.len()
        )));
    }
    for cid in leaf_cids {
        parse_cid(cid)?;
    }

    let mut manifests: Vec<ManifestNode> = Vec::new();
    let mut level: Vec<String> = Vec::new();
    for (group_index, group) in leaf_cids.chunks(fanout).enumerate() {
        let bytes = leaf_manifest_bytes(chunk_size, content_length, group, group_index * fanout);
        let cid = cid_for_raw(&bytes);
        level.push(cid.clone());
        manifests.push(ManifestNode { cid, bytes });
    }
    while level.len() > 1 {
        let mut next: Vec<String> = Vec::new();
        for group in level.chunks(fanout) {
            let bytes = parent_manifest_bytes(chunk_size, content_length, group);
            let cid = cid_for_raw(&bytes);
            next.push(cid.clone());
            manifests.push(ManifestNode { cid, bytes });
        }
        level = next;
    }
    let root_cid = level
        .into_iter()
        .next()
        .expect("leaf list is non-empty for multi-block content");
    Ok(ManifestTree {
        root_cid,
        manifests,
        leaf_cids: leaf_cids.to_vec(),
    })
}

/// Content address for one content given its leaf CIDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentAddressing {
    /// Raw CID for single-block content, manifest root CID otherwise.
    pub root_cid: String,
    /// Every manifest object (empty for single-block content).
    pub manifests: Vec<ManifestNode>,
}

/// Canonical address for content of `content_length` bytes at `chunk_size`,
/// addressed by its leaf CIDs in ascending chunk order. Single-block content
/// (`content_length <= chunk_size`) resolves to the raw CID of its one block.
pub fn addressing_for_parts(
    chunk_size: u32,
    content_length: u64,
    leaf_cids: &[String],
) -> Result<ContentAddressing, ProtocolError> {
    if chunk_size == 0 {
        return Err(invalid("chunk_size must be > 0"));
    }
    if content_length <= chunk_size as u64 {
        if leaf_cids.len() != 1 {
            return Err(invalid(format!(
                "single-block content needs exactly one leaf, got {}",
                leaf_cids.len()
            )));
        }
        parse_cid(&leaf_cids[0])?;
        return Ok(ContentAddressing {
            root_cid: leaf_cids[0].clone(),
            manifests: Vec::new(),
        });
    }
    let tree = manifest_tree(chunk_size, content_length, leaf_cids)?;
    Ok(ContentAddressing {
        root_cid: tree.root_cid,
        manifests: tree.manifests,
    })
}

/// Root CID for content addressed by explicit leaf CIDs.
pub fn content_cid_for_parts(
    chunk_size: u32,
    content_length: u64,
    leaf_cids: &[String],
) -> Result<String, ProtocolError> {
    Ok(addressing_for_parts(chunk_size, content_length, leaf_cids)?.root_cid)
}

/// Leaf CIDs for a payload, in ascending chunk order, plus its total length.
pub fn leaf_cids_for(bytes: &[u8], chunk_size: usize) -> (Vec<String>, u64) {
    assert!(chunk_size > 0, "chunk_size must be > 0");
    let leaves = chunk(bytes, chunk_size).map(cid_for_raw).collect();
    (leaves, bytes.len() as u64)
}

/// Unified content addressing function (设计 §5.3；P1 沿用同一函数).
///
/// - Single-block content (`len <= chunk_size`): CIDv1 raw/SHA-256 of the bytes.
/// - Multi-block content: CIDv1 raw/SHA-256 of the canonical `ChunkManifest`
///   bytes covering the fixed-size leaves (see `manifest_tree`).
///
/// `source/read` and every `blob/*` root MUST be produced by this function so
/// one content never has two comparable addresses.
pub fn content_cid(bytes: &[u8], chunk_size: usize) -> String {
    assert!(chunk_size > 0, "chunk_size must be > 0");
    if bytes.len() <= chunk_size {
        return cid_for_raw(bytes);
    }
    let (leaves, total) = leaf_cids_for(bytes, chunk_size);
    content_cid_for_parts(chunk_size as u32, total, &leaves)
        .expect("locally computed leaves always form a valid manifest")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(count: usize) -> Vec<String> {
        // Distinct, valid leaf CIDs for rule tests (content is irrelevant here).
        (0..count)
            .map(|i| cid_for_raw(format!("leaf-{i}").as_bytes()))
            .collect()
    }

    #[test]
    fn layered_tree_references_every_child_exactly_once() {
        // 5 leaves at fanout 2 forces three levels: [2,2,1] -> [2,1] -> [1].
        let cids = leaves(5);
        let tree = manifest_tree_with_fanout(16, 5 * 16, &cids, 2).unwrap();
        assert_eq!(tree.manifests.len(), 3 + 2 + 1);
        assert_eq!(tree.root_cid, tree.manifests.last().unwrap().cid);
        // Every node's bytes hash to its own CID; every non-root node is
        // referenced exactly once by the level above it.
        let mut referenced: Vec<String> = Vec::new();
        for node in &tree.manifests {
            assert_eq!(node.cid, cid_for_raw(&node.bytes), "node cid mismatch");
        }
        let root = tree.manifests.last().unwrap();
        let decoded = ChunkManifest::decode(root.bytes.as_slice()).unwrap();
        let root_entries = decoded.root.unwrap();
        assert!(root_entries.leaves.is_empty());
        referenced.extend(root_entries.child_manifest_cids);
        for node in &tree.manifests[..tree.manifests.len() - 1] {
            let decoded = ChunkManifest::decode(node.bytes.as_slice()).unwrap();
            let entries = decoded.root.unwrap();
            if !entries.leaves.is_empty() {
                continue;
            }
            referenced.extend(entries.child_manifest_cids);
        }
        assert_eq!(referenced.len(), tree.manifests.len() - 1);
        for node in &tree.manifests[..tree.manifests.len() - 1] {
            assert_eq!(
                referenced.iter().filter(|cid| **cid == node.cid).count(),
                1,
                "child {} must be referenced exactly once",
                node.cid
            );
        }
    }

    #[test]
    fn leaf_entries_carry_logical_lengths_and_total_length() {
        let cids = leaves(3);
        let tree = manifest_tree(16, 16 * 2 + 5, &cids).unwrap();
        let decoded = ChunkManifest::decode(tree.manifests[0].bytes.as_slice()).unwrap();
        assert_eq!(decoded.format_version, MANIFEST_FORMAT_VERSION);
        assert_eq!(decoded.chunk_size, 16);
        assert_eq!(decoded.content_length, "37");
        let lengths: Vec<String> = decoded
            .root
            .unwrap()
            .leaves
            .into_iter()
            .map(|entry| entry.chunk_length)
            .collect();
        assert_eq!(lengths, vec!["16", "16", "5"]);
    }

    #[test]
    fn rejects_invalid_manifest_inputs() {
        let cids = leaves(3);
        assert!(manifest_tree(0, 48, &cids).is_err(), "chunk_size 0");
        assert!(
            manifest_tree(16, 16, &cids).is_err(),
            "single-block content must use the raw CID"
        );
        assert!(
            manifest_tree(16, 48, &cids[..2]).is_err(),
            "leaf count must match content length"
        );
        let mut bad = cids.clone();
        bad[0] = "bafybeigdyrzt5sfp7udm7hu76uh7yganzbm4p7p7z3l3e3q".into();
        assert!(manifest_tree(16, 48, &bad).is_err(), "non-raw leaf CID");
    }

    #[test]
    fn single_block_content_is_the_raw_cid() {
        let payload = vec![7u8; CHUNK_SIZE];
        assert_eq!(content_cid(&payload, CHUNK_SIZE), cid_for_raw(&payload));
    }
}

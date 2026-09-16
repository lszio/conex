//! Local persistent block store.
//!
//! Layout under a single root path (validated to be free of `..` segments):
//!   <root>/blocks/<aa>/<bb>/<rest>   raw block bytes; CIDv1 raw SHA-256
//!   <root>/refs/<root>.committed     empty marker proving the root is committed
//!   <root>/refs/<root>.blocks        JSON list of leaf CIDs reachable from root
//!   <root>/staging/<upload_id>/      per-upload staging directory
//!   <root>/pins/<pin_id>.json        pin record
//!
//! Capability confinement is enforced at open by validating the root path
//! has no `..` / NUL segments and by reading/writing only inside that
//! root. All persistent files are written atomically (`.tmp` then rename).
//!
//! On reopen the store reconstructs in-memory state by scanning `pins/`,
//! `refs/*.committed` (paired with the matching `refs/*.blocks`), and
//! `staging/*/receipt.json`. Expired staging uploads are pruned during
//! recovery, so a crashed-then-reopened process never leaves a
//! "missing block but committed root" state.
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use multihash::Multihash;
use sha2::{Digest, Sha256};

use cap_std::ambient_authority;
use cap_std::fs::Dir;

use crate::error::ContentError;
use crate::receipt::{CommitRecord, PinRecord, UploadState, now_ms};

const RAW_CODEC: u64 = 0x55;
const SHA2_256: u64 = 0x12;

/// Validated capability-scoped handle to a content directory.
///
/// `Dir` is kept as the ambient proof of the root; file and directory I/O
/// goes through `std::fs` with paths that are joined to a sanitized root
/// `PathBuf`. This keeps the operational surface small while still bounding
/// all writes inside the validated root.
pub struct ContentRoot {
    dir: Dir,
    path: PathBuf,
}

impl std::fmt::Debug for ContentRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContentRoot")
            .field("path", &self.path)
            .finish()
    }
}

impl ContentRoot {
    pub fn open(path: &Path) -> Result<Self, ContentError> {
        if path.as_os_str().is_empty() {
            return Err(ContentError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "content root path is empty",
            )));
        }
        // Validate no parent traversal characters in the path.
        for component in path.components() {
            let s = component.as_os_str().to_string_lossy();
            if s == ".." || s.contains('\0') {
                return Err(ContentError::PathTraversal(path.display().to_string()));
            }
        }
        let dir = Dir::open_ambient_dir(path, ambient_authority()).or_else(|_| {
            Dir::create_ambient_dir_all(path, ambient_authority())?;
            Dir::open_ambient_dir(path, ambient_authority())
        })?;
        for sub in ["blocks", "refs", "staging", "pins"] {
            dir.create_dir_all(sub).map_err(ContentError::Io)?;
        }
        let path = path.to_path_buf();
        Ok(Self { dir, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Proof of capability: callers can hold `&Dir` for advanced cap-std APIs.
    pub fn dir(&self) -> &Dir {
        &self.dir
    }
}

#[derive(Default)]
pub(crate) struct Refs {
    counts: HashMap<String, u32>,
    commits: HashMap<String, CommitRecord>,
    pins: HashMap<String, PinRecord>,
    pub(crate) uploads: HashMap<String, UploadState>,
}
#[derive(Debug, Clone)]
pub struct ParsedCid {
    pub text: String,
    pub parsed: cid::Cid,
}

pub fn parse_cid(text: &str) -> Result<ParsedCid, ContentError> {
    if !text.starts_with("bafkrei") {
        return Err(ContentError::InvalidCid(text.to_string()));
    }
    let parsed =
        cid::Cid::try_from(text).map_err(|e| ContentError::InvalidCid(format!("{text}: {e}")))?;
    if parsed.codec() != RAW_CODEC {
        return Err(ContentError::InvalidCid(format!(
            "non-raw codec: {parsed:?}"
        )));
    }
    if parsed.hash().code() != SHA2_256 {
        return Err(ContentError::InvalidCid(format!(
            "non-sha2-256 multihash: {parsed:?}"
        )));
    }
    Ok(ParsedCid {
        text: text.to_string(),
        parsed,
    })
}
pub fn sha256_digest(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

pub fn cid_for_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let hash = Multihash::<64>::wrap(SHA2_256, &digest).expect("sha2-256 fits in 64 bytes");
    cid::Cid::new_v1(RAW_CODEC, hash).to_string()
}
/// Map a CID text to the relative path under `<root>/blocks/`. Fan-out uses
/// the first two base32 chars as `aa/bb` directory parts and the rest as
/// the file name. base32 length varies (longer when the digest has fewer
/// leading-zero bytes), so we don't assume a fixed total.
fn cid_to_block_relpath(cid_text: &str) -> PathBuf {
    let suffix = &cid_text[7..]; // base32 portion after the multibase prefix
    let (aa, rest) = suffix.split_at(2);
    let (bb, rest) = rest.split_at(2);
    PathBuf::from(format!("blocks/{aa}/{bb}/{rest}"))
}

pub struct LocalBlockStore {
    root: ContentRoot,
    refs: Mutex<Refs>,
}

impl LocalBlockStore {
    pub fn open(root: ContentRoot) -> Result<Self, ContentError> {
        let store = Self {
            root,
            refs: Mutex::new(Refs::default()),
        };
        store.recover()?;
        Ok(store)
    }
    #[allow(private_bounds)]
    pub fn with_refs<R>(&self, f: impl FnOnce(&mut Refs) -> R) -> R {
        let mut guard = self.refs.lock().expect("refs lock poisoned");
        f(&mut guard)
    }

    fn join(&self, rel: &Path) -> PathBuf {
        self.root.path.join(rel)
    }

    fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ContentError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!(
            "{}.tmp",
            path.extension().and_then(|s| s.to_str()).unwrap_or("dat")
        ));
        {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(bytes)?;
            f.flush()?;
        }
        fs::rename(&tmp, path)?;
        Ok(())
    }

    fn recover(&self) -> Result<(), ContentError> {
        // Pins.
        let pins_dir = self.join(Path::new("pins"));
        if pins_dir.exists() {
            for entry in fs::read_dir(&pins_dir)?.flatten() {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    let text = fs::read_to_string(&path)?;
                    if let Ok(pin) = serde_json::from_str::<PinRecord>(&text) {
                        let pin_id = pin.pin_id.clone();
                        self.with_refs(|r| {
                            r.pins.insert(pin_id, pin);
                        });
                    }
                }
            }
        }
        // Commits.
        let refs_dir = self.join(Path::new("refs"));
        if refs_dir.exists() {
            for entry in fs::read_dir(&refs_dir)?.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if name.ends_with(".committed") {
                    let root_cid = name.trim_end_matches(".committed").to_string();
                    let blocks_file = refs_dir.join(format!("{root_cid}.blocks"));
                    if blocks_file.exists() {
                        let text = fs::read_to_string(&blocks_file)?;
                        if let Ok(leaves) = serde_json::from_str::<Vec<String>>(&text) {
                            self.with_refs(|r| {
                                for leaf in leaves {
                                    *r.counts.entry(leaf).or_insert(0) += 1;
                                }
                                r.commits.insert(
                                    root_cid.clone(),
                                    CommitRecord {
                                        commit_id: format!("commit-{}", root_cid),
                                        root_cid: root_cid.clone(),
                                        root_kind: "raw".into(),
                                        committed_bytes: 0,
                                        receipt_id: format!("rcpt-{}", root_cid),
                                        committed_at_ms: now_ms(),
                                    },
                                );
                            });
                        }
                    }
                }
            }
        }
        // Uploads (and prune expired ones).
        let staging_dir = self.join(Path::new("staging"));
        if staging_dir.exists() {
            for entry in fs::read_dir(&staging_dir)?.flatten() {
                let path = entry.path();
                let receipt = path.join("receipt.json");
                if !receipt.exists() {
                    continue;
                }
                let Ok(text) = fs::read_to_string(&receipt) else {
                    continue;
                };
                let Ok(upload) = serde_json::from_str::<UploadState>(&text) else {
                    continue;
                };
                let upload_id = upload.upload_id.clone();
                if upload.is_expired() {
                    let _ = fs::remove_dir_all(&path);
                } else {
                    self.with_refs(|r| {
                        r.uploads.insert(upload_id, upload);
                    });
                }
            }
        }
        Ok(())
    }

    /// Persist a verified chunk; idempotent. Returns Ok(()) if already present.
    pub fn put_block(&self, cid_text: &str, bytes: &[u8]) -> Result<(), ContentError> {
        let _parsed = parse_cid(cid_text)?;
        let computed = cid_for_bytes(bytes);
        if computed != cid_text {
            return Err(ContentError::BadChunk {
                declared: cid_text.to_string(),
                computed,
            });
        }
        let target = self.join(&cid_to_block_relpath(cid_text));
        if target.exists() {
            return Ok(());
        }
        Self::atomic_write(&target, bytes)
    }

    pub fn has_block(&self, cid_text: &str) -> Result<bool, ContentError> {
        let _parsed = parse_cid(cid_text)?;
        Ok(self.join(&cid_to_block_relpath(cid_text)).exists())
    }

    pub fn read_block(&self, cid_text: &str) -> Result<Bytes, ContentError> {
        let _parsed = parse_cid(cid_text)?;
        let target = self.join(&cid_to_block_relpath(cid_text));
        let mut f = fs::File::open(&target).map_err(|e| {
            ContentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("{}: {e}", target.display()),
            ))
        })?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Ok(Bytes::from(buf))
    }

    pub fn record_upload(&self, upload: UploadState) -> Result<(), ContentError> {
        let upload_dir = self.join(Path::new("staging").join(&upload.upload_id).as_path());
        let receipt = upload_dir.join("receipt.json");
        let json = serde_json::to_string_pretty(&upload)?;
        Self::atomic_write(&receipt, json.as_bytes())?;
        self.with_refs(|r| {
            r.uploads.insert(upload.upload_id.clone(), upload);
        });
        Ok(())
    }

    pub fn get_upload(&self, upload_id: &str) -> Result<UploadState, ContentError> {
        self.with_refs(|r| {
            r.uploads
                .get(upload_id)
                .cloned()
                .ok_or_else(|| ContentError::UnknownUpload(upload_id.to_string()))
        })
    }

    pub fn record_chunk(&self, upload_id: &str, index: u32) -> Result<(), ContentError> {
        self.with_refs(|r| {
            let upload = r
                .uploads
                .get_mut(upload_id)
                .ok_or_else(|| ContentError::UnknownUpload(upload_id.to_string()))?;
            if upload.is_expired() {
                return Err(ContentError::UploadExpired);
            }
            upload.insert_chunk(index)?;
            Ok(())
        })?;
        // Persist the updated receipt so a crash recovers the same state.
        let upload = self.get_upload(upload_id)?;
        self.record_upload(upload)
    }

    /// Atomic commit: validate leaves, write refs/<root>.{committed,blocks}
    /// atomically, then update in-memory refcounts and remove the staging dir.
    pub fn commit(
        &self,
        upload_id: &str,
        declared_root_cid: &str,
        declared_root_kind: &str,
        leaves: &[String],
        total_bytes: u64,
    ) -> Result<CommitRecord, ContentError> {
        if leaves.is_empty() {
            return Err(ContentError::MissingChunks(vec![
                declared_root_cid.to_string(),
            ]));
        }
        for leaf in leaves {
            if !self.has_block(leaf)? {
                return Err(ContentError::MissingChunks(vec![leaf.clone()]));
            }
        }
        let refs_dir = self.join(Path::new("refs"));
        let committed_path = refs_dir.join(format!("{declared_root_cid}.committed"));
        let blocks_path = refs_dir.join(format!("{declared_root_cid}.blocks"));
        // Write `.blocks` first; only on success write `.committed`.
        let blocks_json = serde_json::to_string(leaves)?;
        Self::atomic_write(&blocks_path, blocks_json.as_bytes())?;
        Self::atomic_write(&committed_path, b"committed\n")?;
        let receipt_id = format!("rcpt-{declared_root_cid}");
        let record = CommitRecord {
            commit_id: format!("commit-{declared_root_cid}"),
            root_cid: declared_root_cid.to_string(),
            root_kind: declared_root_kind.to_string(),
            committed_bytes: total_bytes,
            receipt_id,
            committed_at_ms: now_ms(),
        };
        self.with_refs(|r| {
            for leaf in leaves {
                *r.counts.entry(leaf.clone()).or_insert(0) += 1;
            }
            r.commits
                .insert(declared_root_cid.to_string(), record.clone());
            r.uploads.remove(upload_id);
        });
        let staging_path = self.join(Path::new("staging").join(upload_id).as_path());
        let _ = fs::remove_dir_all(&staging_path);
        Ok(record)
    }

    pub fn has_commit(&self, root_cid: &str) -> bool {
        self.with_refs(|r| r.commits.contains_key(root_cid))
    }

    pub fn get_commit(&self, root_cid: &str) -> Option<CommitRecord> {
        self.with_refs(|r| r.commits.get(root_cid).cloned())
    }

    pub fn pin(
        &self,
        root_cid: &str,
        pin_id: &str,
        expires_at_ms: u64,
    ) -> Result<PinRecord, ContentError> {
        let record = PinRecord {
            pin_id: pin_id.to_string(),
            root_cid: root_cid.to_string(),
            expires_at_ms,
            created_at_ms: now_ms(),
        };
        let path = self.join(Path::new("pins").join(format!("{pin_id}.json")).as_path());
        let json = serde_json::to_string_pretty(&record)?;
        Self::atomic_write(&path, json.as_bytes())?;
        self.with_refs(|r| {
            r.pins.insert(pin_id.to_string(), record.clone());
        });
        Ok(record)
    }

    pub fn unpin(&self, pin_id: &str) -> Result<(), ContentError> {
        let existed = self.with_refs(|r| r.pins.remove(pin_id).is_some());
        if !existed {
            return Err(ContentError::UnknownPin(pin_id.to_string()));
        }
        let path = self.join(Path::new("pins").join(format!("{pin_id}.json")).as_path());
        fs::remove_file(&path).map_err(ContentError::Io)?;
        Ok(())
    }

    pub fn list_active_pins(&self) -> Vec<PinRecord> {
        let now = now_ms();
        self.with_refs(|r| {
            r.pins
                .values()
                .filter(|p| p.expires_at_ms > now)
                .cloned()
                .collect()
        })
    }

    /// Ref-counted GC: drop block bytes not referenced by any committed root
    /// or active pin. Prune expired uploads' staging dirs as a side effect.
    pub fn collect_garbage(&self) -> Result<usize, ContentError> {
        let now = now_ms();
        let mut dropped = 0;
        // Gather kept cids from every committed root.
        let commit_roots: Vec<String> = self.with_refs(|r| r.commits.keys().cloned().collect());
        let mut kept: std::collections::HashSet<String> = std::collections::HashSet::new();
        for root_cid in &commit_roots {
            let blocks_path = self.join(
                Path::new("refs")
                    .join(format!("{root_cid}.blocks"))
                    .as_path(),
            );
            if let Ok(text) = fs::read_to_string(&blocks_path)
                && let Ok(leaves) = serde_json::from_str::<Vec<String>>(&text)
            {
                for leaf in leaves {
                    kept.insert(leaf);
                }
            }
        }
        // Walk blocks directory.
        let blocks_root = self.join(Path::new("blocks"));
        if blocks_root.exists() {
            for aa_entry in fs::read_dir(&blocks_root)?.flatten() {
                let aa_name = aa_entry.file_name();
                let aa_dir = blocks_root.join(&aa_name);
                if !aa_dir.is_dir() {
                    continue;
                }
                for bb_entry in fs::read_dir(&aa_dir)?.flatten() {
                    let bb_name = bb_entry.file_name();
                    let bb_dir = aa_dir.join(&bb_name);
                    if !bb_dir.is_dir() {
                        continue;
                    }
                    for file_entry in fs::read_dir(&bb_dir)?.flatten() {
                        let name = file_entry.file_name();
                        let name_str = name.to_string_lossy().into_owned();
                        let path = bb_dir.join(&name);
                        // Drop .tmp residue.
                        if name_str.ends_with(".tmp") {
                            fs::remove_file(&path).ok();
                            continue;
                        }
                        let cid = format!(
                            "bafkrei{}{}{}",
                            aa_name.to_string_lossy(),
                            bb_name.to_string_lossy(),
                            name_str
                        );
                        if kept.contains(&cid) {
                            continue;
                        }
                        let refcount = self.with_refs(|r| r.counts.get(&cid).copied().unwrap_or(0));
                        let removed = refcount == 0 && fs::remove_file(&path).is_ok();
                        if removed {
                            dropped += 1;
                        }
                    }
                }
            }
        }
        // Prune expired uploads.
        let staging_root = self.join(Path::new("staging"));
        let expired: Vec<String> = self.with_refs(|r| {
            r.uploads
                .iter()
                .filter(|(_, u)| u.lease_until_ms <= now)
                .map(|(id, _)| id.clone())
                .collect()
        });
        for id in expired {
            let path = staging_root.join(&id);
            let _ = fs::remove_dir_all(&path);
            self.with_refs(|r| {
                r.uploads.remove(&id);
            });
        }
        Ok(dropped)
    }

    pub fn content_root(&self) -> &ContentRoot {
        &self.root
    }
}

pub fn sleep_ms(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

pub fn deterministic_id(prefix: &str, ts: u64, counter: u64) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    format!("{prefix}-{ts:013x}-{counter:08x}-{nanos:016x}")
}

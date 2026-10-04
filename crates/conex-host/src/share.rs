//! Group-scoped file sharing for the landing page's file scene.
//!
//! A visitor picks local files in the browser; the host keeps the bytes in
//! memory and offers them to the other clients of the *same group*. The group
//! key is the only access check: a file id is meaningless to anyone whose group
//! differs, and the check lives in one lookup rather than at every route.
//!
//! Everything here is bounded and disposable. Bytes live only while the owner
//! is connected — a disconnect drops them — so the store cannot accumulate a
//! public file archive on a long-running host. That is the ceiling worth
//! naming: this is a demo scene, not a durable share service; a host that needs
//! files to survive a restart would back this with `conex-content` and keep the
//! same group check.
//!
//! The size limits are part of the contract, not a tuning knob: a public
//! anonymous endpoint that accepts uploads must refuse an oversized body
//! before reading it into memory.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use conex_core::CallError;
use conex_proto;

/// Largest single file a visitor may share. The browser's own file picker can
/// hand over far more than this, so the limit is enforced before the body is
/// read.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Largest total a single group may hold at once.
pub const MAX_GROUP_BYTES: u64 = 32 * 1024 * 1024;
/// Largest number of files one client may offer at a time.
pub const MAX_FILES_PER_CLIENT: usize = 32;
/// Longest file name kept. Anything longer is truncated rather than rejected:
/// a long name is a cosmetic problem, not an attempt to abuse the store.
const MAX_NAME_CHARS: usize = 120;
const MAX_NAME_FALLBACK: &str = "file";

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// One shared file, as the browser sees it.
#[derive(Debug, Clone)]
pub struct SharedFile {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub owner_link_id: String,
    pub owner_name: String,
    pub group_key: String,
    pub created_at_ms: u64,
    bytes: Arc<Vec<u8>>,
}

impl SharedFile {
    /// Whether the browser may render this inline.
    ///
    /// The list is deliberately short and excludes anything that can carry
    /// script: `text/html` and `image/svg+xml` are downloadable but never
    /// shown, so a shared file cannot run code in another visitor's session.
    pub fn inline(&self) -> bool {
        matches!(
            self.mime.as_str(),
            "text/plain"
                | "text/markdown"
                | "text/csv"
                | "application/json"
                | "image/png"
                | "image/jpeg"
                | "image/gif"
                | "image/webp"
                | "application/pdf"
        )
    }
}

#[derive(Default)]
struct Inner {
    files: HashMap<String, SharedFile>,
    by_owner: HashMap<String, Vec<String>>,
    group_bytes: HashMap<String, u64>,
}

#[derive(Default, Clone)]
pub struct ShareStore {
    inner: Arc<Mutex<Inner>>,
}

impl ShareStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store one file for a client and return it.
    ///
    /// Quotas are checked before the bytes are stored, and the group's own
    /// current total is what the new file is measured against, so two clients
    /// racing to fill the last megabyte cannot both succeed.
    pub fn put(
        &self,
        owner_link_id: &str,
        owner_name: &str,
        group_key: &str,
        name: &str,
        mime: &str,
        bytes: Arc<Vec<u8>>,
    ) -> Result<SharedFile, CallError> {
        let size = bytes.len() as u64;
        if size > MAX_FILE_BYTES {
            return Err(CallError::new(
                conex_proto::ErrorCode::QuotaExceeded,
                format!("file is too large: {size} bytes, limit {MAX_FILE_BYTES}"),
            ));
        }
        let mut inner = self.inner.lock().expect("share store poisoned");
        let group_total = inner.group_bytes.get(group_key).copied().unwrap_or(0);
        if group_total.saturating_add(size) > MAX_GROUP_BYTES {
            return Err(CallError::new(
                conex_proto::ErrorCode::QuotaExceeded,
                "this group is already sharing too much data",
            ));
        }
        let owned = inner
            .files
            .values()
            .filter(|file| file.owner_link_id == owner_link_id)
            .count();
        if owned >= MAX_FILES_PER_CLIENT {
            return Err(CallError::new(
                conex_proto::ErrorCode::QuotaExceeded,
                "you already share the maximum number of files",
            ));
        }
        let file = SharedFile {
            id: format!("f{:x}", NEXT_ID.fetch_add(1, Ordering::AcqRel)),
            name: sanitize_name(name),
            mime: sanitize_mime(mime),
            size,
            owner_link_id: owner_link_id.to_string(),
            owner_name: owner_name.to_string(),
            group_key: group_key.to_string(),
            created_at_ms: crate::agent::now_ms(),
            bytes,
        };
        *inner.group_bytes.entry(group_key.to_string()).or_default() += size;
        inner
            .by_owner
            .entry(owner_link_id.to_string())
            .or_default()
            .push(file.id.clone());
        inner.files.insert(file.id.clone(), file.clone());
        Ok(file)
    }

    /// Every file in one group, oldest first, for the group browser.
    pub fn list(&self, group_key: &str) -> Vec<SharedFile> {
        let inner = self.inner.lock().expect("share store poisoned");
        let mut rows: Vec<SharedFile> = inner
            .files
            .values()
            .filter(|file| file.group_key == group_key)
            .cloned()
            .collect();
        drop(inner);
        rows.sort_by_key(|file| file.created_at_ms);
        rows
    }

    /// Fetch one file, but only for a caller inside the file's group.
    ///
    /// Returns `None` for an unknown id *and* for another group's id: a caller
    /// must not be able to tell those apart by watching the error.
    pub fn get(&self, id: &str, caller_group: &str) -> Option<SharedFile> {
        let inner = self.inner.lock().expect("share store poisoned");
        let file = inner.files.get(id)?;
        (file.group_key == caller_group).then(|| file.clone())
    }

    /// Bytes plus the metadata needed to answer a download.
    pub fn content(&self, id: &str, caller_group: &str) -> Option<(SharedFile, Arc<Vec<u8>>)> {
        let file = self.get(id, caller_group)?;
        let bytes = file.bytes.clone();
        Some((file, bytes))
    }

    /// Drop one file. Only the owner may withdraw it.
    ///
    /// The ownership check happens before the removal, not after: a refused
    /// request that had already deleted the file would let any group member
    /// destroy another member's upload.
    pub fn remove(&self, id: &str, owner_link_id: &str) -> Result<(), CallError> {
        let mut inner = self.inner.lock().expect("share store poisoned");
        let file = inner.files.get(id).cloned().ok_or_else(unknown_file)?;
        if file.owner_link_id != owner_link_id {
            return Err(forbidden("you do not own this file"));
        }
        inner.files.remove(id);
        if let Some(total) = inner.group_bytes.get_mut(&file.group_key) {
            *total = total.saturating_sub(file.size);
        }
        if let Some(ids) = inner.by_owner.get_mut(&file.owner_link_id) {
            ids.retain(|entry| entry != id);
        }
        Ok(())
    }

    /// Drop everything one client offered, and refresh its status counters.
    ///
    /// Called on disconnect: bytes a disconnected client no longer holds stop
    /// being readable immediately, and the group quota comes back.
    pub fn remove_owner(&self, owner_link_id: &str) -> usize {
        let mut inner = self.inner.lock().expect("share store poisoned");
        let Some(ids) = inner.by_owner.remove(owner_link_id) else {
            return 0;
        };
        let mut dropped = 0;
        for id in ids {
            if let Some(file) = inner.files.remove(&id) {
                if let Some(total) = inner.group_bytes.get_mut(&file.group_key) {
                    *total = total.saturating_sub(file.size);
                }
                dropped += 1;
            }
        }
        dropped
    }
}

fn forbidden(message: &str) -> CallError {
    CallError::new(conex_proto::ErrorCode::Forbidden, message)
}

fn unknown_file() -> CallError {
    CallError::new(conex_proto::ErrorCode::BadRequest, "unknown file")
}

/// Keep the visible name and drop everything that could be used to inject
/// into a header, a path, or the DOM as markup. The name is never used as a
/// filesystem path, but it is echoed into a `Content-Disposition` header and
/// rendered as text by every client in the group.
fn sanitize_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | '"' | '\'') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim();
    let capped: String = trimmed.chars().take(MAX_NAME_CHARS).collect();
    let capped = capped.trim().to_string();
    if capped.is_empty() {
        MAX_NAME_FALLBACK.to_string()
    } else {
        capped
    }
}

/// A MIME type crosses into a response header, so CR/LF and control bytes are
/// refused rather than escaped, and an empty value falls back to binary.
fn sanitize_mime(raw: &str) -> String {
    let mime = raw
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime.is_empty()
        || mime.len() > 128
        || mime
            .bytes()
            .any(|byte| byte == b'\r' || byte == b'\n' || byte == 0 || byte < 0x20)
    {
        "application/octet-stream".to_string()
    } else {
        mime
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ShareStore {
        ShareStore::new()
    }

    fn put(store: &ShareStore, owner: &str, group: &str, name: &str, size: usize) -> SharedFile {
        store
            .put(
                owner,
                "someone",
                group,
                name,
                "text/plain",
                Arc::new(vec![b'x'; size]),
            )
            .expect("put")
    }

    #[test]
    fn another_group_cannot_read_a_file() {
        let store = store();
        let file = put(&store, "a", "team-a", "notes.md", 4);
        assert!(store.get(&file.id, "team-a").is_some());
        assert!(
            store.get(&file.id, "team-b").is_none(),
            "a file id must be meaningless outside its group"
        );
    }

    #[test]
    fn group_quota_is_enforced_before_storing() {
        let store = store();
        let big = MAX_GROUP_BYTES;
        // Fill the group in MAX_FILE_BYTES steps, then one more must fail.
        let mut total = 0u64;
        let mut index = 0;
        while total + MAX_FILE_BYTES <= MAX_GROUP_BYTES {
            put(
                &store,
                &format!("a{index}"),
                "team-a",
                "f.bin",
                MAX_FILE_BYTES as usize,
            );
            total += MAX_FILE_BYTES;
            index += 1;
        }
        let error = store
            .put(
                "overflow",
                "someone",
                "team-a",
                "f.bin",
                "text/plain",
                Arc::new(vec![0u8; 1024]),
            )
            .expect_err("group quota must refuse the overflow");
        assert_eq!(error.code(), conex_proto::ErrorCode::QuotaExceeded as i32);
        assert_eq!(big, MAX_GROUP_BYTES);
    }

    #[test]
    fn a_single_file_over_the_limit_is_refused() {
        let store = store();
        let error = store
            .put(
                "a",
                "someone",
                "team-a",
                "big.bin",
                "application/octet-stream",
                Arc::new(vec![0u8; (MAX_FILE_BYTES + 1) as usize]),
            )
            .expect_err("oversized file must be refused");
        assert_eq!(error.code(), conex_proto::ErrorCode::QuotaExceeded as i32);
    }

    #[test]
    fn per_client_file_count_is_bounded() {
        let store = store();
        for index in 0..MAX_FILES_PER_CLIENT {
            put(&store, "a", "team-a", &format!("{index}.txt"), 1);
        }
        let error = store
            .put(
                "a",
                "someone",
                "team-a",
                "one-too-many.txt",
                "text/plain",
                Arc::new(vec![0u8; 1]),
            )
            .expect_err("file count cap must hold");
        assert_eq!(error.code(), conex_proto::ErrorCode::QuotaExceeded as i32);
    }

    #[test]
    fn disconnect_drops_the_files_and_frees_the_group_quota() {
        let store = store();
        let first = put(&store, "a", "team-a", "one.txt", 100);
        let second = put(&store, "a", "team-a", "two.txt", 200);
        assert_eq!(store.list("team-a").len(), 2);
        assert_eq!(store.remove_owner("a"), 2);
        assert!(store.list("team-a").is_empty());
        assert!(store.get(&first.id, "team-a").is_none());
        assert!(store.get(&second.id, "team-a").is_none());
        // The group total came back, so a former owner's quota is reusable.
        put(&store, "b", "team-a", "fresh.txt", MAX_FILE_BYTES as usize);
    }

    #[test]
    fn only_the_owner_may_withdraw_a_file() {
        let store = store();
        let file = put(&store, "a", "team-a", "one.txt", 10);
        let error = store
            .remove(&file.id, "b")
            .expect_err("a non-owner must not remove the file");
        assert_eq!(error.code(), conex_proto::ErrorCode::Forbidden as i32);
        assert!(store.get(&file.id, "team-a").is_some());
        store.remove(&file.id, "a").expect("owner removes");
        assert!(store.get(&file.id, "team-a").is_none());
    }

    #[test]
    fn active_content_is_never_inline() {
        let store = store();
        let html = store
            .put(
                "a",
                "someone",
                "team-a",
                "page.html",
                "text/html",
                Arc::new(b"<script>alert(1)</script>".to_vec()),
            )
            .expect("put");
        let svg = store
            .put(
                "a",
                "someone",
                "team-a",
                "logo.svg",
                "image/svg+xml",
                Arc::new(b"<svg/>".to_vec()),
            )
            .expect("put");
        let png = store
            .put(
                "a",
                "someone",
                "team-a",
                "shot.png",
                "image/png",
                Arc::new(b"\x89PNG".to_vec()),
            )
            .expect("put");
        assert!(!html.inline(), "html must download only");
        assert!(!svg.inline(), "svg must download only");
        assert!(png.inline());
    }

    #[test]
    fn names_and_mimes_cannot_smuggle_headers_or_paths() {
        let store = store();
        let file = store
            .put(
                "a",
                "someone",
                "team-a",
                "in/ject\"ed\r\nX: y.txt",
                "text/plain\r\nX: y",
                Arc::new(vec![0u8; 1]),
            )
            .expect("put");
        assert!(!file.name.contains('\r'));
        assert!(!file.name.contains('/'));
        assert!(!file.name.contains('"'));
        assert!(!file.mime.contains('\r'));
        assert_eq!(file.mime, "application/octet-stream");
    }

    #[test]
    fn an_empty_name_falls_back_instead_of_rendering_blank() {
        let store = store();
        let file = put(&store, "a", "team-a", "   ", 1);
        assert_eq!(file.name, "file");
    }
}

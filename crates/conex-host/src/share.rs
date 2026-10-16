//! Group-scoped file and clipboard sharing for the landing page.
//!
//! A visitor offers local files (or clipboard content) to the other clients of
//! the *same group*. The host keeps metadata in memory and the bytes in the
//! content store, addressed by CID; the group key is the only access check, so
//! a file id is meaningless to anyone whose group differs, and the check lives
//! in one lookup rather than at every route.
//!
//! Everything here is bounded and disposable. An item lives only while its
//! owner is connected — a disconnect drops it and frees its bytes — so the
//! scene cannot accumulate a public archive on a long-running host. That is
//! the ceiling worth naming: this is a demo scene, not a durable share
//! service. A host that needs files to survive a restart has the P1 blob
//! channel for that, with the same CID addressing underneath.
//!
//! The size limits are part of the contract, not a tuning knob: a public
//! anonymous endpoint that accepts uploads must refuse an oversized item
//! before reading it off the wire.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use conex_core::CallError;
use conex_proto;
use sha2::{Digest, Sha256};

use crate::share_blocks::{SHARE_CHUNK_SIZE, ShareBlocks};

/// Longest name kept, including its directory path. Anything longer is
/// truncated rather than rejected: a long name is a cosmetic problem, not an
/// attempt to abuse the store.
const MAX_NAME_CHARS: usize = 240;
const MAX_NAME_FALLBACK: &str = "file";
/// Longest MIME accepted. A value past this is a header-injection attempt or
/// a broken client, not a format worth remembering.
const MAX_MIME_CHARS: usize = 128;
/// Leading bytes the host offers for a preview read.
///
/// The page fetches this and no more, so a preview of a huge file never costs
/// the whole file. It is one chunk, which is also the smallest unit the
/// content store can serve anyway.
pub const PREVIEW_BYTES: u64 = SHARE_CHUNK_SIZE as u64;

/// What an item is. A clip travels the same path as a file and differs only in
/// policy: it is smaller, it expires, and it is counted separately so a busy
/// clipboard cannot exhaust the file scene's slots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareItemKind {
    File,
    Clip,
}

impl ShareItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ShareItemKind::File => "file",
            ShareItemKind::Clip => "clip",
        }
    }
}

/// Per-kind limits. Keeping them apart is what stops one kind of offer from
/// starving the other: a clipboard that filled the file slots would take the
/// file scene down with it.
#[derive(Debug, Clone)]
struct KindLimit {
    max_bytes: u64,
    max_items: usize,
    /// 0 means the item does not expire on its own.
    ttl_ms: u64,
}

/// The scene's ceilings. Host-configured, and reported to the page with every
/// listing so it renders the real limit instead of a hard-coded one.
#[derive(Debug, Clone)]
pub struct ShareLimits {
    pub max_file_bytes: u64,
    pub max_group_bytes: u64,
    pub max_files_per_client: usize,
    pub max_clips_per_client: usize,
    pub clip_ttl_ms: u64,
}

impl Default for ShareLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: 8 * 1024 * 1024,
            max_group_bytes: 32 * 1024 * 1024,
            max_files_per_client: 32,
            max_clips_per_client: 20,
            clip_ttl_ms: 10 * 60 * 1000,
        }
    }
}

impl ShareLimits {
    /// Largest single item of this kind one client may offer.
    ///
    /// Public because the routes refuse an oversized *declaration* before any
    /// byte moves, which is the only place a big upload can be cheaply refused.
    pub fn max_bytes_for(&self, kind: ShareItemKind) -> u64 {
        self.limit(kind).max_bytes
    }

    fn limit(&self, kind: ShareItemKind) -> KindLimit {
        match kind {
            ShareItemKind::File => KindLimit {
                max_bytes: self.max_file_bytes,
                max_items: self.max_files_per_client,
                ttl_ms: 0,
            },
            ShareItemKind::Clip => KindLimit {
                // A clip is text or a screenshot; it is not a transfer channel.
                max_bytes: self.max_clip_bytes(),
                max_items: self.max_clips_per_client,
                ttl_ms: self.clip_ttl_ms,
            },
        }
    }

    /// A clip never gets the file ceiling: it is exchanged for convenience, so
    /// it stays small enough that the scene's memory and disk stay bounded.
    fn max_clip_bytes(&self) -> u64 {
        self.max_file_bytes.min(1024 * 1024)
    }
}

/// One shared item, as the browser sees it. The host keeps this and nothing
/// else: `root_cid` addresses the bytes, which live in the content store.
#[derive(Debug, Clone)]
pub struct SharedFile {
    pub id: String,
    pub kind: ShareItemKind,
    pub name: String,
    pub mime: String,
    pub size: u64,
    /// CIDv1 raw / sha2-256 of the committed content.
    pub root_cid: String,
    pub owner_link_id: String,
    pub owner_name: String,
    pub group_key: String,
    pub created_at_ms: u64,
    /// Epoch milliseconds after which a clip is no longer served; 0 for a file.
    pub expires_at_ms: u64,
}

impl SharedFile {
    /// Whether the browser may render this inline.
    ///
    /// The list is deliberately short and excludes anything that can carry
    /// script: `text/html` and `image/svg+xml` are downloadable but never
    /// shown, so a shared file cannot run code in another visitor's session.
    pub fn inline(&self) -> bool {
        // A clip is offered from a clipboard the host did not see, so it goes
        // through the same allow-list rather than a wider one.
        matches!(
            self.mime.as_str(),
            "text/plain"
                | "text/markdown"
                | "text/csv"
                | "text/plain; charset=utf-8"
                | "application/json"
                | "image/png"
                | "image/jpeg"
                | "image/gif"
                | "image/webp"
                | "application/pdf"
                | "audio/mpeg"
                | "audio/ogg"
                | "audio/wav"
                | "video/mp4"
                | "video/webm"
        )
    }

    /// Bytes the host offers for a preview read: one chunk at most, and none
    /// for content the browser may not render.
    pub fn preview_bytes(&self) -> u64 {
        if self.inline() {
            self.size.min(PREVIEW_BYTES)
        } else {
            0
        }
    }

    /// Whether this item has aged out. A file never does.
    pub fn expired(&self, now_ms: u64) -> bool {
        self.expires_at_ms != 0 && now_ms >= self.expires_at_ms
    }
}

/// What an in-flight upload will become, declared by the client at init.
///
/// Kept here rather than on the staged upload, because the scene is already the
/// home of everything a restart drops: an upload that outlives a restart has no
/// item to become, and a client that comes back re-declares all of it anyway.
#[derive(Debug, Clone)]
pub struct StagedItem {
    pub name: String,
    pub mime: String,
    pub kind: ShareItemKind,
    pub size_bytes: u64,
    pub owner_link_id: String,
}

#[derive(Default)]
struct Inner {
    files: HashMap<String, SharedFile>,
    /// In-flight uploads, by upload id. Dropped on commit, cancel and restart.
    staged: HashMap<String, StagedItem>,
    /// Ids by owner, so a disconnect drops exactly what that client offered.
    by_owner: HashMap<String, Vec<String>>,
    /// Live bytes per group, measured against `max_group_bytes`.
    group_bytes: HashMap<String, u64>,
    /// Live item count per group and kind.
    group_items: HashMap<(String, &'static str), usize>,
}

/// The scene: metadata in memory, bytes in one content store.
#[derive(Clone)]
pub struct ShareStore {
    inner: Arc<Mutex<Inner>>,
    /// `None` when no share root is configured, which makes every route
    /// unavailable rather than silently falling back to a second engine.
    blocks: Option<ShareBlocks>,
    limits: ShareLimits,
}

/// What a caller needs to offer an item: who it is, and how big.
pub struct Offer<'a> {
    pub owner_link_id: &'a str,
    pub owner_name: &'a str,
    pub group_key: &'a str,
    pub name: &'a str,
    pub mime: &'a str,
    pub kind: ShareItemKind,
    /// Bytes offered, and the `now` the expiry is measured from.
    pub size_bytes: u64,
    pub now_ms: u64,
}

impl ShareStore {
    /// A scene with no storage. Reads answer `unavailable`; writes refuse.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self::with(None, ShareLimits::default())
    }

    /// A scene backed by a share root, with the host's own ceilings.
    pub fn backed(blocks: ShareBlocks, limits: ShareLimits) -> Self {
        Self::with(Some(blocks), limits)
    }

    /// A scene with limits and no storage, for a host that named no share root.
    pub fn with_limits(limits: ShareLimits) -> Self {
        Self::with(None, limits)
    }

    fn with(blocks: Option<ShareBlocks>, limits: ShareLimits) -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::default())),
            blocks,
            limits,
        }
    }

    /// The scene's ceilings, reported to the page with every listing.
    pub fn limits(&self) -> &ShareLimits {
        &self.limits
    }

    /// The share root's own limits, or `None` when the scene is unconfigured.
    pub fn lease_ms(&self) -> Option<u64> {
        self.blocks.as_ref().map(|blocks| blocks.lease_ms())
    }

    pub fn is_backed(&self) -> bool {
        self.blocks.is_some()
    }

    /// The scene's storage, or `None` when the host has no share root. Routes
    /// answer `unavailable` rather than pretending to store bytes.
    pub fn storage(&self) -> Option<&ShareBlocks> {
        self.blocks.as_ref()
    }

    /// Drop one live offer of a root, freeing its bytes when the last offer
    /// goes. Used when an item is refused or withdrawn after its bytes were
    /// already committed.
    pub fn release(&self, root_cid: &str) {
        if let Some(blocks) = self.blocks.as_ref() {
            blocks.release(root_cid);
        }
    }

    /// Record one committed item for a client and return it.
    ///
    /// Quotas are checked before the item is stored, and the group's own
    /// current total is what the new item is measured against, so two clients
    /// racing to fill the last megabyte cannot both succeed. The bytes are
    /// already committed at this point: refusing here leaves them staged until
    /// their lease expires, which the caller releases immediately.
    pub fn admit(&self, root_cid: &str, offer: &Offer<'_>) -> Result<SharedFile, CallError> {
        let limit = self.limits.limit(offer.kind);
        let size = offer.size_bytes;
        if size == 0 {
            return Err(bad_request("the shared item is empty"));
        }
        if size > limit.max_bytes {
            return Err(quota(format!(
                "item is too large: {size} bytes, limit {}",
                limit.max_bytes
            )));
        }
        let mut inner = self.inner.lock().expect("share store poisoned");
        let group_total = inner.group_bytes.get(offer.group_key).copied().unwrap_or(0);
        if group_total.saturating_add(size) > self.limits.max_group_bytes {
            return Err(quota(
                "this group is already sharing too much data".to_string(),
            ));
        }
        let owned = inner
            .files
            .values()
            .filter(|file| file.owner_link_id == offer.owner_link_id)
            .filter(|file| file.kind == offer.kind)
            .count();
        if owned >= limit.max_items {
            return Err(quota(format!(
                "you already share the maximum number of {}s",
                offer.kind.as_str()
            )));
        }
        let file = SharedFile {
            id: format!("f{:x}", NEXT_ID.fetch_add(1, Ordering::AcqRel)),
            kind: offer.kind,
            name: sanitize_name(offer.name),
            mime: sanitize_mime(offer.mime),
            size,
            root_cid: root_cid.to_string(),
            owner_link_id: offer.owner_link_id.to_string(),
            owner_name: offer.owner_name.to_string(),
            group_key: offer.group_key.to_string(),
            created_at_ms: offer.now_ms,
            expires_at_ms: if limit.ttl_ms == 0 {
                0
            } else {
                offer.now_ms.saturating_add(limit.ttl_ms)
            },
        };
        *inner
            .group_bytes
            .entry(offer.group_key.to_string())
            .or_default() += size;
        *inner
            .group_items
            .entry((offer.group_key.to_string(), offer.kind.as_str()))
            .or_default() += 1;
        inner
            .by_owner
            .entry(offer.owner_link_id.to_string())
            .or_default()
            .push(file.id.clone());
        inner.files.insert(file.id.clone(), file.clone());
        Ok(file)
    }

    /// A digest of one group's listing, stable until the group's items change.
    ///
    /// This is what lets a page poll cheaply: an unchanged listing is answered
    /// with a `304` and no body, and the alternative is re-serialising every item
    /// on every poll. It is built from the ids in the order the listing is
    /// served in, so two identical listings always produce the same digest.
    pub fn listing_etag(&self, group_key: &str, now_ms: u64) -> Option<String> {
        let inner = self.inner.lock().expect("share store poisoned");
        let any = inner.files.values().any(|file| file.group_key == group_key);
        if !any {
            // An empty group has no listing to validate, and `None` keeps the
            // 304 machinery from doubling as a "nothing here yet" oracle.
            return None;
        }
        let mut ids: Vec<&str> = inner
            .files
            .values()
            .filter(|file| file.group_key == group_key && !file.expired(now_ms))
            .map(|file| file.id.as_str())
            .collect();
        ids.sort_unstable();
        let digest = Sha256::digest(ids.join("\u{1f}"));
        Some(format!("\"{}\"", hex::encode(digest)))
    }

    /// Every unexpired item in one group, oldest first, for the group browser.
    ///
    /// Expired clips are dropped as a side effect of being read: a clipboard
    /// that nobody looked at still stops being served, which is the whole
    /// point of giving it a lifetime.
    pub fn list(&self, group_key: &str, now_ms: u64) -> Vec<SharedFile> {
        let mut inner = self.inner.lock().expect("share store poisoned");
        let expired: Vec<String> = inner
            .files
            .iter()
            .filter(|(_, file)| file.group_key == group_key && file.expired(now_ms))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &expired {
            if let Some(file) = inner.files.remove(id) {
                Self::release_group(&mut inner, &file);
            }
        }
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

    /// Fetch one item, but only for a caller inside the item's group.
    ///
    /// Returns `None` for an unknown id *and* for another group's id: a caller
    /// must not be able to tell those apart by watching the error. An expired
    /// clip reads the same way — its lifetime is not a distinguishable fact.
    pub fn get(&self, id: &str, caller_group: &str, now_ms: u64) -> Option<SharedFile> {
        let mut inner = self.inner.lock().expect("share store poisoned");
        let file = inner.files.get(id)?.clone();
        if file.expired(now_ms) {
            inner.files.remove(id);
            Self::release_group(&mut inner, &file);
            return None;
        }
        (file.group_key == caller_group).then_some(file)
    }

    /// Metadata only. The bytes are read from the content store by the caller,
    /// which is what lets a preview ask for a range and not the whole item.
    pub fn content(&self, id: &str, caller_group: &str, now_ms: u64) -> Option<SharedFile> {
        self.get(id, caller_group, now_ms)
    }

    /// Drop one item. Only the owner may withdraw it.
    ///
    /// The ownership check happens before the removal, not after: a refused
    /// request that had already deleted the item would let any group member
    /// destroy another member's upload.
    pub fn remove(&self, id: &str, owner_link_id: &str) -> Result<(), CallError> {
        let mut inner = self.inner.lock().expect("share store poisoned");
        let file = inner.files.get(id).cloned().ok_or_else(unknown_file)?;
        if file.owner_link_id != owner_link_id {
            return Err(forbidden("you do not own this file"));
        }
        inner.files.remove(id);
        Self::release_group(&mut inner, &file);
        Ok(())
    }

    /// Remember what an in-flight upload will become, at init.
    pub fn stage(&self, upload_id: &str, item: StagedItem) {
        let mut inner = self.inner.lock().expect("share store poisoned");
        inner.staged.insert(upload_id.to_string(), item);
    }

    /// The declaration for an in-flight upload, if this scene still holds it.
    pub fn staged_item(&self, upload_id: &str) -> Option<StagedItem> {
        let inner = self.inner.lock().expect("share store poisoned");
        inner.staged.get(upload_id).cloned()
    }

    /// Forget an in-flight upload: it became an item, or was abandoned.
    pub fn unstage(&self, upload_id: &str) {
        let mut inner = self.inner.lock().expect("share store poisoned");
        inner.staged.remove(upload_id);
    }

    /// Drop everything one client offered.
    ///
    /// Called on disconnect: items a disconnected client no longer holds stop
    /// being readable immediately. The bytes are freed by the caller through
    /// the block store, because this struct holds the metadata and not the
    /// storage.
    pub fn remove_owner(&self, owner_link_id: &str) -> Vec<SharedFile> {
        let mut inner = self.inner.lock().expect("share store poisoned");
        inner
            .staged
            .retain(|_, item| item.owner_link_id != owner_link_id);
        let Some(ids) = inner.by_owner.remove(owner_link_id) else {
            return Vec::new();
        };
        let mut dropped = Vec::new();
        for id in ids {
            if let Some(file) = inner.files.remove(&id) {
                Self::release_group(&mut inner, &file);
                dropped.push(file);
            }
        }
        dropped
    }

    /// Drop every expired clip in every group. Called on a timer so an expired
    /// clipboard is reclaimed even when nobody browses the scene again.
    pub fn sweep_expired(&self, now_ms: u64) -> Vec<SharedFile> {
        let mut inner = self.inner.lock().expect("share store poisoned");
        let expired: Vec<String> = inner
            .files
            .iter()
            .filter(|(_, file)| file.expired(now_ms))
            .map(|(id, _)| id.clone())
            .collect();
        let mut dropped = Vec::new();
        for id in expired {
            if let Some(file) = inner.files.remove(&id) {
                Self::release_group(&mut inner, &file);
                dropped.push(file);
            }
        }
        dropped
    }

    /// One owner's live counters, for `client/status`.
    pub fn owner_totals(&self, owner_link_id: &str) -> (u32, u64) {
        let inner = self.inner.lock().expect("share store poisoned");
        let owned: Vec<&SharedFile> = inner
            .files
            .values()
            .filter(|file| file.owner_link_id == owner_link_id)
            .collect();
        let bytes: u64 = owned.iter().map(|file| file.size).sum();
        (owned.len() as u32, bytes)
    }

    /// Every group's live counters, for `client/status`.
    pub fn group_totals(&self) -> HashMap<String, (u32, u64)> {
        let inner = self.inner.lock().expect("share store poisoned");
        let mut totals: HashMap<String, (u32, u64)> = HashMap::new();
        for file in inner.files.values() {
            let entry = totals.entry(file.group_key.clone()).or_default();
            entry.0 += 1;
            entry.1 += file.size;
        }
        totals
    }

    /// Bytes of one item, read from the scene's storage.
    ///
    /// Reading is separated from metadata on purpose: a preview asks for a
    /// range and never materialises the whole item, which is what keeps a
    /// multi-gigabyte file cheap to look at.
    pub fn read_range(
        &self,
        file: &SharedFile,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, CallError> {
        let blocks = self
            .blocks
            .as_ref()
            .ok_or_else(|| unavailable("the share scene has no storage configured"))?;
        blocks
            .read_range(&file.root_cid, offset, length)
            .map_err(|error| match error {
                // A root that is no longer committed reads the same as an
                // unknown id, so a caller cannot probe for what the scene once
                // held, nor for how long a clip lived.
                conex_content::ContentError::UnknownCommit(_)
                | conex_content::ContentError::UnknownBlock(_) => not_found("unknown file"),
                conex_content::ContentError::Forbidden => forbidden("you do not own this file"),
                other => crate::broker::content_to_call(other),
            })
    }

    fn release_group(inner: &mut Inner, file: &SharedFile) {
        if let Some(total) = inner.group_bytes.get_mut(&file.group_key) {
            *total = total.saturating_sub(file.size);
        }
        let key = (file.group_key.clone(), file.kind.as_str());
        if let Some(count) = inner.group_items.get_mut(&key) {
            *count = count.saturating_sub(1);
        }
        if let Some(ids) = inner.by_owner.get_mut(&file.owner_link_id) {
            ids.retain(|entry| entry != &file.id);
        }
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn bad_request(message: &str) -> CallError {
    CallError::new(conex_proto::ErrorCode::BadRequest, message)
}

fn forbidden(message: &str) -> CallError {
    CallError::new(conex_proto::ErrorCode::Forbidden, message)
}

fn quota(message: String) -> CallError {
    CallError::new(conex_proto::ErrorCode::QuotaExceeded, message)
}

fn unavailable(message: &str) -> CallError {
    CallError::new(conex_proto::ErrorCode::Unavailable, message)
}

fn not_found(message: &str) -> CallError {
    CallError::new(conex_proto::ErrorCode::BadRequest, message)
}

fn unknown_file() -> CallError {
    CallError::new(conex_proto::ErrorCode::BadRequest, "unknown file")
}

/// Keep the visible name and drop everything that could be injected into a
/// header or the DOM as markup. A relative directory path is allowed — the
/// file scene renders `dir/sub/file` as a tree — but every segment is
/// cleaned and traversal segments are flattened, so the name stays a display
/// string. It is never used as a filesystem path: the bytes live in the
/// content store, keyed by CID.
fn sanitize_name(raw: &str) -> String {
    let mut segments: Vec<String> = Vec::new();
    for segment in raw.split('/') {
        let cleaned: String = segment
            .chars()
            .map(|c| {
                if c.is_control() || matches!(c, '\\' | '"' | '\'') {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        match cleaned.trim() {
            "" | "." => {}
            ".." => segments.push("_".to_string()),
            clean => segments.push(clean.to_string()),
        }
    }
    let joined = segments.join("/");
    let capped: String = joined.chars().take(MAX_NAME_CHARS).collect();
    let capped = capped.trim().trim_matches('/').to_string();
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
        || mime.len() > MAX_MIME_CHARS
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

    fn store_with(limits: ShareLimits) -> ShareStore {
        ShareStore::with_limits(limits)
    }

    fn limits() -> ShareLimits {
        ShareLimits::default()
    }

    fn offer<'a>(owner: &'a str, group: &'a str, name: &'a str) -> Offer<'a> {
        Offer {
            owner_link_id: owner,
            owner_name: "someone",
            group_key: group,
            name,
            mime: "text/plain",
            kind: ShareItemKind::File,
            size_bytes: 4,
            now_ms: 1_000,
        }
    }

    fn admit(store: &ShareStore, owner: &str, group: &str, name: &str) -> SharedFile {
        store
            .admit("root-cid", &offer(owner, group, name))
            .expect("admit")
    }

    #[test]
    fn another_group_cannot_read_an_item() {
        let store = store();
        let file = admit(&store, "a", "team-a", "notes.md");
        assert!(store.get(&file.id, "team-a", 1_000).is_some());
        assert!(
            store.get(&file.id, "team-b", 1_000).is_none(),
            "an item id must be meaningless outside its group"
        );
    }

    #[test]
    fn an_unknown_id_and_a_foreign_id_are_indistinguishable() {
        let store = store();
        let file = admit(&store, "a", "team-a", "notes.md");
        let foreign = store.get(&file.id, "team-b", 1_000);
        let unknown = store.get("f999", "team-a", 1_000);
        assert!(foreign.is_none() && unknown.is_none());
    }

    #[test]
    fn group_quota_is_enforced_before_storing() {
        let store = store_with(ShareLimits {
            max_group_bytes: 64,
            max_file_bytes: 32,
            ..ShareLimits::default()
        });
        for index in 0..2 {
            let owner = format!("a{index}");
            let mut offer = offer(&owner, "team-a", "f.bin");
            offer.size_bytes = 32;
            store.admit("root", &offer).expect("inside quota");
        }
        let mut offer = offer("overflow", "team-a", "f.bin");
        offer.size_bytes = 32;
        let error = store
            .admit("root", &offer)
            .expect_err("group quota must refuse the overflow");
        assert_eq!(error.code(), conex_proto::ErrorCode::QuotaExceeded as i32);
    }

    #[test]
    fn a_single_item_over_the_limit_is_refused() {
        let store = store();
        let mut offer = offer("a", "team-a", "big.bin");
        offer.size_bytes = limits().max_file_bytes + 1;
        let error = store
            .admit("root", &offer)
            .expect_err("oversized item must be refused");
        assert_eq!(error.code(), conex_proto::ErrorCode::QuotaExceeded as i32);
    }

    #[test]
    fn an_empty_item_is_refused() {
        let store = store();
        let mut offer = offer("a", "team-a", "empty.txt");
        offer.size_bytes = 0;
        let error = store
            .admit("root", &offer)
            .expect_err("an empty item must be refused");
        assert_eq!(error.code(), conex_proto::ErrorCode::BadRequest as i32);
    }

    #[test]
    fn per_client_file_count_is_bounded() {
        let store = store();
        for index in 0..limits().max_files_per_client {
            admit(&store, "a", "team-a", &format!("{index}.txt"));
        }
        let error = store
            .admit("root", &offer("a", "team-a", "one-too-many.txt"))
            .expect_err("file count cap must hold");
        assert_eq!(error.code(), conex_proto::ErrorCode::QuotaExceeded as i32);
    }

    #[test]
    fn clips_have_their_own_slot_budget() {
        let store = store();
        let mut clip = offer("a", "team-a", "clip.txt");
        clip.kind = ShareItemKind::Clip;
        for _ in 0..limits().max_clips_per_client {
            store.admit("root", &clip).expect("clip admitted");
        }
        let error = store.admit("root", &clip).expect_err("clip cap must hold");
        assert_eq!(error.code(), conex_proto::ErrorCode::QuotaExceeded as i32);
        // The file slots are untouched by the clip cap.
        admit(&store, "a", "team-a", "notes.txt");
    }

    #[test]
    fn a_clip_stops_being_served_after_its_lifetime() {
        let store = store();
        let mut offer = offer("a", "team-a", "secret.txt");
        offer.kind = ShareItemKind::Clip;
        let file = store.admit("root", &offer).expect("clip admitted");
        assert!(file.expires_at_ms > 0);
        // Before the lifetime it reads like any item.
        assert!(store.get(&file.id, "team-a", file.created_at_ms).is_some());
        // After it, it is gone — and reads exactly like an unknown id.
        let later = file.expires_at_ms;
        assert!(store.get(&file.id, "team-a", later).is_none());
        assert!(store.get("f999", "team-a", later).is_none());
        assert!(store.list("team-a", later).is_empty());
    }

    #[test]
    fn a_disconnect_drops_that_clients_items_and_frees_the_quota() {
        let store = store();
        let first = admit(&store, "a", "team-a", "one.txt");
        let second = admit(&store, "a", "team-a", "two.txt");
        assert_eq!(store.list("team-a", 1_000).len(), 2);
        let dropped = store.remove_owner("a");
        assert_eq!(dropped.len(), 2);
        assert!(dropped.iter().any(|file| file.id == first.id));
        assert!(dropped.iter().any(|file| file.id == second.id));
        assert!(store.list("team-a", 1_000).is_empty());
        assert!(store.get(&first.id, "team-a", 1_000).is_none());
        // The group total came back, so a former owner's quota is reusable.
        let mut fresh = offer("b", "team-a", "fresh.bin");
        fresh.size_bytes = limits().max_file_bytes;
        store.admit("root", &fresh).expect("quota came back");
    }

    #[test]
    fn only_the_owner_may_withdraw_an_item() {
        let store = store();
        let file = admit(&store, "a", "team-a", "one.txt");
        let error = store
            .remove(&file.id, "b")
            .expect_err("a non-owner must not remove the item");
        assert_eq!(error.code(), conex_proto::ErrorCode::Forbidden as i32);
        assert!(store.get(&file.id, "team-a", 1_000).is_some());
        store.remove(&file.id, "a").expect("owner removes");
        assert!(store.get(&file.id, "team-a", 1_000).is_none());
    }

    #[test]
    fn active_content_is_never_inline() {
        let store = store();
        let mut html = offer("a", "team-a", "page.html");
        html.mime = "text/html";
        let html = store.admit("root", &html).expect("admit");
        let mut svg = offer("a", "team-a", "logo.svg");
        svg.mime = "image/svg+xml";
        let svg = store.admit("root", &svg).expect("admit");
        let mut clip = offer("a", "team-a", "fragment.html");
        clip.mime = "text/html";
        clip.kind = ShareItemKind::Clip;
        let clip = store.admit("root", &clip).expect("admit");
        assert!(!html.inline(), "html must download only");
        assert!(!svg.inline(), "svg must download only");
        assert!(!clip.inline(), "a clip is not a wider allow-list");
        assert_eq!(html.preview_bytes(), 0);
    }

    #[test]
    fn a_preview_read_is_bounded_but_never_invents_bytes() {
        let store = store();
        let mut small = offer("a", "team-a", "tiny.txt");
        small.size_bytes = 10;
        let small = store.admit("root", &small).expect("admit");
        assert_eq!(small.preview_bytes(), 10);
        let mut big = offer("a", "team-a", "huge.txt");
        big.size_bytes = 5 * 1024 * 1024;
        let big = store.admit("root", &big).expect("admit");
        assert_eq!(big.preview_bytes(), PREVIEW_BYTES);
    }

    #[test]
    fn names_cannot_smuggle_headers_but_may_carry_directory_paths() {
        let store = store();
        let mut nasty = offer("a", "team-a", "docs/../in ject\"ed\r\nX: y.txt");
        nasty.mime = "text/plain\r\nX: y";
        let nasty = store.admit("root", &nasty).expect("admit");
        assert!(!nasty.name.contains('\r'));
        assert!(!nasty.name.contains('"'));
        assert!(
            !nasty.name.contains(".."),
            "traversal segments are flattened"
        );
        assert_eq!(nasty.name, "docs/_/in ject_ed__X: y.txt");
        assert_eq!(nasty.mime, "application/octet-stream");
        let nested = admit(&store, "a", "team-a", "/照片/2024//聚会.png");
        assert_eq!(nested.name, "照片/2024/聚会.png");
    }

    #[test]
    fn an_empty_name_falls_back_instead_of_rendering_blank() {
        let store = store();
        let file = admit(&store, "a", "team-a", "   ");
        assert_eq!(file.name, "file");
    }

    #[test]
    fn an_unchanged_listing_has_a_stable_digest_and_a_changed_one_does_not() {
        let store = store();
        admit(&store, "a", "team-a", "one.txt");
        let digest = store.listing_etag("team-a", 1_000).expect("digest");
        // A re-read of an unchanged listing must produce the same digest, or
        // every poll would look like a change.
        assert_eq!(store.listing_etag("team-a", 1_000), Some(digest.clone()));
        assert!(digest.starts_with('"') && digest.ends_with('"'), "{digest}");
        // A second item in the same group changes it.
        admit(&store, "b", "team-a", "two.txt");
        assert_ne!(store.listing_etag("team-a", 1_000), Some(digest.clone()));
        // A group with nothing in it has no listing to validate.
        assert!(store.listing_etag("team-b", 1_000).is_none());
    }

    #[test]
    fn an_expired_clip_changes_the_listing_it_was_part_of() {
        let store = store();
        let mut clip = offer("a", "team-a", "secret.txt");
        clip.kind = ShareItemKind::Clip;
        let file = store.admit("root", &clip).expect("clips admit");
        let before = store
            .listing_etag("team-a", file.created_at_ms)
            .expect("digest");
        // Reading it away expires it, and the digest must move with it, or a
        // page holding the old validator would keep showing the row.
        assert!(store.list("team-a", file.expires_at_ms).is_empty());
        assert!(
            store.listing_etag("team-a", file.expires_at_ms).is_none(),
            "a group with nothing left has no digest"
        );
        assert_ne!(
            Some(before),
            store.listing_etag("team-a", file.expires_at_ms)
        );
    }

    #[test]
    fn owner_counters_only_ever_describe_that_owner() {
        let store = store();
        let mut big = offer("a", "team-a", "big.bin");
        big.size_bytes = 100;
        store.admit("root", &big).expect("admit");
        admit(&store, "b", "team-a", "small.txt");
        assert_eq!(store.owner_totals("a"), (1, 100));
        assert_eq!(store.owner_totals("b"), (1, 4));
        let totals = store.group_totals();
        assert_eq!(totals.get("team-a"), Some(&(2, 104)));
    }
}

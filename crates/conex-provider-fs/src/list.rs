//! Directory listing over a bounded, principal-bound snapshot.
use std::io::Read;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use conex_core::{CallContext, CallError, CallResult, ExecutionIo, Handler};
use conex_proto;
use conex_source::contracts::{DEFAULT_PAGE, MAX_DOC_BYTES, MAX_ITEMS};
use conex_source::pagination::{PageItem, SnapshotCache, SnapshotKey};
use serde_json::Value;

use crate::read::{FsRoot, map_io};

pub fn mime_for(resource: &str) -> Option<&'static str> {
    match resource.rsplit_once('.') {
        Some((_, "md")) => Some("text/markdown"),
        Some((_, "org")) => Some("text/org"),
        Some((_, "txt")) => Some("text/plain"),
        Some((_, "html")) => Some("text/html"),
        Some((_, "csv")) => Some("text/csv"),
        Some((_, "json")) => Some("application/json"),
        Some((_, "docx")) => {
            Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document")
        }
        Some((_, "zip")) => Some("application/zip"),
        Some((_, "gz")) => Some("application/gzip"),
        Some((_, "pdf")) => Some("application/pdf"),
        Some((_, "png")) => Some("image/png"),
        Some((_, "jpg")) | Some((_, "jpeg")) => Some("image/jpeg"),
        Some((_, "gif")) => Some("image/gif"),
        Some((_, "webp")) => Some("image/webp"),
        Some((_, "avif")) => Some("image/avif"),
        Some((_, "svg")) => Some("image/svg+xml"),
        Some((_, "mp4")) => Some("video/mp4"),
        Some((_, "webm")) => Some("video/webm"),
        Some((_, "ogv")) => Some("video/ogg"),
        Some((_, "mp3")) => Some("audio/mpeg"),
        Some((_, "wav")) => Some("audio/wav"),
        _ => None,
    }
}

/// Extensions eligible for inline text and content search.
pub fn is_text_resource(resource: &str) -> bool {
    matches!(
        resource.rsplit_once('.'),
        Some((_, "md") | (_, "org") | (_, "txt"))
    )
}

pub fn build_summary(
    resource: &str,
    size: u64,
    revision: Option<String>,
) -> CallResult<conex_proto::ResourceSummary> {
    let mime = mime_for(resource).unwrap_or("application/octet-stream");
    let title = resource.rsplit('/').next().unwrap_or(resource).to_string();
    Ok(conex_proto::ResourceSummary {
        resource_id: resource.to_string(),
        title,
        mime: mime.to_string(),
        size_bytes: Some(size.to_string()),
        revision,
        kind: conex_proto::EntryKind::File as i32,
    })
}

fn read_dir(dir: &cap_std::fs::Dir, path: &str) -> CallResult<Vec<(String, bool)>> {
    let target = if path.is_empty() { "." } else { path };
    let mut entries = Vec::new();
    for entry in dir.read_dir(target).map_err(map_io)? {
        let entry = entry.map_err(map_io)?;
        let file_type = entry.file_type().map_err(map_io)?;
        entries.push((
            entry.file_name().to_string_lossy().to_string(),
            file_type.is_dir(),
        ));
    }
    entries.sort();
    Ok(entries)
}

/// Recursively collect resources under a root. Listing (query=None) includes
/// every regular file regardless of type or size (metadata-only; content is
/// never read). Search (query=Some) restricts to supported text extensions and
/// skips files that are oversized or not valid UTF-8. Symlinks are rejected; a
/// scan over the item budget is a hard quota error.
pub fn scan(
    root: &FsRoot,
    root_resource: &str,
    query: Option<&str>,
    max_items: usize,
) -> CallResult<Vec<PageItem>> {
    let mut items: Vec<PageItem> = Vec::new();
    let mut stack = vec![root_resource.to_string()];
    while let Some(directory) = stack.pop() {
        for (name, is_dir) in read_dir(root.dir(), &directory)? {
            let resource = if directory.is_empty() {
                name.clone()
            } else {
                format!("{directory}/{name}")
            };
            if is_dir {
                stack.push(resource);
                continue;
            }
            let mime = mime_for(&resource).unwrap_or("application/octet-stream");
            let (length, mtime_ns) = root.stat(&resource)?;
            let mut item = PageItem {
                resource_id: resource.clone(),
                title: resource.rsplit('/').next().unwrap_or(&resource).to_string(),
                mime: mime.to_string(),
                size_bytes: length,
                revision: Some(mtime_ns.to_string()),
                excerpt: None,
            };
            if let Some(query) = query {
                if !is_text_resource(&resource) || length > MAX_DOC_BYTES {
                    continue;
                }
                let (file, _) = root.open_regular(&resource)?;
                let mut bytes = Vec::with_capacity(length as usize);
                file.take(MAX_DOC_BYTES)
                    .read_to_end(&mut bytes)
                    .map_err(map_io)?;
                // Non-UTF-8 text files are skipped silently, never forced.
                let Ok(text) = String::from_utf8(bytes) else {
                    continue;
                };
                match make_excerpt(&text, query) {
                    Some(excerpt) => item.excerpt = Some(excerpt),
                    None => continue,
                }
            }
            items.push(item);
            if items.len() > max_items {
                return Err(CallError::new(
                    conex_proto::ErrorCode::QuotaExceeded,
                    "scan exceeds the item budget",
                ));
            }
        }
    }
    items.sort_by(|a, b| a.resource_id.cmp(&b.resource_id));
    Ok(items)
}

fn make_excerpt(text: &str, query: &str) -> Option<String> {
    let index = text.find(query)?;
    let start = floor_char_boundary(text, index.saturating_sub(128));
    let end = ceil_char_boundary(text, (start + 512).min(text.len()));
    Some(text[start..end].to_string())
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

pub struct ListHandler {
    pub root: Arc<FsRoot>,
    pub cache: Arc<Mutex<SnapshotCache>>,
}

impl ListHandler {
    pub(crate) fn key(&self, ctx: &CallContext, root: &str) -> SnapshotKey {
        SnapshotKey {
            tenant_id: ctx.caller.tenant_id.clone(),
            principal_id: ctx.caller.principal_id.clone(),
            endpoint_id: ctx.endpoint_id.clone(),
            method: "source/list".into(),
            root: root.to_string(),
            query: String::new(),
        }
    }
}

#[async_trait]
impl Handler for ListHandler {
    async fn execute(
        &self,
        ctx: &CallContext,
        input: Value,
        _io: ExecutionIo,
    ) -> CallResult<Value> {
        let root_resource = input.get("root").and_then(Value::as_str).unwrap_or("");
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_PAGE as u64) as usize;
        let cursor = input.get("cursor").and_then(Value::as_str);
        let key = self.key(ctx, root_resource);
        let (items, next_cursor) = {
            let mut cache = self.cache.lock().expect("snapshot cache lock poisoned");
            match cursor {
                None => {
                    let scanned = scan(&self.root, root_resource, None, MAX_ITEMS)?;
                    let first = cache.create(key.clone(), scanned)?;
                    cache.page(&first, &key, limit)?
                }
                Some(cursor) => cache.page(cursor, &key, limit)?,
            }
        };
        let response = conex_proto::SourceListResponse {
            items: items.iter().map(to_summary).collect(),
            next_cursor,
        };
        serde_json::to_value(response).map_err(|error| {
            CallError::new(
                conex_proto::ErrorCode::Internal,
                format!("encode response: {error}"),
            )
        })
    }
}

fn to_summary(item: &PageItem) -> conex_proto::ResourceSummary {
    conex_proto::ResourceSummary {
        resource_id: item.resource_id.clone(),
        title: item.title.clone(),
        mime: item.mime.clone(),
        size_bytes: Some(item.size_bytes.to_string()),
        revision: item.revision.clone(),
        kind: conex_proto::EntryKind::File as i32,
    }
}

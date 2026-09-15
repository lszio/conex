//! Directory listing over a bounded, principal-bound snapshot.
use std::io::Read;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use conex_core::{CallContext, CallError, CallResult, ExecutionIo, Handler};
use conex_proto::v1;
use conex_source::contracts::{DEFAULT_PAGE, MAX_DOC_BYTES, MAX_ITEMS};
use conex_source::pagination::{PageItem, SnapshotCache, SnapshotKey};
use serde_json::Value;

use crate::read::{FsRoot, map_io};

pub fn mime_for(resource: &str) -> Option<&'static str> {
    match resource.rsplit('.').next() {
        Some("md") => Some("text/markdown"),
        Some("org") => Some("text/org"),
        _ => None,
    }
}

pub fn build_summary(resource: &str, size: u64) -> CallResult<v1::ResourceSummary> {
    let mime = mime_for(resource)
        .ok_or_else(|| CallError::new(v1::ErrorCode::BadRequest, "unsupported resource type"))?;
    let title = resource.rsplit('/').next().unwrap_or(resource).to_string();
    Ok(v1::ResourceSummary {
        resource_id: resource.to_string(),
        title,
        mime: mime.to_string(),
        size_bytes: Some(size),
        revision: None,
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

/// Recursively collect supported documents under a root. Symlinks and oversized
/// documents are skipped; a scan over the item budget is a hard quota error.
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
            let Some(mime) = mime_for(&resource) else {
                continue;
            };
            let (file, length) = root.open_regular(&resource)?;
            if length > MAX_DOC_BYTES {
                continue;
            }
            let mut item = PageItem {
                resource_id: resource.clone(),
                title: resource.rsplit('/').next().unwrap_or(&resource).to_string(),
                mime: mime.to_string(),
                size_bytes: length,
                excerpt: None,
            };
            if let Some(query) = query {
                let mut text = String::new();
                file.take(MAX_DOC_BYTES + 1)
                    .read_to_string(&mut text)
                    .map_err(map_io)?;
                match make_excerpt(&text, query) {
                    Some(excerpt) => item.excerpt = Some(excerpt),
                    None => continue,
                }
            }
            items.push(item);
            if items.len() > max_items {
                return Err(CallError::new(
                    v1::ErrorCode::QuotaExceeded,
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
        let response = v1::SourceListResponse {
            items: items.iter().map(to_summary).collect(),
            next_cursor,
        };
        serde_json::to_value(response).map_err(|error| {
            CallError::new(v1::ErrorCode::Internal, format!("encode response: {error}"))
        })
    }
}

fn to_summary(item: &PageItem) -> v1::ResourceSummary {
    v1::ResourceSummary {
        resource_id: item.resource_id.clone(),
        title: item.title.clone(),
        mime: item.mime.clone(),
        size_bytes: Some(item.size_bytes),
        revision: None,
    }
}

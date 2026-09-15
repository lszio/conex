//! Literal substring search over a bounded, principal-bound snapshot.
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use conex_core::{CallContext, CallError, CallResult, ExecutionIo, Handler};
use conex_proto::v1;
use conex_source::contracts::{DEFAULT_PAGE, MAX_ITEMS};
use conex_source::pagination::{PageItem, SnapshotCache, SnapshotKey};
use serde_json::Value;

use crate::list::scan;
use crate::read::FsRoot;

pub struct SearchHandler {
    pub root: Arc<FsRoot>,
    pub cache: Arc<Mutex<SnapshotCache>>,
}

#[async_trait]
impl Handler for SearchHandler {
    async fn execute(
        &self,
        ctx: &CallContext,
        input: Value,
        _io: ExecutionIo,
    ) -> CallResult<Value> {
        let root_resource = input.get("root").and_then(Value::as_str).unwrap_or("");
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| CallError::new(v1::ErrorCode::BadRequest, "query is required"))?;
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_PAGE as u64) as usize;
        let cursor = input.get("cursor").and_then(Value::as_str);
        let key = SnapshotKey {
            tenant_id: ctx.caller.tenant_id.clone(),
            principal_id: ctx.caller.principal_id.clone(),
            endpoint_id: ctx.endpoint_id.clone(),
            method: "source/search".into(),
            root: root_resource.to_string(),
            query: query.to_string(),
        };
        let (items, next_cursor) = {
            let mut cache = self.cache.lock().expect("snapshot cache lock poisoned");
            match cursor {
                None => {
                    let scanned = scan(&self.root, root_resource, Some(query), MAX_ITEMS)?;
                    let first = cache.create(key.clone(), scanned)?;
                    cache.page(&first, &key, limit)?
                }
                Some(cursor) => cache.page(cursor, &key, limit)?,
            }
        };
        let hits: Vec<v1::SearchHit> = items
            .iter()
            .map(|item| v1::SearchHit {
                resource: Some(summary(item)),
                excerpt: item.excerpt.clone().unwrap_or_default(),
            })
            .collect();
        let response = v1::SourceSearchResponse {
            items: hits,
            next_cursor,
        };
        serde_json::to_value(response).map_err(|error| {
            CallError::new(v1::ErrorCode::Internal, format!("encode response: {error}"))
        })
    }
}

fn summary(item: &PageItem) -> v1::ResourceSummary {
    v1::ResourceSummary {
        resource_id: item.resource_id.clone(),
        title: item.title.clone(),
        mime: item.mime.clone(),
        size_bytes: Some(item.size_bytes),
        revision: None,
    }
}

//! Second real source provider: a static HTTP catalog behind the shared port.
#![forbid(unsafe_code)]

pub mod catalog;
pub mod contracts;

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bytes::Bytes;
use conex_core::{
    CallContext, CallError, CallResult, ExecutionIo, Handler, Installation, OutboundRequest, Route,
};
use conex_proto::v1;
use conex_source::contracts::{DEFAULT_PAGE, SOURCE_LIST, SOURCE_READ, SOURCE_SEARCH};
use conex_source::pagination::{
    Clock, PageItem, SnapshotCache, SnapshotKey, SnapshotLimits, SystemClock,
};
use serde_json::Value;
use tokio::time::Instant;

pub use catalog::{Catalog, CatalogEntry, MAX_RESPONSE_BYTES};

pub const FACTORY_KIND: &str = "source-http-catalog";
const DEFAULT_FIXED_PATH: &str = "/catalog.json";

pub fn factory(installation: &Installation) -> CallResult<Vec<Route>> {
    let target = installation.target.clone().ok_or_else(|| {
        CallError::new(
            v1::ErrorCode::Internal,
            "http-catalog installation requires a target",
        )
    })?;
    let fixed_path = if target.fixed_path.is_empty() {
        DEFAULT_FIXED_PATH.to_string()
    } else {
        target.fixed_path.clone()
    };
    let max_response = installation
        .provider
        .get("maxResponseBytes")
        .and_then(Value::as_u64)
        .unwrap_or(MAX_RESPONSE_BYTES as u64) as usize;
    let clock: Arc<dyn Clock> = Arc::new(SystemClock);
    let cache = Arc::new(Mutex::new(SnapshotCache::new(
        clock,
        SnapshotLimits::default(),
    )));

    let mut routes = Vec::new();
    for (method, contract) in contracts::contracts() {
        let handler: Arc<dyn Handler> = match method {
            SOURCE_READ => Arc::new(CatalogReadHandler {
                fixed_path: fixed_path.clone(),
                max_response,
            }),
            SOURCE_LIST => Arc::new(CatalogListHandler {
                fixed_path: fixed_path.clone(),
                max_response,
                cache: cache.clone(),
            }),
            SOURCE_SEARCH => Arc::new(CatalogSearchHandler {
                fixed_path: fixed_path.clone(),
                max_response,
                cache: cache.clone(),
            }),
            other => {
                return Err(CallError::new(
                    v1::ErrorCode::UnsupportedCapability,
                    format!("unexpected source method {other}"),
                ));
            }
        };
        routes.push(Route {
            protocol: installation.factory.protocol.clone(),
            version: installation.factory.version,
            endpoint: installation.endpoint.clone(),
            method: method.to_string(),
            contract,
            handler,
            target: installation.target.clone(),
            credential: installation.credential.clone(),
        });
    }
    Ok(routes)
}

struct CatalogReadHandler {
    fixed_path: String,
    max_response: usize,
}

#[async_trait]
impl Handler for CatalogReadHandler {
    async fn execute(
        &self,
        ctx: &CallContext,
        input: Value,
        mut io: ExecutionIo,
    ) -> CallResult<Value> {
        let resource = input
            .get("resourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("resourceId is required"))?;
        let catalog = fetch(&mut io, &self.fixed_path, ctx.deadline, self.max_response).await?;
        let entry = catalog
            .get(resource)
            .ok_or_else(|| bad("resource is not present in the catalog"))?;
        // Same function as every `blob/*` root (design §5.3).
        let cid =
            conex_proto::cid::content_cid(entry.text.as_bytes(), conex_proto::cid::CHUNK_SIZE);
        let summary = v1::ResourceSummary {
            resource_id: entry.resource_id.clone(),
            title: entry.title.clone(),
            mime: entry.mime.clone(),
            size_bytes: Some(entry.text.len() as u64),
            revision: None,
        };
        to_value(v1::SourceReadResponse {
            resource: Some(summary),
            text: entry.text.clone(),
            cid,
        })
    }
}

struct CatalogListHandler {
    fixed_path: String,
    max_response: usize,
    cache: Arc<Mutex<SnapshotCache>>,
}

#[async_trait]
impl Handler for CatalogListHandler {
    async fn execute(
        &self,
        ctx: &CallContext,
        input: Value,
        mut io: ExecutionIo,
    ) -> CallResult<Value> {
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_PAGE as u64) as usize;
        let cursor = input.get("cursor").and_then(Value::as_str);
        let fresh = if cursor.is_none() {
            Some(fetch(&mut io, &self.fixed_path, ctx.deadline, self.max_response).await?)
        } else {
            None
        };
        let key = SnapshotKey {
            tenant_id: ctx.caller.tenant_id.clone(),
            principal_id: ctx.caller.principal_id.clone(),
            endpoint_id: ctx.endpoint_id.clone(),
            method: "source/list".into(),
            root: String::new(),
            query: String::new(),
        };
        let (items, next_cursor) = {
            let mut cache = self.cache.lock().expect("snapshot cache lock poisoned");
            match (cursor, fresh) {
                (None, Some(catalog)) => {
                    let page_items: Vec<PageItem> =
                        catalog.entries().iter().map(to_page_item).collect();
                    let first = cache.create(key.clone(), page_items)?;
                    cache.page(&first, &key, limit)?
                }
                (Some(cursor), _) => cache.page(cursor, &key, limit)?,
                (None, None) => unreachable!("fresh catalog fetched when there is no cursor"),
            }
        };
        let response = v1::SourceListResponse {
            items: items.iter().map(to_summary).collect(),
            next_cursor,
        };
        to_value(response)
    }
}

struct CatalogSearchHandler {
    fixed_path: String,
    max_response: usize,
    cache: Arc<Mutex<SnapshotCache>>,
}

#[async_trait]
impl Handler for CatalogSearchHandler {
    async fn execute(
        &self,
        ctx: &CallContext,
        input: Value,
        mut io: ExecutionIo,
    ) -> CallResult<Value> {
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("query is required"))?
            .to_string();
        let limit = input
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_PAGE as u64) as usize;
        let cursor = input.get("cursor").and_then(Value::as_str);
        let fresh = if cursor.is_none() {
            Some(fetch(&mut io, &self.fixed_path, ctx.deadline, self.max_response).await?)
        } else {
            None
        };
        let key = SnapshotKey {
            tenant_id: ctx.caller.tenant_id.clone(),
            principal_id: ctx.caller.principal_id.clone(),
            endpoint_id: ctx.endpoint_id.clone(),
            method: "source/search".into(),
            root: String::new(),
            query: query.clone(),
        };
        let (items, next_cursor) = {
            let mut cache = self.cache.lock().expect("snapshot cache lock poisoned");
            match (cursor, fresh) {
                (None, Some(catalog)) => {
                    let page_items: Vec<PageItem> = catalog
                        .entries()
                        .iter()
                        .filter_map(|entry| {
                            let excerpt = make_excerpt(&entry.text, &query)?;
                            let mut item = to_page_item(entry);
                            item.excerpt = Some(excerpt);
                            Some(item)
                        })
                        .collect();
                    let first = cache.create(key.clone(), page_items)?;
                    cache.page(&first, &key, limit)?
                }
                (Some(cursor), _) => cache.page(cursor, &key, limit)?,
                (None, None) => unreachable!("fresh catalog fetched when there is no cursor"),
            }
        };
        let hits: Vec<v1::SearchHit> = items
            .iter()
            .map(|item| v1::SearchHit {
                resource: Some(to_summary(item)),
                excerpt: item.excerpt.clone().unwrap_or_default(),
            })
            .collect();
        to_value(v1::SourceSearchResponse {
            items: hits,
            next_cursor,
        })
    }
}

async fn fetch(
    io: &mut ExecutionIo,
    fixed_path: &str,
    deadline: Instant,
    max_response: usize,
) -> CallResult<Catalog> {
    let connection = io.connection.as_mut().ok_or_else(|| {
        CallError::new(
            v1::ErrorCode::Unavailable,
            "catalog requires a verified connection",
        )
    })?;
    let mut headers = http::HeaderMap::new();
    if let Some(secret) = &io.secret {
        let value = http::HeaderValue::from_str(secret.expose())
            .map_err(|_| bad("credential cannot be used as an HTTP header"))?;
        headers.insert(http::header::AUTHORIZATION, value);
    }
    let request = OutboundRequest {
        method: http::Method::GET,
        path: fixed_path.to_string(),
        headers,
        body: Bytes::new(),
    };
    let response = connection.request(request, deadline).await?;
    if response.status != 200 {
        return Err(CallError::new(
            v1::ErrorCode::Unavailable,
            format!("catalog upstream status {}", response.status),
        ));
    }
    if response.body.len() > max_response {
        return Err(CallError::new(
            v1::ErrorCode::PayloadTooLarge,
            "catalog response exceeds maxResponseBytes",
        ));
    }
    Catalog::parse(&response.body)
}

fn to_page_item(entry: &CatalogEntry) -> PageItem {
    PageItem {
        resource_id: entry.resource_id.clone(),
        title: entry.title.clone(),
        mime: entry.mime.clone(),
        size_bytes: entry.text.len() as u64,
        excerpt: None,
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

fn to_value<T: serde::Serialize>(value: T) -> CallResult<Value> {
    serde_json::to_value(value).map_err(|error| {
        CallError::new(v1::ErrorCode::Internal, format!("encode response: {error}"))
    })
}

fn bad(message: impl Into<String>) -> CallError {
    CallError::new(v1::ErrorCode::BadRequest, message)
}

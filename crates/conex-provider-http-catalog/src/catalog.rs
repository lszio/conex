//! Parse a bounded static HTTP catalog into an in-memory partition.
use std::collections::HashSet;

use conex_core::{CallError, CallResult};
use conex_proto::v1;
use serde_json::Value;

pub const MAX_ENTRIES: usize = 10_000;
pub const MAX_DOC_BYTES: usize = 256 * 1024;
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub resource_id: String,
    pub title: String,
    pub mime: String,
    pub text: String,
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
    entries: Vec<CatalogEntry>,
}

impl Catalog {
    pub fn parse(bytes: &[u8]) -> CallResult<Catalog> {
        let document: Value = serde_json::from_slice(bytes)
            .map_err(|error| bad(format!("catalog is not valid JSON: {error}")))?;
        let entries = document
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("catalog.entries must be an array"))?;
        if entries.len() > MAX_ENTRIES {
            return Err(CallError::new(
                v1::ErrorCode::QuotaExceeded,
                "catalog exceeds the item budget",
            ));
        }
        let mut seen: HashSet<String> = HashSet::new();
        let mut parsed = Vec::with_capacity(entries.len());
        for entry in entries {
            let map = entry
                .as_object()
                .ok_or_else(|| bad("catalog entry must be an object"))?;
            let resource_id = string_field(map, "resourceId")?;
            conex_source::resource::normalize_resource(&resource_id)?;
            if resource_id.is_empty() {
                return Err(bad("resourceId must not be empty"));
            }
            if !seen.insert(resource_id.clone()) {
                return Err(bad("duplicate resourceId in catalog"));
            }
            let title = string_field(map, "title")?;
            let mime = string_field(map, "mime")?;
            let text = string_field(map, "text")?;
            if text.len() > MAX_DOC_BYTES {
                return Err(CallError::new(
                    v1::ErrorCode::PayloadTooLarge,
                    "catalog entry exceeds the per-document limit",
                ));
            }
            parsed.push(CatalogEntry {
                resource_id,
                title,
                mime,
                text,
            });
        }
        Ok(Catalog { entries: parsed })
    }

    pub fn get(&self, resource: &str) -> Option<&CatalogEntry> {
        self.entries
            .iter()
            .find(|entry| entry.resource_id == resource)
    }

    pub fn entries(&self) -> &[CatalogEntry] {
        &self.entries
    }
}

fn string_field(map: &serde_json::Map<String, Value>, key: &str) -> CallResult<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| bad(format!("catalog entry field {key} must be a string")))
}

fn bad(message: impl Into<String>) -> CallError {
    CallError::new(v1::ErrorCode::BadRequest, message)
}

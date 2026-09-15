//! Shared source MethodContracts: strict decode, claim extraction, output check.
use conex_core::{CallError, CallResult, MethodContract, PreparedInput};
use conex_proto::v1;
use serde_json::{Map, Value, json};

use crate::resource::{read_claim, subtree_claim};

pub const SOURCE_LIST: &str = "source/list";
pub const SOURCE_READ: &str = "source/read";
pub const SOURCE_SEARCH: &str = "source/search";

pub const DEFAULT_PAGE: u32 = 50;
pub const MAX_PAGE: u32 = 100;
pub const MAX_QUERY_CHARS: usize = 256;
pub const MAX_DOC_BYTES: u64 = 256 * 1024;
pub const MAX_ITEMS: usize = 10_000;

pub fn contracts() -> Vec<(&'static str, MethodContract)> {
    vec![
        (
            SOURCE_LIST,
            MethodContract {
                input_schema: "conex.v1.SourceListRequest",
                output_schema: "conex.v1.SourceListResponse",
                prepare: prepare_list,
                validate_output: validate_list_output,
            },
        ),
        (
            SOURCE_READ,
            MethodContract {
                input_schema: "conex.v1.SourceReadRequest",
                output_schema: "conex.v1.SourceReadResponse",
                prepare: prepare_read,
                validate_output: validate_read_output,
            },
        ),
        (
            SOURCE_SEARCH,
            MethodContract {
                input_schema: "conex.v1.SourceSearchRequest",
                output_schema: "conex.v1.SourceSearchResponse",
                prepare: prepare_search,
                validate_output: validate_search_output,
            },
        ),
    ]
}

fn bad(message: &str) -> CallError {
    CallError::new(v1::ErrorCode::BadRequest, message)
}

fn object(input: &Value) -> CallResult<&Map<String, Value>> {
    input
        .as_object()
        .ok_or_else(|| bad("input must be an object"))
}

fn known_keys(map: &Map<String, Value>, allowed: &[&str]) -> CallResult<()> {
    for key in map.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(bad("unknown input field"));
        }
    }
    Ok(())
}

fn required_string(map: &Map<String, Value>, key: &str) -> CallResult<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| bad(&format!("{key} must be a string")))
}

fn optional_limit(map: &Map<String, Value>) -> CallResult<u32> {
    match map.get("limit") {
        None => Ok(DEFAULT_PAGE),
        Some(Value::Number(number)) => {
            let value = number
                .as_u64()
                .ok_or_else(|| bad("limit must be a non-negative integer"))?;
            if value < 1 || value > MAX_PAGE as u64 {
                return Err(bad("limit is out of range"));
            }
            Ok(value as u32)
        }
        Some(_) => Err(bad("limit must be a number")),
    }
}

fn optional_cursor(map: &Map<String, Value>) -> CallResult<Option<String>> {
    match map.get("cursor") {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(bad("cursor must be a string")),
    }
}

fn prepare_list(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["root", "limit", "cursor"])?;
    let root = required_string(map, "root")?;
    let limit = optional_limit(map)?;
    let cursor = optional_cursor(map)?;
    let claim = subtree_claim(&root, "list")?;
    let canonical = json!({"root": root, "limit": limit, "cursor": cursor});
    Ok(PreparedInput { canonical, claim })
}

fn prepare_read(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["resourceId"])?;
    let resource = required_string(map, "resourceId")?;
    let claim = read_claim(&resource)?;
    let canonical = json!({"resourceId": resource});
    Ok(PreparedInput { canonical, claim })
}

fn prepare_search(input: &Value) -> CallResult<PreparedInput> {
    let map = object(input)?;
    known_keys(map, &["root", "query", "limit", "cursor"])?;
    let root = required_string(map, "root")?;
    let query = required_string(map, "query")?;
    let chars = query.chars().count();
    if chars == 0 || chars > MAX_QUERY_CHARS {
        return Err(bad("query must be 1..=256 characters"));
    }
    let limit = optional_limit(map)?;
    let cursor = optional_cursor(map)?;
    let claim = subtree_claim(&root, "search")?;
    let canonical = json!({"root": root, "query": query, "limit": limit, "cursor": cursor});
    Ok(PreparedInput { canonical, claim })
}

fn summary(value: &Value) -> CallResult<()> {
    let map = value
        .as_object()
        .ok_or_else(|| bad("resource summary must be an object"))?;
    if !map.get("resourceId").is_some_and(Value::is_string) {
        return Err(bad("resourceId must be a string"));
    }
    if !map.get("title").is_some_and(Value::is_string) {
        return Err(bad("title must be a string"));
    }
    if !map.get("mime").is_some_and(Value::is_string) {
        return Err(bad("mime must be a string"));
    }
    Ok(())
}

fn validate_list_output(value: &Value) -> CallResult<()> {
    let map = value
        .as_object()
        .ok_or_else(|| bad("output must be an object"))?;
    let items = map
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("items must be an array"))?;
    for item in items {
        summary(item)?;
    }
    match map.get("nextCursor") {
        None | Some(Value::String(_)) => Ok(()),
        Some(_) => Err(bad("nextCursor must be a string")),
    }
}

fn validate_read_output(value: &Value) -> CallResult<()> {
    let map = value
        .as_object()
        .ok_or_else(|| bad("output must be an object"))?;
    summary(
        map.get("resource")
            .ok_or_else(|| bad("resource is required"))?,
    )?;
    if !map.get("text").is_some_and(Value::is_string) {
        return Err(bad("text must be a string"));
    }
    if !map.get("cid").is_some_and(Value::is_string) {
        return Err(bad("cid must be a string"));
    }
    Ok(())
}

fn validate_search_output(value: &Value) -> CallResult<()> {
    let map = value
        .as_object()
        .ok_or_else(|| bad("output must be an object"))?;
    let items = map
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("items must be an array"))?;
    for item in items {
        let hit = item
            .as_object()
            .ok_or_else(|| bad("search hit must be an object"))?;
        summary(
            hit.get("resource")
                .ok_or_else(|| bad("resource is required"))?,
        )?;
        if !hit.get("excerpt").is_some_and(Value::is_string) {
            return Err(bad("excerpt must be a string"));
        }
    }
    Ok(())
}

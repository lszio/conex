//! Byte delivery for the share scene: the whole route family of `/content`'s
//! little sibling.
//!
//! One route serves three jobs — a preview, a media element, and a download —
//! and the only thing that distinguishes them is the byte range asked for.
//! That is deliberate: a preview is a range read, a seek is a range read, and
//! only a download asks for everything. The store reads a range by mapping it
//! onto the blocks it touches, so a preview of a large item costs one block
//! rather than the whole item.
//!
//! Range grammar is RFC 7233's, with one addition: `range` may travel in the
//! query as well as in the `Range` header, because the elements that render a
//! preview (`<img>`, `<object>`) cannot set a header and the browser does not
//! volunteer one for a fresh load. Both forms go through one parser, so there
//! is one implementation of the rule rather than two.
//!
//! The three answers a range can get are the RFC's, and they are not
//! interchangeable. A well-formed range that no byte satisfies is `416` with
//! `bytes */total`. A malformed or multi-range value is answered as the whole
//! item — that is what the RFC prescribes, and it is also what keeps a curious
//! header from being a probe: an upload that failed and an id that never
//! existed must not be distinguishable, and neither may be by watching which
//! requests error. `If-Range` is the third case: when it does not match the
//! item's own validator the range is ignored and the whole item is answered,
//! which is what stops a resumed media element from stitching together bytes
//! from two different contents.

#![forbid(unsafe_code)]

use std::sync::Arc;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Query, Request};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use super::{
    LINK_HEADER, ascii_name, bad_request, no_link, not_found, percent_encode, status_for,
    status_for_content, unavailable,
};
use crate::http::HttpState;
use crate::share::SharedFile;
use crate::share_blocks::RangeSlice;

/// State carried across polls of the download stream: which slices are left.
type Slices = std::vec::IntoIter<RangeSlice>;

/// The link rides in the query on a plain `<a href>` (which cannot set a
/// header) and in the header on a fetch; either way it is checked against the
/// session before it is used, so this is a transport detail and not a second
/// way in. `range` is the preview form, parsed with the same rules as `Range`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadQuery {
    id: String,
    #[serde(default)]
    link_id: String,
    /// Byte range carried in the query, for the elements that render a preview
    /// and cannot set a header.
    #[serde(default)]
    range: String,
}

pub async fn download_file(
    Extension(state): Extension<Arc<HttpState>>,
    request: Request,
) -> Response {
    let Some(web) = state.web_auth.as_ref() else {
        return unavailable("web auth is not configured");
    };
    let headers = request.headers();
    let session = match web.session_from_headers(headers) {
        Ok(session) => session,
        Err(error) => return status_for(error),
    };
    let query: DownloadQuery = match Query::<DownloadQuery>::try_from_uri(request.uri()) {
        Ok(Query(query)) => query,
        Err(_) => return bad_request("id is required"),
    };
    // The link comes from the query on a plain link and from the header on a
    // fetch; either way it is checked against the session before it is used.
    let link_id = if query.link_id.is_empty() {
        headers
            .get(LINK_HEADER)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
    } else {
        query.link_id.as_str()
    };
    if !web.owns_link(&session, link_id) {
        return no_link();
    }
    let Some(entry) = state.clients.get(link_id) else {
        return no_link();
    };
    // An item outside the caller's group is reported as unknown, not as
    // forbidden: a different answer would confirm the id exists.
    let now = crate::agent::now_ms();
    let Some(file) = state.shares.content(&query.id, &entry.group_key, now) else {
        return not_found();
    };
    // The scope is what was asked for, checked against the item's own
    // validator first: `If-Range` that does not match means the caller's idea
    // of the content is stale, so the range goes away rather than being
    // answered against a different item.
    let etag = etag_for(&file);
    let scope = range_scope(headers, &query.range, &etag, file.size);
    let Some((start, end, partial)) = scope else {
        // Nothing in the item can satisfy a well-formed range.
        return range_not_satisfiable(file.size);
    };
    let Some(blocks) = state.shares.storage() else {
        return unavailable("no share root is configured on this host");
    };
    let plan = match blocks.range_plan(&file.root_cid, start, end - start + 1) {
        Ok(plan) => plan,
        Err(error) => return status_for_content(error),
    };
    // The body streams block by block, so a download costs one block of memory
    // rather than the whole item, and dropping the response stops the reads.

    let stream = futures::stream::try_unfold(plan.slices.into_iter(), read_next_slice);
    let length = end - start + 1;
    let total_for_header = file.size;
    let mut response = Response::new(Body::from_stream(stream));
    if partial {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    }
    let headers_out = response.headers_mut();
    // A text/* answer without a charset is decoded by the browser using a
    // default encoding, which turns a UTF-8 CJK file into mojibake. The
    // declared charset has to be on the response, not assumed by the client.
    let content_type = if file.mime.starts_with("text/") || file.mime == "application/json" {
        format!("{}; charset=utf-8", file.mime)
    } else {
        file.mime.clone()
    };
    if let Ok(mime) = HeaderValue::from_str(&content_type) {
        headers_out.insert(header::CONTENT_TYPE, mime);
    }
    headers_out.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers_out.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers_out.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers_out.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).expect("ascii length"),
    );
    if let Ok(value) = HeaderValue::from_str(&etag) {
        headers_out.insert(header::ETAG, value);
    }
    if partial
        && let Ok(value) = HeaderValue::from_str(&format!("bytes {start}-{end}/{total_for_header}"))
    {
        headers_out.insert(header::CONTENT_RANGE, value);
    }
    let disposition = if file.inline() {
        "inline"
    } else {
        "attachment"
    };
    // The name may carry a directory path (`dir/sub/file.png`); the browser
    // saving through this header should get the bare file name.
    let base = file.name.rsplit('/').next().unwrap_or(&file.name);
    if let Ok(value) = HeaderValue::from_str(&format!(
        "{disposition}; filename=\"{}\"; filename*=UTF-8''{}",
        ascii_name(base),
        percent_encode(base)
    )) {
        headers_out.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// The item's own validator.
///
/// The root CID is the natural one: it names the content, so two items with the
/// same bytes share it, and it never changes for the same bytes. That is what
/// makes it usable both as an `ETag` and as the `If-Range` precondition — a
/// caller holding either is holding a statement about content, not about a
/// transport.
/// One poll of the download stream: read the next block, or finish.
///
/// Written as a named function rather than a closure because the closure's
/// error type would otherwise have to be inferred from a `?` in one arm of a
/// match, and a named return type states it once.
async fn read_next_slice(mut slices: Slices) -> Result<Option<(Vec<u8>, Slices)>, std::io::Error> {
    let Some(slice) = slices.next() else {
        return Ok(None);
    };
    let read = read_slice(slice).await?;
    Ok(Some((read, slices)))
}

/// Read one block's part of a range.
///
/// The block is read whole and then trimmed: a block is one chunk, so trimming
/// is never reading more than the range already pays for, and a stream of whole
/// blocks is what a download wants. Dropping the stream stops the reads, which
/// is what a browser that navigates away mid-download relies on.
async fn read_slice(slice: RangeSlice) -> Result<Vec<u8>, std::io::Error> {
    let block = tokio::fs::read(&slice.path).await.map_err(|error| {
        std::io::Error::other(format!("read {}: {error}", slice.path.display()))
    })?;
    let stop = slice.skip.saturating_add(slice.take).min(block.len());
    block
        .get(slice.skip..stop)
        .map(|piece| piece.to_vec())
        .ok_or_else(|| {
            std::io::Error::other(format!(
                "block {} is shorter than the range it covers",
                slice.path.display()
            ))
        })
}

fn etag_for(file: &SharedFile) -> String {
    format!("\"{}\"", file.root_cid)
}

/// One of three answers to a range request.
enum Scope {
    /// A range the item can answer part of.
    Partial { start: u64, end: u64 },
    /// No range, or one the caller does not get to keep.
    Whole,
    /// A well-formed range that no byte can satisfy.
    Unsatisfiable,
}

fn range_scope(
    headers: &axum::http::HeaderMap,
    query_range: &str,
    etag: &str,
    total: u64,
) -> Option<(u64, u64, bool)> {
    match scope_of(headers, query_range, etag, total) {
        Scope::Partial { start, end } => Some((start, end, start != 0 || end + 1 != total)),
        Scope::Whole => Some((0, total.saturating_sub(1), false)),
        Scope::Unsatisfiable => None,
    }
}

fn scope_of(headers: &axum::http::HeaderMap, query_range: &str, etag: &str, total: u64) -> Scope {
    if total == 0 {
        return Scope::Unsatisfiable;
    }
    let spec = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            let trimmed = query_range.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        });
    let Some(spec) = spec else {
        return Scope::Whole;
    };
    // `If-Range` that does not name this exact content turns the range into a
    // request for the whole representation.
    if let Some(condition) = headers
        .get(header::IF_RANGE)
        .and_then(|value| value.to_str().ok())
        && condition != etag
    {
        return Scope::Whole;
    }
    let spec = match spec.trim().strip_prefix("bytes=") {
        Some(spec) => spec,
        None => return Scope::Whole,
    };
    // Multi-range is answered as a whole item, as the RFC prescribes.
    if spec.contains(',') {
        return Scope::Whole;
    }
    let (start, end) = match spec.split_once('-') {
        Some(pair) => pair,
        None => return Scope::Whole,
    };
    if start.is_empty() {
        // Suffix range: the last n bytes.
        let suffix: u64 = match end.parse() {
            Ok(suffix) => suffix,
            Err(_) => return Scope::Whole,
        };
        if suffix == 0 {
            return Scope::Unsatisfiable;
        }
        let start = total.saturating_sub(suffix);
        return Scope::Partial {
            start,
            end: total - 1,
        };
    }
    let start: u64 = match start.parse() {
        Ok(start) => start,
        Err(_) => return Scope::Whole,
    };
    // Well-formed but past the end: nothing to answer, and saying so is not a
    // probe — the id was already known to the caller.
    if start >= total {
        return Scope::Unsatisfiable;
    }
    let end = if end.is_empty() {
        total - 1
    } else {
        match end.parse::<u64>() {
            Ok(end) => end.min(total - 1),
            Err(_) => return Scope::Whole,
        }
    };
    Scope::Partial {
        start,
        end: end.max(start),
    }
}

/// `416` with the total, so a client can tell how far past the end it asked.
fn range_not_satisfiable(total: u64) -> Response {
    let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
    let headers = response.headers_mut();
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{total}")) {
        headers.insert(header::CONTENT_RANGE, value);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

pub(super) fn header_text(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

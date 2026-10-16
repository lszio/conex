//! Same-origin file and clipboard sharing for the landing page.
//!
//! | Route                                    | Method | Purpose                          |
//! |------------------------------------------|--------|----------------------------------|
//! | `/web/files`                             | GET    | items offered by the caller's group (metadata only) |
//! | `/web/files`                             | POST   | offer one small file in one request |
//! | `/web/files/download`                    | GET    | read bytes, optionally one range |
//! | `/web/files/remove`                      | POST   | withdraw an item you own |
//! | `/web/files/uploads`                     | POST   | begin a chunked upload |
//! | `/web/files/uploads/{id}/chunks/{index}` | PUT    | send one chunk |
//! | `/web/files/uploads/{id}`                | HEAD   | how far an upload got |
//! | `/web/files/uploads/{id}/commit`         | POST   | finish an upload |
//! | `/web/files/uploads/{id}/cancel`         | POST   | abandon an upload |
//!
//! Three rules hold on every route.
//!
//! Reads are safe methods, so they need only the session cookie, exactly like
//! `/web/session`. Writes additionally require the configured Origin and the
//! CSRF nonce, so a third-party page cannot make a visitor's browser offer
//! files.
//!
//! Every path resolves the caller's group from the registry first: an item id
//! is not an access token, and a caller in another group gets a 404 rather than
//! a distinguishable error. The same holds for an upload id, which is bound to
//! the owner's principal rather than to the session.
//!
//! Body limits are per route rather than global, because the rest of the host
//! keeps a 1 MiB JSON limit and a file must not loosen what every other
//! endpoint inherits. A chunk route's limit is one chunk; the whole-body route
//! is the convenience path for a small file.

#![forbid(unsafe_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use conex_core::CallError;

use crate::http::HttpState;
use crate::share::SharedFile;

pub(crate) mod downloads;
pub(crate) mod uploads;

use self::downloads::header_text;

/// Body limit for the whole-body upload route. It matches the scene's per-item
/// cap so an oversized upload is refused by the transport instead of being read
/// and then rejected.
pub const UPLOAD_BODY_LIMIT: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveRequest {
    id: String,
}

/// Header the tab presents its own link on. The cookie alone cannot say which
/// tab is asking — two tabs share one — so every share route resolves the
/// caller through this, and the session checks that the tab owns it.
pub(crate) const LINK_HEADER: &str = "x-conex-link-id";

/// The caller's own client row.
///
/// The WSS loop is what registers it, so a visitor whose socket is not up has
/// no row, no group, and therefore no access to anything anyone shared.
/// Returning `None` rather than an empty list is deliberate: an empty group
/// would look like "nobody has shared anything" instead of "you are not
/// connected yet".
///
/// `None` also covers a link this session does not own: a tab cannot borrow a
/// sibling tab's identity by putting its link on the request.
pub(crate) fn own_entry(
    state: &Arc<HttpState>,
    session: &crate::web_auth::WebSession,
    headers: &axum::http::HeaderMap,
) -> Option<crate::clients::ClientEntry> {
    let web = state.web_auth.as_ref()?;
    let link_id = headers.get(LINK_HEADER)?.to_str().ok()?;
    if !web.owns_link(session, link_id) {
        return None;
    }
    state.clients.get(link_id)
}

pub async fn list_files(Extension(state): Extension<Arc<HttpState>>, request: Request) -> Response {
    let Some(web) = state.web_auth.as_ref() else {
        return unavailable("web auth is not configured");
    };
    let headers = request.headers();
    let session = match web.session_from_headers(headers) {
        Ok(session) => session,
        Err(error) => return status_for(error),
    };
    let Some(entry) = own_entry(&state, &session, headers) else {
        return no_link();
    };
    // Reading the listing is what expires an aged-out clip, so `now` is the
    // request's own clock rather than a stored value.
    let now = crate::agent::now_ms();
    let limits = state.shares.limits();
    // An unchanged listing is answered with a `304` and no body, which is what
    // keeps the cheap poll cheap: the page falls back to polling only when a
    // push was missed, so the steady state is a notification or a 304.
    if let Some(etag) = state.shares.listing_etag(&entry.group_key, now)
        && headers
            .get(header::IF_NONE_MATCH)
            .and_then(|value| value.to_str().ok())
            == Some(etag.as_str())
    {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        if let Ok(value) = HeaderValue::from_str(&etag) {
            response.headers_mut().insert(header::ETAG, value);
        }
        return response;
    }
    let files = state
        .shares
        .list(&entry.group_key, now)
        .into_iter()
        .map(file_to_json)
        .collect::<Vec<_>>();
    let mut response = Json(json!({
        "groupKey": entry.group_key,
        "label": entry.profile.group,
        "selfLinkId": entry.link_id,
        "files": files,
        "limits": {
            "maxFileBytes": limits.max_file_bytes.to_string(),
            "maxFilesPerClient": limits.max_files_per_client.to_string(),
            "maxClipsPerClient": limits.max_clips_per_client.to_string(),
            "clipTtlMs": limits.clip_ttl_ms.to_string(),
        },
    }))
    .into_response();
    if let Some(etag) = state.shares.listing_etag(&entry.group_key, now)
        && let Ok(value) = HeaderValue::from_str(&etag)
    {
        response.headers_mut().insert(header::ETAG, value);
    }
    response
}

/// Tell a group's other tabs that the scene moved.
///
/// Only a *write* pushes, and only to the group that can see it: a scene whose
/// listing changes only on a write should not need polling to be observed. The
/// frame carries no bytes and no group text — the recipient refreshes through
/// the ordinary listing route, where its own group is resolved. A dropped frame
/// costs freshness, never correctness, because the page still polls.
pub(crate) fn notify_group_changed(state: &Arc<HttpState>, entry: &crate::clients::ClientEntry) {
    let clients = state.clients.clone();
    let delivered = clients.notify_group(
        &entry.group_key,
        &entry.link_id,
        "conex/share-changed",
        &json!({ "by": entry.link_id }),
    );
    // A push nobody received is a silent regression, so it is visible where a
    // long-running host can be looked at.
    if delivered == 0 {
        tracing::debug!(
            group = %entry.group_key,
            "share change reached no peer: every tab of this group is the actor"
        );
    }
}

pub async fn upload_file(
    Extension(state): Extension<Arc<HttpState>>,
    request: Request,
) -> Response {
    let Some(web) = state.web_auth.as_ref() else {
        return unavailable("web auth is not configured");
    };
    let headers = request.headers().clone();
    // Offering a file is a mutation of shared state, so it carries the same
    // Origin + CSRF requirement as logout.
    let session = match web.check_mutation(&headers) {
        Ok(session) => session,
        Err(error) => return status_for(error),
    };
    let Some(entry) = own_entry(&state, &session, &headers) else {
        return no_link();
    };
    let max_bytes = state.shares.limits().max_file_bytes;
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<u64>().ok());
    if declared.is_some_and(|length| length > max_bytes) {
        return status_for(too_large(declared.unwrap_or(0), max_bytes));
    }
    let (parts, body) = request.into_parts();
    let bytes = match read_bounded(body, max_bytes).await {
        Ok(bytes) => bytes,
        Err(response) => return *response,
    };
    if bytes.is_empty() {
        return bad_request("the uploaded file is empty");
    }
    let name =
        header_text(&parts.headers, "x-conex-file-name").unwrap_or_else(|| "file".to_string());
    let mime = header_text(&parts.headers, "x-conex-file-type").unwrap_or_default();
    let name = percent_decode(&name).unwrap_or(name);
    let mime = percent_decode(&mime).unwrap_or(mime);
    // The bytes go to the scene's store *before* the item is admitted, so a
    // refused quota leaves them staged and released, not readable.
    let Some(blocks) = state.shares.storage() else {
        return unavailable("no share root is configured on this host");
    };
    let owner = entry.owner();
    let stored = blocks.ingest(&owner, &bytes);
    let stored = match stored {
        Ok(stored) => stored,
        Err(error) => return status_for(crate::broker::content_to_call(error)),
    };
    let offered = crate::share::Offer {
        owner_link_id: &entry.link_id,
        owner_name: &entry.profile.label(&entry.link_id),
        group_key: &entry.group_key,
        name: &name,
        mime: &mime,
        kind: crate::share::ShareItemKind::File,
        size_bytes: stored.size_bytes,
        now_ms: crate::agent::now_ms(),
    };
    let file = match state.shares.admit(&stored.root_cid, &offered) {
        Ok(file) => file,
        Err(error) => {
            // The item was refused, so nothing serves these bytes any more.
            state.shares.release(&stored.root_cid);
            return status_for(error);
        }
    };
    refresh_shared_counters(&state, &entry);
    notify_group_changed(&state, &entry);
    Json(json!({ "file": file_to_json(file) })).into_response()
}

pub async fn remove_file(
    Extension(state): Extension<Arc<HttpState>>,
    request: Request,
) -> Response {
    let Some(web) = state.web_auth.as_ref() else {
        return unavailable("web auth is not configured");
    };
    let headers = request.headers().clone();
    let session = match web.check_mutation(&headers) {
        Ok(session) => session,
        Err(error) => return status_for(error),
    };
    let Some(entry) = own_entry(&state, &session, &headers) else {
        return no_link();
    };
    let body = match axum::body::to_bytes(request.into_body(), 4 * 1024).await {
        Ok(body) => body,
        Err(_) => return bad_request("body is not valid JSON"),
    };
    let parsed: RemoveRequest = match serde_json::from_slice(&body) {
        Ok(parsed) => parsed,
        Err(_) => return bad_request("id is required"),
    };
    if let Err(error) = state.shares.remove(&parsed.id, &entry.link_id) {
        return match error.code() {
            code if code == conex_proto::ErrorCode::Forbidden as i32 => status_for(error),
            _ => not_found(),
        };
    }
    refresh_shared_counters(&state, &entry);
    notify_group_changed(&state, &entry);
    StatusCode::NO_CONTENT.into_response()
}

/// Recompute the owner's shared counters after a write, so `client/status`
/// reports the group total without walking the share store itself.
pub(crate) fn refresh_shared_counters(state: &Arc<HttpState>, entry: &crate::clients::ClientEntry) {
    let (files, bytes) = state.shares.owner_totals(&entry.link_id);
    state.clients.set_shared(&entry.link_id, files, bytes);
}

pub(crate) fn file_to_json(file: SharedFile) -> serde_json::Value {
    json!({
        "id": file.id,
        "kind": file.kind.as_str(),
        "name": file.name,
        "mime": file.mime,
        "size": file.size.to_string(),
        "ownerLinkId": file.owner_link_id,
        "ownerName": file.owner_name,
        "createdAtMs": file.created_at_ms.to_string(),
        "expiresAtMs": file.expires_at_ms.to_string(),
        "inline": file.inline(),
        "previewBytes": file.preview_bytes().to_string(),
    })
}

/// Read a request body, refusing anything over the per-file cap even when the
/// request lied about (or omitted) its `Content-Length`.
///
/// The error is boxed: a `Response` is a large value and this returns once per
/// request, so carrying it by value in the `Err` arm costs more than the
/// allocation it saves.
async fn read_bounded(body: Body, limit: u64) -> Result<Vec<u8>, Box<Response>> {
    use futures::StreamExt;
    let mut stream = body.into_data_stream();
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(next) = stream.next().await {
        let chunk = next.map_err(|_| Box::new(bad_request("upload was interrupted")))?;
        if bytes.len() as u64 + chunk.len() as u64 > limit {
            return Err(Box::new(status_for(too_large(
                bytes.len() as u64 + chunk.len() as u64,
                limit,
            ))));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// Resolve the requested scope against the item's size.
///
/// `Range` and the query form are the same grammar. A suffix range (`-n`), an
/// open range (`n-`) and a full range are accepted; multi-range and anything
/// malformed returns `None`, which callers answer as the whole item.
/// decoded here. A malformed encoding falls back to the raw value, which the
/// store's sanitizer then handles.
fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8(out).ok()
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The ASCII form inside `filename="…"`. It is already sanitized upstream, but
/// a quote would break out of the parameter, so only a known-safe alphabet is
/// allowed through.
fn ascii_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .take(120)
        .collect();
    if safe.trim().is_empty() {
        "file".to_string()
    } else {
        safe.trim().to_string()
    }
}

pub(crate) fn too_large(actual: u64, limit: u64) -> CallError {
    CallError::new(
        conex_proto::ErrorCode::QuotaExceeded,
        format!("item is too large: {actual} bytes, limit {limit}"),
    )
}

pub(crate) fn no_link() -> Response {
    (
        StatusCode::CONFLICT,
        [(header::CONTENT_TYPE, "application/json")],
        "{\"code\":-32005,\"message\":\"connect first: the file scene needs a live link\"}",
    )
        .into_response()
}

pub(crate) fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        "{\"code\":-32602,\"message\":\"unknown file\"}",
    )
        .into_response()
}

pub(crate) fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "code": -32602, "message": message }).to_string(),
    )
        .into_response()
}

/// Map a content-store error onto the share routes' own answers.
///
/// An upload that is gone — committed, cancelled or expired — answers the same
/// 404 the download route gives an item it cannot find. That is the same rule
/// as for item ids: a caller must not be able to tell "never existed" from
/// "expired" from "someone else's", because a distinguishable answer is a
/// probe.
pub(crate) fn status_for_content(error: conex_content::ContentError) -> Response {
    use conex_proto::ErrorCode;
    let code = error.bad_request_code();
    if code == ErrorCode::UnknownProvider as i32 {
        return not_found();
    }
    status_for(crate::broker::content_to_call(error))
}

fn unavailable(message: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "code": -32005, "message": message }).to_string(),
    )
        .into_response()
}

pub(crate) fn status_for(error: CallError) -> Response {
    let status = match error.code() {
        code if code == conex_proto::ErrorCode::Unauthorized as i32 => StatusCode::UNAUTHORIZED,
        code if code == conex_proto::ErrorCode::Forbidden as i32 => StatusCode::FORBIDDEN,
        code if code == conex_proto::ErrorCode::QuotaExceeded as i32 => {
            StatusCode::PAYLOAD_TOO_LARGE
        }
        code if code == conex_proto::ErrorCode::Unavailable as i32 => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        _ => StatusCode::BAD_REQUEST,
    };
    (
        status,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "code": error.code(), "message": error.message() }).to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_round_trip_keeps_unicode_names() {
        let name = "季度 报告.pdf";
        assert_eq!(percent_decode(&percent_encode(name)).as_deref(), Some(name));
    }

    #[test]
    fn a_malformed_encoding_falls_back_instead_of_failing() {
        assert_eq!(percent_decode("bad%zz").as_deref(), Some("bad%zz"));
        assert_eq!(percent_decode("trailing%2").as_deref(), Some("trailing%2"));
    }

    #[test]
    fn the_ascii_fallback_cannot_break_out_of_the_header_parameter() {
        assert_eq!(ascii_name("a\"b\r\nc.txt"), "abc.txt");
        assert_eq!(ascii_name("   "), "file");
    }
}

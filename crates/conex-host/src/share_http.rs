//! Same-origin file sharing for the landing page's file scene.
//!
//! Four routes, all behind the ordinary web session cookie:
//!
//! | Route                    | Method | Purpose                                     |
//! |--------------------------|--------|---------------------------------------------|
//! | `/web/files`             | GET    | files offered by this caller's group        |
//! | `/web/files`             | POST   | offer one local file to the group           |
//! | `/web/files/download`    | GET    | read one file's bytes                       |
//! | `/web/files/remove`      | POST   | withdraw a file you own                     |
//!
//! Two rules hold on every route. Reads are safe methods, so they need only
//! the cookie, exactly like `/web/session`; writes additionally require the
//! configured Origin and the CSRF nonce, so a third-party page cannot make a
//! visitor's browser offer files. And every path resolves the caller's group
//! from the registry first: the file id in the URL is not an access token, and
//! a caller in another group gets a 404 rather than a distinguishable error.
//!
//! The body limit is applied per route rather than globally because the rest
//! of the host keeps a 1 MiB JSON limit; an 8 MiB file must not loosen the
//! limit every other endpoint inherits.

#![forbid(unsafe_code)]

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Query, Request};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde::Deserialize;
use serde_json::json;

use conex_core::CallError;

use crate::http::HttpState;
use crate::share::{MAX_FILE_BYTES, SharedFile};

/// Body limit for the upload route. It matches the store's per-file cap so an
/// oversized upload is refused by the transport instead of being read and then
/// rejected.
pub const UPLOAD_BODY_LIMIT: usize = MAX_FILE_BYTES as usize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadQuery {
    id: String,
    /// The tab's link, for a plain `<a href>` download that cannot set a
    /// header. It is verified against the session exactly like the header is,
    /// so this is a transport detail and not a second way in.
    #[serde(default)]
    link_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoveRequest {
    id: String,
}

/// Header the tab presents its own link on. The cookie alone cannot say which
/// tab is asking — two tabs share one — so every share route resolves the
/// caller through this, and the session checks that the tab owns it.
pub const LINK_HEADER: &str = "x-conex-link-id";

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
fn own_entry(
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
    let files = state
        .shares
        .list(&entry.group_key)
        .into_iter()
        .map(file_to_json)
        .collect::<Vec<_>>();
    Json(json!({
        "groupKey": entry.group_key,
        "label": entry.profile.group,
        "selfLinkId": entry.link_id,
        "files": files,
        "limits": {
            "maxFileBytes": MAX_FILE_BYTES.to_string(),
            "maxFilesPerClient": crate::share::MAX_FILES_PER_CLIENT.to_string(),
        },
    }))
    .into_response()
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
    let declared = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.parse::<u64>().ok());
    if declared.is_some_and(|length| length > MAX_FILE_BYTES) {
        return status_for(too_large(declared.unwrap_or(0)));
    }
    let (parts, body) = request.into_parts();
    let bytes = match read_bounded(body).await {
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
    let stored = state.shares.put(
        &entry.link_id,
        &entry.profile.label(&entry.link_id),
        &entry.group_key,
        &name,
        &mime,
        Arc::new(bytes),
    );
    let file = match stored {
        Ok(file) => file,
        Err(error) => return status_for(error),
    };
    refresh_shared_counters(&state, &entry);
    Json(json!({ "file": file_to_json(file) })).into_response()
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
    // A file outside the caller's group is reported as unknown, not as
    // forbidden: a different answer would confirm the id exists.
    let Some((file, bytes)) = state.shares.content(&query.id, &entry.group_key) else {
        return not_found();
    };
    let length = bytes.len();
    let mut response = Response::new(Body::from(bytes.to_vec()));
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
    headers_out.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).expect("ascii length"),
    );
    let disposition = if file.inline() {
        "inline"
    } else {
        "attachment"
    };
    if let Ok(value) = HeaderValue::from_str(&format!(
        "{disposition}; filename=\"{}\"; filename*=UTF-8''{}",
        ascii_name(&file.name),
        percent_encode(&file.name)
    )) {
        headers_out.insert(header::CONTENT_DISPOSITION, value);
    }
    response
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
    StatusCode::NO_CONTENT.into_response()
}

/// Recompute the owner's shared counters after a write, so `client/status`
/// reports the group total without walking the share store itself.
fn refresh_shared_counters(state: &Arc<HttpState>, entry: &crate::clients::ClientEntry) {
    let files: Vec<SharedFile> = state
        .shares
        .list(&entry.group_key)
        .into_iter()
        .filter(|file| file.owner_link_id == entry.link_id)
        .collect();
    let bytes: u64 = files.iter().map(|file| file.size).sum();
    state
        .clients
        .set_shared(&entry.link_id, files.len() as u32, bytes);
}

fn file_to_json(file: SharedFile) -> serde_json::Value {
    json!({
        "id": file.id,
        "name": file.name,
        "mime": file.mime,
        "size": file.size.to_string(),
        "ownerLinkId": file.owner_link_id,
        "ownerName": file.owner_name,
        "createdAtMs": file.created_at_ms.to_string(),
        "inline": file.inline(),
    })
}

/// Read a request body, refusing anything over the per-file cap even when the
/// request lied about (or omitted) its `Content-Length`.
///
/// The error is boxed: a `Response` is a large value and this returns once per
/// request, so carrying it by value in the `Err` arm costs more than the
/// allocation it saves.
async fn read_bounded(body: Body) -> Result<Vec<u8>, Box<Response>> {
    use futures::StreamExt;
    let mut stream = body.into_data_stream();
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(next) = stream.next().await {
        let chunk = next.map_err(|_| Box::new(bad_request("upload was interrupted")))?;
        if bytes.len() as u64 + chunk.len() as u64 > MAX_FILE_BYTES {
            return Err(Box::new(status_for(too_large(
                bytes.len() as u64 + chunk.len() as u64,
            ))));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn header_text(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

/// Header values are ASCII-only, so a file name travels percent-encoded and is
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

fn too_large(actual: u64) -> CallError {
    CallError::new(
        conex_proto::ErrorCode::QuotaExceeded,
        format!("file is too large: {actual} bytes, limit {MAX_FILE_BYTES}"),
    )
}

fn no_link() -> Response {
    (
        StatusCode::CONFLICT,
        [(header::CONTENT_TYPE, "application/json")],
        "{\"code\":-32005,\"message\":\"connect first: the file scene needs a live link\"}",
    )
        .into_response()
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        "{\"code\":-32602,\"message\":\"unknown file\"}",
    )
        .into_response()
}

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "code": -32602, "message": message }).to_string(),
    )
        .into_response()
}

fn unavailable(message: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "application/json")],
        json!({ "code": -32005, "message": message }).to_string(),
    )
        .into_response()
}

fn status_for(error: CallError) -> Response {
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

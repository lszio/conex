//! Same-origin browser content delivery (plan M3.2).
//!
//! `GET|HEAD /content?endpointId=&resourceId=[&revision=]` maps a browser
//! request onto the authorized source/read path and streams raw bytes. The
//! URL never becomes a filesystem path: resource location comes from the
//! endpoint's own root plus the Host resource policy, and every range slice
//! is fetched through the broker's remote byte path.
//!
//! Guarantees: single-range `Range` support (200/206/416, suffix and open
//! ranges), `ETag` bound to the resource revision, `If-Range` downgrade,
//! multipart/multi-range requests answered as a plain 200, MIME + disposition
//! header injection impossible, `nosniff` always, private no-store caching,
//! and response streaming that cancels upstream the moment the browser stops
//! reading.
#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::body::Body;
use axum::extract::{Query, Request};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use serde::Deserialize;

use conex_core::Caller;
use conex_proto;

use crate::broker::Broker;
use crate::http::HttpState;

const SLICE_BYTES: usize = conex_proto::cid::CHUNK_SIZE;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentQuery {
    endpoint_id: String,
    resource_id: String,
    #[serde(default)]
    revision: Option<String>,
}

/// `Content-Type` value plus whether the browser may render it inline.
struct Delivery {
    mime: String,
    inline: bool,
}

pub async fn content(Extension(state): Extension<Arc<HttpState>>, request: Request) -> Response {
    let Some(web) = state.web_auth.as_ref() else {
        return unavailable_response("web auth is not configured");
    };
    let Some(broker) = state.broker.clone() else {
        return unavailable_response("content delivery is not wired on this host");
    };
    let headers = request.headers().clone();
    // GET is a safe method: cookie only, no Origin/CSRF requirement, same
    // rule the landing page session endpoint uses.
    let session = match web.session_from_headers(&headers) {
        Ok(session) => session,
        Err(error) => return status_for(error),
    };
    let query: ContentQuery = match Query::<ContentQuery>::try_from_uri(request.uri()) {
        Ok(Query(query)) => query,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, "application/json")],
                "{\"code\":-32602,\"message\":\"endpointId and resourceId are required\"}",
            )
                .into_response();
        }
    };
    if query.endpoint_id.is_empty() || query.resource_id.is_empty() {
        return bad_request("endpointId and resourceId are required");
    }

    // Authorization + metadata through the ordinary source/read path.
    let probe = state
        .host
        .invoke(
            &session.caller,
            &query.endpoint_id,
            "source/read",
            serde_json::json!({ "resourceId": query.resource_id }),
            Duration::from_secs(15),
        )
        .await;
    let probe = match probe {
        Ok(value) => value,
        Err(error) => return status_for(error),
    };
    let delivery = match delivery_of(&probe) {
        Some(delivery) => delivery,
        None => return bad_request("resource has no content representation"),
    };
    let revision = probe
        .get("resource")
        .and_then(|resource| resource.get("revision"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            probe
                .get("content")
                .and_then(|content| content.get("revision"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .or(query.revision);
    let total: u64 = match probe
        .get("resource")
        .and_then(|resource| resource.get("sizeBytes"))
        .and_then(serde_json::Value::as_str)
        .and_then(|raw| raw.parse().ok())
        .or_else(|| {
            probe
                .get("content")
                .and_then(|content| content.get("sizeBytes"))
                .and_then(serde_json::Value::as_str)
                .and_then(|raw| raw.parse().ok())
        }) {
        Some(total) => total,
        None => return bad_request("resource size is unavailable"),
    };

    let etag = revision
        .as_deref()
        .map(|revision| format!("\"rev-{revision}\""));
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let if_range = headers
        .get(header::IF_RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    // If-Range mismatch: ignore Range and answer the full representation.
    let range = match (if_range, etag.as_deref()) {
        (Some(cond), Some(tag)) if cond != tag => None,
        (Some(cond), None) if !cond.starts_with("W/") && !cond.starts_with('"') => None,
        _ => range,
    };
    let (start, end) = match range.as_deref() {
        None => (0, total.saturating_sub(1)),
        Some(value) => match parse_single_range(value, total) {
            Some(bounds) => bounds,
            // Multi-range or malformed: RFC says ignore Range → 200 full.
            None => (0, total.saturating_sub(1)),
        },
    };
    if start > end || start >= total {
        return range_not_satisfiable(total);
    }
    let length = end - start + 1;
    let partial = range.is_some() && (start != 0 || end + 1 != total);

    let caller = session.caller.clone();
    let endpoint_id = query.endpoint_id.clone();
    let resource_id = query.resource_id.clone();
    let stream_revision = revision.clone();
    // Unfold state carries every owned value so the FnMut closure only ever
    // borrows: broker, caller, endpoint, resource and revision move through
    // state.
    let state0 = (
        start,
        end,
        broker,
        caller,
        endpoint_id,
        resource_id,
        stream_revision,
    );
    let stream = futures::stream::try_unfold(
        state0,
        move |(cursor, end, broker, caller, endpoint_id, resource_id, stream_revision): (
            u64,
            u64,
            Arc<Broker>,
            Caller,
            String,
            String,
            Option<String>,
        )| async move {
            if cursor > end {
                return Ok(None);
            }
            let slice_len = SLICE_BYTES.min((end - cursor + 1) as usize);
            let chunk = broker
                .content_range_bytes(
                    &caller,
                    &endpoint_id,
                    &resource_id,
                    stream_revision.as_deref(),
                    cursor,
                    slice_len,
                    Duration::from_secs(30),
                )
                .await
                // Browser aborted the body: the stream (and with it this
                // upstream read) is dropped, nothing replays.
                .map_err(|error| std::io::Error::other(error.message()))?;
            if let Some(expected) = stream_revision.as_deref()
                && chunk.revision != expected
            {
                return Err(std::io::Error::other(
                    "resource revision moved while streaming",
                ));
            }
            let bytes: Vec<u8> = chunk.chunk.to_vec();
            let next = cursor + bytes.len() as u64;
            if bytes.is_empty() {
                return Ok(None);
            }
            Ok(Some((
                futures::TryStreamExt::into_stream(futures::stream::once(futures::future::ready(
                    Ok::<Vec<u8>, std::io::Error>(bytes),
                ))),
                (
                    next,
                    end,
                    broker,
                    caller,
                    endpoint_id,
                    resource_id,
                    stream_revision,
                ),
            )))
        },
    )
    .try_flatten();

    let mut response = Response::new(Body::from_stream(stream));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        content_type_header(&delivery.mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    // Public and private answers share one policy: never let a shared cache
    // keep serving content after a policy withdrawal (plan M3.2).
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    let disposition = disposition_for(&delivery, &query.resource_id);
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition)
            .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
    );
    if let Some(etag) = etag
        && let Ok(value) = HeaderValue::from_str(&etag)
    {
        headers.insert(header::ETAG, value);
    }
    if partial {
        headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))
                .expect("ascii content range"),
        );
    }
    *response.status_mut() = if partial {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    // The body streams, but the slice length is known up front. Declaring it
    // lets a media element treat the response as seekable: without a total
    // length, `<video>`/`<audio>` clamp `currentTime` to 0 even though
    // `accept-ranges: bytes` is advertised. The stream still cancels
    // upstream when the browser stops reading.
    if let Ok(value) = HeaderValue::from_str(&length.to_string()) {
        response.headers_mut().insert(header::CONTENT_LENGTH, value);
    }
    response
}

/// RFC 7233 single-range parsing: `a-b`, `a-`, `-n`. Multi-range
/// (`a-b,c-d`) and malformed values return None so the caller answers 200.
fn parse_single_range(value: &str, total: u64) -> Option<(u64, u64)> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        if suffix == 0 || total == 0 {
            return None;
        }
        let start = total.saturating_sub(suffix);
        return Some((start, total - 1));
    }
    let start: u64 = start.parse().ok()?;
    if start >= total {
        return Some((start, start));
    }
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<u64>().ok()?.min(total - 1)
    };
    Some((start, end.max(start)))
}

fn delivery_of(probe: &serde_json::Value) -> Option<Delivery> {
    let mime = probe
        .get("resource")
        .and_then(|resource| resource.get("mime"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            probe
                .get("content")
                .and_then(|content| content.get("mime"))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_string();
    let inline = matches!(
        mime.as_str(),
        "text/plain"
            | "text/markdown"
            | "text/org"
            | "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/bmp"
            | "image/avif"
            | "video/mp4"
            | "video/webm"
            | "video/ogg"
            | "audio/mpeg"
            | "audio/ogg"
            | "audio/wav"
    );
    Some(Delivery { mime, inline })
}

fn content_type_header(mime: &str) -> Result<HeaderValue, InvalidMime> {
    // Header values cannot contain CR/LF or NUL; a mime carrying them is
    // refused instead of escaped.
    if mime.is_empty()
        || mime.len() > 128
        || mime
            .bytes()
            .any(|byte| byte == b'\r' || byte == b'\n' || byte == 0 || byte < 0x20)
    {
        return Err(InvalidMime);
    }
    let with_charset = if mime.starts_with("text/") {
        format!("{mime}; charset=utf-8")
    } else {
        mime.to_string()
    };
    HeaderValue::from_str(&with_charset).map_err(|_| InvalidMime)
}

#[derive(Debug)]
struct InvalidMime;

fn disposition_for(delivery: &Delivery, resource_id: &str) -> String {
    let filename = resource_id.rsplit('/').next().unwrap_or("download");
    // Only the sanitized basename survives; quotes/CR/LF cannot be injected.
    let safe: String = filename
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
        .take(120)
        .collect();
    let safe = if safe.trim().is_empty() {
        "download"
    } else {
        safe.trim()
    };
    let kind = if delivery.inline {
        "inline"
    } else {
        "attachment"
    };
    format!(
        "{kind}; filename=\"{safe}\"; filename*=UTF-8''{}",
        percent_encode(safe)
    )
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

fn bad_request(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json")],
        format!("{{\"code\":-32602,\"message\":{:?}}}", message),
    )
        .into_response()
}

fn range_not_satisfiable(total: u64) -> Response {
    let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{total}")) {
        response.headers_mut().insert(header::CONTENT_RANGE, value);
    }
    response
}

fn unavailable_response(message: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        [(header::CONTENT_TYPE, "application/json")],
        format!("{{\"code\":-32005,\"message\":{:?}}}", message),
    )
        .into_response()
}

fn status_for(error: conex_core::CallError) -> Response {
    let status = match error.code() {
        code if code == conex_proto::ErrorCode::Unauthorized as i32 => StatusCode::UNAUTHORIZED,
        code if code == conex_proto::ErrorCode::Forbidden as i32 => StatusCode::FORBIDDEN,
        code if code == conex_proto::ErrorCode::Unavailable as i32 => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        code if code == conex_proto::ErrorCode::StaleRevision as i32 => StatusCode::CONFLICT,
        _ => StatusCode::BAD_REQUEST,
    };
    (
        status,
        format!(
            "{{\"code\":{},\"message\":{:?}}}",
            error.code(),
            error.message()
        ),
    )
        .into_response()
}

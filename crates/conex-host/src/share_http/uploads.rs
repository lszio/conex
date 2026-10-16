//! Chunked upload with resume, for the landing page's file scene.
//!
//! The whole-body route in `mod.rs` is a convenience for a small file. This is
//! the path for anything worth resuming: a client declares the CIDs of the
//! chunks it is about to send, then sends them one at a time, then commits.
//!
//! Why declare the CIDs up front rather than as each chunk goes out:
//!
//! * **A staged upload is bound to a content, not to an attempt.** The host
//!   derives the root CID from the declared list, so a second attempt at the
//!   same file from the same owner finds the *same* staging and reports how far
//!   it got. Resume therefore needs no client-side state at all: the client
//!   re-declares what it was always going to send, and the answer tells it
//!   where to continue. A dropped connection and a reloaded tab are the same
//!   case, handled by the same code.
//! * **A bad chunk is refused where it happens.** Each arriving chunk is checked
//!   against the entry the client declared for its index, before the store is
//!   touched. Discovering the corruption at commit instead would mean learning
//!   about it after every byte was already sent.
//!
//! The scene deliberately accepts chunks **in order only**. One writer that
//! always appends turns "where do I resume?" into a single integer, which is
//! what `HEAD` reports; parallel or out-of-order delivery would buy throughput
//! the scene does not need and cost a missing-chunk set nobody wants to
//! reconcile. A retransmitted chunk is a no-op by construction: the CID is
//! recomputed from the bytes, and identical bytes are the same block.
//!
//! What is *not* reclaimed here is worth naming. Staging is dropped by its
//! lease, not by the owner disconnecting: a visitor who closes a tab mid-upload
//! leaves the staged bytes on disk until the lease expires. Cancelling on
//! disconnect would defeat reload-resume, and the lease plus the host-wide cap
//! is what bounds it.

#![forbid(unsafe_code)]

use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::extract::{Path, Request};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use super::{
    bad_request, file_to_json, no_link, notify_group_changed, own_entry, refresh_shared_counters,
    status_for, unavailable,
};
use crate::agent::now_ms;
use crate::http::HttpState;
use crate::share::{Offer, ShareItemKind};
use crate::share_blocks::SHARE_CHUNK_SIZE;

/// Body limit for one chunk route. A chunk is `SHARE_CHUNK_SIZE` bytes, and the
/// slack covers a client that misreads the size; anything past it is a refusal,
/// not a partial chunk.
pub const CHUNK_BODY_LIMIT: usize = SHARE_CHUNK_SIZE + 1024 * 1024;

/// `x-conex-received-count`: how many chunks the store already holds. The
/// client resumes from exactly this chunk index.
const RECEIVED_COUNT_HEADER: &str = "x-conex-received-count";
const RECEIVED_BYTES_HEADER: &str = "x-conex-received-bytes";
const CHUNK_SIZE_HEADER: &str = "x-conex-chunk-size";
const LEASE_MS_HEADER: &str = "x-conex-lease-ms";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitRequest {
    name: String,
    #[serde(default)]
    mime: String,
    /// Bytes the client will send, as a string: JSON numbers past 2^53 lose
    /// precision, and this one crosses that line for a multi-gigabyte file.
    size_bytes: String,
    #[serde(default)]
    chunk_size: u32,
    /// Leaf CIDs, in order, one per chunk.
    #[serde(default)]
    chunk_cids: Vec<String>,
    #[serde(default)]
    kind: String,
}

/// Begin, or rejoin, a staged upload.
///
/// Rejoining is the interesting half: an init for content the owner already has
/// staging for does not create a second upload, it reports the first one's
/// progress. The client cannot tell the difference, and does not need to.
pub async fn begin_upload(
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
    let Some(blocks) = state.shares.storage() else {
        return unavailable("no share root is configured on this host");
    };
    let (_parts, body) = request.into_parts();
    let bytes = match super::read_bounded(body, 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(response) => return *response,
    };
    let parsed: InitRequest = match serde_json::from_slice(&bytes) {
        Ok(parsed) => parsed,
        Err(_) => return bad_request("name, sizeBytes and chunkCids are required"),
    };
    let size_bytes = match parsed.size_bytes.parse::<u64>() {
        Ok(size) => size,
        Err(_) => return bad_request("sizeBytes must be a whole number of bytes"),
    };
    let chunk_size = if parsed.chunk_size == 0 {
        SHARE_CHUNK_SIZE as u32
    } else {
        parsed.chunk_size
    };
    if chunk_size != SHARE_CHUNK_SIZE as u32 {
        return bad_request(&format!(
            "chunkSize must be {}; one value, not a negotiation",
            SHARE_CHUNK_SIZE
        ));
    }
    let kind = match parsed.kind.as_str() {
        "" | "file" => ShareItemKind::File,
        "clip" => ShareItemKind::Clip,
        other => return bad_request(&format!("unknown share kind {other}")),
    };
    if size_bytes == 0 {
        return bad_request("an empty upload has nothing to resume");
    }
    // The ceiling is a property of the declaration alone, so it is checked
    // before anything is staged. Refusing after staging would leave bytes on
    // disk for a file that was never going to be accepted.
    let ceiling = state.shares.limits().max_bytes_for(kind);
    if size_bytes > ceiling {
        return status_for(super::too_large(size_bytes, ceiling));
    }
    let owner = entry.owner();
    let staged = match blocks.begin(&owner, size_bytes, &parsed.chunk_cids) {
        Ok(staged) => staged,
        Err(error) => return super::status_for_content(error),
    };
    // What the item will be called lives with the staged upload, not on the
    // commit: the client declares it once and a resume must not have to repeat
    // it. The scene keeps it, because the scene is also where a restart drops
    // it — an upload that outlives a restart has no item to become.
    state.shares.stage(
        &staged.upload_id,
        crate::share::StagedItem {
            name: parsed.name.clone(),
            mime: parsed.mime.clone(),
            kind,
            size_bytes,
            owner_link_id: entry.link_id.clone(),
        },
    );
    let ceiling = state.shares.limits().max_bytes_for(kind);
    if size_bytes > ceiling {
        // Refused on the declared size, before a byte moves: the alternative is
        // discovering it after the whole body was read.
        let _ = blocks.cancel(&owner, &staged.upload_id);
        return status_for(super::too_large(size_bytes, ceiling));
    }
    let mut response = Json(json!({
        "uploadId": staged.upload_id,
        "chunkSize": chunk_size,
        "receivedCount": staged.received_count.to_string(),
        "receivedBytes": staged.received_bytes.to_string(),
        "sizeBytes": size_bytes.to_string(),
        "leaseMs": blocks.lease_ms().to_string(),
        "maxFileBytes": ceiling.to_string(),
    }))
    .into_response();
    insert_number(&mut response, RECEIVED_COUNT_HEADER, staged.received_count);
    insert_number(&mut response, RECEIVED_BYTES_HEADER, staged.received_bytes);
    insert_number(&mut response, CHUNK_SIZE_HEADER, chunk_size as u64);
    insert_number(&mut response, LEASE_MS_HEADER, blocks.lease_ms());
    response
}

/// Accept one chunk. It must be the next one the store expects.
pub async fn put_chunk(
    Extension(state): Extension<Arc<HttpState>>,
    Path((upload_id, index)): Path<(String, u32)>,
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
    let Some(blocks) = state.shares.storage() else {
        return unavailable("no share root is configured on this host");
    };
    let (_parts, body) = request.into_parts();
    let bytes = match super::read_bounded(body, CHUNK_BODY_LIMIT as u64).await {
        Ok(bytes) => bytes,
        Err(response) => return *response,
    };
    if bytes.is_empty() {
        return bad_request("a chunk cannot be empty");
    }
    if bytes.len() > SHARE_CHUNK_SIZE {
        return bad_request(&format!(
            "a chunk is at most {} bytes, this one is {}",
            SHARE_CHUNK_SIZE,
            bytes.len()
        ));
    }
    let owner = entry.owner();
    let received = match blocks.put_chunk(&owner, &upload_id, index as usize, &bytes) {
        Ok(received) => received,
        Err(error) => return super::status_for_content(error),
    };
    let mut response = Json(json!({
        "uploadId": upload_id,
        "chunkIndex": index.to_string(),
        "receivedCount": received.to_string(),
    }))
    .into_response();
    insert_number(&mut response, RECEIVED_COUNT_HEADER, received);
    insert_number(&mut response, CHUNK_SIZE_HEADER, SHARE_CHUNK_SIZE as u64);
    response
}

/// How far an upload got. The answer is one number, which is the whole point.
pub async fn upload_status(
    Extension(state): Extension<Arc<HttpState>>,
    Path(upload_id): Path<String>,
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
    let Some(entry) = own_entry(&state, &session, headers) else {
        return no_link();
    };
    let Some(blocks) = state.shares.storage() else {
        return unavailable("no share root is configured on this host");
    };
    let owner = entry.owner();
    let staged = match blocks.staged_by_id(&owner, &upload_id) {
        Ok(staged) => staged,
        Err(error) => return super::status_for_content(error),
    };
    let mut response = Json(json!({
        "uploadId": staged.upload_id,
        "sizeBytes": staged.size_bytes.to_string(),
        "receivedCount": staged.received_count.to_string(),
        "receivedBytes": staged.received_bytes.to_string(),
        "chunkSize": SHARE_CHUNK_SIZE.to_string(),
        "leaseMs": blocks.lease_ms().to_string(),
    }))
    .into_response();
    insert_number(&mut response, RECEIVED_COUNT_HEADER, staged.received_count);
    insert_number(&mut response, RECEIVED_BYTES_HEADER, staged.received_bytes);
    insert_number(&mut response, CHUNK_SIZE_HEADER, SHARE_CHUNK_SIZE as u64);
    response
}

/// Finish a staged upload and admit the item to the group.
///
/// The host re-derives the root from the chunks it actually received, so a
/// commit can only ever name content this upload delivered; it never trusts a
/// block list from the caller.
pub async fn commit_upload(
    Extension(state): Extension<Arc<HttpState>>,
    Path(upload_id): Path<String>,
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
    let Some(blocks) = state.shares.storage() else {
        return unavailable("no share root is configured on this host");
    };
    let owner = entry.owner();
    let Some(staged_item) = state.shares.staged_item(&upload_id) else {
        // Without the declaration there is no name to show and no kind to
        // apply a policy to, so the upload cannot become an item.
        return bad_request("this upload has no pending item");
    };
    let stored = match blocks.commit(&owner, &upload_id) {
        Ok(stored) => stored,
        Err(error) => return super::status_for_content(error),
    };
    // The bytes are committed before the item is admitted, so a refused quota
    // releases them immediately rather than leaving them unreadable on disk.
    let offered = Offer {
        owner_link_id: &entry.link_id,
        owner_name: &entry.profile.label(&entry.link_id),
        group_key: &entry.group_key,
        name: &staged_item.name,
        mime: &staged_item.mime,
        kind: staged_item.kind,
        size_bytes: stored.size_bytes,
        now_ms: now_ms(),
    };
    let file = match state.shares.admit(&stored.root_cid, &offered) {
        Ok(file) => file,
        Err(error) => {
            state.shares.release(&stored.root_cid);
            return status_for(error);
        }
    };
    state.shares.unstage(&upload_id);
    refresh_shared_counters(&state, &entry);
    notify_group_changed(&state, &entry);
    Json(json!({ "file": file_to_json(file) })).into_response()
}

/// Abandon a staged upload.
///
/// The bytes already written are not deleted here: they stay in the content
/// store until the lease expires, so an abandoned upload never leaves a
/// half-written item readable, and a re-init still finds the same staging if
/// the client changes its mind.
pub async fn cancel_upload(
    Extension(state): Extension<Arc<HttpState>>,
    Path(upload_id): Path<String>,
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
    let Some(blocks) = state.shares.storage() else {
        return unavailable("no share root is configured on this host");
    };
    let owner = entry.owner();
    if let Err(error) = blocks.cancel(&owner, &upload_id) {
        return super::status_for_content(error);
    }
    state.shares.unstage(&upload_id);
    StatusCode::NO_CONTENT.into_response()
}

fn insert_number(response: &mut Response, name: &'static str, value: u64) {
    if let Ok(value) = HeaderValue::from_str(&value.to_string()) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(name), value);
    }
}

//! M3.2 browser content delivery over the real router: range semantics,
//! authorization, response-header hygiene and streaming byte equality.
#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::serve;
use conex_host::serve::build_router;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RANGE};
use reqwest::{Response, StatusCode};
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

/// A remote endpoint served through the binary agent link, with a
/// deterministic binary payload large enough to span several slices.
struct Fixture {
    addr: SocketAddr,
    _payload: Vec<u8>,
    revision: String,
    _dir: tempfile::TempDir,
    _agent: tokio::task::JoinHandle<()>,
}

fn notes_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/p0/notes")
}

fn write_payload(dir: &Path, name: &str, bytes: &[u8]) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create dir");
    }
    std::fs::write(path, bytes).expect("write payload");
}

async fn start(payload: Vec<u8>) -> Fixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let content = dir.path().join("content");
    let session = dir.path().join("session");
    let operation = dir.path().join("operation");
    let notes = dir.path().join("notes");
    std::fs::create_dir_all(&notes).expect("notes dir");
    write_payload(&notes, "clip.mp4", &payload);
    // Deterministic revision: the file's mtime is whatever the OS stamped;
    // we read it back from the first successful response.
    let config = format!(
        "listen = \"127.0.0.1:0\"\nallow_loopback_http = true\naudience = \"host.local\"\nweb_origin = \"http://127.0.0.1:0\"\ncontent_root = \"{}\"\nsession_root = \"{}\"\noperation_root = \"{}\"\n\n[web_guest]\nprincipal_id = \"guest\"\ntenant_id = \"tenant-a\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\nrole = \"ui\"\n\n[[policy]]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"media\"\nactions = [\"list\", \"read\", \"search\"]\nroot = \"\"\nsubtree = true\n\n[[endpoints]]\nid = \"media\"\ntenant_id = \"tenant-a\"\nprovider_id = \"media\"\nkind = \"source-fs\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"{}\"\n",
        content.display(),
        session.display(),
        operation.display(),
        sha256_hex("ui-secret"),
        notes.display(),
    );
    let path = dir.path().join("host.toml");
    std::fs::write(&path, config).expect("write config");
    let loaded = conex_host::config::HostConfig::load(&path).expect("load config");
    let app = build_router(&loaded).expect("build router");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    drop(app);
    drop(loaded);
    // Re-write with the real port so web_origin matches the request Origin.
    let config = std::fs::read_to_string(&path)
        .expect("read config")
        .replace("127.0.0.1:0", &addr.to_string());
    std::fs::write(&path, config).expect("rewrite config");
    let loaded = conex_host::config::HostConfig::load(&path).expect("load config");
    let app = build_router(&loaded).expect("build router");
    let task = tokio::spawn(async move { serve(listener, app).await.expect("serve") });
    tokio::time::sleep(Duration::from_millis(30)).await;
    let revision = std::fs::metadata(notes.join("clip.mp4"))
        .expect("stat payload")
        .modified()
        .expect("mtime")
        .duration_since(std::time::UNIX_EPOCH)
        .expect("epoch")
        .as_nanos()
        .to_string();
    Fixture {
        addr,
        _payload: payload,
        revision,
        _dir: dir,
        _agent: task,
    }
}

async fn login(addr: SocketAddr) -> (reqwest::Client, String) {
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/web/login"))
        .header("origin", format!("http://{addr}"))
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login");
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .expect("cookie")
        .to_string();
    (client, cookie)
}

fn content_url(fixture: &Fixture, resource: &str) -> String {
    format!(
        "http://{}/content?endpointId=media&resourceId={resource}",
        fixture.addr
    )
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value: &HeaderValue| value.to_str().ok())
        .map(str::to_string)
}

async fn body_bytes(response: Response) -> Vec<u8> {
    response.bytes().await.expect("body").to_vec()
}

#[tokio::test(flavor = "multi_thread")]
async fn full_body_matches_source_bytes_exactly() {
    // 700 KiB of deterministic pseudo-random bytes: spans three 256 KiB
    // slices and includes arbitrary binary (not valid UTF-8).
    let payload: Vec<u8> = (0..(700 * 1024u32))
        .map(|index| (index.wrapping_mul(31).wrapping_add(7) % 251) as u8)
        .collect();
    let fixture = start(payload.clone()).await;
    let (client, cookie) = login(fixture.addr).await;
    let response = client
        .get(content_url(&fixture, "clip.mp4"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("content");
    if response.status() != StatusCode::OK {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        panic!("status {status} body {body}");
    }
    assert_eq!(
        header(response.headers(), "accept-ranges").as_deref(),
        Some("bytes")
    );
    assert_eq!(
        header(response.headers(), "x-content-type-options").as_deref(),
        Some("nosniff")
    );
    assert_eq!(
        header(response.headers(), "cache-control").as_deref(),
        Some("private, no-store"),
        "public and private answers share one no-store policy"
    );
    let disposition = header(response.headers(), "content-disposition").expect("disposition");
    assert!(
        disposition.starts_with("inline;"),
        "video/mp4 renders inline: {disposition}"
    );
    assert!(disposition.contains("clip.mp4"));
    assert!(
        response.headers().get("content-range").is_none(),
        "a complete answer carries no content-range"
    );
    let etag = header(response.headers(), "etag").expect("etag");
    assert_eq!(etag, format!("\"rev-{}\"", fixture.revision));
    assert_eq!(
        body_bytes(response).await,
        payload,
        "byte-for-byte equality"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn single_range_slices_are_exact_and_bounded() {
    let payload: Vec<u8> = (0..(600 * 1024u32)).map(|i| (i % 253) as u8).collect();
    let fixture = start(payload.clone()).await;
    let (client, cookie) = login(fixture.addr).await;

    // Explicit middle range.
    let response = client
        .get(content_url(&fixture, "clip.mp4"))
        .header("cookie", &cookie)
        .header(RANGE, "bytes=100-199")
        .send()
        .await
        .expect("range");
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        header(response.headers(), "content-range").as_deref(),
        Some(format!("bytes 100-199/{}", payload.len()).as_str())
    );
    assert_eq!(body_bytes(response).await, payload[100..200].to_vec());

    // Open-ended range.
    let open = client
        .get(content_url(&fixture, "clip.mp4"))
        .header("cookie", &cookie)
        .header(RANGE, "bytes=524000-")
        .send()
        .await
        .expect("open range");
    assert_eq!(open.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        body_bytes(open).await,
        payload[524000..].to_vec(),
        "open range runs to EOF"
    );

    // Suffix range.
    let suffix = client
        .get(content_url(&fixture, "clip.mp4"))
        .header("cookie", &cookie)
        .header(RANGE, "bytes=-256")
        .send()
        .await
        .expect("suffix");
    assert_eq!(suffix.status(), StatusCode::PARTIAL_CONTENT);
    let tail = payload.len() - 256;
    assert_eq!(body_bytes(suffix).await, payload[tail..].to_vec());

    // A range beyond EOF is unsatisfiable, not a silently truncated 200.
    let past = client
        .get(content_url(&fixture, "clip.mp4"))
        .header("cookie", &cookie)
        .header(RANGE, "bytes=99999999-")
        .send()
        .await
        .expect("past eof");
    assert_eq!(past.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        header(past.headers(), "content-range").as_deref(),
        Some(format!("bytes */{}", payload.len()).as_str())
    );

    // Multi-range requests are answered as a plain 200 per RFC 9110.
    let multi = client
        .get(content_url(&fixture, "clip.mp4"))
        .header("cookie", &cookie)
        .header(RANGE, "bytes=0-9,20-29")
        .send()
        .await
        .expect("multi");
    assert_eq!(multi.status(), StatusCode::OK);
    assert_eq!(body_bytes(multi).await, payload);
}

#[tokio::test(flavor = "multi_thread")]
async fn if_range_mismatch_downgrades_to_the_full_body() {
    let payload = vec![0x5Au8; 4096];
    let fixture = start(payload.clone()).await;
    let (client, cookie) = login(fixture.addr).await;
    let stale = client
        .get(content_url(&fixture, "clip.mp4"))
        .header("cookie", &cookie)
        .header(RANGE, "bytes=0-9")
        .header(
            HeaderName::from_static("if-range"),
            "\"rev-does-not-match\"",
        )
        .send()
        .await
        .expect("if-range");
    assert_eq!(
        stale.status(),
        StatusCode::OK,
        "a changed entity ignores Range and returns the full representation"
    );
    assert_eq!(body_bytes(stale).await, payload);
}

#[tokio::test(flavor = "multi_thread")]
async fn unauthorized_and_missing_never_leak_bytes() {
    let fixture = start(vec![1u8; 1024]).await;
    let client = reqwest::Client::new();

    // No session cookie at all.
    let anonymous = client
        .get(content_url(&fixture, "clip.mp4"))
        .send()
        .await
        .expect("anonymous");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let refused = body_bytes(anonymous).await;
    assert!(
        !refused.windows(5).any(|window| window == b"root:"),
        "an unauthenticated error body never contains host filesystem content"
    );

    // A session without policy on this endpoint is forbidden, and traversal
    // never resolves outside the endpoint root.
    let (_client, other_cookie) = login(fixture.addr).await;
    let traversal = reqwest::Client::new()
        .get(format!(
            "http://{}/content?endpointId=media&resourceId=../../etc/passwd",
            fixture.addr
        ))
        .header("cookie", &other_cookie)
        .send()
        .await
        .expect("traversal");
    assert!(
        traversal.status() == StatusCode::BAD_REQUEST
            || traversal.status() == StatusCode::FORBIDDEN,
        "path traversal is rejected, got {}",
        traversal.status()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_resource_is_not_found_not_empty_success() {
    let fixture = start(vec![2u8; 512]).await;
    let (client, cookie) = login(fixture.addr).await;
    let missing = client
        .get(content_url(&fixture, "absent.bin"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("missing");
    assert_ne!(
        missing.status(),
        StatusCode::OK,
        "missing resources never answer 200"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_binary_types_are_attachments_and_mimes_cannot_inject_headers() {
    // A `.bin` payload is octet-stream: forced attachment, never inline.
    let dir = tempfile::tempdir().expect("tempdir");
    let notes = dir.path().join("notes");
    std::fs::create_dir_all(&notes).expect("notes");
    std::fs::write(notes.join("blob.bin"), [0u8, 0xFF, 0x0A, 0x0D, 0x1A]).expect("write");
    let content = dir.path().join("content");
    let session = dir.path().join("session");
    let operation = dir.path().join("operation");
    let config = format!(
        "listen = \"127.0.0.1:0\"\nallow_loopback_http = true\naudience = \"host.local\"\nweb_origin = \"http://127.0.0.1:0\"\ncontent_root = \"{}\"\nsession_root = \"{}\"\noperation_root = \"{}\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\nrole = \"ui\"\n\n[[policy]]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"media\"\nactions = [\"list\", \"read\", \"search\"]\nroot = \"\"\nsubtree = true\n\n[[endpoints]]\nid = \"media\"\ntenant_id = \"tenant-a\"\nprovider_id = \"media\"\nkind = \"source-fs\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"{}\"\n",
        content.display(),
        session.display(),
        operation.display(),
        sha256_hex("ui-secret"),
        notes.display(),
    );
    let path = dir.path().join("host.toml");
    std::fs::write(&path, config).expect("write config");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let config = std::fs::read_to_string(&path)
        .expect("read config")
        .replace("127.0.0.1:0", &addr.to_string());
    std::fs::write(&path, config).expect("rewrite config");
    let loaded = conex_host::config::HostConfig::load(&path).expect("load");
    let app = build_router(&loaded).expect("router");
    let task = tokio::spawn(async move { serve(listener, app).await.expect("serve") });
    tokio::time::sleep(Duration::from_millis(30)).await;
    let (client, cookie) = login(addr).await;
    let response = client
        .get(format!(
            "http://{addr}/content?endpointId=media&resourceId=blob.bin"
        ))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("bin content");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        header(response.headers(), "content-type").as_deref(),
        Some("application/octet-stream")
    );
    let disposition = header(response.headers(), "content-disposition").expect("disposition");
    assert!(
        disposition.starts_with("attachment;"),
        "unknown binary is always a download: {disposition}"
    );
    // The raw bytes survive untouched: no UTF-8 transformation.
    assert_eq!(
        body_bytes(response).await,
        vec![0u8, 0xFF, 0x0A, 0x0D, 0x1A]
    );
    let _ = notes_dir();
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn head_request_reports_headers_without_a_body() {
    let payload = vec![0x11u8; 300];
    let fixture = start(payload).await;
    let (client, cookie) = login(fixture.addr).await;
    let head = client
        .head(content_url(&fixture, "clip.mp4"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("head");
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(
        header(head.headers(), "accept-ranges").as_deref(),
        Some("bytes")
    );
    assert!(header(head.headers(), "etag").is_some());
    assert!(body_bytes(head).await.is_empty());
}

//! Group-scoped file sharing over the real host: two live browser links, real
//! WebSocket handshakes, real HTTP routes.
//!
//! These tests drive the same path the landing page does. A client only exists
//! once its WSS link reaches `ready`, so every case below opens a socket
//! before it touches `/web/files`; asserting on a route whose caller was never
//! registered would prove nothing about a real session.
#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

const ULID_HELLO: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const ULID_READY: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAW";

/// Crockford base32 ids in the shape `sdk/typescript/src/client.ts` mints.
/// The host rejects a non-ULID request id and answers `id: null`, so the test
/// has to mint them exactly as the real client does.
fn mint_ulid() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut bytes = [0u8; 16];
    // 48-bit big-endian timestamp in the leading 6 bytes; the rest is a
    // monotonic sequence, standing in for the SDK's random tail.
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_millis() as u64;
    bytes[..6].copy_from_slice(&millis.to_be_bytes()[2..]);
    let sequence = NEXT_ID.fetch_add(1, Ordering::AcqRel);
    // The tail is 10 bytes; only the first 8 carry the counter, which is
    // plenty for uniqueness inside one test binary.
    bytes[6..14].copy_from_slice(&sequence.to_be_bytes());
    // 128 bits little-end-first into 26 base32 characters, most significant
    // first, exactly as the SDK shifts the accumulated bit string.
    let mut bits: u128 = 0;
    for byte in bytes {
        bits = (bits << 8) | u128::from(byte);
    }
    let mut out = vec![b'0'; 26];
    let mut remaining = bits;
    for index in (0..26).rev() {
        out[index] = ALPHABET[(remaining & 31) as usize];
        remaining >>= 5;
    }
    String::from_utf8(out).expect("ascii")
}

static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

const PROFILE: &str = "conex-jsonrpc2-wss";

fn tmp(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    std::fs::create_dir_all(&path).expect("mkdir");
    (dir, path)
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Host {
    addr: SocketAddr,
    child: std::process::Child,
    _content: tempfile::TempDir,
    _session: tempfile::TempDir,
    _operation: tempfile::TempDir,
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn start() -> Host {
    let (t_content, content_root) = tmp("content");
    let (t_session, session_root) = tmp("session");
    let (t_operation, operation_root) = tmp("operation");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    let config = format!(
        "listen = \"127.0.0.1:{port}\"\n\
allow_loopback_http = true\n\
audience = \"host.local\"\n\
host_origin = \"conex://broker.local\"\n\
web_origin = \"http://127.0.0.1:{port}\"\n\
content_root = \"{content_root}\"\n\
session_root = \"{session_root}\"\n\
operation_root = \"{operation_root}\"\n\
\n\
[web_guest]\n\
principal_id = \"guest\"\n\
tenant_id = \"tenant-a\"\n\
max_sessions = 32\n",
        content_root = content_root.display(),
        session_root = session_root.display(),
        operation_root = operation_root.display(),
    );
    let cfg_path = content_root.parent().expect("root").join("host.toml");
    std::fs::write(&cfg_path, config).expect("write config");
    let binary = env!("CARGO_BIN_EXE_conex-host");
    // stderr goes to a file, not a pipe: nobody reads the pipe during the
    // test, and once its buffer fills the host blocks mid-run, which looks
    // exactly like a hang.
    let log_path = content_root.parent().expect("root").join("host.log");
    let log = std::fs::File::create(&log_path).expect("create log");
    let mut child = std::process::Command::new(binary)
        .arg(&cfg_path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(log))
        .spawn()
        .expect("spawn conex-host");
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("addr");
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Host {
                addr,
                child,
                _content: t_content,
                _session: t_session,
                _operation: t_operation,
            };
        }
        if let Ok(Some(status)) = child.try_wait() {
            let stderr = std::fs::read_to_string(&log_path).unwrap_or_default();
            panic!("conex-host exited with {status}: {stderr}\nconfig at {cfg_path:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("conex-host did not become ready on {addr} (config at {cfg_path:?})");
}

/// A visitor: its web session (cookie + csrf) and its live WSS link.
struct Visitor {
    cookie: String,
    csrf: String,
    link_id: String,
    socket: Option<Socket>,
}

impl Visitor {
    /// Log in over HTTP, then complete the hello/ready handshake. The client
    /// row exists only after `ready`, which is what makes the visitor
    /// addressable and group-scoped.
    async fn join(host: &Host) -> Self {
        let http = reqwest::Client::new();
        let origin = format!("http://{}", host.addr);
        // `/web/login` is for a UI bearer token. This host has none, so the
        // session comes from the guest path: `/web/session` issues the cookie
        // itself when none is present, exactly as a real visitor gets one.
        let session = http
            .get(format!("{origin}/web/session"))
            .header("origin", &origin)
            .send()
            .await
            .expect("session");
        assert_eq!(session.status(), reqwest::StatusCode::OK);
        let cookie = session
            .headers()
            .get("set-cookie")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .expect("cookie")
            .to_string();
        let csrf = session.json::<Value>().await.expect("session json")["csrf"]
            .as_str()
            .expect("csrf")
            .to_string();

        let ticket = http
            .post(format!("{origin}/tickets"))
            .header("cookie", &cookie)
            .header("origin", &origin)
            .header("x-csrf-token", &csrf)
            .json(&json!({}))
            .send()
            .await
            .expect("ticket")
            .json::<Value>()
            .await
            .expect("ticket json")["ticket"]
            .as_str()
            .expect("ticket value")
            .to_string();

        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://{}/wss?ticket={ticket}", host.addr)
            .into_client_request()
            .expect("ws request");
        request
            .headers_mut()
            .insert("origin", origin.parse().expect("origin header"));
        let (mut socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("wss connect");

        socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0", "id": ULID_HELLO, "method": "conex/hello",
                    "params": {"profileId": PROFILE, "plane": "broker", "provides": [], "requires": []}
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send hello");
        let hello: Value = next_json(&mut socket).await;
        let negotiation = hello["result"]["negotiationId"]
            .as_str()
            .expect("negotiationId")
            .to_string();
        socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0", "id": ULID_READY, "method": "conex/ready",
                    "params": {
                        "negotiationId": negotiation, "profileId": PROFILE, "plane": "broker",
                        "provides": hello["result"]["provides"], "limits": hello["result"]["limits"]
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send ready");
        let ready = next_json(&mut socket).await;
        assert!(ready["result"]["negotiationId"].is_string(), "{ready}");
        Self {
            cookie,
            csrf,
            link_id: String::new(),
            socket: Some(socket),
        }
    }

    /// Declare the visitor's name and group, and remember the link id the
    /// host assigned. Returns the full `self` summary.
    async fn set_profile(&mut self, name: &str, group: &str) -> Value {
        let response = self
            .call(
                "client/profile",
                json!({ "profile": { "displayName": name, "group": group, "visible": true } }),
            )
            .await;
        if let Some(link_id) = response["result"]["self"]["linkId"].as_str() {
            self.link_id = link_id.to_string();
        }
        response
    }

    /// One business call over the visitor's own link, framed exactly as the
    /// SDK frames it: `context` + `timeoutBudgetMs` + `input`, with a
    /// JSON-RPC id the reply is matched on.
    async fn call(&mut self, method: &str, input: Value) -> Value {
        let mut socket = self.socket.take().expect("socket");
        // The host rejects a non-ULID request id outright, and the reply then
        // carries `id: null`, so an invented id string means waiting forever
        // for a result that was already refused.
        let id = mint_ulid();
        socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "method": method,
                    "params": {
                        "context": { "providerEndpointId": "", "plane": "broker" },
                        "timeoutBudgetMs": 8000,
                        "input": input,
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("send call");
        let mut value = next_json(&mut socket).await;
        // A pushed `conex/client-hello` carries no id; skip anything that is
        // not the answer to this call rather than mistaking it for a result.
        for _ in 0..4 {
            if value.get("id").and_then(Value::as_str) == Some(id.as_str()) {
                break;
            }
            value = next_json(&mut socket).await;
        }
        self.socket = Some(socket);
        value
    }

    fn http(&self, host: &Host) -> reqwest::RequestBuilder {
        reqwest::Client::new()
            .get(format!("http://{}/web/files", host.addr))
            .header("cookie", &self.cookie)
            .header("origin", format!("http://{}", host.addr))
    }

    /// The download route, not the listing route: the two are different paths
    /// and a query string on the listing route would simply be ignored.
    fn download(&self, host: &Host, id: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new()
            .get(format!("http://{}/web/files/download", host.addr))
            .query(&[("id", id)])
            .header("cookie", &self.cookie)
            .header("origin", format!("http://{}", host.addr))
    }

    async fn list(&self, host: &Host) -> Value {
        let response = self.http(host).send().await.expect("list files");
        response.json().await.expect("list json")
    }

    /// The `Result` is returned rather than unwrapped: a body-limit refusal
    /// can close the connection mid-send, and that has to be assertable.
    async fn upload(
        &self,
        host: &Host,
        name: &str,
        mime: &str,
        bytes: Vec<u8>,
    ) -> Result<reqwest::Response, reqwest::Error> {
        reqwest::Client::new()
            .post(format!("http://{}/web/files", host.addr))
            .header("cookie", &self.cookie)
            .header("origin", format!("http://{}", host.addr))
            .header("x-csrf-token", &self.csrf)
            .header("x-conex-file-name", name)
            .header("x-conex-file-type", mime)
            .body(bytes)
            .send()
            .await
    }
}

async fn next_json(socket: &mut Socket) -> Value {
    for _ in 0..32 {
        match socket.next().await.expect("message").expect("ws message") {
            Message::Text(text) => return serde_json::from_str(&text).expect("json"),
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) => continue,
            Message::Close(frame) => panic!("socket closed: {frame:?}"),
            _ => continue,
        }
    }
    panic!("no json message arrived");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_in_the_same_group_can_list_preview_and_download() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    let mut bob = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    bob.set_profile("bob", "team-a").await;

    let payload = "同组共享的内容\nsecond line\n".as_bytes().to_vec();
    let uploaded = alice
        .upload(&host, "notes.txt", "text/plain", payload.clone())
        .await
        .expect("upload");
    assert_eq!(uploaded.status(), reqwest::StatusCode::OK);
    let body = uploaded.json::<Value>().await.expect("upload json");
    let file_id = body["file"]["id"].as_str().expect("file id").to_string();
    assert_eq!(body["file"]["name"].as_str(), Some("notes.txt"));
    assert_eq!(body["file"]["inline"].as_bool(), Some(true));

    // The peer sees it in its own group's listing, attributed to the owner.
    let listing = bob.list(&host).await;
    let files = listing["files"].as_array().expect("files array");
    assert_eq!(files.len(), 1, "{listing}");
    assert_eq!(files[0]["name"].as_str(), Some("notes.txt"));
    assert_eq!(files[0]["ownerName"].as_str(), Some("alice"));
    assert_eq!(
        files[0]["size"].as_str(),
        Some(payload.len().to_string().as_str())
    );

    // And the bytes come back exactly, inline, with a safe content type.
    let download = bob
        .download(&host, &file_id)
        .send()
        .await
        .expect("download");
    assert_eq!(download.status(), reqwest::StatusCode::OK);
    assert_eq!(
        download
            .headers()
            .get("x-content-type-options")
            .and_then(|v| v.to_str().ok()),
        Some("nosniff")
    );
    let disposition = download
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .expect("disposition")
        .to_string();
    assert!(disposition.starts_with("inline;"), "{disposition}");
    assert_eq!(download.bytes().await.expect("bytes").to_vec(), payload);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_client_in_another_group_gets_a_404_for_the_same_id() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    let mut outsider = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    outsider.set_profile("mallory", "team-b").await;

    let uploaded = alice
        .upload(&host, "secret.txt", "text/plain", b"not for you".to_vec())
        .await
        .expect("upload");
    let file_id = uploaded.json::<Value>().await.expect("json")["file"]["id"]
        .as_str()
        .expect("id")
        .to_string();

    // The other group sees an empty listing, not a filtered one.
    assert!(
        outsider.list(&host).await["files"]
            .as_array()
            .expect("files")
            .is_empty()
    );
    let response = outsider
        .download(&host, &file_id)
        .send()
        .await
        .expect("download");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::NOT_FOUND,
        "a cross-group id must be indistinguishable from an unknown one"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_cross_group_hello_is_refused_but_a_same_group_one_round_trips() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    let mut bob = Visitor::join(&host).await;
    let mut mallory = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    bob.set_profile("bob", "team-a").await;
    mallory.set_profile("mallory", "team-b").await;

    // Mallory's list holds exactly her own row: the team is invisible, and
    // the caller's own row is what the page renders itself from.
    let listed = mallory.call("client/list", json!({})).await;
    let clients = listed["result"]["clients"].as_array().expect("clients");
    assert_eq!(
        clients.len(),
        1,
        "a group must not be visible from another: {listed}"
    );
    assert_eq!(
        clients[0]["profile"]["displayName"].as_str(),
        Some("mallory")
    );
    assert!(
        !clients
            .iter()
            .any(|row| row["linkId"] == alice.link_id || row["linkId"] == bob.link_id),
        "no teammate may appear in another group's list: {listed}"
    );

    let refused = alice
        .call(
            "client/hello",
            json!({ "targetLinkId": mallory.link_id, "text": "hi" }),
        )
        .await;
    assert_eq!(
        refused["error"]["code"].as_i64(),
        Some(conex_proto::ErrorCode::Forbidden as i64),
        "{refused}"
    );

    // A same-group hello needs the target to answer the pushed frame, so the
    // test drives bob's socket in the background exactly as the page does.
    let mut bob_socket = bob.socket.take().expect("bob socket");
    let responder = tokio::spawn(async move {
        for _ in 0..8 {
            let value: Value = next_json(&mut bob_socket).await;
            if value["method"].as_str() == Some("conex/client-hello") {
                let reply_id = value["params"]["replyId"].as_str().expect("replyId");
                bob_socket
                    .send(Message::Text(
                        json!({
                            "jsonrpc": "2.0", "method": "conex/client-pong",
                            "params": { "replyId": reply_id, "reply": "pong" }
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .expect("pong");
                break;
            }
        }
        bob_socket
    });
    let ok = alice
        .call(
            "client/hello",
            json!({ "targetLinkId": bob.link_id, "text": "hello" }),
        )
        .await;
    assert_eq!(ok["result"]["reply"].as_str(), Some("pong"), "{ok}");
    bob.socket = Some(responder.await.expect("responder"));

    // The measured round trip is now real and recorded on both clients.
    let status = alice.call("client/status", json!({})).await;
    let groups = status["result"]["groups"].as_array().expect("groups");
    // Matched on the isolation key, not the label: two groups may legitimately
    // carry the same label text, and the key is what the host scopes by.
    let team_key = alice.call("client/list", json!({})).await["result"]["clients"][0]["groupKey"]
        .as_str()
        .expect("alice group key")
        .to_string();
    let team = groups
        .iter()
        .find(|group| group["groupKey"].as_str() == Some(team_key.as_str()))
        .expect("team-a row");
    // uint32/uint64 fields cross the JSON plane as decimal strings, matching
    // every other length and counter in this protocol.
    assert_eq!(team["clientsOnline"].as_str(), Some("2"));
    assert_eq!(team["roundTrips"].as_str(), Some("2"), "both ends count");
    assert_eq!(team["label"].as_str(), Some("team-a"));
    assert_ne!(
        team["lastRoundTripMs"].as_str(),
        Some("0"),
        "a completed hello yields a measured latency, not an unmeasured zero"
    );
    // Two groups exist on this host, and both are reported as counters only.
    assert_eq!(status["result"]["groupsOnline"].as_str(), Some("2"));
    assert_eq!(status["result"]["clientsOnline"].as_str(), Some("3"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_the_owner_may_withdraw_a_file() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    let mut bob = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    bob.set_profile("bob", "team-a").await;
    let uploaded = alice
        .upload(&host, "keep.txt", "text/plain", b"mine".to_vec())
        .await
        .expect("upload");
    let file_id = uploaded.json::<Value>().await.expect("json")["file"]["id"]
        .as_str()
        .expect("id")
        .to_string();

    let refused = reqwest::Client::new()
        .post(format!("http://{}/web/files/remove", host.addr))
        .header("cookie", &bob.cookie)
        .header("origin", format!("http://{}", host.addr))
        .header("x-csrf-token", &bob.csrf)
        .json(&json!({ "id": file_id }))
        .send()
        .await
        .expect("remove");
    assert_eq!(refused.status(), reqwest::StatusCode::FORBIDDEN);
    assert_eq!(
        alice.list(&host).await["files"]
            .as_array()
            .expect("files")
            .len(),
        1,
        "a refused withdrawal must not delete the file"
    );

    let removed = reqwest::Client::new()
        .post(format!("http://{}/web/files/remove", host.addr))
        .header("cookie", &alice.cookie)
        .header("origin", format!("http://{}", host.addr))
        .header("x-csrf-token", &alice.csrf)
        .json(&json!({ "id": file_id }))
        .send()
        .await
        .expect("remove");
    assert_eq!(removed.status(), reqwest::StatusCode::NO_CONTENT);
    assert!(
        bob.list(&host).await["files"]
            .as_array()
            .expect("files")
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_write_without_the_csrf_token_is_refused() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    let response = reqwest::Client::new()
        .post(format!("http://{}/web/files", host.addr))
        .header("cookie", &alice.cookie)
        .header("origin", format!("http://{}", host.addr))
        .header("x-conex-file-name", "nope.txt")
        .body(b"data".to_vec())
        .send()
        .await
        .expect("upload");
    assert_eq!(
        response.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "offering a file is a mutation and needs the CSRF nonce"
    );
    assert!(
        alice.list(&host).await["files"]
            .as_array()
            .expect("files")
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_active_content_file_downloads_instead_of_rendering() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    let uploaded = alice
        .upload(
            &host,
            "page.html",
            "text/html",
            b"<script>alert(1)</script>".to_vec(),
        )
        .await;
    let body = uploaded
        .expect("upload")
        .json::<Value>()
        .await
        .expect("json");
    assert_eq!(
        body["file"]["inline"].as_bool(),
        Some(false),
        "html must never be previewed in another visitor's session"
    );
    let file_id = body["file"]["id"].as_str().expect("id").to_string();
    let download = alice
        .download(&host, &file_id)
        .send()
        .await
        .expect("download");
    let disposition = download
        .headers()
        .get("content-disposition")
        .and_then(|v| v.to_str().ok())
        .expect("disposition")
        .to_string();
    assert!(disposition.starts_with("attachment;"), "{disposition}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_disconnect_reclaims_the_files_it_left_behind() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    let mut bob = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    bob.set_profile("bob", "team-a").await;
    let uploaded = alice
        .upload(&host, "temporary.txt", "text/plain", b"bye".to_vec())
        .await
        .expect("upload");
    let file_id = uploaded.json::<Value>().await.expect("json")["file"]["id"]
        .as_str()
        .expect("id")
        .to_string();
    assert_eq!(
        bob.list(&host).await["files"].as_array().expect("f").len(),
        1
    );

    // Alice closes her socket: her link is gone, so her bytes must go with it
    // instead of staying readable and charged to the group's quota.
    if let Some(mut socket) = alice.socket.take() {
        socket.close(None).await.expect("close");
    }
    for _ in 0..60 {
        if bob.list(&host).await["files"]
            .as_array()
            .expect("files")
            .is_empty()
        {
            let response = bob
                .download(&host, &file_id)
                .send()
                .await
                .expect("download");
            assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the disconnected client's files were never reclaimed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_oversized_upload_is_refused_by_the_transport() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    // One byte over the per-file cap: the body limit must reject it without
    // the store ever holding it.
    let oversize = vec![b'x'; (conex_host::share::MAX_FILE_BYTES as usize) + 1];
    // A body-limit rejection may answer 413 and close, or drop the connection
    // mid-send; both are refusals, and neither may leave a stored file.
    let refused = match alice
        .upload(&host, "big.bin", "application/octet-stream", oversize)
        .await
    {
        Ok(response) => response.status() == reqwest::StatusCode::PAYLOAD_TOO_LARGE,
        Err(error) => {
            assert!(
                error.is_body() || error.is_request(),
                "an oversized upload must be refused, not misreported: {error}"
            );
            true
        }
    };
    assert!(
        refused,
        "an oversized upload must be refused by the transport"
    );
    assert!(
        alice.list(&host).await["files"]
            .as_array()
            .expect("files")
            .is_empty()
    );
}

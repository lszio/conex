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
    // Picking a port by bind-then-drop races with every other test spawning
    // its own host at the same moment: a stolen port makes our child exit at
    // bind while the readiness connect succeeds against the thief, and the
    // first real request dies with ConnectionRefused. Retry on a fresh port;
    // anything other than an address-in-use failure is a real bug and panics
    // with the host log on the first attempt.
    let mut last_error = String::new();
    for _ in 0..5 {
        match try_start().await {
            Ok(host) => return host,
            Err(error) => {
                assert!(error.contains("in use"), "{error}");
                last_error = error;
            }
        }
    }
    panic!("conex-host could not claim a free port after 5 attempts: {last_error}");
}

async fn try_start() -> Result<Host, String> {
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
    let exited = |child: &mut std::process::Child| {
        let status = child.try_wait().ok().flatten();
        status.map(|status| {
            let stderr = std::fs::read_to_string(&log_path).unwrap_or_default();
            format!("conex-host exited with {status}: {stderr}\nconfig at {cfg_path:?}")
        })
    };
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            // The connect may have reached the thief that took the port
            // between our drop and our child's bind. A bind failure is
            // immediate, so one quiet beat is enough to prove the port is
            // really ours: a child that lost the race is gone by now.
            tokio::time::sleep(Duration::from_millis(50)).await;
            return match exited(&mut child) {
                Some(error) => Err(error),
                None => Ok(Host {
                    addr,
                    child,
                    _content: t_content,
                    _session: t_session,
                    _operation: t_operation,
                }),
            };
        }
        if let Some(error) = exited(&mut child) {
            return Err(error);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!(
        "conex-host did not become ready on {addr} (config at {cfg_path:?})"
    ))
}

/// A visitor: its web session (cookie + csrf) and its live WSS link.
struct Visitor {
    cookie: String,
    csrf: String,
    link_id: String,
    socket: Option<Socket>,
    /// Every frame the host pushed at this tab, in arrival order.
    pushes: std::sync::Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl Visitor {
    /// Log in over HTTP, then complete the hello/ready handshake. The client
    /// row exists only after `ready`, which is what makes the visitor
    /// addressable and group-scoped.
    async fn join(host: &Host) -> Self {
        Self::join_inner(host, false).await
    }

    /// Join, name this tab, and record every frame the host pushes at it.
    ///
    /// The profile is set before the reader takes the socket, because that is
    /// the one business call the tab has to make: the recording reader owns the
    /// socket from then on, and a socket cannot serve a caller and a reader at
    /// the same time. A test that wants to watch pushes and a test that wants
    /// to send calls are testing different things, so they do not share one.
    async fn join_recording(host: &Host, name: &str, group: &str) -> Self {
        let mut visitor = Self::join_inner(host, false).await;
        visitor.set_profile(name, group).await;
        visitor.install_reader();
        visitor
    }

    async fn join_inner(host: &Host, record_pushes: bool) -> Self {
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

        let ticket_body = http
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
            .expect("ticket json");
        let ticket = ticket_body["ticket"]
            .as_str()
            .expect("ticket value")
            .to_string();
        // The link is minted per ticket, so two tabs on one cookie are two
        // clients. The share routes resolve the caller from it.
        let link_id = ticket_body["linkId"]
            .as_str()
            .expect("ticket linkId")
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
        let pushes = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let socket = if record_pushes {
            let push_sink = pushes.clone();
            tokio::spawn(async move {
                while let Some(Ok(message)) = socket.next().await {
                    if let Message::Text(text) = message {
                        push_sink.lock().await.push(text.to_string());
                    }
                }
            });
            None
        } else {
            Some(socket)
        };
        Self {
            cookie,
            csrf,
            link_id,
            socket,
            pushes,
        }
    }

    /// Hand this tab's socket to a reader that records every pushed frame.
    ///
    /// The handshake frames already consumed are gone, and from here on
    /// anything the host pushes is recorded where a test can see it: a second
    /// reader would race the real one, and a test that inferred a push from a
    /// listing would pass even with the push removed.
    fn install_reader(&mut self) {
        let Some(mut socket) = self.socket.take() else {
            return;
        };
        let push_sink = self.pushes.clone();
        tokio::spawn(async move {
            while let Some(Ok(message)) = socket.next().await {
                if let Message::Text(text) = message {
                    push_sink.lock().await.push(text.to_string());
                }
            }
        });
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
            .header("x-conex-link-id", &self.link_id)
    }

    /// The download route, not the listing route: the two are different paths
    /// and a query string on the listing route would simply be ignored. The
    /// link rides in the query here because this is what a plain `<a href>`
    /// does; the host checks it against the session all the same.
    fn download(&self, host: &Host, id: &str) -> reqwest::RequestBuilder {
        reqwest::Client::new()
            .get(format!("http://{}/web/files/download", host.addr))
            .query(&[("id", id), ("linkId", &self.link_id)])
            .header("cookie", &self.cookie)
            .header("origin", format!("http://{}", host.addr))
    }

    /// The listing's validator: what makes an unchanged poll a `304` instead of
    /// a whole body.
    async fn etag(&self, host: &Host) -> Option<String> {
        self.http(host)
            .send()
            .await
            .expect("list files")
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
    }

    /// The listing with a validator attached, so the status says whether the
    /// host thinks it changed.
    async fn list_status(&self, host: &Host, etag: &str) -> reqwest::StatusCode {
        reqwest::Client::new()
            .get(format!("http://{}/web/files", host.addr))
            .header("cookie", &self.cookie)
            .header("origin", format!("http://{}", host.addr))
            .header("x-conex-link-id", &self.link_id)
            .header("if-none-match", etag)
            .send()
            .await
            .expect("list files")
            .status()
    }

    /// Withdraw one item this tab owns.
    async fn withdraw(&self, host: &Host, id: &str) -> Result<(), String> {
        let response = reqwest::Client::new()
            .post(format!("http://{}/web/files/remove", host.addr))
            .header("cookie", &self.cookie)
            .header("origin", format!("http://{}", host.addr))
            .header("x-conex-link-id", &self.link_id)
            .header("x-csrf-token", &self.csrf)
            .json(&json!({ "id": id }))
            .send()
            .await
            .expect("withdraw");
        if response.status() == reqwest::StatusCode::NO_CONTENT {
            Ok(())
        } else {
            Err(format!("withdraw answered {}", response.status()))
        }
    }

    /// Wait for a pushed frame matching `wanted`, including one that already
    /// arrived: a push that landed before the assertion runs is not a miss.
    async fn wait_for_push(&self, wanted: impl Fn(&str) -> bool) -> Option<String> {
        for _ in 0..200 {
            let recorded = self.pushes.lock().await.clone();
            if let Some(frame) = recorded.iter().find(|frame| wanted(frame)) {
                return Some(frame.clone());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        None
    }

    /// How many frames the host has pushed at this tab.
    async fn push_count(&self) -> usize {
        self.pushes.lock().await.len()
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
            .header("x-conex-link-id", &self.link_id)
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
    // The group text is the join secret: stats rows carry the key only, the
    // label is always empty on the wire.
    assert_eq!(team["label"].as_str(), Some(""));
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
        // Bob's own link: he is a real client here, and ownership — not his
        // mere session — is what must refuse him.
        .header("x-conex-link-id", &bob.link_id)
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
        .header("x-conex-link-id", &alice.link_id)
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
    let cap = conex_host::share::ShareLimits::default().max_file_bytes;
    let oversize = vec![b'x'; (cap as usize) + 1];
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

/// The download route, not the listing route: a query string on the listing
/// route would simply be ignored. Used by the range assertions below.
fn ranged(visitor: &Visitor, host: &Host, id: &str, range: &str) -> reqwest::RequestBuilder {
    reqwest::Client::new()
        .get(format!("http://{}/web/files/download", host.addr))
        .query(&[
            ("id", id.to_string()),
            ("linkId", visitor.link_id.clone()),
            ("range", range.to_string()),
        ])
        .header("cookie", &visitor.cookie)
        .header("origin", format!("http://{}", host.addr))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_preview_reads_only_the_head_the_host_offered() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    let mut bob = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    bob.set_profile("bob", "team-a").await;

    // Big enough to be more than the host's preview budget, and mixed bytes so
    // a wrong offset cannot accidentally match.
    let payload: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
    let uploaded = alice
        .upload(&host, "big.bin", "text/plain", payload.clone())
        .await
        .expect("upload");
    assert_eq!(uploaded.status(), reqwest::StatusCode::OK);
    let body = uploaded.json::<Value>().await.expect("upload json");
    let file_id = body["file"]["id"].as_str().expect("file id").to_string();
    // The listing carries the host's own preview budget, and the page renders
    // the limit from it rather than hard-coding one.
    let bob_listing = bob.list(&host).await;
    let row = &bob_listing["files"][0];
    let preview_bytes: u64 = row["previewBytes"]
        .as_str()
        .expect("previewBytes")
        .parse()
        .expect("number");
    assert!(preview_bytes > 0);
    assert!(
        preview_bytes < payload.len() as u64,
        "a preview is a head, not the whole item"
    );
    assert_eq!(
        row["size"].as_str(),
        Some(payload.len().to_string().as_str())
    );

    // Asking for exactly that budget reads exactly those bytes.
    let head = ranged(
        &bob,
        &host,
        &file_id,
        &format!("bytes=0-{}", preview_bytes - 1),
    )
    .send()
    .await
    .expect("preview");
    assert_eq!(head.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        head.headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok()),
        Some(format!("bytes 0-{}/{}", preview_bytes - 1, payload.len()).as_str())
    );
    assert_eq!(
        head.headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok()),
        Some(preview_bytes.to_string().as_str())
    );
    assert_eq!(
        head.bytes().await.expect("bytes").to_vec(),
        payload[..preview_bytes as usize]
    );

    // A suffix range answers the tail, and the open range answers the rest.
    let tail = ranged(&bob, &host, &file_id, "bytes=-4")
        .send()
        .await
        .expect("tail");
    assert_eq!(tail.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        tail.bytes().await.expect("bytes").to_vec(),
        payload[payload.len() - 4..]
    );

    // No range at all answers the whole item, which is what a download is.
    let whole = bob
        .download(&host, &file_id)
        .send()
        .await
        .expect("download");
    assert_eq!(whole.status(), reqwest::StatusCode::OK);
    assert!(whole.headers().get("content-range").is_none());
    assert_eq!(whole.bytes().await.expect("bytes").to_vec(), payload);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_unparsable_range_reads_the_whole_item_instead_of_an_error() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    let payload = b"plain bytes".to_vec();
    let uploaded = alice
        .upload(&host, "notes.txt", "text/plain", payload.clone())
        .await
        .expect("upload");
    let body = uploaded.json::<Value>().await.expect("upload json");
    let file_id = body["file"]["id"].as_str().expect("file id").to_string();

    // Multi-range and malformed values are answered as a whole item (RFC
    // 7233), so a curious header is not a distinguishable probe.
    for range in ["bytes=0-1,3-4", "bytes=abc", "bytes=-", "bytes="] {
        let response = ranged(&alice, &host, &file_id, range)
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{range}");
        assert_eq!(response.bytes().await.expect("bytes").to_vec(), payload);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_range_past_the_end_is_answered_416_with_the_total() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    let payload = b"plain bytes".to_vec();
    let uploaded = alice
        .upload(&host, "notes.txt", "text/plain", payload)
        .await
        .expect("upload");
    let body = uploaded.json::<Value>().await.expect("upload json");
    let file_id = body["file"]["id"].as_str().expect("file id").to_string();

    // A well-formed range that no byte satisfies says so, with the total, so a
    // client can tell how far past the end it asked. This is not the same
    // thing as a malformed value: that one is answered as a whole item above.
    let refused = ranged(&alice, &host, &file_id, "bytes=999999-")
        .send()
        .await
        .expect("request");
    assert_eq!(refused.status(), reqwest::StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        refused
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok()),
        Some("bytes */11")
    );
    assert_eq!(
        refused
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok()),
        Some("bytes")
    );
    // A suffix range of zero bytes has nothing to answer either.
    let empty = ranged(&alice, &host, &file_id, "bytes=-0")
        .send()
        .await
        .expect("request");
    assert_eq!(empty.status(), reqwest::StatusCode::RANGE_NOT_SATISFIABLE);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_item_carries_an_etag_that_if_range_is_checked_against() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    let payload = b"0123456789".to_vec();
    let uploaded = alice
        .upload(&host, "notes.txt", "text/plain", payload.clone())
        .await
        .expect("upload");
    let body = uploaded.json::<Value>().await.expect("upload json");
    let file_id = body["file"]["id"].as_str().expect("file id").to_string();

    let whole = alice
        .download(&host, &file_id)
        .send()
        .await
        .expect("download");
    let etag = whole
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .expect("etag")
        .to_string();
    // The validator names the content, and it is stable across requests.
    assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");
    let again = alice
        .download(&host, &file_id)
        .send()
        .await
        .expect("download");
    assert_eq!(
        again.headers().get("etag").and_then(|v| v.to_str().ok()),
        Some(etag.as_str())
    );

    let client = reqwest::Client::new();
    let origin = format!("http://{}", host.addr);
    let ranged_with = |range: &str, if_range: Option<&str>| {
        let mut request = client
            .get(format!("{origin}/web/files/download"))
            .query(&[
                ("id", file_id.clone()),
                ("linkId", alice.link_id.clone()),
                ("range", range.to_string()),
            ])
            .header("cookie", &alice.cookie)
            .header("origin", &origin);
        if let Some(condition) = if_range {
            request = request.header("if-range", condition);
        }
        request
    };
    // Matching validator: the range is answered as a range.
    let matched = ranged_with("bytes=0-3", Some(&etag))
        .send()
        .await
        .expect("matched");
    assert_eq!(matched.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(matched.bytes().await.expect("bytes").to_vec(), b"0123");

    // A validator naming other content: the range is ignored and the whole item
    // is answered, which is what stops a resumed element from stitching bytes
    // out of two different contents.
    let stale = ranged_with("bytes=0-3", Some("\"something-else\""))
        .send()
        .await
        .expect("stale");
    assert_eq!(stale.status(), reqwest::StatusCode::OK);
    assert_eq!(stale.bytes().await.expect("bytes").to_vec(), payload);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_large_download_streams_block_by_block() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    // Several blocks, so a stream is doing more than one read. The content is a
    // counter so a wrong block order or a wrong boundary cannot pass by luck.
    let payload: Vec<u8> = (0..(256 * 1024 * 4u32)).map(|i| (i % 251) as u8).collect();
    let uploaded = alice
        .upload(
            &host,
            "stream.bin",
            "application/octet-stream",
            payload.clone(),
        )
        .await
        .expect("upload");
    let body = uploaded.json::<Value>().await.expect("upload json");
    let file_id = body["file"]["id"].as_str().expect("file id").to_string();

    let download = alice
        .download(&host, &file_id)
        .send()
        .await
        .expect("download");
    assert_eq!(download.status(), reqwest::StatusCode::OK);
    // The declared length is what a seekable element needs: without a total,
    // `<video>`/`<audio>` clamp `currentTime` to 0 even though ranges work.
    assert_eq!(
        download
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok()),
        Some(payload.len().to_string().as_str())
    );
    assert_eq!(download.bytes().await.expect("bytes").to_vec(), payload);

    // A range spanning two block boundaries, read as part of the same stream.
    let middle = ranged(&alice, &host, &file_id, "bytes=262100-262300")
        .send()
        .await
        .expect("middle");
    assert_eq!(middle.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        middle.bytes().await.expect("bytes").to_vec(),
        payload[262_100..=262_300]
    );

    // The first byte of the item, which is inside the first block only.
    let first = ranged(&alice, &host, &file_id, "bytes=0-0")
        .send()
        .await
        .expect("first");
    assert_eq!(
        first.bytes().await.expect("bytes").to_vec(),
        vec![payload[0]]
    );
}

/// A push replaces a poll: the peer's tab is told, and does not have to ask.
///
/// Asserted on the frame the peer's socket actually receives, because a listing
/// that eventually updates would pass even with the push removed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_upload_pushes_to_the_peers_socket_and_a_poll_answers_304() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    let bob = Visitor::join_recording(&host, "bob", "team-a").await;
    // Something shared first: an empty group has no listing to validate, so
    // there is nothing for a validator to be unchanged against.
    alice
        .upload(&host, "notes.txt", "text/plain", b"pushed".to_vec())
        .await
        .expect("upload");
    let listing = bob.list(&host).await;
    assert_eq!(listing["files"].as_array().expect("files").len(), 1);
    // The validator is what makes an unchanged poll cheap.
    let etag = bob.etag(&host).await.expect("etag");
    let unchanged = bob.list_status(&host, &etag).await;
    assert_eq!(unchanged, reqwest::StatusCode::NOT_MODIFIED);

    // Wait for the first write's push to be *recorded* before taking a
    // baseline: the reader is a separate task, so counting before it has run
    // makes the next write's push look like two.
    assert!(
        bob.wait_for_push(|frame| frame.contains("conex/share-changed"))
            .await
            .is_some(),
        "the first upload is pushed"
    );
    let before = bob.push_count().await;
    // Alice offers a second file. Bob's tab is told rather than polling for it.
    alice
        .upload(&host, "again.txt", "text/plain", b"pushed again".to_vec())
        .await
        .expect("upload");
    let frame = bob
        .wait_for_push(|frame| frame.contains("conex/share-changed"))
        .await
        .expect("a push arrives");
    assert!(
        !frame.contains("team-a"),
        "a push carries no group text: {frame}"
    );
    // One write is one push: the actor's own tab is not told about its own
    // write, and neither is anybody twice.
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(bob.push_count().await, before + 1, "one write is one push");

    // And the validator moved with the listing, so the next poll re-reads it.
    let after = bob.list_status(&host, &etag).await;
    assert_eq!(after, reqwest::StatusCode::OK, "the listing changed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_withdrawal_is_pushed_too() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    let bob = Visitor::join_recording(&host, "bob", "team-a").await;
    let uploaded = alice
        .upload(&host, "notes.txt", "text/plain", b"then withdrawn".to_vec())
        .await
        .expect("upload");
    let body = uploaded.json::<Value>().await.expect("upload json");
    let file_id = body["file"]["id"].as_str().expect("file id").to_string();
    assert!(
        bob.wait_for_push(|frame| frame.contains("conex/share-changed"))
            .await
            .is_some(),
        "the upload is pushed"
    );

    alice.withdraw(&host, &file_id).await.expect("withdraw");
    let withdrawn = bob.push_count().await;
    assert!(
        bob.wait_for_push(|frame| frame.contains("conex/share-changed"))
            .await
            .is_some(),
        "the withdrawal is pushed too"
    );
    assert_eq!(
        bob.push_count().await,
        withdrawn,
        "and it is one push, not one per row"
    );
    // The listing the peer now reads is empty, which is what the push told it.
    let listing = bob.list(&host).await;
    assert!(listing["files"].as_array().expect("files").is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_peer_in_another_group_is_not_pushed_to() {
    let host = start().await;
    let mut alice = Visitor::join(&host).await;
    alice.set_profile("alice", "team-a").await;
    let outsider = Visitor::join_recording(&host, "mallory", "team-b").await;
    alice
        .upload(&host, "secret.txt", "text/plain", b"not for you".to_vec())
        .await
        .expect("upload");
    assert!(
        outsider
            .wait_for_push(|frame| frame.contains("conex/share-changed"))
            .await
            .is_none(),
        "another group must not be told about a scene it cannot see"
    );
    // ...and its own listing is unchanged by the write.
    let listing = outsider.list(&host).await;
    assert!(listing["files"].as_array().expect("files").is_empty());
}

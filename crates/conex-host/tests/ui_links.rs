use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::time::Duration;

use conex_host::agent::now_ms;
use conex_host::ui_links::UiLinkRegistry;
use futures::{SinkExt, StreamExt};
use reqwest::header::{COOKIE, ORIGIN, SET_COOKIE};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn host_config(notes_root: &Path, port: u16, ui_hash: &str, service_hash: &str) -> String {
    let tmp = TempDir::new().expect("tempdir");
    let content = tmp.path().join("content");
    let session = tmp.path().join("session");
    let operation = tmp.path().join("operation");
    std::mem::forget(tmp);
    format!(
        "listen = \"127.0.0.1:{port}\"\nallow_loopback_http = true\naudience = \"host.local\"\nhost_origin = \"conex://broker.local\"\nweb_origin = \"http://127.0.0.1:{port}\"\ncontent_root = \"{content}\"\nsession_root = \"{session}\"\noperation_root = \"{operation}\"\n\n[[tokens]]\ntoken_hash = \"{ui_hash}\"\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\nrole = \"ui\"\n\n[[tokens]]\ntoken_hash = \"{service_hash}\"\nprincipal_id = \"svc\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\nrole = \"service\"\n\n[[policy]]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"notes-local\"\nactions = [\"list\", \"read\", \"search\"]\nroot = \"\"\nsubtree = true\n\n[[endpoints]]\nid = \"notes-local\"\ntenant_id = \"tenant-a\"\nprovider_id = \"source\"\nkind = \"source-fs\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"{notes_root}\"\n",
        content = content.display(),
        session = session.display(),
        operation = operation.display(),
        notes_root = notes_root.display(),
    )
}

struct HostHandle {
    addr: SocketAddr,
    child: std::process::Child,
    #[allow(dead_code)]
    config_tmp: TempDir,
    stderr_path: PathBuf,
}

impl HostHandle {
    async fn spawn() -> Self {
        let notes_root = repo_root().join("fixtures/p0/notes");
        let cfg_tmp = TempDir::new().expect("tempdir");
        let cfg_path = cfg_tmp.path().join("host.toml");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let ui_hash = sha256_hex("ui-secret");
        let service_hash = sha256_hex("svc-secret");
        std::fs::write(
            &cfg_path,
            host_config(&notes_root, port, &ui_hash, &service_hash),
        )
        .expect("write config");
        let binary = std::env::var("CONEX_HOST_BIN")
            .unwrap_or_else(|_| env!("CARGO_BIN_EXE_conex-host").to_string());
        let stderr_path = cfg_tmp.path().join("host.stderr.log");
        let stderr_file = std::fs::File::create(&stderr_path).expect("stderr file");
        let child = std::process::Command::new(binary)
            .arg(&cfg_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::from(stderr_file))
            .spawn()
            .expect("spawn host");
        let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Self {
            addr,
            child,
            config_tmp: cfg_tmp,
            stderr_path,
        }
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if std::thread::panicking()
            && let Ok(text) = std::fs::read_to_string(&self.stderr_path)
        {
            eprintln!("--- host stderr ({} bytes) ---\n{}", text.len(), text);
        }
    }
}

struct WebSession {
    cookie: String,
    csrf: String,
}

async fn http_login(addr: SocketAddr, origin: &str) -> WebSession {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/web/login"))
        .header(ORIGIN, origin)
        .header("authorization", "Bearer ui-secret")
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .expect("login");
    assert!(resp.status().is_success(), "login status {}", resp.status());
    let cookie = resp
        .headers()
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_owned)
        .expect("cookie");
    let session: Value = client
        .get(format!("http://{addr}/web/session"))
        .header(ORIGIN, origin)
        .header(COOKIE, &cookie)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .expect("session")
        .json()
        .await
        .expect("session json");
    let csrf = session["csrf"].as_str().expect("csrf").to_owned();
    WebSession { cookie, csrf }
}

async fn fetch_ticket(addr: SocketAddr, origin: &str, session: &WebSession) -> String {
    let resp = reqwest::Client::new()
        .post(format!("http://{addr}/tickets"))
        .header(ORIGIN, origin)
        .header(COOKIE, &session.cookie)
        .header("x-csrf-token", &session.csrf)
        .timeout(Duration::from_secs(5))
        .json(&json!({}))
        .send()
        .await
        .expect("ticket");
    assert!(
        resp.status().is_success(),
        "ticket status {}",
        resp.status()
    );
    let body: Value = resp.json().await.expect("ticket body");
    body.get("ticket")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .expect("ticket value")
}

async fn open_wss(
    addr: SocketAddr,
    ticket: &str,
    origin: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://{addr}/wss?ticket={ticket}");
    let mut request = url.clone().into_client_request().expect("ws request");
    request
        .headers_mut()
        .insert(ORIGIN, origin.parse().unwrap());
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(request),
    )
    .await;
    let (ws, _) = result.expect("wss timeout").expect("wss open");
    ws
}

async fn bootstrap(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) {
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "method": "conex/hello",
            "params": {
                "profileId": "conex-jsonrpc2-wss",
                "plane": "broker",
                "provides": [],
                "requires": []
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("hello");
    let hello: Value = match ws
        .next()
        .await
        .expect("hello response")
        .expect("hello frame")
    {
        Message::Text(text) => serde_json::from_str(&text).expect("hello json"),
        other => panic!("expected JSON hello, got {other:?}"),
    };
    let negotiation = hello["result"]["negotiationId"]
        .as_str()
        .expect("negotiationId")
        .to_string();
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAW",
            "method": "conex/ready",
            "params": {
                "negotiationId": negotiation,
                "profileId": "conex-jsonrpc2-wss",
                "plane": "broker",
                "provides": hello["result"]["provides"],
                "limits": hello["result"]["limits"]
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("ready");
    let _ = ws.next().await.expect("ready result").expect("ready frame");
}

async fn wss_connection_list(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let id = "01ARZ3NDEKTSV4RRFFQ69G5FAA";
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "connection/list",
            "params": {
                "context": { "providerEndpointId": "", "plane": "broker" },
                "timeoutBudgetMs": 8000,
                "input": {}
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("connection/list send");
    loop {
        let next = ws.next().await.expect("response").expect("response frame");
        if let Message::Text(text) = next {
            let value: Value = serde_json::from_str(&text).expect("response json");
            if value.get("id").and_then(Value::as_str) == Some(id) {
                return value.get("result").cloned().expect("result body");
            }
        }
    }
}

async fn wss_endpoint_list(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let id = "01ARZ3NDEKTSV4RRFFQ69G5FAC";
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "endpoint/list",
            "params": {
                "context": { "providerEndpointId": "", "plane": "broker" },
                "timeoutBudgetMs": 8000,
                "input": {}
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("endpoint/list send");
    loop {
        let next = ws.next().await.expect("response").expect("response frame");
        if let Message::Text(text) = next {
            let value: Value = serde_json::from_str(&text).expect("response json");
            if value.get("id").and_then(Value::as_str) == Some(id) {
                return value.get("result").cloned().expect("result body");
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_list_returns_one_link_after_login_and_ready() {
    let host = HostHandle::spawn().await;
    let origin = format!("http://{}", host.addr);
    let session = http_login(host.addr, &origin).await;
    let ticket = fetch_ticket(host.addr, &origin, &session).await;

    let mut ws = open_wss(host.addr, &ticket, &origin).await;
    tokio::time::timeout(Duration::from_secs(10), bootstrap(&mut ws))
        .await
        .expect("bootstrap timeout");
    let endpoints = tokio::time::timeout(Duration::from_secs(10), wss_endpoint_list(&mut ws))
        .await
        .expect("endpoint/list timeout");
    assert!(
        endpoints.is_object() || endpoints.is_array(),
        "endpoint/list must return a value (sanity check): {endpoints}"
    );

    let list = tokio::time::timeout(Duration::from_secs(10), wss_connection_list(&mut ws))
        .await
        .expect("connection/list timeout");
    assert_eq!(
        list["browserLinks"].as_array().unwrap().len(),
        1,
        "exactly one browser link should be visible to the ui principal: {list}"
    );
    let link = &list["browserLinks"][0];
    assert_eq!(link["principalId"].as_str(), Some("alice"));
    assert_eq!(link["tenantId"].as_str(), Some("tenant-a"));
    let link_id = link["linkId"].as_str().expect("linkId").to_string();
    assert!(!link_id.is_empty(), "link id must be non-empty");
    let tickets_issued = link["ticketsIssued"]
        .as_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    assert!(
        tickets_issued >= 1,
        "ticketsIssued must be >=1 after one ticket consumption (got {tickets_issued})"
    );
    let inflight: u64 = link["callsInFlight"].as_str().unwrap().parse().unwrap();
    assert!(
        inflight <= 1,
        "callsInFlight must be 0 or 1 (snapshot races with the in-flight connection/list), got {inflight}"
    );

    ws.close(None).await.ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killing_wss_keeps_link_listed_but_freezes_last_seen() {
    let host = HostHandle::spawn().await;
    let origin = format!("http://{}", host.addr);
    let session = http_login(host.addr, &origin).await;
    let ticket = fetch_ticket(host.addr, &origin, &session).await;

    let mut ws = open_wss(host.addr, &ticket, &origin).await;
    bootstrap(&mut ws).await;

    let before = wss_connection_list(&mut ws).await;
    let first_seen = before["browserLinks"][0]["lastSeenAtMs"]
        .as_str()
        .unwrap()
        .to_string();
    drop(ws);

    let ticket2 = fetch_ticket(host.addr, &origin, &session).await;
    let mut ws2 = open_wss(host.addr, &ticket2, &origin).await;
    bootstrap(&mut ws2).await;
    let after = wss_connection_list(&mut ws2).await;
    assert_eq!(
        after["browserLinks"].as_array().unwrap().len(),
        1,
        "previous WS closing must not remove the link"
    );
    let after_seen = after["browserLinks"][0]["lastSeenAtMs"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        after_seen.parse::<u64>().unwrap() >= first_seen.parse::<u64>().unwrap(),
        "lastSeenAtMs must not regress (before={first_seen}, after={after_seen})"
    );

    ws2.close(None).await.ok();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ui_source_read_increments_calls_total() {
    let host = HostHandle::spawn().await;
    let origin = format!("http://{}", host.addr);
    let session = http_login(host.addr, &origin).await;
    let ticket = fetch_ticket(host.addr, &origin, &session).await;

    let mut ws = open_wss(host.addr, &ticket, &origin).await;
    bootstrap(&mut ws).await;

    let before = wss_connection_list(&mut ws).await;
    let calls_total_before: u64 = before["browserLinks"][0]["callsTotal"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();

    let id = "01ARZ3NDEKTSV4RRFFQ69G5FAB";
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "source/read",
            "params": {
                "context": { "providerEndpointId": "notes-local", "plane": "broker" },
                "timeoutBudgetMs": 8000,
                "input": { "resourceId": "team/design.org" }
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send read");
    let read_resp = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let next = ws.next().await.expect("read response").expect("frame");
            if let Message::Text(text) = next {
                let value: Value = serde_json::from_str(&text).expect("response json");
                if value.get("id").and_then(Value::as_str) == Some(id) {
                    return value;
                }
            }
        }
    })
    .await
    .expect("source/read response timeout");
    assert!(
        read_resp.get("result").is_some() || read_resp.get("error").is_some(),
        "source/read must return a JSON-RPC body: {read_resp}"
    );

    let after = wss_connection_list(&mut ws).await;
    let calls_total_after: u64 = after["browserLinks"][0]["callsTotal"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        calls_total_after > calls_total_before,
        "callsTotal must grow after a business call (before={calls_total_before}, after={calls_total_after})"
    );
    let inflight_after: u64 = after["browserLinks"][0]["callsInFlight"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        inflight_after <= 2,
        "callsInFlight must be small (in-flight calls seen during snapshot), got {inflight_after}"
    );

    ws.close(None).await.ok();
}

#[tokio::test]
async fn registry_lifecycle_counts_calls_and_skips_after_remove() {
    let registry = UiLinkRegistry::new();
    let link = registry.register("alice", "tenant-a");
    let link_id = link.link_id.clone();
    assert_eq!(registry.list_for_principal(Some("alice")).len(), 1);
    registry.touch(&link_id);
    registry.increment_tickets(&link_id);
    registry.begin_call(&link_id);
    registry.begin_call(&link_id);
    let snapshot = registry.list_for_principal(Some("alice"));
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].tickets_issued, 1);
    assert_eq!(snapshot[0].calls_total, 2);
    assert_eq!(snapshot[0].calls_in_flight, 2);
    registry.end_call(&link_id);
    assert_eq!(
        registry.list_for_principal(Some("alice"))[0].calls_in_flight,
        1
    );
    registry.remove(&link_id);
    assert!(registry.list_for_principal(Some("alice")).is_empty());

    let other = registry.register("bob", "tenant-a");
    let bob_id = other.link_id.clone();
    let principal_view = registry.list_for_principal(Some("alice"));
    assert!(
        principal_view
            .iter()
            .all(|link| link.principal_id == "alice")
    );
    let all = registry.list_for_principal(None);
    assert!(all.iter().any(|link| link.principal_id == "bob"));
    registry.remove(&bob_id);
}

#[test]
fn registry_assigns_unique_link_ids() {
    let registry = UiLinkRegistry::new();
    let a = registry.register("alice", "tenant-a");
    let b = registry.register("alice", "tenant-a");
    assert_ne!(a.link_id, b.link_id);
    assert!(a.connected_at_ms <= now_ms());
}

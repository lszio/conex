#![forbid(unsafe_code)]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use axum::serve;
use conex_host::serve::build_router;
use reqwest::StatusCode;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::net::TcpListener;

fn sha256_hex(input: &str) -> String { hex::encode(Sha256::digest(input.as_bytes())) }

fn config_text(root: &std::path::Path, ui_hash: &str, service_hash: &str) -> String {
    let content = root.join("content");
    let session = root.join("session");
    let operation = root.join("operation");
    let notes = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/p0/notes");
    format!(
        "listen = \"127.0.0.1:0\"\nallow_loopback_http = true\naudience = \"host.local\"\nweb_origin = \"http://127.0.0.1:0\"\ncontent_root = \"{}\"\nsession_root = \"{}\"\noperation_root = \"{}\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\nrole = \"ui\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"svc\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\nrole = \"service\"\n\n[[policy]]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"notes-local\"\nactions = [\"list\", \"read\", \"search\"]\nroot = \"\"\nsubtree = true\n\n[[endpoints]]\nid = \"notes-local\"\ntenant_id = \"tenant-a\"\nprovider_id = \"source\"\nkind = \"source-fs\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"{}\"\n",
        content.display(), session.display(), operation.display(), ui_hash, service_hash, notes.display()
    )
}

async fn spawn_host(config: &str, root: &std::path::Path) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let config = config.replace("127.0.0.1:0", &addr.to_string());
    let path = root.join("host.toml");
    std::fs::write(&path, &config).expect("write config");
    let config = conex_host::config::HostConfig::load(&path).expect("load config");
    let app = build_router(&config).expect("build router");
    let task = tokio::spawn(async move { serve(listener, app).await.expect("serve"); });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (addr, task)
}

fn cookie_pair(response: &reqwest::Response) -> String {
    response.headers().get("set-cookie").and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next()).expect("cookie").to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn ui_login_creates_http_only_session_cookie() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(root.path(), &sha256_hex("ui-secret"), &sha256_hex("svc-secret"));
    let (addr, task) = spawn_host(&config, root.path()).await;
    let client = reqwest::Client::new();
    let response = client.post(format!("http://{addr}/web/login"))
        .header("origin", format!("http://{addr}"))
        .header("authorization", "Bearer ui-secret").send().await.expect("login request");
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers().get("set-cookie").and_then(|v| v.to_str().ok()).expect("session cookie");
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    let body: Value = response.json().await.expect("login json");
    assert_eq!(body["principalId"], "alice");
    task.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn ui_bearer_cannot_call_rpc_without_session() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(root.path(), &sha256_hex("ui-secret"), &sha256_hex("svc-secret"));
    let (addr, task) = spawn_host(&config, root.path()).await;
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/rpc"))
        .header("content-type", "application/json")
        .header("authorization", "Bearer ui-secret")
        .body("{}")
        .send().await.expect("rpc");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn session_csrf_derives_ticket_identity_and_rejects_wrong_role() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(root.path(), &sha256_hex("ui-secret"), &sha256_hex("svc-secret"));
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    let wrong = client.post(format!("{origin}/web/login")).header("origin", &origin).header("authorization", "Bearer svc-secret").send().await.expect("wrong-role login");
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let forged = client.post(format!("{origin}/tickets"))
        .header("authorization", "Bearer svc-secret")
        .json(&serde_json::json!({"principalId":"attacker","tenantId":"other","origin":"http://evil","targetHost":"host","peerRole":"ui"}))
        .send().await.expect("forged bearer ticket");
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    let login = client.post(format!("{origin}/web/login")).header("origin", &origin).header("authorization", "Bearer ui-secret").send().await.expect("login");
    let cookie = cookie_pair(&login);
    let session: Value = client.get(format!("{origin}/web/session")).header("cookie", &cookie).send().await.expect("session").json().await.expect("session json");
    let csrf = session["csrf"].as_str().expect("csrf");
    let ticket: Value = client.post(format!("{origin}/tickets")).header("origin", &origin).header("cookie", &cookie).header("x-csrf-token", csrf).json(&serde_json::json!({"principalId":"attacker","tenantId":"other","peerRole":"agent","capabilityCaps":["blob/get"]})).send().await.expect("ticket").json().await.expect("ticket json");
    assert_eq!(ticket["principalId"], "alice");
    assert_eq!(ticket["tenantId"], "tenant-a");
    assert_eq!(ticket["peerRole"], "ui");
    assert!(ticket["ticket"].as_str().is_some());
    let no_csrf = client.post(format!("{origin}/tickets")).header("origin", &origin).header("cookie", &cookie).json(&serde_json::json!({})).send().await.expect("missing csrf");
    assert_eq!(no_csrf.status(), StatusCode::UNAUTHORIZED);
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn logout_invalidates_cookie_and_unconsumed_tickets() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(root.path(), &sha256_hex("ui-secret"), &sha256_hex("svc-secret"));
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    let login = client.post(format!("{origin}/web/login")).header("origin", &origin).header("authorization", "Bearer ui-secret").send().await.expect("login");
    let cookie = cookie_pair(&login);
    let session: Value = client.get(format!("{origin}/web/session")).header("cookie", &cookie).send().await.expect("session").json().await.expect("session json");
    let csrf = session["csrf"].as_str().unwrap();
    let ticket = client.post(format!("{origin}/tickets")).header("origin", &origin).header("cookie", &cookie).header("x-csrf-token", csrf).json(&serde_json::json!({})).send().await.expect("ticket");
    assert_eq!(ticket.status(), StatusCode::CREATED);
    let logout = client.post(format!("{origin}/web/logout")).header("origin", &origin).header("cookie", &cookie).header("x-csrf-token", csrf).send().await.expect("logout");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    let old_session = client.get(format!("{origin}/web/session")).header("cookie", &cookie).send().await.expect("old session");
    assert_eq!(old_session.status(), StatusCode::UNAUTHORIZED);
    task.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn web_sessions_enforce_per_principal_cap() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(root.path(), &sha256_hex("ui-secret"), &sha256_hex("svc-secret"));
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    for _ in 0..8 {
        let response = client.post(format!("{origin}/web/login"))
            .header("origin", &origin)
            .header("authorization", "Bearer ui-secret")
            .send().await.expect("login");
        assert_eq!(response.status(), StatusCode::OK);
    }
    let overflow = client.post(format!("{origin}/web/login"))
        .header("origin", &origin)
        .header("authorization", "Bearer ui-secret")
        .send().await.expect("overflow login");
    assert_eq!(overflow.status(), StatusCode::TOO_MANY_REQUESTS);
    task.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn ticket_only_websocket_upgrade_works_and_ambiguous_auth_fails() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(root.path(), &sha256_hex("ui-secret"), &sha256_hex("svc-secret"));
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    let login = client.post(format!("{origin}/web/login")).header("origin", &origin).header("authorization", "Bearer ui-secret").send().await.expect("login");
    let cookie = cookie_pair(&login);
    let session: Value = client.get(format!("{origin}/web/session")).header("cookie", &cookie).send().await.expect("session").json().await.expect("session json");
    let csrf = session["csrf"].as_str().unwrap();
    let ticket: Value = client.post(format!("{origin}/tickets")).header("origin", &origin).header("cookie", &cookie).header("x-csrf-token", csrf).json(&serde_json::json!({})).send().await.expect("ticket").json().await.expect("ticket json");
    let ticket_value = ticket["ticket"].as_str().unwrap();
    let ws_url = format!("ws://{addr}/wss?ticket={ticket_value}");
    let mut request = ws_url.clone().into_client_request().expect("ws request");
    request.headers_mut().insert("Origin", origin.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request).await.expect("ticket-only websocket");
    ws.send(Message::Text(serde_json::json!({"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","method":"conex/hello","params":{"profileId":"conex-jsonrpc2-wss-v1","plane":"broker","provides":[],"requires":[]}}).to_string().into())).await.expect("hello");
    let hello = ws.next().await.expect("hello response").expect("hello frame");
    let hello: Value = match hello {
        Message::Text(text) => serde_json::from_str(&text).expect("hello json"),
        other => panic!("expected JSON hello response, got {other:?}"),
    };
    let provides = hello["result"]["provides"].as_array().expect("provides");
    assert!(!provides.iter().any(|value| value.as_str() == Some("blob/get")));
    let mut ambiguous = ws_url.into_client_request().expect("ambiguous request");
    ambiguous.headers_mut().insert("Origin", origin.parse().unwrap());
    ambiguous.headers_mut().insert("Authorization", "Bearer svc-secret".parse().unwrap());
    assert!(tokio_tungstenite::connect_async(ambiguous).await.is_err(), "ticket plus bearer must be rejected");
    task.abort();
}
#[test]
fn tickets_are_random_one_use_and_origin_bound() {
    let registry = conex_host::agent::TicketRegistry::new();
    let first = registry.issue("alice", "tenant-a", "http://ui.example", "host.example", "ui", vec!["source/read".into()], Some("session".into())).expect("first ticket");
    let second = registry.issue("alice", "tenant-a", "http://ui.example", "host.example", "ui", vec!["source/read".into()], Some("session".into())).expect("second ticket");
    assert_ne!(first.ticket, second.ticket);
    assert!(registry.consume_for(&first.ticket, "http://other.example", "host.example").is_err());
    assert!(registry.consume_for(&first.ticket, "http://ui.example", "host.example").is_ok());
    assert!(registry.consume_for(&first.ticket, "http://ui.example", "host.example").is_err());
    let agent = registry.issue("agent", "tenant-a", "http://ui.example", "host.example", "agent", Vec::new(), None).expect("agent ticket");
    assert_eq!(agent.peer_role, "agent");
}

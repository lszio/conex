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

fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

fn config_text(root: &std::path::Path, ui_hash: &str, service_hash: &str) -> String {
    config_text_with(root, ui_hash, service_hash, "", "")
}

fn config_text_with(
    root: &std::path::Path,
    ui_hash: &str,
    service_hash: &str,
    guest_section: &str,
    guest_policies: &str,
) -> String {
    let content = root.join("content");
    let session = root.join("session");
    let operation = root.join("operation");
    let notes = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/p0/notes");
    format!(
        "listen = \"127.0.0.1:0\"\nallow_loopback_http = true\naudience = \"host.local\"\nweb_origin = \"http://127.0.0.1:0\"\ncontent_root = \"{}\"\nsession_root = \"{}\"\noperation_root = \"{}\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\nrole = \"ui\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"svc\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\nrole = \"service\"\n\n[[policy]]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"notes-local\"\nactions = [\"list\", \"read\", \"search\"]\nroot = \"\"\nsubtree = true\n{guest_policies}{guest_section}\n[[endpoints]]\nid = \"notes-local\"\ntenant_id = \"tenant-a\"\nprovider_id = \"source\"\nkind = \"source-fs\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"{}\"\n",
        content.display(),
        session.display(),
        operation.display(),
        ui_hash,
        service_hash,
        notes.display()
    )
}

async fn spawn_host(
    config: &str,
    root: &std::path::Path,
) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let config = config.replace("127.0.0.1:0", &addr.to_string());
    let path = root.join("host.toml");
    std::fs::write(&path, &config).expect("write config");
    let config = conex_host::config::HostConfig::load(&path).expect("load config");
    let app = build_router(&config).expect("build router");
    let task = tokio::spawn(async move {
        serve(listener, app).await.expect("serve");
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (addr, task)
}

fn cookie_pair(response: &reqwest::Response) -> String {
    response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .expect("cookie")
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn ui_login_creates_http_only_session_cookie() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(
        root.path(),
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
    );
    let (addr, task) = spawn_host(&config, root.path()).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("http://{addr}/web/login"))
        .header("origin", format!("http://{addr}"))
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login request");
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .expect("session cookie");
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    let body: Value = response.json().await.expect("login json");
    assert_eq!(body["principalId"], "alice");
    task.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn ui_bearer_cannot_call_rpc_without_session() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(
        root.path(),
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
    );
    let (addr, task) = spawn_host(&config, root.path()).await;
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/rpc"))
        .header("content-type", "application/json")
        .header("authorization", "Bearer ui-secret")
        .body("{}")
        .send()
        .await
        .expect("rpc");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn session_csrf_derives_ticket_identity_and_rejects_wrong_role() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(
        root.path(),
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
    );
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    let wrong = client
        .post(format!("{origin}/web/login"))
        .header("origin", &origin)
        .header("authorization", "Bearer svc-secret")
        .send()
        .await
        .expect("wrong-role login");
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let forged = client.post(format!("{origin}/tickets"))
        .header("authorization", "Bearer svc-secret")
        .json(&serde_json::json!({"principalId":"attacker","tenantId":"other","origin":"http://evil","targetHost":"host","peerRole":"ui"}))
        .send().await.expect("forged bearer ticket");
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    let login = client
        .post(format!("{origin}/web/login"))
        .header("origin", &origin)
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login");
    let cookie = cookie_pair(&login);
    let session: Value = client
        .get(format!("{origin}/web/session"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("session")
        .json()
        .await
        .expect("session json");
    let csrf = session["csrf"].as_str().expect("csrf");
    let ticket: Value = client.post(format!("{origin}/tickets")).header("origin", &origin).header("cookie", &cookie).header("x-csrf-token", csrf).json(&serde_json::json!({"principalId":"attacker","tenantId":"other","peerRole":"agent","capabilityCaps":["blob/get"]})).send().await.expect("ticket").json().await.expect("ticket json");
    assert_eq!(ticket["principalId"], "alice");
    assert_eq!(ticket["tenantId"], "tenant-a");
    assert_eq!(ticket["peerRole"], "ui");
    assert!(ticket["ticket"].as_str().is_some());
    let no_csrf = client
        .post(format!("{origin}/tickets"))
        .header("origin", &origin)
        .header("cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("missing csrf");
    assert_eq!(no_csrf.status(), StatusCode::UNAUTHORIZED);
    task.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn logout_invalidates_cookie_and_unconsumed_tickets() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(
        root.path(),
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
    );
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    let login = client
        .post(format!("{origin}/web/login"))
        .header("origin", &origin)
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login");
    let cookie = cookie_pair(&login);
    let session: Value = client
        .get(format!("{origin}/web/session"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("session")
        .json()
        .await
        .expect("session json");
    let csrf = session["csrf"].as_str().unwrap();
    let ticket = client
        .post(format!("{origin}/tickets"))
        .header("origin", &origin)
        .header("cookie", &cookie)
        .header("x-csrf-token", csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("ticket");
    assert_eq!(ticket.status(), StatusCode::CREATED);
    let logout = client
        .post(format!("{origin}/web/logout"))
        .header("origin", &origin)
        .header("cookie", &cookie)
        .header("x-csrf-token", csrf)
        .send()
        .await
        .expect("logout");
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    let old_session = client
        .get(format!("{origin}/web/session"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("old session");
    assert_eq!(old_session.status(), StatusCode::UNAUTHORIZED);
    task.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn web_sessions_enforce_per_principal_cap() {
    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(
        root.path(),
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
    );
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    for _ in 0..8 {
        let response = client
            .post(format!("{origin}/web/login"))
            .header("origin", &origin)
            .header("authorization", "Bearer ui-secret")
            .send()
            .await
            .expect("login");
        assert_eq!(response.status(), StatusCode::OK);
    }
    let overflow = client
        .post(format!("{origin}/web/login"))
        .header("origin", &origin)
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("overflow login");
    assert_eq!(overflow.status(), StatusCode::TOO_MANY_REQUESTS);
    task.abort();
}
#[tokio::test(flavor = "multi_thread")]
async fn ticket_only_websocket_upgrade_works_and_ambiguous_auth_fails() {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let root = tempfile::tempdir().expect("tempdir");
    let config = config_text(
        root.path(),
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
    );
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    let login = client
        .post(format!("{origin}/web/login"))
        .header("origin", &origin)
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login");
    let cookie = cookie_pair(&login);
    let session: Value = client
        .get(format!("{origin}/web/session"))
        .header("cookie", &cookie)
        .send()
        .await
        .expect("session")
        .json()
        .await
        .expect("session json");
    let csrf = session["csrf"].as_str().unwrap();
    let ticket: Value = client
        .post(format!("{origin}/tickets"))
        .header("origin", &origin)
        .header("cookie", &cookie)
        .header("x-csrf-token", csrf)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("ticket")
        .json()
        .await
        .expect("ticket json");
    let ticket_value = ticket["ticket"].as_str().unwrap();
    let ws_url = format!("ws://{addr}/wss?ticket={ticket_value}");
    let mut request = ws_url.clone().into_client_request().expect("ws request");
    request
        .headers_mut()
        .insert("Origin", origin.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("ticket-only websocket");
    ws.send(Message::Text(serde_json::json!({"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","method":"conex/hello","params":{"profileId":"conex-jsonrpc2-wss","plane":"broker","provides":[],"requires":[]}}).to_string().into())).await.expect("hello");
    let hello = ws
        .next()
        .await
        .expect("hello response")
        .expect("hello frame");
    let hello: Value = match hello {
        Message::Text(text) => serde_json::from_str(&text).expect("hello json"),
        other => panic!("expected JSON hello response, got {other:?}"),
    };
    let provides = hello["result"]["provides"].as_array().expect("provides");
    assert!(
        !provides
            .iter()
            .any(|value| value.as_str() == Some("blob/get"))
    );
    let mut ambiguous = ws_url.into_client_request().expect("ambiguous request");
    ambiguous
        .headers_mut()
        .insert("Origin", origin.parse().unwrap());
    ambiguous
        .headers_mut()
        .insert("Authorization", "Bearer svc-secret".parse().unwrap());
    assert!(
        tokio_tungstenite::connect_async(ambiguous).await.is_err(),
        "ticket plus bearer must be rejected"
    );
    task.abort();
}
#[test]
fn tickets_are_random_one_use_and_origin_bound() {
    let registry = conex_host::agent::TicketRegistry::new();
    let first = registry
        .issue(
            "alice",
            "tenant-a",
            "http://ui.example",
            "host.example",
            "ui",
            vec!["source/read".into()],
            Some("session".into()),
        )
        .expect("first ticket");
    let second = registry
        .issue(
            "alice",
            "tenant-a",
            "http://ui.example",
            "host.example",
            "ui",
            vec!["source/read".into()],
            Some("session".into()),
        )
        .expect("second ticket");
    assert_ne!(first.ticket, second.ticket);
    assert!(
        registry
            .consume_for(&first.ticket, "http://other.example", "host.example")
            .is_err()
    );
    assert!(
        registry
            .consume_for(&first.ticket, "http://ui.example", "host.example")
            .is_ok()
    );
    assert!(
        registry
            .consume_for(&first.ticket, "http://ui.example", "host.example")
            .is_err()
    );
    let agent = registry
        .issue(
            "agent",
            "tenant-a",
            "http://ui.example",
            "host.example",
            "agent",
            Vec::new(),
            None,
        )
        .expect("agent ticket");
    assert_eq!(agent.peer_role, "agent");
}

const GUEST_SECTION: &str = "\n[web_guest]\nprincipal_id = \"visitor\"\ntenant_id = \"tenant-a\"\nmax_sessions = 3\nidle_ttl_ms = 600000\nmax_issue_per_minute = 100\n";
const GUEST_POLICIES: &str = "\n[[policy]]\nprincipal_id = \"visitor\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"notes-local\"\nactions = [\"list\", \"read\", \"search\"]\nroot = \"\"\nsubtree = true\n";

async fn spawn_guest_host(root: &std::path::Path) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let config = config_text_with(
        root,
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
        GUEST_SECTION,
        GUEST_POLICIES,
    );
    spawn_host(&config, root).await
}

/// Anonymous visitor sessions obey the configured global bound, independent
/// from the per-principal cap that bounds authenticated users (plan M1.1).
#[tokio::test(flavor = "multi_thread")]
async fn guest_sessions_obey_configured_bound_independently() {
    let root = tempfile::tempdir().expect("tempdir");
    let (addr, task) = spawn_guest_host(root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    let mut cookies = Vec::new();
    for _ in 0..3 {
        let response = client
            .get(format!("{origin}/web/session"))
            .header("origin", &origin)
            .send()
            .await
            .expect("guest session");
        assert_eq!(response.status(), StatusCode::OK);
        cookies.push(cookie_pair(&response));
    }
    let overflow = client
        .get(format!("{origin}/web/session"))
        .header("origin", &origin)
        .send()
        .await
        .expect("overflow guest session");
    assert_eq!(overflow.status(), StatusCode::TOO_MANY_REQUESTS);
    // The anonymous bound does not consume the authenticated principal cap.
    let login = client
        .post(format!("{origin}/web/login"))
        .header("origin", &origin)
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login");
    assert_eq!(login.status(), StatusCode::OK);
    task.abort();
}

/// A stale guest cookie reissues a fresh anonymous session; an authenticated
/// user's dead session must surface the auth error, never downgrade (M1.1).
#[tokio::test(flavor = "multi_thread")]
async fn stale_guest_cookie_reissues_but_authenticated_does_not_downgrade() {
    let root = tempfile::tempdir().expect("tempdir");
    let (addr, task) = spawn_guest_host(root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();

    let guest = client
        .get(format!("{origin}/web/session"))
        .header("origin", &origin)
        .send()
        .await
        .expect("guest session");
    assert_eq!(guest.status(), StatusCode::OK);
    let guest_cookie = cookie_pair(&guest);
    client
        .post(format!("{origin}/web/logout"))
        .header("origin", &origin)
        .header("cookie", &guest_cookie)
        .header(
            "x-csrf-token",
            guest.json::<Value>().await.expect("guest json")["csrf"]
                .as_str()
                .expect("csrf"),
        )
        .send()
        .await
        .expect("guest logout");

    let reissued = client
        .get(format!("{origin}/web/session"))
        .header("origin", &origin)
        .header("cookie", &guest_cookie)
        .send()
        .await
        .expect("reissue");
    assert_eq!(
        reissued.status(),
        StatusCode::OK,
        "stale guest cookie must reissue"
    );
    assert_ne!(
        cookie_pair(&reissued),
        guest_cookie,
        "reissue must mint a new session"
    );

    let login = client
        .post(format!("{origin}/web/login"))
        .header("origin", &origin)
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login");
    let alice_cookie = cookie_pair(&login);
    assert_eq!(login.status(), StatusCode::OK);
    // /web/login does not expose the CSRF token; the session endpoint does.
    let alice_session = client
        .get(format!("{origin}/web/session"))
        .header("origin", &origin)
        .header("cookie", &alice_cookie)
        .send()
        .await
        .expect("alice session");
    let alice_csrf = alice_session.json::<Value>().await.expect("alice json")["csrf"]
        .as_str()
        .expect("csrf")
        .to_string();
    client
        .post(format!("{origin}/web/logout"))
        .header("origin", &origin)
        .header("cookie", &alice_cookie)
        .header("x-csrf-token", &alice_csrf)
        .send()
        .await
        .expect("alice logout");
    let downgraded = client
        .get(format!("{origin}/web/session"))
        .header("origin", &origin)
        .header("cookie", &alice_cookie)
        .send()
        .await
        .expect("stale authenticated session");
    assert_eq!(
        downgraded.status(),
        StatusCode::UNAUTHORIZED,
        "authenticated sessions must not silently become guests"
    );
    task.abort();
}

/// One image is served under several domains (production plus one preview
/// domain per pull request). The Origin gate must accept every configured
/// origin and refuse anything else — this is what keeps a preview deployment
/// from loading the page and then failing every call.
#[tokio::test(flavor = "multi_thread")]
async fn an_additional_configured_origin_is_accepted_and_others_are_not() {
    let root = tempfile::tempdir().expect("tempdir");
    let base = config_text(
        root.path(),
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
    );
    let (addr, task) = spawn_host(&base, root.path()).await;
    let primary = format!("http://{addr}");
    let client = reqwest::Client::new();

    // A domain that is not configured is refused even with a valid token.
    let refused = client
        .post(format!("{primary}/web/login"))
        .header("origin", "https://evil.example")
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login request");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);

    drop(task);

    // Same host, but the second origin is now configured. The key has to go in
    // the top-level table: appending it to the file would land it inside the
    // trailing `[[endpoints]]` table.
    let multi = base.replacen(
        "web_origin = \"http://127.0.0.1:0\"",
        "web_origin = \"http://127.0.0.1:0\"\nweb_origins = [\"https://preview-conex.lszio.space\"]",
        1,
    );
    let (addr, task) = spawn_host(&multi, root.path()).await;
    let primary = format!("http://{addr}");
    let accepted = client
        .post(format!("{primary}/web/login"))
        .header("origin", "https://preview-conex.lszio.space")
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login request");
    assert_eq!(
        accepted.status(),
        StatusCode::OK,
        "a configured secondary origin must be able to log in"
    );

    let refused = client
        .post(format!("{primary}/web/login"))
        .header("origin", "https://still-not-configured.example")
        .header("authorization", "Bearer ui-secret")
        .send()
        .await
        .expect("login request");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    task.abort();
}

/// A malformed additional origin fails at load, not at the first request: a
/// preview build must not come up with an origin it can never match.
#[test]
fn a_malformed_additional_origin_fails_config_load() {
    let root = tempfile::tempdir().expect("tempdir");
    let base = config_text(
        root.path(),
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
    );
    let path = root.path().join("host.toml");
    let broken = base.replacen(
        "web_origin = \"http://127.0.0.1:0\"",
        "web_origin = \"http://127.0.0.1:0\"\nweb_origins = [\"preview.example/with/path\"]",
        1,
    );
    std::fs::write(&path, broken).expect("write");
    let error = conex_host::config::HostConfig::load(&path).expect_err("must not load");
    assert!(
        error.message().contains("web_origins"),
        "the error must name the offending field: {}",
        error.message()
    );
}

/// A cookie minted by a *previous* process — after a restart or redeploy the
/// registry is empty while every visitor's cookie is still in their jar. Before
/// this was fixed the visitor was refused with 401 forever, because the id had
/// no tombstone to classify it as a guest session.
#[tokio::test(flavor = "multi_thread")]
async fn a_cookie_from_a_previous_process_becomes_a_fresh_guest_session() {
    let root = tempfile::tempdir().expect("tempdir");
    // Guest access is what a returning visitor has; without `[web_guest]` the
    // first request is refused and there is no cookie to carry over.
    let config = config_text_with(
        root.path(),
        &sha256_hex("ui-secret"),
        &sha256_hex("svc-secret"),
        "[web_guest]\nprincipal_id = \"guest\"\ntenant_id = \"tenant-a\"\nmax_sessions = 16\n",
        "",
    );
    // First process: issue a real guest session and keep the cookie.
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let client = reqwest::Client::new();
    let first = client
        .get(format!("{origin}/web/session"))
        .header("origin", &origin)
        .send()
        .await
        .expect("first session");
    assert_eq!(first.status(), StatusCode::OK);
    let stale_cookie = cookie_pair(&first);
    task.abort();

    // A second process. It takes a new port — a restarted host is reached at
    // whatever address the deployment gives it — and the cookie is now from a
    // process that no longer exists, so nothing in memory recognises it.
    let (addr, task) = spawn_host(&config, root.path()).await;
    let origin = format!("http://{addr}");
    let reissued = client
        .get(format!("{origin}/web/session"))
        .header("origin", &origin)
        .header("cookie", &stale_cookie)
        .send()
        .await
        .expect("second session");
    assert_eq!(
        reissued.status(),
        StatusCode::OK,
        "a returning visitor must not be locked out by a restart"
    );
    let body: Value = reissued.json().await.expect("json");
    assert_eq!(body["principalId"].as_str(), Some("guest"));
    assert!(body["csrf"].as_str().is_some_and(|csrf| !csrf.is_empty()));
    task.abort();
}

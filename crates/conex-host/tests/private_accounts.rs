//! Route A (local admin accounts): the private store v1→v2 migration must be
//! purely additive, and the account login flow must gate provisioning on a
//! bootstrap-derived session while password login serves every restart after.

#![forbid(unsafe_code)]

use std::time::Duration;

use axum::serve;
use conex_host::private_access::{
    BOOTSTRAP_TTL_MS, PrivateSettings, bootstrap_hash, put_bootstrap_blocking,
};
use conex_host::serve::build_router;
use conex_proto::now_ms;
use reqwest::StatusCode;
use serde_json::Value;
use tokio::net::TcpListener;

fn private_settings(root: &std::path::Path) -> PrivateSettings {
    let key = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
        .expect("generate ed25519 key");
    let key_path = root.join("signing.pkcs8");
    std::fs::write(&key_path, key.as_ref()).expect("write key");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600)).expect("key mode");
    PrivateSettings {
        issuer: "conex://hub.test".into(),
        kid: "test-1".into(),
        tenant: "private".into(),
        signing_key_backend: format!("file:{}", key_path.display()),
        store_path: root.join("private.sqlite"),
        principal: "owner".into(),
        admin_principals: ["owner".to_owned()].into_iter().collect(),
        node_principals: ["node-a".to_owned()].into_iter().collect(),
    }
}

// ---------------------------------------------------------------------------
// store: additive migration v1 -> v2
// ---------------------------------------------------------------------------

fn create_v1_database(path: &std::path::Path) {
    let connection = rusqlite::Connection::open(path).expect("open v1 database");
    connection
        .execute_batch(
            "PRAGMA user_version=1;
             CREATE TABLE bootstrap_codes(
                 code_hash TEXT PRIMARY KEY,
                 expires_at_ms INTEGER NOT NULL,
                 consumed INTEGER NOT NULL DEFAULT 0 CHECK(consumed IN (0,1))
             );
             CREATE TABLE grant_ledger(
                 jti TEXT PRIMARY KEY,
                 principal TEXT NOT NULL,
                 tenant TEXT NOT NULL,
                 node_id TEXT NOT NULL,
                 node_epoch TEXT NOT NULL,
                 purpose TEXT NOT NULL,
                 scope_json TEXT NOT NULL,
                 issued_at_ms INTEGER NOT NULL,
                 expires_at_ms INTEGER NOT NULL
             );
             CREATE TABLE revocations(
                 revision INTEGER PRIMARY KEY AUTOINCREMENT,
                 kind TEXT NOT NULL CHECK(kind IN ('jti','principal','kid')),
                 target TEXT NOT NULL,
                 created_at_ms INTEGER NOT NULL
             );
             INSERT INTO bootstrap_codes VALUES('legacy-hash', 99999999999999, 0);
             INSERT INTO grant_ledger VALUES('legacy-jti', 'owner', 'private',
                 '01ARZ3NDEKTSV4RRFFQ69G5FAX', '01ARZ3NDEKTSV4RRFFQ69G5FBX',
                 'direct', '{}', 1, 2);",
        )
        .expect("create v1 schema");
}

#[tokio::test]
async fn migration_from_v1_preserves_every_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The store directory must exist with 0700 before the store opens it.
    conex_host::private_access::ensure_private_dir(dir.path()).expect("private dir");
    let path = dir.path().join("private.sqlite");
    create_v1_database(&path);

    let store = conex_host::private_store::PrivateStore::open(&path).expect("migrate to v2");
    assert_eq!(store.count_accounts().await.expect("count"), 0);
    let now = now_ms();
    store
        .put_account(conex_host::private_store::AccountRow {
            username: "owner".into(),
            kind: "local".into(),
            password_hash: Some("$argon2id$placeholder".into()),
            role: "admin".into(),
            created_at_ms: now,
            updated_at_ms: now,
        })
        .await
        .expect("put account");

    // Reopen: the migrated version is stable, and the v1 rows survived the
    // upgrade verbatim — a migration must never touch existing data.
    drop(store);
    let store = conex_host::private_store::PrivateStore::open(&path).expect("reopen v2");
    assert_eq!(store.count_accounts().await.expect("count"), 1);
    let account = store
        .get_account("owner".into())
        .await
        .expect("get")
        .expect("account exists");
    assert_eq!(account.kind, "local");
    assert_eq!(
        account.password_hash.as_deref(),
        Some("$argon2id$placeholder")
    );
    drop(store);

    let raw = rusqlite::Connection::open(&path).expect("raw open");
    let version: i64 = raw
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .expect("version");
    assert_eq!(version, 2);
    let consumed: i64 = raw
        .query_row(
            "SELECT consumed FROM bootstrap_codes WHERE code_hash = 'legacy-hash'",
            [],
            |row| row.get(0),
        )
        .expect("legacy bootstrap row");
    assert_eq!(consumed, 0);
    let legacy_grant: String = raw
        .query_row(
            "SELECT principal FROM grant_ledger WHERE jti = 'legacy-jti'",
            [],
            |row| row.get(0),
        )
        .expect("legacy grant row");
    assert_eq!(legacy_grant, "owner");
}

#[tokio::test]
async fn account_constraints_hold() {
    let dir = tempfile::tempdir().expect("tempdir");
    conex_host::private_access::ensure_private_dir(dir.path()).expect("private dir");
    let store = conex_host::private_store::PrivateStore::open(&dir.path().join("private.sqlite"))
        .expect("open");
    let now = now_ms();
    let row = |username: &str, hash: Option<&str>| conex_host::private_store::AccountRow {
        username: username.into(),
        kind: "local".into(),
        password_hash: hash.map(str::to_owned),
        role: "admin".into(),
        created_at_ms: now,
        updated_at_ms: now,
    };
    // kind=local demands a password hash.
    assert!(store.put_account(row("a", None)).await.is_err());
    store
        .put_account(row("a", Some("$argon2id$h")))
        .await
        .expect("first");
    // The username is the primary key.
    assert!(
        store
            .put_account(row("a", Some("$argon2id$h")))
            .await
            .is_err()
    );
    // Password updates only land on local accounts that exist.
    store
        .set_password_hash("a".to_string(), "$argon2id$h2".into(), now + 1)
        .await
        .expect("update");
    assert!(
        store
            .set_password_hash("missing".to_string(), "$argon2id$h".into(), now)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .get_account("a".into())
            .await
            .expect("get")
            .expect("row")
            .password_hash
            .as_deref(),
        Some("$argon2id$h2")
    );
}

// ---------------------------------------------------------------------------
// HTTP flow: bootstrap -> provision -> password login -> recovery reset
// ---------------------------------------------------------------------------

async fn spawn_private_host(
    root: &std::path::Path,
) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("address");
    let settings = private_settings(root);
    let config = format!(
        "listen = \"127.0.0.1:0\"\nallow_loopback_http = true\naudience = \"hub.test\"\nweb_origin = \"http://127.0.0.1:0\"\ncontent_root = \"{c}\"\nsession_root = \"{s}\"\noperation_root = \"{o}\"\n\n[private]\nissuer = \"conex://hub.test\"\nkid = \"test-1\"\nsigning_key_backend = \"file:{k}\"\ntenant = \"private\"\nstore_path = \"{st}\"\nprincipal = \"owner\"\nadmin_principals = [\"owner\"]\nnode_principals = [\"node-a\"]\n",
        c = root.join("content").display(),
        s = root.join("session").display(),
        o = root.join("operation").display(),
        k = root.join("signing.pkcs8").display(),
        st = settings.store_path.display(),
    );
    let path = root.join("host.toml");
    // `listen`/`web_origin` are written as 127.0.0.1:0 and pinned to the
    // actually bound address so the Origin check accepts test requests.
    std::fs::write(&path, config.replace("127.0.0.1:0", &addr.to_string())).expect("write config");
    let config = conex_host::config::HostConfig::load(&path).expect("load config");
    let app = build_router(&config).expect("build router");
    let task = tokio::spawn(async move {
        serve(listener, app).await.expect("serve");
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    (addr, task)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client")
}

async fn bootstrap_login(
    addr: std::net::SocketAddr,
    store_path: &std::path::Path,
) -> (String, String) {
    static CODE_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let code = format!(
        "test-code-{}-0123456789abcdef",
        CODE_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    put_bootstrap_blocking(
        store_path,
        &bootstrap_hash(&code),
        now_ms() + BOOTSTRAP_TTL_MS,
    )
    .expect("mint code");
    let origin = format!("http://{addr}");
    let response = client()
        .post(format!("http://{addr}/private/session"))
        .header("Origin", &origin)
        .json(&serde_json::json!({ "code": code }))
        .send()
        .await
        .expect("login");
    let set_cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::to_owned);
    let status = response.status();
    let body_text = response.text().await.expect("body");
    assert_eq!(status, StatusCode::OK, "bootstrap login: {body_text}");
    let cookie = set_cookie.expect("set-cookie");
    let body: Value = serde_json::from_str(&body_text).expect("session body");
    let csrf = body["csrf"].as_str().expect("csrf").to_owned();
    (cookie, csrf)
}

#[tokio::test(flavor = "multi_thread")]
async fn bootstrap_provisions_admin_then_password_takes_over() {
    let dir = tempfile::tempdir().expect("tempdir");
    let store_path = dir.path().join("private.sqlite");
    let (addr, task) = spawn_private_host(dir.path()).await;
    let origin = format!("http://{addr}");

    let status = client()
        .get(format!("http://{addr}/private/account/status"))
        .send()
        .await
        .expect("status");
    assert_eq!(
        status.json::<Value>().await.expect("body"),
        serde_json::json!({"accountExists": false})
    );

    // Password login before provisioning: refused without leaking whether
    // the username exists. A dedicated username keeps its backoff isolated
    // from the real account's attempts below.
    let early = client()
        .post(format!("http://{addr}/private/account/password-session"))
        .header("Origin", &origin)
        .json(&serde_json::json!({"username": "ghost", "password": "whatever-pass"}))
        .send()
        .await
        .expect("early login");
    assert_eq!(early.status(), StatusCode::UNAUTHORIZED);

    let (cookie, csrf) = bootstrap_login(addr, &store_path).await;

    // Provisioning needs the cookie+CSRF session.
    let naked = client()
        .post(format!("http://{addr}/private/account"))
        .header("Origin", &origin)
        .json(&serde_json::json!({"username": "owner", "password": "str0ng-passphrase"}))
        .send()
        .await
        .expect("naked provision");
    assert_eq!(naked.status(), StatusCode::UNAUTHORIZED);

    let provision = client()
        .post(format!("http://{addr}/private/account"))
        .header("Origin", &origin)
        .header("x-csrf-token", &csrf)
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"username": "owner", "password": "str0ng-passphrase"}))
        .send()
        .await
        .expect("provision");
    assert_eq!(
        provision.status(),
        StatusCode::OK,
        "{:?}",
        provision.text().await
    );

    // A second account is refused even with a valid session.
    let again = client()
        .post(format!("http://{addr}/private/account"))
        .header("Origin", &origin)
        .header("x-csrf-token", &csrf)
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"username": "other", "password": "str0ng-passphrase"}))
        .send()
        .await
        .expect("second provision");
    assert_eq!(again.status(), StatusCode::FORBIDDEN);

    let status = client()
        .get(format!("http://{addr}/private/account/status"))
        .send()
        .await
        .expect("status");
    assert_eq!(
        status.json::<Value>().await.expect("body"),
        serde_json::json!({"accountExists": true})
    );

    // Wrong password: uniform 401.
    let wrong = client()
        .post(format!("http://{addr}/private/account/password-session"))
        .header("Origin", &origin)
        .json(&serde_json::json!({"username": "owner", "password": "wrong-passphrase"}))
        .send()
        .await
        .expect("wrong login");
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);

    // The failure starts a backoff on the username; the next attempt inside
    // the window is throttled (429), which is the brute-force behavior we
    // want — so the retry below waits the base window out.
    tokio::time::sleep(Duration::from_millis(700)).await;

    // Correct password: a session identical in shape to the bootstrap one.
    let login = client()
        .post(format!("http://{addr}/private/account/password-session"))
        .header("Origin", &origin)
        .json(&serde_json::json!({"username": "owner", "password": "str0ng-passphrase"}))
        .send()
        .await
        .expect("password login");
    assert_eq!(login.status(), StatusCode::OK, "{:?}", login.text().await);
    let body = login.json::<Value>().await.expect("session body");
    assert_eq!(body["principalId"], serde_json::json!("owner"));
    assert_eq!(body["role"], serde_json::json!("ui"));
    assert!(body["csrf"].as_str().is_some_and(|csrf| !csrf.is_empty()));

    // Recovery: a bootstrap session (here, a fresh code) resets the password,
    // after which the old password is dead and the new one works.
    let (recovery_cookie, recovery_csrf) = bootstrap_login(addr, &store_path).await;
    let reset = client()
        .post(format!("http://{addr}/private/account/password"))
        .header("Origin", &origin)
        .header("x-csrf-token", &recovery_csrf)
        .header("Cookie", &recovery_cookie)
        .json(&serde_json::json!({"username": "owner", "password": "n3w-passphrase!"}))
        .send()
        .await
        .expect("reset");
    assert_eq!(reset.status(), StatusCode::OK, "{:?}", reset.text().await);
    let stale = client()
        .post(format!("http://{addr}/private/account/password-session"))
        .header("Origin", &origin)
        .json(&serde_json::json!({"username": "owner", "password": "str0ng-passphrase"}))
        .send()
        .await
        .expect("stale login");
    assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);
    // The stale-password failure restarted the backoff; wait it out again.
    tokio::time::sleep(Duration::from_millis(600)).await;
    let fresh = client()
        .post(format!("http://{addr}/private/account/password-session"))
        .header("Origin", &origin)
        .json(&serde_json::json!({"username": "owner", "password": "n3w-passphrase!"}))
        .send()
        .await
        .expect("fresh login");
    assert_eq!(fresh.status(), StatusCode::OK);

    task.abort();
}

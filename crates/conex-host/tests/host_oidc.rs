//! Real-host OIDC verified mode: with `[oidc]` configured, `/oidc/token`
//! accepts a valid RS256 idToken and issues a ticket; a bogus token is
//! rejected 401. The idToken is signed with the same fixture RSA key as
//! tests/oidc_jwt.rs.

mod support;

use std::net::TcpListener;
use std::path::PathBuf;

use reqwest::StatusCode;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use conex_host::oidc_jwt::claims_json;
use support::{TEST_JWKS_JSON, sign_id_token};

fn tmp(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    std::fs::create_dir_all(&path).expect("mkdir");
    (dir, path)
}

fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oidc_verified_token_issues_ticket() {
    let (_t_content, content_root) = tmp("content");
    let (_t_session, session_root) = tmp("session");
    let (_t_operation, operation_root) = tmp("operation");
    let notes_root = repo_root().join("fixtures/p0/notes");
    let token = "e2e-token";
    let token_hash = sha256_hex(token);
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    // [oidc] as a TOML inline table containing the JWKS inline (multi-line
    // string works in TOML basic strings? use a single-line JSON via the
    // adjacent-key form; simplest: write jwks to a file and use jwks_path).
    let jwks_path = content_root.parent().unwrap().join("jwks.json");
    std::fs::write(&jwks_path, TEST_JWKS_JSON).expect("write jwks");

    let config = format!(
        "listen = \"127.0.0.1:{port}\"\n\
allow_loopback_http = true\n\
audience = \"host.local\"\n\
host_origin = \"conex://broker.local\"\n\
\n\
content_root = \"{content_root}\"\n\
session_root = \"{session_root}\"\n\
operation_root = \"{operation_root}\"\n\
\n\
[oidc]\n\
id_token_issuer = \"https://issuer.example\"\n\
client_id = \"conex-host\"\n\
nonce = \"nohunter2\"\n\
jwks_path = \"{jwks_path}\"\n\
\n\
[[tokens]]\n\
token_hash = \"{token_hash}\"\n\
principal_id = \"alice\"\n\
tenant_id = \"tenant-a\"\n\
audience = \"host.local\"\n\
\n\
[[endpoints]]\n\
id = \"notes-local\"\n\
tenant_id = \"tenant-a\"\n\
provider_id = \"source\"\n\
kind = \"source-fs\"\n\
provides = [\"source/list\", \"source/read\", \"source/search\"]\n\
root = \"{notes_root}\"\n",
        content_root = content_root.display(),
        session_root = session_root.display(),
        operation_root = operation_root.display(),
        notes_root = notes_root.display(),
        jwks_path = jwks_path.display(),
    );
    let cfg_path = content_root.parent().unwrap().join("host.toml");
    std::fs::write(&cfg_path, &config).expect("write config");

    let binary = std::env::var("CONEX_HOST_BIN")
        .unwrap_or_else(|_| "/home/lszio/Projects/conex/target/debug/conex-host".to_string());
    let mut child = std::process::Command::new(&binary)
        .arg(&cfg_path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn conex-host");

    // Wait for readiness.
    let mut ready = false;
    for _ in 0..100 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(ready, "host not ready on {port}");

    let base = format!("http://127.0.0.1:{port}");
    let client = reqwest::Client::new();

    // Valid idToken → 200 + ticket.
    let valid = sign_id_token(
        support::TEST_KID,
        &claims_json(
            "alice",
            "https://issuer.example",
            "conex-host",
            3600,
            Some("nohunter2"),
        ),
        "RS256",
    );
    let ok = client
        .post(format!("{base}/oidc/token"))
        .header("content-type", "application/json")
        .json(&json!({ "idToken": valid }))
        .send()
        .await
        .expect("oidc/token valid");
    assert_eq!(ok.status(), StatusCode::OK);
    let body: Value = ok.json().await.expect("json");
    assert_eq!(body["principalId"].as_str(), Some("alice"));
    assert!(body["ticket"].as_str().is_some());

    // Wrong issuer → 401.
    let evil = sign_id_token(
        support::TEST_KID,
        &claims_json(
            "alice",
            "https://evil.example",
            "conex-host",
            3600,
            Some("nohunter2"),
        ),
        "RS256",
    );
    let bad = client
        .post(format!("{base}/oidc/token"))
        .header("content-type", "application/json")
        .json(&json!({ "idToken": evil }))
        .send()
        .await
        .expect("oidc/token evil");
    assert_eq!(bad.status(), StatusCode::UNAUTHORIZED);

    // Garbage token → 401.
    let garbage = client
        .post(format!("{base}/oidc/token"))
        .header("content-type", "application/json")
        .json(&json!({ "idToken": "not.a.jwt" }))
        .send()
        .await
        .expect("oidc/token garbage");
    assert_eq!(garbage.status(), StatusCode::UNAUTHORIZED);

    // Missing idToken → 400.
    let missing = client
        .post(format!("{base}/oidc/token"))
        .header("content-type", "application/json")
        .json(&json!({ "code": "x", "codeVerifier": "y", "origin": "http://z" }))
        .send()
        .await
        .expect("oidc/token missing");
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

    let _ = child.kill();
    let _ = child.wait();
}

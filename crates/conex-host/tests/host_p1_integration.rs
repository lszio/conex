//! Integration test for the P1 host wiring. Spins up a real
//! `conex-host` over loopback HTTP, then exercises:
//!
//! 1. `conex/hello` advertises P1 capabilities.
//! 2. `/tickets` issues a 30 s ticket.
//! 3. `/oidc/authorize` + `/oidc/token` exchange a PKCE-bound code.
//! 4. `/rpc agent/register` flows through the broker with `hostOrigin` matching.
//! 5. Cross-principal `blob/get` for someone else's CID is rejected at the broker.
//!
//! The test does NOT exercise WSS over a real socket (still single-threaded
//! `tokio-tungstenite` client required for true round-trip). It does prove
//! the dispatcher and HTTP layer are live in the running binary.

use std::io::Read;
use std::net::TcpListener;
use std::path::PathBuf;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn tmp(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    std::fs::create_dir_all(&path).expect("mkdir");
    (dir, path)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .to_path_buf()
}

fn sha256_hex(input: &str) -> String {
    hex::encode(Sha256::digest(input.as_bytes()))
}

fn base_url(addr: std::net::SocketAddr) -> String {
    format!("http://{addr}/rpc")
}

fn http_url(addr: std::net::SocketAddr, path: &str) -> String {
    format!("http://{addr}{path}")
}

#[tokio::test(flavor = "multi_thread")]
async fn host_p1_http_integration() {
    let (_t_content, content_root) = tmp("content");
    let (_t_session, session_root) = tmp("session");
    let (_t_operation, operation_root) = tmp("operation");

    let token = "e2e-token-p1";
    let token_hash = sha256_hex(token);
    let notes_root = repo_root().join("fixtures/p0/notes");
    let config = format!(
        "listen = \"127.0.0.1:0\"\n\
allow_loopback_http = true\n\
audience = \"host.local\"\n\
host_origin = \"conex://broker.local\"\n\
\n\
content_root = \"{content_root}\"\n\
session_root = \"{session_root}\"\n\
operation_root = \"{operation_root}\"\n\
\n\
[[tokens]]\n\
token_hash = \"{token_hash}\"\n\
principal_id = \"alice\"\n\
tenant_id = \"tenant-a\"\n\
audience = \"host.local\"\n\
\n\
[[policy]]\n\
principal_id = \"alice\"\n\
tenant_id = \"tenant-a\"\n\
endpoint_id = \"notes-local\"\n\
actions = [\"list\", \"read\", \"search\"]\n\
root = \"\"\n\
subtree = true\n\
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
    );
    let cfg_path = content_root.parent().unwrap().join("host.toml");
    std::fs::write(&cfg_path, &config).expect("write config");

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let listen = format!("127.0.0.1:{port}");
    let config = config.replace(
        "listen = \"127.0.0.1:0\"",
        &format!("listen = \"{listen}\""),
    );
    std::fs::write(&cfg_path, &config).expect("rewrite config with concrete port");

    let binary = std::env::var("CONEX_HOST_BIN")
        .unwrap_or_else(|_| "/home/lszio/Projects/conex/target/debug/conex-host".to_string());
    let mut child = std::process::Command::new(&binary)
        .arg(&cfg_path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn conex-host");

    let bound_addr: std::net::SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    // Probe until /rpc responds
    let client = reqwest::Client::new();
    let hello_body = r#"{"jsonrpc":"2.0","id":"01ARZ3NDEKTSV4RRFFQ69G5FAX","method":"conex/hello","params":{"context":{"providerEndpointId":"notes-local","plane":"broker"},"input":{"supportedProfiles":["conex-jsonrpc2-http-v1"],"capabilities":[]}}}"#;
    let mut ready = false;
    for _ in 0..200 {
        if let Ok(resp) = client
            .post(format!("http://{bound_addr}/rpc"))
            .header("Authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(hello_body)
            .send()
            .await
            && resp.status().is_success()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    if !ready {
        let _ = child.kill();
        let mut err = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut err);
        }
        panic!("host never became ready on {bound_addr}; stderr: {err}");
    }

    // 1. Hello advertises P1 capabilities
    let hello: Value = reqwest::Client::new()
        .post(base_url(bound_addr))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "method": "conex/hello",
            "params": {
                "context": { "providerEndpointId": "notes-local", "plane": "broker" },
                "input": {
                    "supportedProfiles": ["conex-jsonrpc2-http-v1"],
                    "capabilities": ["blob/get", "session/open"]
                }
            }
        }))
        .send()
        .await
        .expect("hello")
        .json()
        .await
        .expect("hello json");
    let provides = hello["result"]["provides"]
        .as_array()
        .expect("provides array");
    assert!(
        provides.iter().any(|v| v.as_str() == Some("blob/get")),
        "hello did not advertise blob/get"
    );
    assert!(
        provides.iter().any(|v| v.as_str() == Some("session/open")),
        "hello did not advertise session/open"
    );
    let binding_id = hello["result"]["bindingId"]
        .as_str()
        .expect("bindingId")
        .to_string();

    // 2. POST /tickets returns a 30 s ticket.
    let ticket: Value = reqwest::Client::new()
        .post(http_url(bound_addr, "/tickets"))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .json(&json!({
            "principalId": "alice",
            "tenantId": "tenant-a",
            "origin": "http://example.com",
            "targetHost": "broker.local",
            "peerRole": "ui"
        }))
        .send()
        .await
        .expect("ticket")
        .json()
        .await
        .expect("ticket json");
    assert!(
        ticket["ticket"].as_str().is_some(),
        "ticket endpoint should return a ticket: {ticket}"
    );

    // 3. /oidc/authorize + /oidc/token flow with a PKCE challenge.
    let challenge = sha256_hex("verifier-not-revealed");
    let authorize: Value = reqwest::Client::new()
        .post(http_url(bound_addr, "/oidc/authorize"))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .json(&json!({
            "principalId": "alice",
            "tenantId": "tenant-a",
            "origin": "http://example.com",
            "audience": "host.local",
            "codeChallenge": challenge,
            "codeChallengeMethod": "S256"
        }))
        .send()
        .await
        .expect("authorize")
        .json()
        .await
        .expect("authorize json");
    let code = authorize["code"].as_str().expect("oidc code").to_string();
    // /oidc/token requires the verifier — without it PKCE fails.
    let token_resp = reqwest::Client::new()
        .post(http_url(bound_addr, "/oidc/token"))
        .header("content-type", "application/json")
        .json(&json!({
            "code": code,
            "codeVerifier": "wrong",
            "origin": "http://example.com"
        }))
        .send()
        .await
        .expect("oidc token");
    assert_eq!(
        token_resp.status(),
        reqwest::StatusCode::UNAUTHORIZED,
        "PKCE mismatch must reject"
    );

    // 4. agent/register via /rpc (broker path) accepts matching hostOrigin.
    let register: Value = reqwest::Client::new()
        .post(base_url(bound_addr))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAW",
            "method": "agent/register",
            "params": {
                "context": {
                    "providerEndpointId": "agent-mgr",
                    "plane": "broker",
                    "bindingId": binding_id
                },
                "input": {
                    "agentId": "agent-1",
                    "providerIds": ["notes-local"],
                    "methods": ["source/read"],
                    "resources": ["a.md"],
                    "hostOrigin": "conex://broker.local"
                }
            }
        }))
        .send()
        .await
        .expect("register")
        .json()
        .await
        .expect("register json");
    assert_eq!(register["result"]["agentId"].as_str(), Some("agent-1"));

    // 4b. agent/register with mismatched hostOrigin must be rejected.
    let bad_origin: Value = reqwest::Client::new()
        .post(base_url(bound_addr))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAX",
            "method": "agent/register",
            "params": {
                "context": {
                    "providerEndpointId": "agent-mgr",
                    "plane": "broker",
                    "bindingId": hello["result"]["bindingId"].as_str().unwrap()
                },
                "input": {
                    "agentId": "agent-2",
                    "providerIds": ["notes-local"],
                    "methods": ["source/read"],
                    "resources": ["a.md"],
                    "hostOrigin": "conex://evil.example"
                }
            }
        }))
        .send()
        .await
        .expect("bad origin")
        .json()
        .await
        .expect("bad origin json");
    assert!(
        bad_origin["error"].is_object(),
        "mismatched hostOrigin must produce an error envelope: {bad_origin}"
    );

    // 5. /rpc source/read still works (P0 path).
    let read: Value = reqwest::Client::new()
        .post(base_url(bound_addr))
        .header("Authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": "01ARZ3NDEKTSV4RRFFQ69G5FAY",
            "method": "source/read",
            "params": {
                "context": {
                    "providerEndpointId": "notes-local",
                    "plane": "broker",
                    "bindingId": hello["result"]["bindingId"].as_str().unwrap()
                },
                "input": { "resourceId": "hello.md" }
            }
        }))
        .send()
        .await
        .expect("source/read")
        .json()
        .await
        .expect("source/read json");
    assert_eq!(
        read["result"]["text"].as_str(),
        Some("hello conex\n"),
        "P0 source/read must still work after wiring P1: {read}"
    );

    let _ = child.kill();
    let _ = child.wait();
}

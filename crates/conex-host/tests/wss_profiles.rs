//! Real-socket WSS profile tests: both `conex-jsonrpc2-wss-v1` (JSON text)
//! and `conex-protobuf-wss-v1` (prost binary) run a real hello/ready
//! handshake against the live `conex-host` binary and complete a business
//! call. The bootstrap is UTF-8 JSON-RPC text for both profiles; the
//! business encoding switches after `ready` (design §4.4).

use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::Message;

use conex_proto::v1;
use prost::Message as ProstMessage;

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

const ULID_A: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
const ULID_B: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAW";

/// Spawn a live host with P1 backends and return `(port, child, keepalive)`.
fn spawn_host() -> (u16, std::process::Child, Vec<tempfile::TempDir>) {
    let (t_content, content_root) = tmp("content");
    let (t_session, session_root) = tmp("session");
    let (t_operation, operation_root) = tmp("operation");
    let notes_root = repo_root().join("fixtures/p0/notes");
    let token_hash = sha256_hex("e2e-token");
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

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

    let binary = std::env::var("CONEX_HOST_BIN")
        .unwrap_or_else(|_| "/home/lszio/Projects/conex/target/debug/conex-host".to_string());
    let child = std::process::Command::new(&binary)
        .arg(&cfg_path)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn conex-host");
    (port, child, vec![t_content, t_session, t_operation])
}

async fn connect_with_bearer(
    url: &str,
    token: &str,
) -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = url.into_client_request()?;
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(request).await?;
    Ok(ws)
}

/// Run the JSON bootstrap (hello → ready) and return the negotiated
/// negotiation id / profile, validating the flow with the JSON profile.
async fn bootstrap_json(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    profile: &str,
) -> String {
    // hello
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": ULID_A,
            "method": "conex/hello",
            "params": {
                "profileId": profile,
                "plane": "broker",
                "provides": [],
                "requires": []
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send hello");
    let hello: Value = match ws.next().await.expect("hello reply").expect("hello msg") {
        Message::Text(text) => serde_json::from_str(&text).expect("hello json"),
        other => panic!("expected text hello result, got {other:?}"),
    };
    let negotiation_id = hello["result"]["negotiationId"]
        .as_str()
        .expect("negotiationId")
        .to_string();
    let echoed_profile = hello["result"]["profileId"]
        .as_str()
        .expect("profileId")
        .to_string();
    assert_eq!(
        echoed_profile, profile,
        "hello must echo the requested profile"
    );

    // ready
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": ULID_B,
            "method": "conex/ready",
            "params": {
                "negotiationId": negotiation_id,
                "profileId": profile,
                "plane": "broker"
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send ready");
    let ready: Value = match ws.next().await.expect("ready reply").expect("ready msg") {
        Message::Text(text) => serde_json::from_str(&text).expect("ready json"),
        other => panic!("expected text ready result, got {other:?}"),
    };
    assert!(
        ready["result"]["negotiationId"].as_str().is_some(),
        "ready result must carry negotiationId: {ready}"
    );
    negotiation_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_json_profile_round_trip() {
    let (port, mut child, _keepalive) = spawn_host();
    // wait for readiness: TCP accept on /wss
    let mut ready = false;
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if !ready {
        use std::io::Read;
        let mut err = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut err);
        }
        panic!("host not ready on {port}; stderr: {err}");
    }

    let url = format!("ws://127.0.0.1:{port}/wss");
    let mut ws = connect_with_bearer(&url, "e2e-token")
        .await
        .expect("ws connect (json)");
    bootstrap_json(&mut ws, "conex-jsonrpc2-wss-v1").await;

    // Business call over the JSON profile: agent/register with the
    // configured host_origin succeeds. Identity comes from the upgrade
    // bearer, not from the frame context.
    ws.send(Message::Text(
        json!({
            "request_id": ULID_A,
            "method": "agent/register",
            "context": {
                "providerEndpointId": "agent-mgr",
                "plane": 1
            },
            "params": {
                "agentId": "ws-agent-json",
                "providerIds": ["notes-local"],
                "methods": ["source/read"],
                "resources": ["a.md"],
                "hostOrigin": "conex://broker.local"
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send business frame");
    let reply: Value = match ws
        .next()
        .await
        .expect("business reply")
        .expect("business msg")
    {
        Message::Text(text) => serde_json::from_str(&text).expect("business json"),
        other => panic!("expected text business result, got {other:?}"),
    };
    assert_eq!(
        reply["result"]["agentId"].as_str(),
        Some("ws-agent-json"),
        "agent/register over JSON WSS must succeed: {reply}"
    );

    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wss_protobuf_profile_round_trip() {
    let (port, mut child, _keepalive) = spawn_host();
    let mut ready = false;
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(ready, "host not ready on {port}");

    let url = format!("ws://127.0.0.1:{port}/wss");
    let mut ws = connect_with_bearer(&url, "e2e-token")
        .await
        .expect("ws connect (protobuf)");

    bootstrap_json(&mut ws, "conex-protobuf-wss-v1").await;

    // Business call over the protobuf profile: a typed v1::Message Request
    // for agent/register, encoded with prost.
    use std::collections::HashMap;
    let mut fields: HashMap<String, pbjson_types::Value> = HashMap::new();
    fields.insert("agentId".into(), "ws-agent-proto".into());
    fields.insert(
        "providerIds".into(),
        vec![<pbjson_types::Value>::from("notes-local")].into(),
    );
    fields.insert(
        "methods".into(),
        vec![<pbjson_types::Value>::from("source/read")].into(),
    );
    fields.insert(
        "resources".into(),
        vec![<pbjson_types::Value>::from("a.md")].into(),
    );
    fields.insert("hostOrigin".into(), "conex://broker.local".into());
    let input = v1::CallParams {
        context: Some(v1::RequestContext {
            provider_endpoint_id: "agent-mgr".into(),
            plane: v1::Plane::Broker as i32,
            binding_id: None,
        }),
        timeout_budget_ms: 8000,
        input: Some(fields.into()),
    };
    let request = v1::Message {
        body: Some(v1::message::Body::Request(v1::Request {
            request_id: ULID_A.into(),
            method: "agent/register".into(),
            params: Some(input),
        })),
    };
    let mut buf = Vec::new();
    prost::Message::encode(&request, &mut buf).expect("encode request");
    ws.send(Message::Binary(buf.into()))
        .await
        .expect("send protobuf frame");

    let reply = ws
        .next()
        .await
        .expect("protobuf reply")
        .expect("protobuf msg");
    let Message::Binary(bytes) = reply else {
        panic!("expected binary business result, got {reply:?}");
    };
    let message = v1::Message::decode(bytes.as_ref()).expect("decode response");
    match message.body {
        Some(v1::message::Body::Success(success)) => {
            let result = success.result.expect("result");
            let value: Value = serde_json::to_value(&result).expect("result to json");
            assert_eq!(
                value["agentId"].as_str(),
                Some("ws-agent-proto"),
                "agent/register over protobuf WSS must succeed: {value}"
            );
        }
        other => panic!("expected success body, got {other:?}"),
    }

    let _ = child.kill();
    let _ = child.wait();
}

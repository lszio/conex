use std::net::TcpListener;
use std::path::PathBuf;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tempfile::TempDir;
use tokio_tungstenite::tungstenite::Message;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn spawn_host() -> (u16, std::process::Child, Vec<TempDir>) {
    let content = tempfile::tempdir().expect("content tempdir");
    let session = tempfile::tempdir().expect("session tempdir");
    let operation = tempfile::tempdir().expect("operation tempdir");
    let notes = repo_root().join("fixtures/p0/notes");
    let token_hash = hex::encode(Sha256::digest(b"e2e-token"));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    drop(listener);
    let config = format!(
        "listen = \"127.0.0.1:{port}\"\n\
allow_loopback_http = true\n\
audience = \"host.local\"\n\
host_origin = \"conex://broker.local\"\n\
content_root = \"{}\"\n\
session_root = \"{}\"\n\
operation_root = \"{}\"\n\
[[tokens]]\n\
token_hash = \"{token_hash}\"\n\
principal_id = \"alice\"\n\
tenant_id = \"tenant-a\"\n\
audience = \"host.local\"\n\
[[policy]]\n\
principal_id = \"alice\"\n\
tenant_id = \"tenant-a\"\n\
endpoint_id = \"notes-local\"\n\
actions = [\"list\", \"read\", \"search\"]\n\
root = \"\"\n\
subtree = true\n\
[[endpoints]]\n\
id = \"notes-local\"\n\
tenant_id = \"tenant-a\"\n\
provider_id = \"source\"\n\
kind = \"source-fs\"\n\
provides = [\"source/list\", \"source/read\", \"source/search\"]\n\
root = \"{}\"\n",
        content.path().display(),
        session.path().display(),
        operation.path().display(),
        notes.display(),
    );
    let config_path = content.path().parent().unwrap().join("host.toml");
    std::fs::write(&config_path, config).expect("write config");
    let binary = std::env::var("CONEX_HOST_BIN")
        .unwrap_or_else(|_| format!("{}/target/debug/conex-host", repo_root().display()));
    let child = std::process::Command::new(binary)
        .arg(config_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn host");
    (port, child, vec![content, session, operation])
}

async fn ready(
    ws: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) {
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": "hello",
            "method": "conex/hello",
            "params": {
                "profileId": "conex-jsonrpc2-wss",
                "plane": "broker",
                "provides": ["ui/probe"],
                "requires": []
            }
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("hello");
    let hello: Value = match ws.next().await.expect("hello frame").expect("hello result") {
        Message::Text(text) => serde_json::from_str(&text).expect("hello json"),
        other => panic!("unexpected hello {other:?}"),
    };
    let negotiation_id = hello["result"]["negotiationId"]
        .as_str()
        .expect("negotiation id");
    ws.send(Message::Text(
        json!({
            "jsonrpc": "2.0",
            "id": "ready",
            "method": "conex/ready",
            "params": {
                "negotiationId": negotiation_id,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn websocket_reader_keeps_processing_while_requests_run() {
    let (port, mut child, _keepalive) = spawn_host();
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let request = format!("ws://127.0.0.1:{port}/wss");
    let mut request = request
        .into_client_request()
        .expect("client request");
    request.headers_mut().insert(
        "Authorization",
        "Bearer e2e-token".parse().expect("authorization"),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connect");
    ready(&mut ws).await;

    for (id, method) in [
        ("01ARZ3NDEKTSV4RRFFQ69G5FAV", "source/list"),
        ("01ARZ3NDEKTSV4RRFFQ69G5FAW", "source/list"),
    ] {
        ws.send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": method,
                "params": {
                    "context": {"providerEndpointId": "notes-local", "plane": "broker"},
                    "timeoutBudgetMs": 8000,
                    "input": {"root": ""}
                }
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("business request");
    }
    let mut seen = Vec::new();
    for _ in 0..2 {
        let response = ws.next().await.expect("response frame").expect("response");
        let Message::Text(text) = response else {
            panic!("expected JSON response");
        };
        let value: Value = serde_json::from_str(&text).expect("response json");
        seen.push(value["id"].as_str().unwrap_or_default().to_owned());
        assert!(value.get("result").is_some(), "business call failed: {value}");
    }
    seen.sort();
    assert_eq!(seen, ["01ARZ3NDEKTSV4RRFFQ69G5FAV", "01ARZ3NDEKTSV4RRFFQ69G5FAW"]);
    let _ = child.kill();
    let _ = child.wait();
}

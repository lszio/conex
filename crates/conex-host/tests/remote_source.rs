use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use axum::extract::ws::Message;
use conex_core::Caller;
use conex_host::serve::{BuiltHost, build};
use conex_host::{
    AgentConfig, EndpointConfig, HostConfig, PendingRequests, PolicyConfig, RemoteLink, TokenConfig,
};
use conex_proto;
use conex_proto::wire::decode_wire;
use prost::Message as _;
use serde_json::json;
use sha2::Digest;
use tokio::sync::mpsc;

fn hash(token: &str) -> String {
    hex::encode(sha2::Sha256::digest(token.as_bytes()))
}

fn config(temp: &tempfile::TempDir) -> HostConfig {
    let root = temp.path();
    HostConfig {
        listen: "127.0.0.1:0".into(),
        allow_loopback_http: true,
        audience: Some("test-host".into()),
        tls: None,
        audit_file: None,
        audit_capacity: None,
        tokens: vec![
            TokenConfig {
                token_hash: hash("agent-a"),
                principal_id: "agent-a".into(),
                tenant_id: "tenant-a".into(),
                audience: "test-host".into(),
                role: "agent".into(),
            },
            TokenConfig {
                token_hash: hash("agent-b"),
                principal_id: "agent-b".into(),
                tenant_id: "tenant-b".into(),
                audience: "test-host".into(),
                role: "agent".into(),
            },
        ],
        policy: vec![
            PolicyConfig {
                principal_id: "alice-a".into(),
                tenant_id: "tenant-a".into(),
                endpoint_id: "endpoint-a".into(),
                actions: vec!["read".into(), "list".into(), "search".into()],
                root: String::new(),
                subtree: true,
            },
            PolicyConfig {
                principal_id: "alice-b".into(),
                tenant_id: "tenant-b".into(),
                endpoint_id: "endpoint-b".into(),
                actions: vec!["read".into(), "list".into(), "search".into()],
                root: String::new(),
                subtree: true,
            },
        ],
        endpoints: vec![
            EndpointConfig {
                id: "endpoint-a".into(),
                tenant_id: "tenant-a".into(),
                provider_id: "provider-a".into(),
                kind: "source-remote".into(),
                provides: vec![
                    "source/read".into(),
                    "source/list".into(),
                    "source/search".into(),
                ],
                root: Some("team".into()),
                origin: None,
                fixed_path: None,
                ca_pem: None,
                expected_server_name: None,
                max_response_bytes: None,
                allow_loopback_http: false,
                credential_name: None,
                credential_backend: None,
                agent_id: Some("agent-a".into()),
            },
            EndpointConfig {
                id: "endpoint-b".into(),
                tenant_id: "tenant-b".into(),
                provider_id: "provider-b".into(),
                kind: "source-remote".into(),
                provides: vec![
                    "source/read".into(),
                    "source/list".into(),
                    "source/search".into(),
                ],
                root: Some("team".into()),
                origin: None,
                fixed_path: None,
                ca_pem: None,
                expected_server_name: None,
                max_response_bytes: None,
                allow_loopback_http: false,
                credential_name: None,
                credential_backend: None,
                agent_id: Some("agent-b".into()),
            },
        ],
        web_origin: None,
        web_root: None,
        agents: vec![
            AgentConfig {
                id: "agent-a".into(),
                tenant_id: "tenant-a".into(),
                display_name: None,
                credential_name: Some("token".into()),
                credential_backend: Some("env:TOKEN_A".into()),
            },
            AgentConfig {
                id: "agent-b".into(),
                tenant_id: "tenant-b".into(),
                display_name: None,
                credential_name: Some("token".into()),
                credential_backend: Some("env:TOKEN_B".into()),
            },
        ],
        content_root: Some(root.join("content")),
        session_root: Some(root.join("session")),
        operation_root: Some(root.join("operation")),
        host_origin: Some("conex://test-host".into()),
        oidc: None,
        web_guest: None,
    }
}

fn response(text: &str) -> serde_json::Value {
    json!({"resource":{"resourceId":"team/same","title":"same","mime":"text/plain","sizeBytes":text.len().to_string(),"kind":"ENTRY_KIND_FILE"},"text":text,"cid":"bafkreicnbk2gkwoowxiq3loyrlyfdd42wwhcyvv67ny33drroaybh4qvxy"})
}

async fn link(
    built: &BuiltHost,
    agent: &str,
    generation: u64,
) -> (Arc<PendingRequests>, mpsc::Receiver<(Message, usize)>) {
    let (tx, rx) = mpsc::channel(8);
    let pending = Arc::new(PendingRequests::new(generation));
    let queue = Arc::new(AtomicUsize::new(0));
    let remote = RemoteLink::new(generation, tx, queue, pending.clone(), 1_048_576);
    let side = built.host_side.as_ref().expect("p1 side");
    side.connections.stage(agent, generation, remote).await;
    // The test config pairs each agent (agent-a/agent-b) with its same-named
    // endpoint; registration grants those accepted ids.
    let accepted: Vec<String> = vec![agent.replace("agent", "endpoint")];
    side.connections
        .activate(agent, generation, &accepted)
        .await
        .expect("activate");
    (pending, rx)
}

async fn reply_once(
    mut rx: mpsc::Receiver<(Message, usize)>,
    pending: Arc<PendingRequests>,
    generation: u64,
    text: &'static str,
) {
    // Agent links are protobuf binary since M3.
    let Some((Message::Binary(frame), _)) = rx.recv().await else {
        panic!("missing remote request")
    };
    let message = conex_proto::Message::decode(frame.as_ref()).expect("wire request");
    let conex_proto::message::Body::Request(request) = message.body.expect("request") else {
        panic!("not request")
    };
    pending
        .route(
            generation,
            &request.request_id,
            Ok(conex_host::ws_transport::AgentReply::Value(response(text))),
        )
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_agents_are_isolated_and_denied_before_send() {
    let temp = tempfile::tempdir().unwrap();
    let built = build(&config(&temp)).unwrap();
    let (pending_a, rx_a) = link(&built, "agent-a", 1).await;
    let (pending_b, rx_b) = link(&built, "agent-b", 2).await;
    let side = built.host_side.as_ref().unwrap().clone();
    tokio::spawn(reply_once(rx_a, pending_a, 1, "A"));
    tokio::spawn(reply_once(rx_b, pending_b, 2, "B"));
    let caller_a = Caller {
        principal_id: "alice-a".into(),
        tenant_id: "tenant-a".into(),
        actor_peer_id: "test".into(),
    };
    let caller_b = Caller {
        principal_id: "alice-b".into(),
        tenant_id: "tenant-b".into(),
        actor_peer_id: "test".into(),
    };
    let a = built
        .host
        .invoke(
            &caller_a,
            "endpoint-a",
            "source/read",
            json!({"resourceId":"team/same"}),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(a["text"], "A");
    let b = built
        .host
        .invoke(
            &caller_b,
            "endpoint-b",
            "source/read",
            json!({"resourceId":"team/same"}),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(b["text"], "B");
    side.connections.remove("agent-b", 2).await;
    let error = built
        .host
        .invoke(
            &caller_b,
            "endpoint-b",
            "source/read",
            json!({"resourceId":"team/same"}),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Unavailable));
    let error = built
        .host
        .invoke(
            &caller_a,
            "endpoint-a",
            "source/read",
            json!({"resourceId":"outside/file"}),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
    let traversal = built
        .host
        .invoke(
            &caller_a,
            "endpoint-a",
            "source/read",
            json!({"resourceId":"team/../secret"}),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
    assert_eq!(
        traversal.code_enum(),
        Some(conex_proto::ErrorCode::BadRequest)
    );
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));
    let wrong_tenant = built
        .host
        .invoke(
            &caller_a,
            "endpoint-b",
            "source/read",
            json!({"resourceId":"team/same"}),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
    assert_eq!(
        wrong_tenant.code_enum(),
        Some(conex_proto::ErrorCode::Forbidden)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_cancels_old_request_and_new_generation_cannot_receive_late_response() {
    let temp = tempfile::tempdir().unwrap();
    let built = build(&config(&temp)).unwrap();
    let (old_pending, mut old_rx) = link(&built, "agent-a", 10).await;
    let old = tokio::spawn(async move {
        let Some((Message::Text(frame), _)) = old_rx.recv().await else {
            return String::new();
        };
        let message = decode_wire(frame.as_bytes()).unwrap();
        let conex_proto::message::Body::Request(request) = message.body.unwrap() else {
            return String::new();
        };
        request.request_id
    });
    let caller = Caller {
        principal_id: "alice-a".into(),
        tenant_id: "tenant-a".into(),
        actor_peer_id: "test".into(),
    };
    let timeout = built
        .host
        .invoke(
            &caller,
            "endpoint-a",
            "source/read",
            json!({"resourceId":"team/same"}),
            Duration::from_millis(20),
        )
        .await
        .unwrap_err();
    assert_eq!(timeout.code_enum(), Some(conex_proto::ErrorCode::Timeout));
    let old_id = old.await.unwrap();
    let (new_pending, new_rx) = link(&built, "agent-a", 11).await;
    assert!(
        !old_pending
            .route(
                10,
                &old_id,
                Ok(conex_host::ws_transport::AgentReply::Value(response(
                    "late"
                )))
            )
            .await
    );
    tokio::spawn(reply_once(new_rx, new_pending, 11, "fresh"));
    let fresh = built
        .host
        .invoke(
            &caller,
            "endpoint-a",
            "source/read",
            json!({"resourceId":"team/same"}),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(fresh["text"], "fresh");
}

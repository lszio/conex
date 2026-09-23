use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;
use conex_core::Caller;
use conex_host::broker::BrokerCall;
use conex_host::config::{AgentConfig, EndpointConfig, HostConfig, PolicyConfig, TokenConfig};
use conex_host::serve::build;
use conex_proto::v1;
use serde_json::json;
use tempfile::TempDir;
use tokio::time::Instant;

fn config(dir: &TempDir) -> HostConfig {
    let notes = dir.path().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(notes.join("secret.md"), "secret").unwrap();
    let root = dir.path().join("p1");
    HostConfig {
        listen: "127.0.0.1:0".into(),
        allow_loopback_http: true,
        audience: Some("host.local".into()),
        tls: None,
        audit_file: None,
        audit_capacity: None,
        tokens: vec![
            TokenConfig {
                token_hash: "0".repeat(64),
                principal_id: "alice".into(),
                tenant_id: "tenant-a".into(),
                audience: "host.local".into(),
                role: "ui".into(),
            },
            TokenConfig {
                token_hash: "1".repeat(64),
                principal_id: "agent-a".into(),
                tenant_id: "tenant-a".into(),
                audience: "host.local".into(),
                role: "agent".into(),
            },
        ],
        policy: vec![
            PolicyConfig {
                principal_id: "alice".into(),
                tenant_id: "tenant-a".into(),
                endpoint_id: "notes-local".into(),
                actions: vec!["read".into(), "list".into()],
                root: "team".into(),
                subtree: true,
            },
            PolicyConfig {
                principal_id: "alice".into(),
                tenant_id: "tenant-a".into(),
                endpoint_id: "notes-remote".into(),
                actions: vec!["read".into()],
                root: "team".into(),
                subtree: true,
            },
        ],
        endpoints: vec![
            EndpointConfig {
                id: "notes-local".into(),
                tenant_id: "tenant-a".into(),
                provider_id: "notes".into(),
                kind: "source-fs".into(),
                provides: vec![
                    "source/list".into(),
                    "source/read".into(),
                    "source/search".into(),
                ],
                root: Some(notes),
                origin: None,
                fixed_path: None,
                ca_pem: None,
                expected_server_name: None,
                max_response_bytes: None,
                allow_loopback_http: false,
                credential_name: None,
                credential_backend: None,
                agent_id: None,
            },
            EndpointConfig {
                id: "notes-remote".into(),
                tenant_id: "tenant-a".into(),
                provider_id: "notes".into(),
                kind: "source-remote".into(),
                provides: vec!["source/read".into()],
                root: Some(PathBuf::from("team")),
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
        ],
        web_origin: None,
        web_root: None,
        agents: vec![AgentConfig {
            id: "agent-a".into(),
            tenant_id: "tenant-a".into(),
            display_name: Some("Private notes".into()),
            credential_name: Some("agent-a".into()),
            credential_backend: Some("env:CONEX_AGENT_A".into()),
        }],
        content_root: Some(root.join("content")),
        session_root: Some(root.join("session")),
        operation_root: Some(root.join("operation")),
        host_origin: Some("conex://broker.local".into()),
        oidc: None,
    }
}

fn caller(principal_id: &str) -> Caller {
    Caller {
        principal_id: principal_id.into(),
        tenant_id: "tenant-a".into(),
        actor_peer_id: "test".into(),
    }
}

#[tokio::test]
async fn catalog_filters_scopes_paginates_and_keeps_remote_offline_visible() {
    let dir = tempfile::tempdir().unwrap();
    let built = build(&config(&dir)).unwrap();
    let catalog = built.catalog.clone().unwrap();
    let alice = caller("alice");

    let page = catalog.list(&alice, json!({"limit": 1})).await.unwrap();
    let page: v1::EndpointListResult = serde_json::from_value(page).unwrap();
    assert_eq!(page.endpoints.len(), 1);
    assert_eq!(page.endpoints[0].endpoint_id, "notes-local");
    assert_eq!(page.endpoints[0].available_methods, ["source/list", "source/read"]);
    assert_eq!(page.endpoints[0].connection_state, v1::ConnectionState::NotApplicable as i32);
    assert!(page.endpoints[0].authorized_scopes.iter().all(|scope| scope.root == "team"));
    assert_eq!(page.next_after_endpoint_id.as_deref(), Some("notes-local"));

    let second = catalog
        .list(&alice, json!({"afterEndpointId":"notes-local"}))
        .await
        .unwrap();
    let second: v1::EndpointListResult = serde_json::from_value(second).unwrap();
    assert_eq!(second.endpoints.len(), 1);
    assert_eq!(second.endpoints[0].display_name, "Private notes");
    assert_eq!(second.endpoints[0].endpoint_id, "notes-remote");
    assert_eq!(second.endpoints[0].connection_state, v1::ConnectionState::Offline as i32);
    let bob = catalog.list(&caller("bob"), json!({})).await.unwrap();
    let bob: v1::EndpointListResult = serde_json::from_value(bob).unwrap();
    assert!(bob.endpoints.is_empty());

    let encoded = serde_json::to_string(&second).unwrap();
    assert!(!encoded.contains(dir.path().to_string_lossy().as_ref()));

    let error = built
        .host
        .invoke(
            &alice,
            "notes-local",
            "source/read",
            json!({"resourceId":"outside.md"}),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code_enum(), Some(v1::ErrorCode::Forbidden));
}

#[tokio::test]
async fn broker_endpoint_list_strictly_decodes_input_and_hides_other_principals() {
    let dir = tempfile::tempdir().unwrap();
    let built = build(&config(&dir)).unwrap();
    let broker = built.broker.unwrap();
    let alice = caller("alice");
    let base = || BrokerCall {
        caller: alice.clone(),
        endpoint_id: String::new(),
        method: "endpoint/list".into(),
        input: json!({}),
        deadline: Instant::now() + Duration::from_secs(1),
        role: "service".into(),
    };
    let result = broker.invoke(base()).await.unwrap();
    let result: v1::EndpointListResult = serde_json::from_value(result).unwrap();
    assert_eq!(result.endpoints.len(), 2);

    let mut invalid = base();
    invalid.input = json!({"limit": 101});
    assert_eq!(broker.invoke(invalid).await.unwrap_err().code_enum(), Some(v1::ErrorCode::BadRequest));
    let mut unknown = base();
    unknown.input = json!({"unexpected": true});
    assert_eq!(broker.invoke(unknown).await.unwrap_err().code_enum(), Some(v1::ErrorCode::BadRequest));
}
#[tokio::test]
async fn catalog_reflects_remote_ready_state() {
    let dir = tempfile::tempdir().unwrap();
    let built = build(&config(&dir)).unwrap();
    let side = built.host_side.as_ref().unwrap();
    let (tx, _rx) = tokio::sync::mpsc::channel(1);
    let link = conex_host::remote::RemoteLink::new(
        7,
        tx,
        Arc::new(AtomicUsize::new(0)),
        Arc::new(conex_host::ws_transport::PendingRequests::new(7)),
        1024,
    );
    side.prepare_connection("agent-a", link).await;
    side.activate_connection("agent-a", 7).await.unwrap();

    let value = built
        .catalog
        .unwrap()
        .list(&caller("alice"), json!({"afterEndpointId":"notes-local"}))
        .await
        .unwrap();
    let result: v1::EndpointListResult = serde_json::from_value(value).unwrap();
    assert_eq!(
        result.endpoints[0].connection_state,
        v1::ConnectionState::Ready as i32
    );
}

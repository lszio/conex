use conex_core::Caller;
use conex_host::broker::BrokerCall;
use conex_host::config::{AgentConfig, EndpointConfig, HostConfig, PolicyConfig, TokenConfig};
use conex_host::serve::build;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;
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
        allow_plaintext_bind: false,
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
        web_guest: None,
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
    let page: conex_proto::EndpointListResult = serde_json::from_value(page).unwrap();
    assert_eq!(page.endpoints.len(), 1);
    assert_eq!(page.endpoints[0].endpoint_id, "notes-local");
    assert_eq!(
        page.endpoints[0].available_methods,
        ["source/list", "source/read"]
    );
    assert_eq!(
        page.endpoints[0].connection_state,
        conex_proto::ConnectionState::NotApplicable as i32
    );
    assert!(
        page.endpoints[0]
            .authorized_scopes
            .iter()
            .all(|scope| scope.root == "team")
    );
    assert_eq!(page.next_after_endpoint_id.as_deref(), Some("notes-local"));

    let second = catalog
        .list(&alice, json!({"afterEndpointId":"notes-local"}))
        .await
        .unwrap();
    let second: conex_proto::EndpointListResult = serde_json::from_value(second).unwrap();
    assert_eq!(second.endpoints.len(), 1);
    assert_eq!(second.endpoints[0].display_name, "Private notes");
    assert_eq!(second.endpoints[0].endpoint_id, "notes-remote");
    assert_eq!(
        second.endpoints[0].connection_state,
        conex_proto::ConnectionState::Offline as i32
    );
    let bob = catalog.list(&caller("bob"), json!({})).await.unwrap();
    let bob: conex_proto::EndpointListResult = serde_json::from_value(bob).unwrap();
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
    assert_eq!(error.code_enum(), Some(conex_proto::ErrorCode::Forbidden));
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
        link_id: None,
    };
    let result = broker.invoke(base()).await.unwrap();
    let result: conex_proto::EndpointListResult = serde_json::from_value(result).unwrap();
    assert_eq!(result.endpoints.len(), 2);

    let mut invalid = base();
    invalid.input = json!({"limit": 101});
    assert_eq!(
        broker.invoke(invalid).await.unwrap_err().code_enum(),
        Some(conex_proto::ErrorCode::BadRequest)
    );
    let mut unknown = base();
    unknown.input = json!({"unexpected": true});
    assert_eq!(
        broker.invoke(unknown).await.unwrap_err().code_enum(),
        Some(conex_proto::ErrorCode::BadRequest)
    );
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
    side.stage_connection("agent-a", link).await;
    side.activate_connection("agent-a", 7, &["notes-remote".to_string()])
        .await
        .unwrap();

    let value = built
        .catalog
        .unwrap()
        .list(&caller("alice"), json!({"afterEndpointId":"notes-local"}))
        .await
        .unwrap();
    let result: conex_proto::EndpointListResult = serde_json::from_value(value).unwrap();
    assert_eq!(
        result.endpoints[0].connection_state,
        conex_proto::ConnectionState::Ready as i32
    );
}

/// Anonymous visitors share one principal (plan M1.1): connection/list must
/// isolate each visitor to its own browser link and never expose agent
/// resource declarations to them.
#[tokio::test]
async fn connection_list_isolates_guest_visitors_and_hides_agent_links() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(&dir);
    config.web_guest = Some(conex_host::config::WebGuestConfig {
        principal_id: "visitor".into(),
        tenant_id: "tenant-a".into(),
        max_sessions: 16,
        idle_ttl_ms: 600_000,
        max_issue_per_minute: 60,
    });
    let built = build(&config).unwrap();
    let broker = built.broker.unwrap();
    let ui_links = Arc::new(conex_host::ui_links::UiLinkRegistry::new());
    broker.attach_ui_links(ui_links.clone());

    let link_a = ui_links.register("visitor", "tenant-a");
    let link_b = ui_links.register("visitor", "tenant-a");
    ui_links.register("alice", "tenant-a");

    let connection_list = |caller: Caller, link_id: String| BrokerCall {
        caller,
        endpoint_id: String::new(),
        method: "connection/list".into(),
        input: json!({}),
        deadline: Instant::now() + Duration::from_secs(1),
        role: "ui".into(),
        link_id: Some(link_id),
    };

    let result = broker
        .invoke(connection_list(caller("visitor"), link_a.link_id.clone()))
        .await
        .unwrap();
    let browser = result["browserLinks"].as_array().unwrap();
    assert_eq!(browser.len(), 1, "guest sees only its own link");
    assert_eq!(browser[0]["linkId"], link_a.link_id.as_str());
    assert!(
        result["agentLinks"].as_array().unwrap().is_empty(),
        "guests must not receive agent resource declarations"
    );

    // A different visitor session never sees visitor A's activity.
    let result_b = broker
        .invoke(connection_list(caller("visitor"), link_b.link_id.clone()))
        .await
        .unwrap();
    let browser_b = result_b["browserLinks"].as_array().unwrap();
    assert_eq!(browser_b.len(), 1);
    assert_eq!(browser_b[0]["linkId"], link_b.link_id.as_str());

    // Authenticated principals keep the aggregated panel semantics.
    let result_alice = broker
        .invoke(connection_list(caller("alice"), "unused".into()))
        .await
        .unwrap();
    let browser_alice = result_alice["browserLinks"].as_array().unwrap();
    assert_eq!(browser_alice.len(), 1);
    assert_eq!(browser_alice[0]["principalId"], "alice");
}

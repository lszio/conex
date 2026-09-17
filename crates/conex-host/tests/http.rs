//! The /rpc entry authenticates, validates bindings and reuses the one Host.
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use conex_core::{
    AllowedTarget, AuditEnd, AuditReservation, AuditSink, AuditStart, CallError, CallResult,
    Caller, Connection, Connector, CredentialKey, CredentialStore, Endpoint, FactoryKey, Host,
    HostLimits, Installation, Limits, Policy, PolicyRule, Registry, Resolver, Secret, StaticPolicy,
    TargetPolicy, VerifiedPeer,
};
use conex_host::{
    BindingStore, HttpState, PROFILE_ID, StaticBearerAuth, TokenRecord, attach_state,
    build_router as build_p0_router,
};
use conex_proto::v1;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::Instant;

const ULID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

struct UnusedResolver;
#[async_trait]
impl Resolver for UnusedResolver {
    async fn resolve(&self, _hostname: &str, _deadline: Instant) -> CallResult<Vec<IpAddr>> {
        Err(CallError::new(
            v1::ErrorCode::Unavailable,
            "unused resolver",
        ))
    }
}

struct UnusedConnector;
#[async_trait]
impl Connector for UnusedConnector {
    async fn connect(
        &self,
        _target: &AllowedTarget,
        _deadline: Instant,
    ) -> CallResult<Box<dyn Connection>> {
        Err(CallError::new(
            v1::ErrorCode::Unavailable,
            "unused connector",
        ))
    }
}

struct UnusedCredentials;
#[async_trait]
impl CredentialStore for UnusedCredentials {
    async fn resolve(&self, _key: &CredentialKey, _peer: &VerifiedPeer) -> CallResult<Secret> {
        Err(CallError::new(
            v1::ErrorCode::Unavailable,
            "unused credentials",
        ))
    }
}

struct NullAudit;
struct NullReservation;
impl AuditReservation for NullReservation {
    fn finish(self: Box<Self>, _end: AuditEnd) -> CallResult<()> {
        Ok(())
    }
}
impl AuditSink for NullAudit {
    fn reserve(&self, _start: &AuditStart) -> CallResult<Box<dyn AuditReservation>> {
        Ok(Box::new(NullReservation))
    }
}

struct HttpFixture {
    host: Arc<Host>,
    addr: SocketAddr,
    alice: Caller,
    _dir: tempfile::TempDir,
}

impl HttpFixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.md"), "hello conex\n").unwrap();

        let mut registry = Registry::new();
        registry
            .register_factory(
                FactoryKey {
                    kind: "source-fs".into(),
                    protocol: "conex".into(),
                    version: 1,
                },
                conex_provider_fs::factory,
            )
            .unwrap();
        registry
            .install(Installation {
                endpoint: Endpoint {
                    id: "notes".into(),
                    provider_id: "source".into(),
                    tenant_id: "tenant-a".into(),
                    plane: v1::Plane::Broker,
                    provides: vec![
                        "source/list".into(),
                        "source/read".into(),
                        "source/search".into(),
                    ],
                    limits: Limits::default(),
                },
                factory: FactoryKey {
                    kind: "source-fs".into(),
                    protocol: "conex".into(),
                    version: 1,
                },
                provider: serde_json::json!({"root": dir.path()}),
                target: None,
                credential: None,
            })
            .unwrap();

        let policy = Arc::new(StaticPolicy::new(vec![PolicyRule {
            principal_id: "alice".into(),
            tenant_id: "tenant-a".into(),
            endpoint_id: "notes".into(),
            actions: vec!["list".into(), "read".into(), "search".into()],
            root: String::new(),
            subtree: true,
        }]));

        let host = Arc::new(
            Host::new(
                registry,
                policy as Arc<dyn Policy>,
                Arc::new(TargetPolicy::new()),
                Arc::new(UnusedResolver),
                Arc::new(UnusedConnector),
                Arc::new(UnusedCredentials),
                Arc::new(NullAudit),
                HostLimits::default(),
            )
            .unwrap(),
        );

        let mut capabilities = HashMap::new();
        capabilities.insert(
            "tenant-a".to_string(),
            vec![
                "source/list".to_string(),
                "source/read".to_string(),
                "source/search".to_string(),
            ],
        );
        let bindings = Arc::new(BindingStore::new(
            PROFILE_ID,
            "host.example",
            capabilities,
            Limits::default(),
        ));
        let auth = Arc::new(StaticBearerAuth::new(vec![
            TokenRecord::from_plaintext("test-token", "alice", "tenant-a", "host.example"),
            TokenRecord::from_plaintext("bob-token", "bob", "tenant-a", "host.example"),
        ]));
        let app = attach_state(
            Arc::new(HttpState {
                host: host.clone(),
                bindings,
                auth,
                broker: None,
                host_side: None,
                p1_provides: Vec::new(),
            }),
            build_p0_router(),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        Self {
            host,
            addr,
            alice: Caller {
                principal_id: "alice".into(),
                tenant_id: "tenant-a".into(),
                actor_peer_id: "inbound-http".into(),
            },
            _dir: dir,
        }
    }
}

async fn rpc(addr: SocketAddr, authorization: Option<&str>, body: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let auth = authorization
        .map(|value| format!("authorization: {value}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST /rpc HTTP/1.1\r\nhost: localhost\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n{}\r\n{}",
        body.len(),
        auth,
        body
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).to_string();
    let status: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
    (status, body)
}

fn hello_body() -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":\"{ULID}\",\"method\":\"conex/hello\",\"params\":{{\"context\":{{\"providerEndpointId\":\"\",\"plane\":\"broker\"}},\"timeoutBudgetMs\":8000,\"input\":{{\"profileId\":\"{PROFILE_ID}\",\"plane\":\"broker\",\"provides\":[],\"requires\":[\"source/read\"]}}}}}}"
    )
}

fn read_body(binding_id: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":\"{ULID}\",\"method\":\"source/read\",\"params\":{{\"context\":{{\"providerEndpointId\":\"notes\",\"plane\":\"broker\",\"bindingId\":\"{binding_id}\"}},\"timeoutBudgetMs\":8000,\"input\":{{\"resourceId\":\"hello.md\"}}}}}}"
    )
}

fn binding_id(body: &str) -> String {
    let value: Value = serde_json::from_str(body).unwrap();
    value["result"]["bindingId"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn missing_authorization_returns_401() {
    let fixture = HttpFixture::new().await;
    let (status, _) = rpc(fixture.addr, None, &hello_body()).await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn hello_then_read_matches_inproc() {
    let fixture = HttpFixture::new().await;
    let (status, hello) = rpc(fixture.addr, Some("Bearer test-token"), &hello_body()).await;
    assert_eq!(status, 200, "{hello}");
    let binding = binding_id(&hello);

    let (status, read) = rpc(
        fixture.addr,
        Some("Bearer test-token"),
        &read_body(&binding),
    )
    .await;
    assert_eq!(status, 200, "{read}");
    let value: Value = serde_json::from_str(&read).unwrap();
    assert_eq!(value["result"]["text"], "hello conex\n");

    let inproc = fixture
        .host
        .invoke(
            &fixture.alice,
            "notes",
            "source/read",
            serde_json::json!({"resourceId": "hello.md"}),
            Duration::from_secs(2),
        )
        .await
        .unwrap();
    assert_eq!(inproc["text"], value["result"]["text"]);
    assert_eq!(inproc["cid"], value["result"]["cid"]);
}

#[tokio::test]
async fn another_principal_cannot_reuse_a_binding() {
    let fixture = HttpFixture::new().await;
    let (_, hello) = rpc(fixture.addr, Some("Bearer test-token"), &hello_body()).await;
    let binding = binding_id(&hello);
    let (status, body) = rpc(fixture.addr, Some("Bearer bob-token"), &read_body(&binding)).await;
    assert_eq!(status, 200, "{body}");
    let value: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["error"]["code"], serde_json::json!(-32002));
}

#[tokio::test]
async fn wrong_media_type_is_rejected() {
    let fixture = HttpFixture::new().await;
    let mut stream = TcpStream::connect(fixture.addr).await.unwrap();
    let body = hello_body();
    let request = format!(
        "POST /rpc HTTP/1.1\r\nhost: localhost\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).to_string();
    let status: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    assert_eq!(status, 415);
}

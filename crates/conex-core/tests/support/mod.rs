//! Shared test rig. Counters live at the port boundary, not inside the Host.
#![allow(dead_code)]
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use conex_core::{
    AllowedTarget, AuditEnd, AuditReservation, AuditSink, AuditStart, CallContext, CallError,
    CallResult, Caller, Connection, Connector, CredentialKey, CredentialStore, Endpoint,
    ExecutionIo, FactoryKey, Handler, Host, HostLimits, Installation, Limits, MethodContract,
    OutboundRequest, OutboundResponse, PeerVerification, PolicyRule, PreparedInput, Registry,
    Resolver, ResourceClaim, Route, Secret, StaticPolicy, Target, TargetPolicy, TlsTrust,
    VerifiedPeer,
};
use conex_proto::v1;
use serde_json::{Value, json};
use tokio::time::Instant;

static HANDLERS: OnceLock<Mutex<HashMap<u64, Arc<dyn Handler>>>> = OnceLock::new();
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

fn handlers() -> &'static Mutex<HashMap<u64, Arc<dyn Handler>>> {
    HANDLERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_handler(handler: Arc<dyn Handler>) -> u64 {
    let token = NEXT_TOKEN.fetch_add(1, Ordering::SeqCst);
    handlers().lock().unwrap().insert(token, handler);
    token
}

fn fixture_key() -> FactoryKey {
    FactoryKey {
        kind: "fixture".into(),
        protocol: "conex".into(),
        version: 1,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub connect: usize,
    pub resolve: usize,
    pub execute: usize,
}

#[derive(Debug, Clone)]
pub struct TestEvent {
    pub phase: String,
    pub outcome: String,
    pub error_code: Option<i32>,
}

struct Counters {
    connect: AtomicUsize,
    resolve: AtomicUsize,
    execute: AtomicUsize,
}

impl Counters {
    fn new() -> Self {
        Self {
            connect: AtomicUsize::new(0),
            resolve: AtomicUsize::new(0),
            execute: AtomicUsize::new(0),
        }
    }

    fn snapshot(&self) -> Counts {
        Counts {
            connect: self.connect.load(Ordering::SeqCst),
            resolve: self.resolve.load(Ordering::SeqCst),
            execute: self.execute.load(Ordering::SeqCst),
        }
    }
}

struct CountingHandler {
    counts: Arc<Counters>,
    sleep: Option<Duration>,
}

#[async_trait]
impl Handler for CountingHandler {
    async fn execute(
        &self,
        _ctx: &CallContext,
        input: Value,
        _io: ExecutionIo,
    ) -> CallResult<Value> {
        self.counts.execute.fetch_add(1, Ordering::SeqCst);
        if let Some(delay) = self.sleep {
            tokio::time::sleep(delay).await;
        }
        Ok(json!({"text": "ok", "echo": input}))
    }
}

struct FakeResolver;

#[async_trait]
impl Resolver for FakeResolver {
    async fn resolve(&self, _hostname: &str, _deadline: Instant) -> CallResult<Vec<IpAddr>> {
        Ok(vec!["198.51.100.7".parse().unwrap()])
    }
}

struct FakeConnection {
    peer: VerifiedPeer,
}

#[async_trait]
impl Connection for FakeConnection {
    fn peer(&self) -> &VerifiedPeer {
        &self.peer
    }

    async fn request(
        &mut self,
        _request: OutboundRequest,
        _deadline: Instant,
    ) -> CallResult<OutboundResponse> {
        Ok(OutboundResponse {
            status: 200,
            headers: http::HeaderMap::new(),
            body: bytes::Bytes::new(),
        })
    }
}

struct FakeConnector {
    counts: Arc<Counters>,
    policy: Arc<StaticPolicy>,
    fail: bool,
    revoke: bool,
}

#[async_trait]
impl Connector for FakeConnector {
    async fn connect(
        &self,
        _target: &AllowedTarget,
        _deadline: Instant,
    ) -> CallResult<Box<dyn Connection>> {
        self.counts.connect.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(CallError::new(
                v1::ErrorCode::PeerUntrusted,
                "tls verification failed",
            ));
        }
        if self.revoke {
            self.policy.replace_rules(vec![]);
        }
        Ok(Box::new(FakeConnection {
            peer: VerifiedPeer {
                target_id: "catalog".into(),
                audience: "catalog.example".into(),
                address: "198.51.100.7:443".parse().unwrap(),
                verification: PeerVerification::TlsServer,
            },
        }))
    }
}

struct FakeCredentials {
    counts: Arc<Counters>,
}

#[async_trait]
impl CredentialStore for FakeCredentials {
    async fn resolve(&self, _key: &CredentialKey, _peer: &VerifiedPeer) -> CallResult<Secret> {
        self.counts.resolve.fetch_add(1, Ordering::SeqCst);
        Ok(Secret::new("test-secret"))
    }
}

struct TestAudit {
    events: Arc<Mutex<Vec<TestEvent>>>,
    fail: bool,
}

struct TestReservation {
    events: Arc<Mutex<Vec<TestEvent>>>,
}

impl AuditReservation for TestReservation {
    fn finish(self: Box<Self>, end: AuditEnd) -> CallResult<()> {
        self.events.lock().unwrap().push(TestEvent {
            phase: end.phase,
            outcome: end.outcome,
            error_code: end.error_code,
        });
        Ok(())
    }
}

impl AuditSink for TestAudit {
    fn reserve(&self, _start: &AuditStart) -> CallResult<Box<dyn AuditReservation>> {
        if self.fail {
            return Err(CallError::new(
                v1::ErrorCode::Unavailable,
                "audit sink unavailable",
            ));
        }
        Ok(Box::new(TestReservation {
            events: self.events.clone(),
        }))
    }
}

fn prepare_test_read(value: &Value) -> CallResult<PreparedInput> {
    let map = value
        .as_object()
        .ok_or_else(|| CallError::new(v1::ErrorCode::BadRequest, "input must be an object"))?;
    for key in map.keys() {
        if key != "resourceId" {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "unknown input field",
            ));
        }
    }
    let resource_id = map
        .get("resourceId")
        .and_then(Value::as_str)
        .unwrap_or("ok.md")
        .to_string();
    Ok(PreparedInput {
        canonical: value.clone(),
        claim: ResourceClaim {
            resource_id,
            action: "read".into(),
            subtree: false,
        },
        ..Default::default()
    })
}

fn validate_test_output(_: &Value) -> CallResult<()> {
    Ok(())
}

fn test_factory(installation: &Installation) -> CallResult<Vec<Route>> {
    let token = installation
        .provider
        .get("handlerToken")
        .and_then(Value::as_u64)
        .ok_or_else(|| CallError::new(v1::ErrorCode::Internal, "missing handler token"))?;
    let handler = handlers()
        .lock()
        .unwrap()
        .get(&token)
        .cloned()
        .ok_or_else(|| CallError::new(v1::ErrorCode::Internal, "missing handler"))?;
    Ok(installation
        .endpoint
        .provides
        .iter()
        .map(|method| Route {
            protocol: "conex".into(),
            version: 1,
            endpoint: installation.endpoint.clone(),
            method: method.clone(),
            contract: MethodContract {
                adapter_id: "test",
                input_schema: "test.ReadRequest",
                output_schema: "test.ReadResponse",
                prepare: prepare_test_read,
                validate_output: validate_test_output,
            },
            handler: handler.clone(),
            target: installation.target.clone(),
            credential: installation.credential.clone(),
        })
        .collect())
}

fn catalog_target() -> Target {
    Target {
        id: "catalog".into(),
        origin: "https://catalog.example".into(),
        fixed_path: "/".into(),
        allowed_addresses: vec![],
        tls_trust: TlsTrust::default(),
        allow_loopback_http: false,
    }
}

fn catalog_credential() -> CredentialKey {
    CredentialKey {
        tenant_id: "tenant-a".into(),
        holder: "host".into(),
        provider_id: "catalog".into(),
        audience: "catalog.example".into(),
        name: "catalog/key".into(),
        principal_id: None,
    }
}

fn endpoint(id: &str) -> Endpoint {
    Endpoint {
        id: id.into(),
        provider_id: "source".into(),
        tenant_id: "tenant-a".into(),
        plane: v1::Plane::Broker,
        provides: vec!["test/read".into()],
        limits: Limits::default(),
    }
}

struct RigConfig {
    allow: bool,
    fail_connect: bool,
    revoke: bool,
    with_target: bool,
    fail_audit: bool,
    sleep: Option<Duration>,
    slow_endpoint: bool,
}

pub struct TestRig {
    pub host: Host,
    pub policy: Arc<StaticPolicy>,
    counts: Arc<Counters>,
    events: Arc<Mutex<Vec<TestEvent>>>,
    caller: Caller,
}

impl TestRig {
    pub fn new(allow: bool) -> Self {
        Self::build(RigConfig {
            allow,
            fail_connect: false,
            revoke: false,
            with_target: true,
            fail_audit: false,
            sleep: None,
            slow_endpoint: false,
        })
    }

    pub fn failing_connect(allow: bool) -> Self {
        Self::build(RigConfig {
            allow,
            fail_connect: true,
            revoke: false,
            with_target: true,
            fail_audit: false,
            sleep: None,
            slow_endpoint: false,
        })
    }

    pub fn revoked_during_connect() -> Self {
        Self::build(RigConfig {
            allow: true,
            fail_connect: false,
            revoke: true,
            with_target: true,
            fail_audit: false,
            sleep: None,
            slow_endpoint: false,
        })
    }

    pub fn local_only(allow: bool) -> Self {
        Self::build(RigConfig {
            allow,
            fail_connect: false,
            revoke: false,
            with_target: false,
            fail_audit: false,
            sleep: None,
            slow_endpoint: false,
        })
    }

    pub fn with_failing_audit() -> Self {
        Self::build(RigConfig {
            allow: true,
            fail_connect: false,
            revoke: false,
            with_target: true,
            fail_audit: true,
            sleep: None,
            slow_endpoint: false,
        })
    }

    pub fn slow(allow: bool, sleep: Duration) -> Self {
        Self::build(RigConfig {
            allow,
            fail_connect: false,
            revoke: false,
            with_target: true,
            fail_audit: false,
            sleep: Some(sleep),
            slow_endpoint: false,
        })
    }

    pub fn aggregate() -> Self {
        Self::build(RigConfig {
            allow: true,
            fail_connect: false,
            revoke: false,
            with_target: false,
            fail_audit: false,
            sleep: None,
            slow_endpoint: true,
        })
    }

    fn build(config: RigConfig) -> Self {
        let counts = Arc::new(Counters::new());
        let events = Arc::new(Mutex::new(Vec::new()));

        let rules = if config.allow {
            vec![
                PolicyRule {
                    principal_id: "alice".into(),
                    tenant_id: "tenant-a".into(),
                    endpoint_id: "test".into(),
                    actions: vec!["read".into()],
                    root: String::new(),
                    subtree: true,
                },
                PolicyRule {
                    principal_id: "alice".into(),
                    tenant_id: "tenant-a".into(),
                    endpoint_id: "test-slow".into(),
                    actions: vec!["read".into()],
                    root: String::new(),
                    subtree: true,
                },
            ]
        } else {
            vec![]
        };
        let policy = Arc::new(StaticPolicy::new(rules));

        let mut registry = Registry::new();
        registry
            .register_factory(fixture_key(), test_factory)
            .unwrap();

        let handler: Arc<dyn Handler> = Arc::new(CountingHandler {
            counts: counts.clone(),
            sleep: config.sleep,
        });
        let token = register_handler(handler);
        let (target, credential) = if config.with_target {
            (Some(catalog_target()), Some(catalog_credential()))
        } else {
            (None, None)
        };
        registry
            .install(Installation {
                endpoint: endpoint("test"),
                factory: fixture_key(),
                provider: json!({"handlerToken": token}),
                target,
                credential,
            })
            .unwrap();

        if config.slow_endpoint {
            let slow: Arc<dyn Handler> = Arc::new(CountingHandler {
                counts: counts.clone(),
                sleep: Some(Duration::from_secs(30)),
            });
            let slow_token = register_handler(slow);
            registry
                .install(Installation {
                    endpoint: endpoint("test-slow"),
                    factory: fixture_key(),
                    provider: json!({"handlerToken": slow_token}),
                    target: None,
                    credential: None,
                })
                .unwrap();
        }

        let connector = Arc::new(FakeConnector {
            counts: counts.clone(),
            policy: policy.clone(),
            fail: config.fail_connect,
            revoke: config.revoke,
        });
        let host = Host::new(
            registry,
            policy.clone(),
            Arc::new(TargetPolicy::new()),
            Arc::new(FakeResolver),
            connector,
            Arc::new(FakeCredentials {
                counts: counts.clone(),
            }),
            Arc::new(TestAudit {
                events: events.clone(),
                fail: config.fail_audit,
            }),
            HostLimits::default(),
        )
        .unwrap();

        Self {
            host,
            policy,
            counts,
            events,
            caller: Caller {
                principal_id: "alice".into(),
                tenant_id: "tenant-a".into(),
                actor_peer_id: "peer-1".into(),
            },
        }
    }

    pub fn caller(&self) -> Caller {
        self.caller.clone()
    }

    pub fn counts(&self) -> Counts {
        self.counts.snapshot()
    }

    pub fn events(&self) -> Vec<TestEvent> {
        self.events.lock().unwrap().clone()
    }
}

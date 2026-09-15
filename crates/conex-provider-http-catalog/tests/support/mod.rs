//! End-to-end catalog fixture: real TLS server, real Host, injected secret reader.
#![allow(dead_code)]
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use conex_core::{
    AuditEnd, AuditReservation, AuditSink, AuditStart, CallResult, Caller, CredentialKey, Endpoint,
    FactoryKey, Host, HostLimits, Installation, Limits, Policy, PolicyRule, Registry, Resolver,
    StaticPolicy, Target, TargetPolicy, TlsTrust,
};
use conex_host::{CredentialBackend, CredentialBinding, EnvFileStore, SecretReader};
use conex_proto::v1;
use conex_transport_http::{HttpConnector, TlsTrustConfig};
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::Instant;
use tokio_rustls::TlsAcceptor;

pub const CALLER_PRINCIPAL: &str = "alice";
pub const TENANT: &str = "tenant-a";
pub const ENDPOINT: &str = "catalog-work";

pub fn catalog_body() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "entries": [
            {"resourceId": "hello.md", "title": "Hello", "mime": "text/markdown", "text": "hello conex\n"},
            {"resourceId": "team/design.org", "title": "Design", "mime": "text/plain", "text": "* conex routing\n"}
        ]
    }))
    .unwrap()
}

pub struct CatalogServer {
    pub addr: SocketAddr,
    pub ca_pem: Vec<u8>,
    pub gets: Arc<AtomicUsize>,
}

impl CatalogServer {
    pub async fn start(body: Vec<u8>, status: u16) -> CatalogServer {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "conex catalog test CA");
        let ca_issuer = CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();

        let leaf_key = KeyPair::generate().unwrap();
        let mut leaf_params = CertificateParams::new(vec!["catalog.example".to_string()]).unwrap();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "catalog.example");
        let leaf_cert = leaf_params.signed_by(&leaf_key, &ca_issuer).unwrap();

        let chain = vec![
            CertificateDer::from(leaf_cert.der().to_vec()),
            CertificateDer::from(ca_issuer.der().to_vec()),
        ];
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let gets = Arc::new(AtomicUsize::new(0));
        let counter = gets.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let acceptor = acceptor.clone();
                let counter = counter.clone();
                let body = body.clone();
                tokio::spawn(async move {
                    let Ok(mut stream) = acceptor.accept(socket).await else {
                        return;
                    };
                    let mut buffer = [0u8; 2048];
                    let _ = stream.read(&mut buffer).await;
                    counter.fetch_add(1, Ordering::SeqCst);
                    let reason = match status {
                        200 => "200 OK",
                        302 => "302 Found",
                        other => {
                            let _ = other;
                            "500 Internal Server Error"
                        }
                    };
                    let header = format!(
                        "HTTP/1.1 {reason}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                    let _ = stream.flush().await;
                });
            }
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        CatalogServer {
            addr,
            ca_pem: ca_issuer.pem().into_bytes(),
            gets,
        }
    }

    pub fn get_count(&self) -> usize {
        self.gets.load(Ordering::SeqCst)
    }
}

struct StaticResolver;
#[async_trait]
impl Resolver for StaticResolver {
    async fn resolve(&self, _hostname: &str, _deadline: Instant) -> CallResult<Vec<IpAddr>> {
        Ok(vec!["127.0.0.1".parse().unwrap()])
    }
}

struct FixedReader {
    secret: String,
}
#[async_trait]
impl SecretReader for FixedReader {
    async fn read_env(&self, _name: &str) -> CallResult<String> {
        Ok(self.secret.clone())
    }
    async fn read_file(&self, _path: &Path) -> CallResult<String> {
        Ok(self.secret.clone())
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

pub struct CatalogFixture {
    pub host: Host,
    pub caller: Caller,
    pub server: CatalogServer,
    pub policy: Arc<StaticPolicy>,
}

impl CatalogFixture {
    pub async fn source_wide() -> Self {
        Self::build(200, vec![("read", ""), ("list", ""), ("search", "")]).await
    }

    pub async fn with_file_only_permission() -> Self {
        Self::build(200, vec![("read", "team")]).await
    }

    pub async fn with_status(status: u16) -> Self {
        Self::build(status, vec![("read", ""), ("list", ""), ("search", "")]).await
    }

    async fn build(status: u16, grants: Vec<(&str, &str)>) -> Self {
        let server = CatalogServer::start(catalog_body(), status).await;
        let mut registry = Registry::new();
        registry
            .register_factory(
                FactoryKey {
                    kind: "source-http-catalog".into(),
                    protocol: "conex".into(),
                    version: 1,
                },
                conex_provider_http_catalog::factory,
            )
            .unwrap();

        let target = Target {
            id: "catalog".into(),
            origin: format!("https://catalog.example:{}", server.addr.port()),
            fixed_path: "/catalog.json".into(),
            allowed_addresses: vec!["127.0.0.1".parse().unwrap()],
            tls_trust: TlsTrust {
                ca_pem: Some(server.ca_pem.clone()),
                expected_server_name: Some("catalog.example".into()),
                ..Default::default()
            },
            allow_loopback_http: true,
        };
        let installation = Installation {
            endpoint: Endpoint {
                id: ENDPOINT.into(),
                provider_id: "catalog".into(),
                tenant_id: TENANT.into(),
                plane: v1::Plane::Broker,
                provides: vec![
                    "source/list".into(),
                    "source/read".into(),
                    "source/search".into(),
                ],
                limits: Limits::default(),
            },
            factory: FactoryKey {
                kind: "source-http-catalog".into(),
                protocol: "conex".into(),
                version: 1,
            },
            provider: serde_json::json!({"maxResponseBytes": 1048576}),
            target: Some(target),
            credential: Some(CredentialKey {
                tenant_id: TENANT.into(),
                holder: "host".into(),
                provider_id: "catalog".into(),
                audience: "catalog.example".into(),
                name: "catalog/key".into(),
                principal_id: None,
            }),
        };
        registry.install(installation).unwrap();

        let rules: Vec<PolicyRule> = grants
            .into_iter()
            .map(|(action, root)| PolicyRule {
                principal_id: CALLER_PRINCIPAL.into(),
                tenant_id: TENANT.into(),
                endpoint_id: ENDPOINT.into(),
                actions: vec![action.into()],
                root: root.into(),
                subtree: true,
            })
            .collect();
        let policy = Arc::new(StaticPolicy::new(rules));

        let credentials = Arc::new(
            EnvFileStore::with_reader(
                vec![CredentialBinding {
                    key: CredentialKey {
                        tenant_id: TENANT.into(),
                        holder: "host".into(),
                        provider_id: "catalog".into(),
                        audience: "catalog.example".into(),
                        name: "catalog/key".into(),
                        principal_id: None,
                    },
                    backend: CredentialBackend::Env("CATALOG_KEY".into()),
                }],
                Arc::new(FixedReader {
                    secret: "test-token".into(),
                }),
            )
            .unwrap(),
        );

        let connector = Arc::new(
            HttpConnector::new(
                TlsTrustConfig {
                    ca_pem: Some(server.ca_pem.clone()),
                    ..Default::default()
                },
                1_048_576,
            )
            .unwrap(),
        );
        let host = Host::new(
            registry,
            policy.clone() as Arc<dyn Policy>,
            Arc::new(TargetPolicy::new()),
            Arc::new(StaticResolver),
            connector,
            credentials,
            Arc::new(NullAudit),
            HostLimits::default(),
        )
        .unwrap();

        Self {
            host,
            caller: Caller {
                principal_id: CALLER_PRINCIPAL.into(),
                tenant_id: TENANT.into(),
                actor_peer_id: "peer-1".into(),
            },
            server,
            policy,
        }
    }
}

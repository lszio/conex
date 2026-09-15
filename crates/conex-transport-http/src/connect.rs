//! Connector that verifies the peer before exposing a business connection.
use std::sync::Arc;

use async_trait::async_trait;
use conex_core::{
    AllowedTarget, CallError, CallResult, Connection, Connector, PeerVerification, TlsTrust,
    VerifiedPeer,
};
use conex_proto::v1;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout_at};
use tokio_rustls::TlsConnector;

use crate::AsyncReadWrite;
use crate::connection::HttpConnection;

#[derive(Clone, Default)]
pub struct TlsTrustConfig {
    pub ca_pem: Option<Vec<u8>>,
    pub expected_server_name: Option<String>,
    pub pinned_cert: Option<Vec<u8>>,
    pub use_webpki_roots: bool,
}

impl TlsTrustConfig {
    /// Build a trust config from the core target. A pin or explicit CA disables
    /// the public webpki roots.
    pub fn from_target(target: &TlsTrust) -> Self {
        Self {
            ca_pem: target.ca_pem.clone(),
            expected_server_name: target.expected_server_name.clone(),
            pinned_cert: target.pinned_cert.clone(),
            use_webpki_roots: target.ca_pem.is_none() && target.pinned_cert.is_none(),
        }
    }
}

pub struct HttpConnector {
    trust: TlsTrustConfig,
    max_response_bytes: usize,
}

impl HttpConnector {
    pub fn new(trust: TlsTrustConfig, max_response_bytes: usize) -> CallResult<HttpConnector> {
        Ok(Self {
            trust,
            max_response_bytes,
        })
    }
}

#[async_trait]
impl Connector for HttpConnector {
    async fn connect(
        &self,
        target: &AllowedTarget,
        deadline: Instant,
    ) -> CallResult<Box<dyn Connection>> {
        let tcp = timeout_at(deadline, TcpStream::connect(target.pinned_address))
            .await
            .map_err(|_| unavailable("tcp connect exceeded the deadline"))?
            .map_err(|error| unavailable(format!("tcp connect failed: {error}")))?;
        let _ = tcp.set_nodelay(true);

        let (io, verification): (Box<dyn AsyncReadWrite>, PeerVerification) = if target.scheme
            == "https"
        {
            let config = build_client_config(&self.trust)?;
            let connector = TlsConnector::from(Arc::new(config));
            let server_name = ServerName::try_from(target.server_name.clone()).map_err(|_| {
                CallError::new(v1::ErrorCode::BadRequest, "invalid TLS server name")
            })?;
            let stream = timeout_at(deadline, connector.connect(server_name, tcp))
                .await
                .map_err(|_| unavailable("tls handshake exceeded the deadline"))?
                .map_err(|error| {
                    CallError::new(
                        v1::ErrorCode::PeerUntrusted,
                        format!("tls verification failed: {error}"),
                    )
                })?;
            (Box::new(stream), PeerVerification::TlsServer)
        } else {
            (Box::new(tcp), PeerVerification::InsecureLoopback)
        };

        let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(io))
            .await
            .map_err(|error| unavailable(format!("http handshake failed: {error}")))?;
        tokio::spawn(async move {
            let _ = connection.await;
        });

        let peer = VerifiedPeer {
            target_id: target.target_id.clone(),
            audience: target.audience.clone(),
            address: target.pinned_address,
            verification,
        };
        Ok(Box::new(HttpConnection {
            peer,
            sender,
            max_response_bytes: self.max_response_bytes,
            fixed_path: target.fixed_path.clone(),
        }))
    }
}

fn unavailable(message: impl Into<String>) -> CallError {
    CallError::new(v1::ErrorCode::Unavailable, message)
}

pub fn build_client_config(trust: &TlsTrustConfig) -> CallResult<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    if let Some(pem) = &trust.ca_pem {
        add_pem_roots(&mut roots, pem)?;
    }
    if let Some(pinned) = &trust.pinned_cert {
        add_pem_roots(&mut roots, pinned)?;
    }
    if trust.use_webpki_roots {
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    if roots.is_empty() {
        return Err(CallError::new(
            v1::ErrorCode::Internal,
            "no TLS trust anchors configured",
        ));
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| CallError::new(v1::ErrorCode::Internal, format!("tls config: {error}")))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(config)
}

fn add_pem_roots(roots: &mut rustls::RootCertStore, pem: &[u8]) -> CallResult<()> {
    let mut cursor = std::io::Cursor::new(pem);
    let mut added = 0usize;
    for cert in rustls_pemfile::certs(&mut cursor) {
        let cert = cert.map_err(|error| {
            CallError::new(v1::ErrorCode::Internal, format!("invalid CA pem: {error}"))
        })?;
        roots.add(cert).map_err(|error| {
            CallError::new(v1::ErrorCode::Internal, format!("invalid CA cert: {error}"))
        })?;
        added += 1;
    }
    if added == 0 {
        return Err(CallError::new(
            v1::ErrorCode::Internal,
            "no certificates found in trust material",
        ));
    }
    Ok(())
}

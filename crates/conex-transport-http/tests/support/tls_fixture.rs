//! Local TLS server with a CA-signed certificate for the wrong DNS name.
#![allow(dead_code)]
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use conex_core::AllowedTarget;
use conex_transport_http::TlsTrustConfig;
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::allowed_target;

pub struct TlsFixture {
    pub addr: SocketAddr,
    pub ca_pem: Vec<u8>,
    pub requests: Arc<AtomicUsize>,
}

impl TlsFixture {
    pub async fn wrong_hostname() -> Self {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "conex test CA");
        let ca_issuer = CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();

        let leaf_key = KeyPair::generate().unwrap();
        let mut leaf_params = CertificateParams::new(vec!["wrong.example".to_string()]).unwrap();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "wrong.example");
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
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    break;
                };
                let acceptor = acceptor.clone();
                let counter = counter.clone();
                tokio::spawn(async move {
                    let Ok(mut stream) = acceptor.accept(socket).await else {
                        return;
                    };
                    let mut buffer = [0u8; 2048];
                    let _ = stream.read(&mut buffer).await;
                    counter.fetch_add(1, Ordering::SeqCst);
                    let body = b"ok";
                    let header = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(body).await;
                    let _ = stream.flush().await;
                });
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        Self {
            addr,
            ca_pem: ca_issuer.pem().into_bytes(),
            requests,
        }
    }

    pub fn client_trust(&self) -> TlsTrustConfig {
        TlsTrustConfig {
            ca_pem: Some(self.ca_pem.clone()),
            ..Default::default()
        }
    }

    pub fn allowed_target(&self) -> AllowedTarget {
        allowed_target(self.addr, "https", "catalog.example", "/")
    }

    pub fn http_requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

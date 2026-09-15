//! Config validation and a real TLS listener (production requires cert/key).
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use conex_core::{AllowedTarget, Connector, OutboundRequest};
use conex_host::config::HostConfig;
use conex_host::serve;
use conex_transport_http::{HttpConnector, TlsTrustConfig};
use sha2::{Digest, Sha256};
use tokio::time::Instant;

fn base_config(listen: &str, allow_loopback: bool) -> String {
    format!(
        "listen = \"{listen}\"\nallow_loopback_http = {allow_loopback}\naudience = \"host.example\"\n"
    )
}

fn write_config(dir: &Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).unwrap();
    path
}

#[test]
fn plaintext_needs_loopback_and_flag() {
    let dir = tempfile::tempdir().unwrap();
    // No TLS and no flag.
    let path = write_config(dir.path(), "a.toml", &base_config("127.0.0.1:0", false));
    assert!(HostConfig::load(&path).is_err());
    // Flag set but non-loopback bind.
    let path = write_config(dir.path(), "b.toml", &base_config("0.0.0.0:8787", true));
    assert!(HostConfig::load(&path).is_err());
}

#[test]
fn rejects_duplicate_endpoints_unknown_kind_and_unknown_fields() {
    let dir = tempfile::tempdir().unwrap();
    let endpoints = "\
[[endpoints]]
id = \"a\"
tenant_id = \"t\"
provider_id = \"source\"
kind = \"source-fs\"
provides = [\"source/read\"]
root = \"fixtures/p0/notes\"
\
[[endpoints]]
id = \"a\"
tenant_id = \"t\"
provider_id = \"source\"
kind = \"source-fs\"
provides = [\"source/read\"]
root = \"fixtures/p0/notes\"
";
    let path = write_config(
        dir.path(),
        "dup.toml",
        &format!("{}{}", base_config("127.0.0.1:0", true), endpoints),
    );
    assert!(HostConfig::load(&path).is_err());

    let unknown_kind = "\
[[endpoints]]
id = \"a\"
tenant_id = \"t\"
provider_id = \"source\"
kind = \"source-magic\"
provides = [\"source/read\"]
";
    let path = write_config(
        dir.path(),
        "kind.toml",
        &format!("{}{}", base_config("127.0.0.1:0", true), unknown_kind),
    );
    assert!(HostConfig::load(&path).is_err());

    let unknown_field = format!("{}exec = \"rm -rf /\"\n", base_config("127.0.0.1:0", true));
    let path = write_config(dir.path(), "exec.toml", &unknown_field);
    assert!(HostConfig::load(&path).is_err());
}

#[test]
fn tls_config_requires_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let missing = "\
[tls]
cert = \"/nonexistent/cert.pem\"
key = \"/nonexistent/key.pem\"
";
    let path = write_config(
        dir.path(),
        "tls.toml",
        &format!("{}{}", base_config("127.0.0.1:0", false), missing),
    );
    assert!(HostConfig::load(&path).is_err());
}

fn test_pki() -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair};
    let ca_key = KeyPair::generate().unwrap();
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "conex host test CA");
    let ca_issuer = CertifiedIssuer::self_signed(ca_params, ca_key).unwrap();

    let leaf_key = KeyPair::generate().unwrap();
    let mut leaf_params = CertificateParams::new(vec!["host.example".to_string()]).unwrap();
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "host.example");
    let leaf_cert = leaf_params.signed_by(&leaf_key, &ca_issuer).unwrap();

    let chain = format!("{}{}", leaf_cert.pem(), ca_issuer.pem());
    (
        chain.into_bytes(),
        leaf_key.serialize_pem().into_bytes(),
        ca_issuer.pem().into_bytes(),
    )
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

async fn rpc_call(
    addr: SocketAddr,
    ca_pem: Vec<u8>,
    authorization: Option<&str>,
    body: &str,
) -> (u16, String) {
    let connector = HttpConnector::new(
        TlsTrustConfig {
            ca_pem: Some(ca_pem),
            expected_server_name: Some("host.example".into()),
            ..Default::default()
        },
        1_048_576,
    )
    .unwrap();
    let target = AllowedTarget {
        target_id: "host".into(),
        audience: "host.example".into(),
        scheme: "https".into(),
        hostname: "host.example".into(),
        server_name: "host.example".into(),
        port: addr.port(),
        pinned_address: addr,
        fixed_path: "/".into(),
        allow_loopback_http: true,
    };
    let mut connection = connector
        .connect(&target, Instant::now() + Duration::from_secs(5))
        .await
        .unwrap();
    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );
    if let Some(value) = authorization {
        headers.insert(http::header::AUTHORIZATION, value.parse().unwrap());
    }
    let response = connection
        .request(
            OutboundRequest {
                method: http::Method::POST,
                path: "/rpc".into(),
                headers,
                body: Bytes::from(body.to_string()),
            },
            Instant::now() + Duration::from_secs(5),
        )
        .await
        .unwrap();
    (
        response.status,
        String::from_utf8_lossy(&response.body).to_string(),
    )
}

#[tokio::test]
async fn tls_listener_serves_authenticated_rpc() {
    let dir = tempfile::tempdir().unwrap();
    let (chain, key, ca_pem) = test_pki();
    let cert_path = dir.path().join("host.crt");
    let key_path = dir.path().join("host.key");
    std::fs::write(&cert_path, &chain).unwrap();
    std::fs::write(&key_path, &key).unwrap();

    let token_hash = hex::encode(Sha256::digest(b"test-token"));
    let port = free_port();
    let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/p0/notes");
    let config_body = format!(
        "listen = \"127.0.0.1:{port}\"\nallow_loopback_http = false\naudience = \"host.example\"\n\
[tls]\ncert = \"{}\"\nkey = \"{}\"\n\
[[tokens]]\ntoken_hash = \"{token_hash}\"\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\naudience = \"host.example\"\n\
[[policy]]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"notes-local\"\nactions = [\"read\"]\nroot = \"\"\nsubtree = true\n\
[[endpoints]]\nid = \"notes-local\"\ntenant_id = \"tenant-a\"\nprovider_id = \"source\"\nkind = \"source-fs\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"{}\"\n",
        cert_path.display(),
        key_path.display(),
        fixtures.display()
    );
    let config = HostConfig::load(&write_config(dir.path(), "host.toml", &config_body)).unwrap();
    serve::build(&config).expect("host must build");
    let server = tokio::spawn(serve::serve(config));
    tokio::time::sleep(Duration::from_millis(300)).await;

    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let hello = "{\"jsonrpc\":\"2.0\",\"id\":\"01ARZ3NDEKTSV4RRFFQ69G5FAV\",\"method\":\"conex/hello\",\"params\":{\"context\":{\"providerEndpointId\":\"\",\"plane\":\"broker\"},\"timeoutBudgetMs\":8000,\"input\":{\"profileId\":\"conex-jsonrpc2-http-v1\",\"plane\":\"broker\",\"provides\":[],\"requires\":[\"source/read\"]}}}";
    let (status, _) = rpc_call(addr, ca_pem.clone(), None, hello).await;
    assert_eq!(status, 401);
    let (status, body) = rpc_call(addr, ca_pem, Some("Bearer test-token"), hello).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("bindingId"));

    server.abort();
}

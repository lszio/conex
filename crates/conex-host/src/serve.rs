//! Config-driven assembly and standalone serving (TLS or explicit loopback).
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum_server::Handle;
use conex_core::{
    AuditSink, CallError, CredentialKey, Endpoint, FactoryKey, Host, HostLimits, Installation,
    Limits, Policy, PolicyRule, Registry, RegistryError, StaticPolicy, Target, TargetPolicy,
    TlsTrust,
};
use conex_proto::v1;
use conex_transport_http::{HttpConnector, TlsTrustConfig, TokioResolver};
use serde_json::{Value, json};

use crate::audit_file::{FileAuditSink, NullAudit};
use crate::auth::{InboundAuth, StaticBearerAuth, TokenRecord};
use crate::binding::BindingStore;
use crate::config::{EndpointConfig, HostConfig};
use crate::credentials::{CredentialBackend, CredentialBinding, EnvFileStore};
use crate::http::{HttpState, PROFILE_ID, router};

pub struct BuiltHost {
    pub host: Arc<Host>,
    pub bindings: Arc<BindingStore>,
    pub auth: Arc<dyn InboundAuth>,
}

pub fn build(config: &HostConfig) -> Result<BuiltHost, CallError> {
    config.validate()?;
    let mut registry = Registry::new();
    conex_assembly::register_all(&mut registry).map_err(registry_error)?;

    let mut credential_bindings: Vec<CredentialBinding> = Vec::new();
    let mut capabilities: HashMap<String, Vec<String>> = HashMap::new();
    for endpoint in &config.endpoints {
        let (installation, credential_binding) = build_installation(endpoint)?;
        capabilities
            .entry(endpoint.tenant_id.clone())
            .or_default()
            .extend(endpoint.provides.clone());
        if let Some(binding) = credential_binding {
            credential_bindings.push(binding);
        }
        registry.install(installation).map_err(registry_error)?;
    }
    for values in capabilities.values_mut() {
        values.sort();
        values.dedup();
    }

    let rules: Vec<PolicyRule> = config
        .policy
        .iter()
        .map(|rule| PolicyRule {
            principal_id: rule.principal_id.clone(),
            tenant_id: rule.tenant_id.clone(),
            endpoint_id: rule.endpoint_id.clone(),
            actions: rule.actions.clone(),
            root: rule.root.clone(),
            subtree: rule.subtree,
        })
        .collect();
    let policy = Arc::new(StaticPolicy::new(rules));

    let mut ca_pem: Vec<u8> = Vec::new();
    for endpoint in &config.endpoints {
        if let Some(path) = &endpoint.ca_pem
            && let Ok(bytes) = std::fs::read(path)
        {
            ca_pem.extend_from_slice(&bytes);
        }
    }
    let trust = TlsTrustConfig {
        ca_pem: if ca_pem.is_empty() {
            None
        } else {
            Some(ca_pem)
        },
        expected_server_name: None,
        pinned_cert: None,
        use_webpki_roots: false,
    };
    let connector = Arc::new(HttpConnector::new(trust, 1024 * 1024)?);
    let credentials = Arc::new(EnvFileStore::new(credential_bindings)?);
    let audit: Arc<dyn AuditSink> = match &config.audit_file {
        Some(path) => Arc::new(FileAuditSink::open(
            path,
            config.audit_capacity.unwrap_or(1024),
        )?),
        None => Arc::new(NullAudit),
    };

    let host = Arc::new(Host::new(
        registry,
        policy as Arc<dyn Policy>,
        Arc::new(TargetPolicy::new()),
        Arc::new(TokioResolver),
        connector,
        credentials,
        audit,
        HostLimits::default(),
    )?);
    let bindings = Arc::new(BindingStore::new(
        PROFILE_ID,
        config.audience(),
        capabilities,
        Limits::default(),
    ));
    let tokens = config
        .tokens
        .iter()
        .map(token_record)
        .collect::<Result<Vec<_>, _>>()?;
    let auth: Arc<dyn InboundAuth> = Arc::new(StaticBearerAuth::new(tokens));
    Ok(BuiltHost {
        host,
        bindings,
        auth,
    })
}

pub fn build_router(config: &HostConfig) -> Result<axum::Router, CallError> {
    let built = build(config)?;
    Ok(router(Arc::new(HttpState {
        host: built.host,
        bindings: built.bindings,
        auth: built.auth,
    })))
}

pub async fn serve(config: HostConfig) -> Result<(), CallError> {
    config.validate()?;
    // axum-server may enable aws-lc-rs; pin the process provider to ring explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let app_router = build_router(&config)?;
    let address: SocketAddr = config.listen.parse().map_err(invalid)?;

    let handle = Handle::new();
    let shutdown_handle = handle.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        shutdown_handle.graceful_shutdown(Some(Duration::from_secs(10)));
    });

    let result = if let Some(tls) = &config.tls {
        let rustls = axum_server::tls_rustls::RustlsConfig::from_pem_file(&tls.cert, &tls.key)
            .await
            .map_err(invalid)?;
        axum_server::bind_rustls(address, rustls)
            .handle(handle)
            .serve(app_router.into_make_service())
            .await
    } else {
        // validate() already enforced allow_loopback_http + loopback.
        axum_server::bind(address)
            .handle(handle)
            .serve(app_router.into_make_service())
            .await
    };
    result.map_err(invalid)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}

fn build_installation(
    endpoint: &EndpointConfig,
) -> Result<(Installation, Option<CredentialBinding>), CallError> {
    let factory = FactoryKey {
        kind: endpoint.kind.clone(),
        protocol: "conex".into(),
        version: 1,
    };
    let (provider, target, credential_key, credential_backend) = match endpoint.kind.as_str() {
        "source-fs" => {
            let root = endpoint
                .root
                .clone()
                .ok_or_else(|| invalid("source-fs requires root"))?;
            (json!({"root": root}), None, None, None)
        }
        "source-http-catalog" => {
            let origin = endpoint
                .origin
                .clone()
                .ok_or_else(|| invalid("source-http-catalog requires origin"))?;
            let fixed_path = endpoint
                .fixed_path
                .clone()
                .unwrap_or_else(|| "/catalog.json".into());
            let audience = endpoint
                .expected_server_name
                .clone()
                .unwrap_or_else(|| host_of(&origin));
            let ca_pem = match &endpoint.ca_pem {
                Some(path) => Some(std::fs::read(path).map_err(invalid)?),
                None => None,
            };
            let target = Target {
                id: endpoint.id.clone(),
                origin,
                fixed_path,
                allowed_addresses: vec![],
                tls_trust: TlsTrust {
                    ca_pem,
                    expected_server_name: Some(audience.clone()),
                    pinned_cert: None,
                },
                allow_loopback_http: endpoint.allow_loopback_http,
            };
            let (key, backend) = match (&endpoint.credential_name, &endpoint.credential_backend) {
                (Some(name), Some(backend)) => {
                    let key = CredentialKey {
                        tenant_id: endpoint.tenant_id.clone(),
                        holder: "host".into(),
                        provider_id: endpoint.provider_id.clone(),
                        audience,
                        name: name.clone(),
                        principal_id: None,
                    };
                    let backend = match backend.strip_prefix("env:") {
                        Some(variable) => CredentialBackend::Env(variable.to_string()),
                        None => CredentialBackend::File(PathBuf::from(
                            backend.strip_prefix("file:").unwrap_or(backend),
                        )),
                    };
                    (Some(key), Some(backend))
                }
                (None, None) => (None, None),
                _ => {
                    return Err(invalid(
                        "credential_name and credential_backend must be provided together",
                    ));
                }
            };
            let mut provider = serde_json::Map::new();
            if let Some(max) = endpoint.max_response_bytes {
                provider.insert("maxResponseBytes".into(), json!(max));
            }
            (Value::Object(provider), Some(target), key, backend)
        }
        other => return Err(invalid(format!("unknown factory kind {other}"))),
    };

    let credential_binding = credential_key
        .clone()
        .zip(credential_backend)
        .map(|(key, backend)| CredentialBinding { key, backend });

    Ok((
        Installation {
            endpoint: Endpoint {
                id: endpoint.id.clone(),
                provider_id: endpoint.provider_id.clone(),
                tenant_id: endpoint.tenant_id.clone(),
                plane: v1::Plane::Broker,
                provides: endpoint.provides.clone(),
                limits: Limits::default(),
            },
            factory,
            provider,
            target,
            credential: credential_key,
        },
        credential_binding,
    ))
}

fn token_record(token: &crate::config::TokenConfig) -> Result<TokenRecord, CallError> {
    let bytes = hex::decode(token.token_hash.trim()).map_err(invalid)?;
    if bytes.len() != 32 {
        return Err(invalid("token_hash must decode to 32 bytes"));
    }
    let mut token_hash = [0u8; 32];
    token_hash.copy_from_slice(&bytes);
    Ok(TokenRecord {
        token_hash,
        principal_id: token.principal_id.clone(),
        tenant_id: token.tenant_id.clone(),
        actor_peer_id: "inbound-http".into(),
        audience: token.audience.clone(),
    })
}

fn host_of(origin: &str) -> String {
    origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin)
        .split([':', '/'])
        .next()
        .unwrap_or("")
        .to_string()
}

fn registry_error(error: RegistryError) -> CallError {
    CallError::new(v1::ErrorCode::Internal, error.to_string())
}

fn invalid(message: impl std::fmt::Display) -> CallError {
    CallError::new(v1::ErrorCode::Internal, message.to_string())
}

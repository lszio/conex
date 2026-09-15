//! Registry: duplicate keys, honest capability advertisement, tenant binding.
use std::sync::Arc;

use async_trait::async_trait;
use conex_core::{
    CallContext, CallError, CallResult, Endpoint, ExecutionIo, FactoryKey, Handler, Installation,
    Limits, MethodContract, PreparedInput, Registry, RegistryError, ResourceClaim, Route,
};
use conex_proto::v1;
use serde_json::Value;

const METHODS: [&str; 3] = ["source/list", "source/read", "source/search"];

struct EchoHandler;

#[async_trait]
impl Handler for EchoHandler {
    async fn execute(
        &self,
        _ctx: &CallContext,
        input: Value,
        _io: ExecutionIo,
    ) -> CallResult<Value> {
        Ok(input)
    }
}

fn prepare_ok(value: &Value) -> CallResult<PreparedInput> {
    Ok(PreparedInput {
        canonical: value.clone(),
        claim: ResourceClaim {
            resource_id: String::new(),
            action: "read".into(),
            subtree: false,
        },
    })
}

fn validate_ok(_: &Value) -> CallResult<()> {
    Ok(())
}

fn contract() -> MethodContract {
    MethodContract {
        input_schema: "conex.v1.SourceReadRequest",
        output_schema: "conex.v1.SourceReadResponse",
        prepare: prepare_ok,
        validate_output: validate_ok,
    }
}

fn endpoint(id: &str, tenant: &str, provides: &[&str]) -> Endpoint {
    Endpoint {
        id: id.into(),
        provider_id: id.into(),
        tenant_id: tenant.into(),
        plane: v1::Plane::Broker,
        provides: provides.iter().map(|s| s.to_string()).collect(),
        limits: Limits::default(),
    }
}

fn route(endpoint: &Endpoint, method: &str) -> Route {
    Route {
        protocol: "conex".into(),
        version: 1,
        endpoint: endpoint.clone(),
        method: method.into(),
        contract: contract(),
        handler: Arc::new(EchoHandler),
        target: None,
        credential: None,
    }
}

fn installation(id: &str, tenant: &str, provides: &[&str]) -> Installation {
    Installation {
        endpoint: endpoint(id, tenant, provides),
        factory: fk(),
        provider: Value::Null,
        target: None,
        credential: None,
    }
}

fn fk() -> FactoryKey {
    FactoryKey {
        kind: "fixture".into(),
        protocol: "conex".into(),
        version: 1,
    }
}

fn refusing_factory(_: &Installation) -> CallResult<Vec<Route>> {
    Err(CallError::new(
        v1::ErrorCode::UnsupportedCapability,
        "test only",
    ))
}

fn make_routes(installation: &Installation, methods: &[&str]) -> CallResult<Vec<Route>> {
    Ok(methods
        .iter()
        .map(|m| route(&installation.endpoint, m))
        .collect())
}

fn factory_full(i: &Installation) -> CallResult<Vec<Route>> {
    make_routes(i, &METHODS)
}

fn factory_missing(i: &Installation) -> CallResult<Vec<Route>> {
    make_routes(i, &["source/list", "source/read"])
}

fn factory_extra(i: &Installation) -> CallResult<Vec<Route>> {
    make_routes(
        i,
        &[
            "source/list",
            "source/read",
            "source/search",
            "source/write",
        ],
    )
}

fn factory_tenant_mismatch(i: &Installation) -> CallResult<Vec<Route>> {
    let mut routes = make_routes(i, &["source/read"])?;
    routes[0].endpoint.tenant_id = "other-tenant".into();
    Ok(routes)
}

#[test]
fn duplicate_factory_is_rejected() {
    let mut registry = Registry::new();
    let key = fk();
    registry
        .register_factory(key.clone(), refusing_factory)
        .unwrap();
    assert!(matches!(
        registry.register_factory(key, refusing_factory),
        Err(RegistryError::DuplicateKey(_))
    ));
}

#[test]
fn test_factory_refuses_instead_of_faking_a_provider() {
    assert!(refusing_factory(&installation("a", "t", &["source/read"])).is_err());
}

#[test]
fn advertised_without_handler_is_rejected() {
    let mut registry = Registry::new();
    registry.register_factory(fk(), factory_missing).unwrap();
    let err = registry
        .install(installation("a", "t", &METHODS))
        .unwrap_err();
    assert!(
        matches!(err, RegistryError::InvalidInstallation(_)),
        "{err:?}"
    );
}

#[test]
fn extra_unadvertised_route_is_rejected() {
    let mut registry = Registry::new();
    registry.register_factory(fk(), factory_extra).unwrap();
    let err = registry
        .install(installation("a", "t", &METHODS))
        .unwrap_err();
    assert!(
        matches!(err, RegistryError::InvalidInstallation(_)),
        "{err:?}"
    );
}

#[test]
fn same_method_on_different_endpoints_does_not_conflict() {
    let mut registry = Registry::new();
    registry.register_factory(fk(), factory_full).unwrap();
    registry.install(installation("a", "t", &METHODS)).unwrap();
    registry.install(installation("b", "t", &METHODS)).unwrap();
    assert!(registry.route("a", "conex", 1, "source/read").is_some());
    assert!(registry.route("b", "conex", 1, "source/read").is_some());
    assert_eq!(registry.method_count(), 6);
    assert_eq!(registry.routes_for_endpoint("a").len(), 3);
}

#[test]
fn tenant_mismatch_is_rejected() {
    let mut registry = Registry::new();
    registry
        .register_factory(fk(), factory_tenant_mismatch)
        .unwrap();
    let err = registry
        .install(installation("a", "t", &["source/read"]))
        .unwrap_err();
    assert!(
        matches!(err, RegistryError::InvalidInstallation(_)),
        "{err:?}"
    );
}

#[test]
fn unknown_factory_is_rejected() {
    let mut registry = Registry::new();
    let mut inst = installation("a", "t", &["source/read"]);
    inst.factory.kind = "missing".into();
    assert!(matches!(
        registry.install(inst),
        Err(RegistryError::UnknownFactory(_))
    ));
}

#[test]
fn relay_plane_is_rejected_in_p0() {
    let mut registry = Registry::new();
    registry.register_factory(fk(), factory_full).unwrap();
    let mut inst = installation("a", "t", &METHODS);
    inst.endpoint.plane = v1::Plane::Relay;
    assert!(matches!(
        registry.install(inst),
        Err(RegistryError::InvalidInstallation(_))
    ));
}

#[test]
fn unknown_protocol_version_is_rejected() {
    let mut registry = Registry::new();
    let key = FactoryKey {
        kind: "fixture".into(),
        protocol: "conex".into(),
        version: 2,
    };
    registry.register_factory(key, factory_full).unwrap();
    let mut inst = installation("a", "t", &METHODS);
    inst.factory.version = 2;
    assert!(matches!(
        registry.install(inst),
        Err(RegistryError::InvalidInstallation(_))
    ));
}

#[test]
fn unknown_route_is_none() {
    let mut registry = Registry::new();
    registry.register_factory(fk(), factory_full).unwrap();
    registry.install(installation("a", "t", &METHODS)).unwrap();
    assert!(registry.route("a", "conex", 1, "source/write").is_none());
    assert!(registry.route("a", "conex", 2, "source/read").is_none());
    assert!(registry.route("nope", "conex", 1, "source/read").is_none());
}

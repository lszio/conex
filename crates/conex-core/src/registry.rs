//! Factory and route tables. Data-driven: dispatch never branches on provider.
use std::collections::{HashMap, HashSet};

use conex_proto::v1;

use crate::types::{Endpoint, FactoryFn, FactoryKey, Installation, RegistryError, Route};

/// P0 is fixed to one protocol/version; new protocols are separate registrations.
pub const P0_PROTOCOL: &str = "conex";
pub const P0_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RouteKey {
    pub endpoint_id: String,
    pub protocol: String,
    pub version: u32,
    pub method: String,
}

#[derive(Default)]
pub struct Registry {
    factories: HashMap<FactoryKey, FactoryFn>,
    routes: HashMap<RouteKey, Route>,
    endpoints: HashMap<String, Endpoint>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_factory(
        &mut self,
        key: FactoryKey,
        factory: FactoryFn,
    ) -> Result<(), RegistryError> {
        if self.factories.contains_key(&key) {
            return Err(RegistryError::DuplicateKey(format!("{key:?}")));
        }
        self.factories.insert(key, factory);
        Ok(())
    }

    pub fn install(&mut self, installation: Installation) -> Result<(), RegistryError> {
        let endpoint = installation.endpoint.clone();

        if endpoint.plane != v1::Plane::Broker {
            return Err(RegistryError::InvalidInstallation(format!(
                "P0 only supports the broker plane, got {:?}",
                endpoint.plane
            )));
        }
        if installation.factory.protocol != P0_PROTOCOL
            || installation.factory.version != P0_VERSION
        {
            return Err(RegistryError::InvalidInstallation(format!(
                "P0 only supports {P0_PROTOCOL}/v{P0_VERSION}, got {}/v{}",
                installation.factory.protocol, installation.factory.version
            )));
        }
        if let Some(existing) = self.endpoints.get(&endpoint.id)
            && existing != &endpoint
        {
            return Err(RegistryError::DuplicateKey(format!(
                "endpoint {}",
                endpoint.id
            )));
        }

        let factory = self
            .factories
            .get(&installation.factory)
            .ok_or_else(|| RegistryError::UnknownFactory(format!("{:?}", installation.factory)))?;
        let routes = factory(&installation).map_err(|e| {
            RegistryError::InvalidInstallation(format!("factory error: {}", e.message()))
        })?;

        let advertised: HashSet<&str> = endpoint.provides.iter().map(String::as_str).collect();
        let mut returned: HashSet<&str> = HashSet::new();
        let mut validated: Vec<RouteKey> = Vec::new();
        for route in &routes {
            if route.protocol != installation.factory.protocol
                || route.version != installation.factory.version
            {
                return Err(RegistryError::InvalidInstallation(format!(
                    "route {} declares {}/v{}, expected {}/v{}",
                    route.method,
                    route.protocol,
                    route.version,
                    installation.factory.protocol,
                    installation.factory.version
                )));
            }
            if route.endpoint.id != endpoint.id || route.endpoint.tenant_id != endpoint.tenant_id {
                return Err(RegistryError::InvalidInstallation(format!(
                    "route {} is not bound to endpoint {} tenant {}",
                    route.method, endpoint.id, endpoint.tenant_id
                )));
            }
            let method = route.method.as_str();
            if !returned.insert(method) {
                return Err(RegistryError::DuplicateKey(format!(
                    "route {} on {}",
                    method, endpoint.id
                )));
            }
            if !advertised.contains(method) {
                return Err(RegistryError::InvalidInstallation(format!(
                    "route {} is not advertised by endpoint {}",
                    method, endpoint.id
                )));
            }
            validated.push(RouteKey {
                endpoint_id: endpoint.id.clone(),
                protocol: route.protocol.clone(),
                version: route.version,
                method: route.method.clone(),
            });
        }
        for provided in &endpoint.provides {
            if !returned.contains(provided.as_str()) {
                return Err(RegistryError::InvalidInstallation(format!(
                    "advertised method {} has no handler on endpoint {}",
                    provided, endpoint.id
                )));
            }
        }
        for key in &validated {
            if self.routes.contains_key(key) {
                return Err(RegistryError::DuplicateKey(format!("{key:?}")));
            }
        }

        self.endpoints.insert(endpoint.id.clone(), endpoint.clone());
        for (key, mut route) in validated.into_iter().zip(routes) {
            // Normalize the endpoint to the validated installation value.
            route.endpoint = endpoint.clone();
            self.routes.insert(key, route);
        }
        Ok(())
    }

    pub fn route(
        &self,
        endpoint_id: &str,
        protocol: &str,
        version: u32,
        method: &str,
    ) -> Option<&Route> {
        self.routes.get(&RouteKey {
            endpoint_id: endpoint_id.to_string(),
            protocol: protocol.to_string(),
            version,
            method: method.to_string(),
        })
    }

    pub fn endpoint(&self, endpoint_id: &str) -> Option<&Endpoint> {
        self.endpoints.get(endpoint_id)
    }

    pub fn routes_for_endpoint(&self, endpoint_id: &str) -> Vec<&Route> {
        let mut routes: Vec<&Route> = self
            .routes
            .values()
            .filter(|route| route.endpoint.id == endpoint_id)
            .collect();
        routes.sort_by(|a, b| a.method.cmp(&b.method));
        routes
    }

    pub fn method_count(&self) -> usize {
        self.routes.len()
    }
}

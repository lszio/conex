//! Evaluate approved application definitions once, before spawning any worker or
//! contacting the hub. Runtime factories receive node-owned creation identity.
use crate::application::{
    ApprovedFactory, ApprovedRemoteEndpoint, ImplementationEntry, ImplementationRegistry,
    SourceStrategy, resolve_application,
};
use crate::node_config::{ApprovedCounterConfig, NodeConfig};
use crate::node_types::{PreparedApplication, PreparedLocalModule};
use conex_sdk::{Contract, Execution, Method, ModuleRef, RemoteBinding};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::sync::Arc;

/// Fixed-size, unambiguous persistent identity for the (app, module) pair.
pub fn module_key(app: &str, module: &str) -> String {
    let mut digest = Sha256::new();
    digest.update((app.len() as u64).to_be_bytes());
    digest.update(app.as_bytes());
    digest.update(module.as_bytes());
    hex::encode(digest.finalize())
}

pub fn counter_metadata() -> Result<Arc<Contract>, String> {
    Contract::new(
        "conex.counter",
        BTreeMap::from([
            (
                "counter/read".into(),
                Method::new(
                    "conex.CounterReadInput",
                    "conex.CounterResult",
                    Execution::ReadOnly,
                ),
            ),
            (
                "counter/add".into(),
                Method::new(
                    "conex.CounterAddInput",
                    "conex.CounterResult",
                    Execution::NonReplayable,
                ),
            ),
        ]),
        BTreeMap::new(),
    )
    .map(Arc::new)
    .map_err(|error| error.to_string())
}

pub fn load_applications(config: &NodeConfig) -> Result<Vec<PreparedApplication>, String> {
    let metadata = counter_metadata()?;
    let contracts = BTreeMap::from([(
        metadata.id.clone(),
        metadata.methods.keys().cloned().collect::<BTreeSet<_>>(),
    )]);
    let configs: BTreeMap<_, _> = config.configs.iter().map(|(key, value)| {
        (key.clone(), serde_json::json!({"external_socket":value.external_socket,"external_token_backend":value.external_token_backend}))
    }).collect();
    let mut entries = Vec::new();
    for (key, external) in [("controlled-counter", false), ("external-counter", true)] {
        let factory: ApprovedFactory = Arc::new(move |value| {
            let approved = if value.is_null() {
                ApprovedCounterConfig {
                    external_socket: None,
                    external_token_backend: None,
                }
            } else {
                serde_json::from_value::<ApprovedCounterConfig>(value.clone())
                    .map_err(|_| "invalid approved counter configuration")?
            };
            if external != approved.external_socket.is_some()
                || external != approved.external_token_backend.is_some()
            {
                return Err("implementation/config_ref mismatch".into());
            }
            Ok(crate::builtins::factory(approved, external))
        });
        entries.push(ImplementationEntry::new(
            key,
            factory,
            contracts.clone(),
            configs.clone(),
            config.permissions.clone(),
            SourceStrategy::RelayAllowed,
        )?);
    }
    let approved_remotes = config
        .remote_endpoints
        .iter()
        .map(|endpoint| {
            ApprovedRemoteEndpoint::new(
                &endpoint.node_ref,
                &endpoint.provider_endpoint_ref,
                &endpoint.contract_id,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let registry = ImplementationRegistry::new(entries, approved_remotes)?;
    let mut resolved = Vec::new();
    let mut ids = BTreeSet::new();
    for path in &config.applications {
        let file = std::fs::File::open(path)
            .map_err(|error| format!("application read {}: {error}", path.display()))?;
        let mut bytes = Vec::new();
        file.take((crate::application::MAX_PACKAGE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        let application =
            resolve_application(&bytes, &registry, &config.node_ref, &config.permissions)
                .map_err(|error| format!("application {}: {error}", path.display()))?;
        if !ids.insert(application.app_id.clone()) {
            return Err(format!("duplicate installed app_id {}", application.app_id));
        }
        resolved.push(application);
    }
    // Configuration parsing and all factory preparation succeed for every app
    // before Node receives any executable start closure.
    let mut applications = Vec::new();
    let mut endpoints = BTreeSet::new();
    for application in resolved {
        let mut ordered_modules = Vec::new();
        for module in &application.ordered_modules {
            let factory = registry.instantiate(module)?;
            if module
                .exports
                .iter()
                .any(|export| export.strategy == SourceStrategy::RelayAllowed)
                && !endpoints.insert(format!(
                    "{}-{}-{}",
                    config.publication.endpoint_prefix, application.app_id, module.id
                ))
            {
                return Err("duplicate application publication endpoint".into());
            }
            ordered_modules.push(PreparedLocalModule {
                name: module.id.clone(),
                module_key: module_key(&application.app_id, &module.id),
                dependencies: module
                    .local_dependencies
                    .iter()
                    .map(|dependency| (dependency.alias.clone(), dependency.target_id.clone()))
                    .collect(),
                permissions: module.permissions.iter().cloned().collect(),
                relay_allowed: module
                    .exports
                    .iter()
                    .any(|export| export.strategy == SourceStrategy::RelayAllowed),
                exposed_methods: module
                    .exports
                    .iter()
                    .flat_map(|export| export.members.iter())
                    .filter(|method| module.permissions.contains(&method.replace('/', ".")))
                    .cloned()
                    .collect(),
                factory,
            });
        }
        let mut bindings = Vec::new();
        for binding in application.remote_bindings {
            let approved = config
                .remote_endpoints
                .iter()
                .find(|endpoint| {
                    endpoint.node_ref == binding.node_ref
                        && endpoint.provider_endpoint_ref == binding.provider_endpoint_ref
                        && endpoint.contract_id == binding.contract_id
                })
                .ok_or("remote binding not approved")?;
            if approved.contract_id != metadata.id {
                return Err("remote contract not installed".into());
            }
            bindings.push(RemoteBinding {
                dependency: binding.name,
                target: ModuleRef {
                    space_id: approved.space_id.clone(),
                    provider_endpoint_id: approved.provider_endpoint_id.clone(),
                    registration_id: approved.registration_id.clone(),
                    contract_id: approved.contract_id.clone(),
                },
                contract: metadata.clone(),
            });
        }
        applications.push(PreparedApplication {
            app_id: application.app_id,
            ordered_modules,
            bindings,
        });
    }
    Ok(applications)
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn ambiguous_labels_do_not_share_generations_or_ownership_constraints() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            conex_agent_domain::Store::open(dir.path().join("node.db"), "node", Default::default())
                .await
                .unwrap();
        let left = super::module_key("a/b", "c");
        let right = super::module_key("a", "b/c");
        assert_eq!(store.next_module_generation(&left).await.unwrap(), 1);
        assert_eq!(store.next_module_generation(&right).await.unwrap(), 1);
        assert_eq!(store.next_module_generation(&left).await.unwrap(), 2);
    }
}

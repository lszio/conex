//! P2.1 application packages and the static implementation environment
//! (docs/plans/2026-10-29-conex-mvp-p2.md §2.1, docs/contracts/private-runtime.md §2).
//!
//! An application package is installation *definition data* from a trusted
//! operator: it can only reference implementations, config refs, permissions
//! and remote endpoints that the node's static [`ImplementationRegistry`]
//! approves. Packages can never carry code, executable commands, file/env/URL
//! references or a self-claimed principal.
//!
//! [`resolve_application`] validates the whole package up front — schema,
//! byte size, identifiers, counts, owner name, duplicate ids/bindings,
//! implementation & config-ref approval, permission ceilings, the local
//! dependency graph (stable Kahn order with the module id/name as tie-breaker,
//! concrete cycle path), source-data strategy intersection and remote-binding
//! approval — and returns one immutable [`ResolvedApplication`]. No process or
//! factory instance is created here: [`ImplementationRegistry::instantiate`] is
//! the only place a config-approved runtime factory is produced. The Node
//! passes persistent creation identity explicitly when starting it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::node_types::RuntimeFactory;
use serde::{Deserialize, Serialize};

/// `schema = "conex.application/1"` (private-runtime §2).
pub const SCHEMA: &str = "conex.application/1";
/// Hard package byte ceiling (private-runtime §2).
pub const MAX_PACKAGE_BYTES: usize = 256 * 1024;
/// Hard per-package module count ceiling.
pub const MAX_MODULES: usize = 64;
/// Hard per-package remote binding count ceiling.
pub const MAX_BINDINGS: usize = 64;
/// Hard per-module local dependency alias count ceiling.
pub const MAX_LOCAL_DEPENDENCIES: usize = 64;
/// Every identifier/name in a package and in the registry must fit this bound.
pub const MAX_IDENTIFIER_BYTES: usize = 64;

/// Source data export policy (private-runtime §5). `DirectOnly` is the
/// strictest; `Ord` follows strictness (`DirectOnly < RelayAllowed`), so the
/// intersection of source policies is the `min`/stricter value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceStrategy {
    DirectOnly,
    RelayAllowed,
}
impl SourceStrategy {
    /// Default export strategy when a package omits `strategy`.
    pub const DEFAULT: Self = Self::DirectOnly;
    /// Intersection of two source policies — the stricter one wins.
    pub fn stricter(self, other: Self) -> Self {
        if self <= other { self } else { other }
    }
}
impl std::fmt::Display for SourceStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::DirectOnly => "direct-only",
            Self::RelayAllowed => "relay-allowed",
        })
    }
}
fn default_strategy() -> SourceStrategy {
    SourceStrategy::DEFAULT
}

/// Parse node-approved configuration without executing module code. The returned
/// factory takes explicit node-owned runtime identity plus SDK ModuleContext.
/// Never invoked by [`resolve_application`].
pub type ApprovedFactory =
    Arc<dyn Fn(&serde_json::Value) -> Result<RuntimeFactory, String> + Send + Sync>;

/// One static, node-approved implementation registration.
#[derive(Clone)]
pub struct ImplementationEntry {
    /// Implementation key the package references in `ModuleSpec.implementation`.
    pub key: String,
    /// Real config-approved factory; the only way code is selected.
    pub factory: ApprovedFactory,
    /// Contract members this implementation *actually* implements:
    /// `contract_id -> member names`. Package exports must come from here,
    /// never from package self-declaration.
    pub implemented_contracts: BTreeMap<String, BTreeSet<String>>,
    /// Node-approved configuration table referenced by `config_ref`; the
    /// package cannot pass arbitrary env/file/command/URL values.
    pub approved_config_refs: BTreeMap<String, serde_json::Value>,
    /// Node-approved authorization ceiling for this implementation.
    pub permission_ceiling: BTreeSet<String>,
    /// The implementation's own approved source-data policy.
    pub source_strategy: SourceStrategy,
}
impl ImplementationEntry {
    pub fn new(
        key: impl Into<String>,
        factory: ApprovedFactory,
        implemented_contracts: BTreeMap<String, BTreeSet<String>>,
        approved_config_refs: BTreeMap<String, serde_json::Value>,
        permission_ceiling: BTreeSet<String>,
        source_strategy: SourceStrategy,
    ) -> Result<Self, String> {
        let key = key.into();
        if !valid_identifier(&key) {
            return Err(format!(
                "implementation key {key:?} must be a non-empty identifier of at most \
                 {MAX_IDENTIFIER_BYTES} bytes without control or whitespace"
            ));
        }
        for config_ref in approved_config_refs.keys() {
            if !valid_identifier(config_ref) {
                return Err(format!(
                    "approved config ref {config_ref:?} of implementation {key:?} is not a valid identifier"
                ));
            }
        }
        for (contract_id, members) in &implemented_contracts {
            if !valid_identifier(contract_id) {
                return Err(format!(
                    "implemented contract id {contract_id:?} of implementation {key:?} is not a valid identifier"
                ));
            }
            for member in members {
                if !valid_identifier(member) {
                    return Err(format!(
                        "contract member {member:?} of {contract_id:?}/{} is not a valid identifier",
                        key
                    ));
                }
            }
        }
        for permission in &permission_ceiling {
            if !valid_identifier(permission) {
                return Err(format!(
                    "ceiling permission {permission:?} of implementation {key:?} is not a valid identifier"
                ));
            }
        }
        Ok(Self {
            key,
            factory,
            implemented_contracts,
            approved_config_refs,
            permission_ceiling,
            source_strategy,
        })
    }
}

/// Node-approved remote endpoint a package binding may reference. A remote
/// binding is a *reference*, never an owned resource; it does not enter the
/// local start topology.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ApprovedRemoteEndpoint {
    pub node_ref: String,
    pub provider_endpoint_ref: String,
    pub contract_id: String,
}
impl ApprovedRemoteEndpoint {
    pub fn new(
        node_ref: impl Into<String>,
        provider_endpoint_ref: impl Into<String>,
        contract_id: impl Into<String>,
    ) -> Result<Self, String> {
        let endpoint = Self {
            node_ref: node_ref.into(),
            provider_endpoint_ref: provider_endpoint_ref.into(),
            contract_id: contract_id.into(),
        };
        for field in [
            &endpoint.node_ref,
            &endpoint.provider_endpoint_ref,
            &endpoint.contract_id,
        ] {
            if !valid_identifier(field) {
                return Err(format!(
                    "approved remote endpoint field {field:?} must be a non-empty identifier of at \
                     most {MAX_IDENTIFIER_BYTES} bytes"
                ));
            }
        }
        Ok(endpoint)
    }
}

/// Static, immutable registry of everything the node approves. Built from node
/// configuration only — never from a package.
#[derive(Clone, Default)]
pub struct ImplementationRegistry {
    implementations: Arc<BTreeMap<String, ImplementationEntry>>,
    remote_endpoints: Arc<BTreeSet<ApprovedRemoteEndpoint>>,
}
impl ImplementationRegistry {
    pub fn new(
        implementations: Vec<ImplementationEntry>,
        remote_endpoints: Vec<ApprovedRemoteEndpoint>,
    ) -> Result<Self, String> {
        let mut map = BTreeMap::new();
        for entry in implementations {
            if map.insert(entry.key.clone(), entry).is_some() {
                return Err("duplicate implementation key in node registry".to_owned());
            }
        }
        let mut endpoints = BTreeSet::new();
        for endpoint in remote_endpoints {
            endpoints.insert(endpoint);
        }
        Ok(Self {
            implementations: Arc::new(map),
            remote_endpoints: Arc::new(endpoints),
        })
    }

    pub fn implementation(&self, key: &str) -> Option<&ImplementationEntry> {
        self.implementations.get(key)
    }

    /// Config-approved instantiation for the Node at start time. Never called
    /// during [`resolve_application`]; the resolved module carries the approved
    /// config value, and the factory is the only code selector.
    pub fn instantiate(&self, module: &ResolvedModule) -> Result<RuntimeFactory, String> {
        let entry = self
            .implementations
            .get(&module.implementation)
            .ok_or_else(|| {
                format!(
                    "implementation {:?} missing from node registry",
                    module.implementation
                )
            })?;
        (entry.factory)(&module.config)
    }
}

/// Exact assembly-error surface (private-runtime §2). Diagnostics are
/// non-secret identifiers and file/parse errors only.
#[derive(Clone, Debug, PartialEq)]
pub enum AppError {
    /// Package defect: wrong schema, size, identifier, count, parse error,
    /// unknown/duplicate key, duplicate list item. Diagnostic is non-secret.
    InvalidPackage {
        diagnostic: String,
    },
    UnknownImplementation {
        implementation: String,
    },
    ConfigRefNotAllowed {
        implementation: String,
        config_ref: String,
    },
    DuplicateModule {
        module: String,
    },
    DuplicateBinding {
        name: String,
    },
    MissingDependency {
        module: String,
        dependency: String,
    },
    CircularDependency {
        cycle: Vec<String>,
    },
    GrantNotAllowed {
        module: String,
        permission: String,
    },
    ExportNotImplemented {
        module: String,
        contract_id: String,
    },
    ExportNotAllowed {
        module: String,
        contract_id: String,
        strategy: SourceStrategy,
    },
    OwnerMismatch {
        expected: String,
        actual: String,
    },
}
impl AppError {
    fn invalid_package(diagnostic: impl Into<String>) -> Self {
        Self::InvalidPackage {
            diagnostic: diagnostic.into(),
        }
    }
}
impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPackage { diagnostic } => {
                write!(f, "invalid application package: {diagnostic}")
            }
            Self::UnknownImplementation { implementation } => {
                write!(f, "unknown implementation {implementation:?}")
            }
            Self::ConfigRefNotAllowed {
                implementation,
                config_ref,
            } => write!(
                f,
                "reference {config_ref:?} is not in the node-approved table of {implementation:?}"
            ),
            Self::DuplicateModule { module } => write!(f, "duplicate module {module:?}"),
            Self::DuplicateBinding { name } => write!(f, "duplicate remote binding {name:?}"),
            Self::MissingDependency { module, dependency } => write!(
                f,
                "module {module:?} has a local dependency on unknown module {dependency:?}"
            ),
            Self::CircularDependency { cycle } => {
                write!(f, "local dependency cycle: {}", cycle.join(" -> "))
            }
            Self::GrantNotAllowed { module, permission } => write!(
                f,
                "permission {permission:?} of module {module:?} exceeds the node-approved ceiling"
            ),
            Self::ExportNotImplemented {
                module,
                contract_id,
            } => write!(
                f,
                "module {module:?} exports contract {contract_id:?} that its implementation \
                 does not actually implement"
            ),
            Self::ExportNotAllowed {
                module,
                contract_id,
                strategy,
            } => write!(
                f,
                "module {module:?} export {contract_id:?} with strategy {strategy} exceeds the \
                 source-data policy intersection"
            ),
            Self::OwnerMismatch { expected, actual } => {
                write!(
                    f,
                    "package owner {actual:?} does not match trusted node {expected:?}"
                )
            }
        }
    }
}
impl std::error::Error for AppError {}

// --- Package TOML schema (strict: unknown and duplicate keys rejected) ------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AppPackage {
    schema: String,
    app_id: String,
    version: String,
    owner_node_ref: String,
    #[serde(default)]
    modules: Vec<ModuleSpec>,
    #[serde(default)]
    remote_bindings: Vec<RemoteBindingSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModuleSpec {
    id: String,
    implementation: String,
    #[serde(default)]
    config_ref: Option<String>,
    #[serde(default)]
    permissions: Vec<String>,
    /// `alias -> target module id`; the alias is the name the module's factory
    /// uses for `Deps::local(alias)`, the value is the resolved module id.
    #[serde(default)]
    local_dependencies: BTreeMap<String, String>,
    #[serde(default)]
    exports: Vec<ExportSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExportSpec {
    contract_id: String,
    #[serde(default = "default_strategy")]
    strategy: SourceStrategy,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteBindingSpec {
    name: String,
    node_ref: String,
    provider_endpoint_ref: String,
    contract_id: String,
}

// --- Resolved, immutable application ----------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedApplication {
    pub app_id: String,
    pub version: String,
    pub owner_node_ref: String,
    /// Modules in stable Kahn order (module id/name tie-breaker).
    pub ordered_modules: Vec<ResolvedModule>,
    /// Approved bindings, sorted by name; never part of the local topology.
    pub remote_bindings: Vec<ResolvedRemoteBinding>,
    /// Flattened exports sorted by `(module_id, contract_id)`.
    pub exports: Vec<ResolvedExport>,
}
impl ResolvedApplication {
    pub fn module(&self, id: &str) -> Option<&ResolvedModule> {
        self.ordered_modules.iter().find(|module| module.id == id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedModule {
    pub id: String,
    pub implementation: String,
    /// Approved config for the module's `config_ref`; `Value::Null` when the
    /// package declared no config_ref (implementation-selected defaults).
    pub config: serde_json::Value,
    /// Final permission set: subset of the implementation ceiling AND the node
    /// installation policy; sorted.
    pub permissions: Vec<String>,
    /// `alias -> target module id`, sorted by alias. This is the exact mapping
    /// the Node needs to build per-consumer SDK `Deps` for DI.
    pub local_dependencies: Vec<LocalDependency>,
    /// This module's exports, sorted by contract id.
    pub exports: Vec<ResolvedExport>,
    /// Intersection of the implementation's own approved strategy and every
    /// dependency's effective strategy.
    pub effective_source_strategy: SourceStrategy,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LocalDependency {
    /// The name the consuming module's factory passes to `Deps::local`.
    pub alias: String,
    /// The resolved upstream module id.
    pub target_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedRemoteBinding {
    pub name: String,
    pub node_ref: String,
    pub provider_endpoint_ref: String,
    pub contract_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResolvedExport {
    pub module_id: String,
    pub contract_id: String,
    pub strategy: SourceStrategy,
    /// Members the implementation actually implements for this contract;
    /// never package-claimed.
    pub members: BTreeSet<String>,
}

// --- Validation --------------------------------------------------------------

fn valid_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_IDENTIFIER_BYTES
        && !bytes
            .iter()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
}

fn check_identifier(value: &str) -> Result<(), AppError> {
    if valid_identifier(value) {
        Ok(())
    } else {
        Err(AppError::invalid_package(format!(
            "identifier {value:?} must be non-empty, at most {MAX_IDENTIFIER_BYTES} bytes, \
             without ASCII control or whitespace"
        )))
    }
}

/// Validate a whole package without any side effect and return one immutable
/// resolved application. All identifiers, limits, approvals, dependency-graph
/// and source-policy constraints are enforced before a Node may create any
/// process or instance.
pub fn resolve_application(
    package_bytes: &[u8],
    registry: &ImplementationRegistry,
    owner_node: &str,
    node_policy: &BTreeSet<String>,
) -> Result<ResolvedApplication, AppError> {
    if package_bytes.len() > MAX_PACKAGE_BYTES {
        return Err(AppError::invalid_package(format!(
            "application package is {} bytes, exceeds the {MAX_PACKAGE_BYTES} byte limit",
            package_bytes.len()
        )));
    }
    let text = std::str::from_utf8(package_bytes)
        .map_err(|error| AppError::invalid_package(format!("package is not UTF-8: {error}")))?;
    let package: AppPackage = toml::from_str(text).map_err(|error| {
        AppError::invalid_package(format!(
            "package TOML parse failed at bytes {:?}",
            error.span()
        ))
    })?;
    if package.schema != SCHEMA {
        return Err(AppError::invalid_package(format!(
            "unsupported schema {:?}, expected {SCHEMA:?}",
            package.schema
        )));
    }
    for field in [&package.app_id, &package.version, &package.owner_node_ref] {
        check_identifier(field)?;
    }
    if package.modules.is_empty() {
        return Err(AppError::invalid_package(
            "application package declares no modules",
        ));
    }
    if package.modules.len() > MAX_MODULES {
        return Err(AppError::invalid_package(format!(
            "application package declares {} modules, exceeds the {MAX_MODULES} module limit",
            package.modules.len()
        )));
    }
    if package.remote_bindings.len() > MAX_BINDINGS {
        return Err(AppError::invalid_package(format!(
            "application package declares {} remote bindings, exceeds the {MAX_BINDINGS} \
             binding limit",
            package.remote_bindings.len()
        )));
    }
    if package.owner_node_ref != owner_node {
        return Err(AppError::OwnerMismatch {
            expected: owner_node.to_owned(),
            actual: package.owner_node_ref.clone(),
        });
    }

    // Identifiers, duplicates and per-module list limits.
    let mut seen_modules: BTreeSet<String> = BTreeSet::new();
    for module in &package.modules {
        check_identifier(&module.id)?;
        check_identifier(&module.implementation)?;
        if let Some(config_ref) = &module.config_ref {
            check_identifier(config_ref)?;
        }
        for alias in module.local_dependencies.keys() {
            check_identifier(alias)?;
        }
        for target in module.local_dependencies.values() {
            check_identifier(target)?;
        }
        for permission in &module.permissions {
            check_identifier(permission)?;
        }
        for export in &module.exports {
            check_identifier(&export.contract_id)?;
        }
        if !seen_modules.insert(module.id.clone()) {
            return Err(AppError::DuplicateModule {
                module: module.id.clone(),
            });
        }
        if module.local_dependencies.len() > MAX_LOCAL_DEPENDENCIES {
            return Err(AppError::invalid_package(format!(
                "module {:?} declares {} local dependencies, exceeds the \
                 {MAX_LOCAL_DEPENDENCIES} dependency limit",
                module.id,
                module.local_dependencies.len()
            )));
        }
        let mut seen_permissions: BTreeSet<String> = BTreeSet::new();
        for permission in &module.permissions {
            if !seen_permissions.insert(permission.clone()) {
                return Err(AppError::invalid_package(format!(
                    "module {:?} declares permission {:?} more than once",
                    module.id, permission
                )));
            }
        }
        let mut seen_exports: BTreeSet<String> = BTreeSet::new();
        for export in &module.exports {
            if !seen_exports.insert(export.contract_id.clone()) {
                return Err(AppError::invalid_package(format!(
                    "module {:?} exports contract {:?} more than once",
                    module.id, export.contract_id
                )));
            }
        }
    }
    let mut seen_bindings: BTreeSet<String> = BTreeSet::new();
    for binding in &package.remote_bindings {
        check_identifier(&binding.name)?;
        check_identifier(&binding.node_ref)?;
        check_identifier(&binding.provider_endpoint_ref)?;
        check_identifier(&binding.contract_id)?;
        if !seen_bindings.insert(binding.name.clone()) {
            return Err(AppError::DuplicateBinding {
                name: binding.name.clone(),
            });
        }
    }

    // Implementation existence, config_ref approval and permission ceilings.
    let index_by_id: BTreeMap<String, usize> = package
        .modules
        .iter()
        .enumerate()
        .map(|(i, module)| (module.id.clone(), i))
        .collect();
    let mut configs: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for module in &package.modules {
        let entry = registry
            .implementations
            .get(&module.implementation)
            .ok_or_else(|| AppError::UnknownImplementation {
                implementation: module.implementation.clone(),
            })?;
        let config = match &module.config_ref {
            Some(config_ref) => entry
                .approved_config_refs
                .get(config_ref)
                .cloned()
                .ok_or_else(|| AppError::ConfigRefNotAllowed {
                    implementation: module.implementation.clone(),
                    config_ref: config_ref.clone(),
                })?,
            None => serde_json::Value::Null,
        };
        // Requested permissions must be inside the implementation ceiling AND
        // the node installation policy; both are approved, nothing package-chosen.
        for permission in &module.permissions {
            if !entry.permission_ceiling.contains(permission) || !node_policy.contains(permission) {
                return Err(AppError::GrantNotAllowed {
                    module: module.id.clone(),
                    permission: permission.clone(),
                });
            }
        }
        configs.insert(module.id.clone(), config);
    }

    // Local dependency targets must exist before any graph ordering.
    for module in &package.modules {
        for target in module.local_dependencies.values() {
            if !index_by_id.contains_key(target) {
                return Err(AppError::MissingDependency {
                    module: module.id.clone(),
                    dependency: target.clone(),
                });
            }
        }
    }

    // Stable Kahn order (module name tie-breaker) with a concrete cycle path.
    let order_ids = kahn_order(&package.modules)?;

    // Effective source strategies in topological order, then export checks.
    let mut effective: BTreeMap<String, SourceStrategy> = BTreeMap::new();
    for id in &order_ids {
        let module = &package.modules[index_by_id[id]];
        let entry = &registry.implementations[&module.implementation];
        let mut strategy = entry.source_strategy;
        for target in module.local_dependencies.values() {
            strategy = strategy.stricter(effective[target]);
        }
        effective.insert(id.clone(), strategy);
    }
    let mut app_exports: Vec<ResolvedExport> = Vec::new();
    for id in &order_ids {
        let module = &package.modules[index_by_id[id]];
        let entry = &registry.implementations[&module.implementation];
        let allowed = effective[id];
        for export in &module.exports {
            let members = entry
                .implemented_contracts
                .get(&export.contract_id)
                .cloned()
                .ok_or_else(|| AppError::ExportNotImplemented {
                    module: id.clone(),
                    contract_id: export.contract_id.clone(),
                })?;
            // The package can never raise its own strategy (registry-owned) and
            // can never wash out a stricter upstream: `allowed` is the
            // intersection of every source policy.
            if export.strategy > allowed {
                return Err(AppError::ExportNotAllowed {
                    module: id.clone(),
                    contract_id: export.contract_id.clone(),
                    strategy: export.strategy,
                });
            }
            app_exports.push(ResolvedExport {
                module_id: id.clone(),
                contract_id: export.contract_id.clone(),
                strategy: export.strategy,
                members,
            });
        }
    }

    // Remote bindings: shape is approved by the registry; they never enter the
    // cross-machine start transaction or the local topology.
    let mut bindings: Vec<ResolvedRemoteBinding> =
        Vec::with_capacity(package.remote_bindings.len());
    for binding in &package.remote_bindings {
        let approved = ApprovedRemoteEndpoint::new(
            &binding.node_ref,
            &binding.provider_endpoint_ref,
            &binding.contract_id,
        )
        .map_err(AppError::invalid_package)?;
        if !registry.remote_endpoints.contains(&approved) {
            return Err(AppError::ConfigRefNotAllowed {
                implementation: format!("remote-binding:{:?}", binding.name),
                config_ref: format!(
                    "{}/{}/{}",
                    binding.node_ref, binding.provider_endpoint_ref, binding.contract_id
                ),
            });
        }
        bindings.push(ResolvedRemoteBinding {
            name: binding.name.clone(),
            node_ref: binding.node_ref.clone(),
            provider_endpoint_ref: binding.provider_endpoint_ref.clone(),
            contract_id: binding.contract_id.clone(),
        });
    }
    bindings.sort_by(|a, b| a.name.cmp(&b.name));

    // Assemble the immutable resolved application in the validated order.
    let mut ordered = Vec::with_capacity(order_ids.len());
    for id in order_ids {
        let module = &package.modules[index_by_id[&id]];
        let config = configs.remove(&id).expect("config resolved above");
        let permissions: Vec<String> = module
            .permissions
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let local_dependencies: Vec<LocalDependency> = module
            .local_dependencies
            .iter()
            .map(|(alias, target_id)| LocalDependency {
                alias: alias.clone(),
                target_id: target_id.clone(),
            })
            .collect();
        let mut exports: Vec<ResolvedExport> = app_exports
            .iter()
            .filter(|export| export.module_id == id)
            .cloned()
            .collect();
        exports.sort_by(|a, b| a.contract_id.cmp(&b.contract_id));
        let effective_source_strategy = effective[&id];
        ordered.push(ResolvedModule {
            id,
            implementation: module.implementation.clone(),
            config,
            permissions,
            local_dependencies,
            exports,
            effective_source_strategy,
        });
    }
    app_exports.sort_by(|a, b| (&a.module_id, &a.contract_id).cmp(&(&b.module_id, &b.contract_id)));

    Ok(ResolvedApplication {
        app_id: package.app_id,
        version: package.version,
        owner_node_ref: package.owner_node_ref,
        ordered_modules: ordered,
        remote_bindings: bindings,
        exports: app_exports,
    })
}

/// Stable Kahn order over `local_dependencies` only. The ready set is keyed by
/// module id, so ties break deterministically by module name. Leftover nodes
/// produce a concrete cycle path.
fn kahn_order(modules: &[ModuleSpec]) -> Result<Vec<String>, AppError> {
    let index: BTreeMap<String, usize> = modules
        .iter()
        .enumerate()
        .map(|(i, module)| (module.id.clone(), i))
        .collect();
    let count = modules.len();
    let mut dependents: Vec<Vec<String>> = vec![Vec::new(); count];
    let mut indegree = vec![0usize; count];
    for (i, module) in modules.iter().enumerate() {
        // Multiple aliases to the same target count once.
        let targets: BTreeSet<&str> = module
            .local_dependencies
            .values()
            .map(String::as_str)
            .collect();
        for target in targets {
            let upstream = index[target];
            dependents[upstream].push(module.id.clone());
            indegree[i] += 1;
        }
    }
    let mut ready: BTreeSet<String> = (0..count)
        .filter(|&i| indegree[i] == 0)
        .map(|i| modules[i].id.clone())
        .collect();
    let mut order = Vec::with_capacity(count);
    while let Some(id) = ready.iter().next().cloned() {
        ready.remove(&id);
        order.push(id.clone());
        for downstream in &dependents[index[&id]] {
            let downstream_index = index[downstream];
            indegree[downstream_index] -= 1;
            if indegree[downstream_index] == 0 {
                ready.insert(modules[downstream_index].id.clone());
            }
        }
    }
    if order.len() == count {
        return Ok(order);
    }
    // Every leftover node has at least one dependency edge to another leftover;
    // walking those edges revisits a node — that segment is the cycle.
    let remaining: BTreeSet<String> = modules
        .iter()
        .map(|module| module.id.clone())
        .filter(|id| !order.contains(id))
        .collect();
    let mut path = vec![
        remaining
            .iter()
            .next()
            .expect("leftover modules exist")
            .clone(),
    ];
    loop {
        let current = path.last().expect("non-empty").clone();
        let module = &modules[index[&current]];
        let next = module
            .local_dependencies
            .values()
            .map(String::as_str)
            .find(|target| remaining.contains(*target))
            .expect("leftover modules always have a leftover dependency")
            .to_owned();
        if let Some(position) = path.iter().position(|node| node == &next) {
            path.drain(..position);
            break;
        }
        path.push(next);
    }
    Err(AppError::CircularDependency { cycle: path })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const HEADER: &str = "schema = \"conex.application/1\"\napp_id = \"workbench\"\nversion = \"0.1.0\"\nowner_node_ref = \"home-node\"\n";

    fn stub_factory() -> ApprovedFactory {
        // Resolution must never execute a module's runtime factory.
        Arc::new(
            |config: &serde_json::Value| -> Result<RuntimeFactory, String> {
                if !config.is_object() && *config != serde_json::Value::Null {
                    return Err(
                        "stub implementation accepts only an approved object or null config"
                            .to_owned(),
                    );
                }
                Ok(
                    Arc::new(
                        |_runtime: crate::node_types::ModuleRuntime,
                         _context: conex_sdk::plugin::ModuleContext|
                         -> conex_sdk::capability::HandlerFuture<
                            conex_sdk::plugin::StartedModule,
                        > {
                            Box::pin(async {
                                Err(conex_sdk::Error::new(
                                    conex_sdk::proto::ErrorCode::BadRequest,
                                    "stub module must never be started by resolve",
                                    "not_started",
                                    "never",
                                ))
                            })
                        },
                    ),
                )
            },
        )
    }

    fn counter_entry() -> ImplementationEntry {
        ImplementationEntry::new(
            "counter",
            stub_factory(),
            BTreeMap::from([(
                "conex.counter/1".to_owned(),
                BTreeSet::from(["read".to_owned(), "add".to_owned()]),
            )]),
            BTreeMap::from([("counter-default".to_owned(), json!({"external": false}))]),
            BTreeSet::from([
                "counter.read".to_owned(),
                "counter.add".to_owned(),
                // In the impl ceiling but NOT in the default node policy, so
                // the node-policy intersection branch is exercised.
                "agent.execute".to_owned(),
            ]),
            SourceStrategy::DirectOnly,
        )
        .unwrap()
    }

    fn metrics_entry() -> ImplementationEntry {
        ImplementationEntry::new(
            "linux-metrics",
            stub_factory(),
            BTreeMap::from([(
                "conex.metrics/1".to_owned(),
                BTreeSet::from(["collect".to_owned()]),
            )]),
            BTreeMap::new(),
            BTreeSet::new(),
            SourceStrategy::RelayAllowed,
        )
        .unwrap()
    }

    fn summary_entry() -> ImplementationEntry {
        ImplementationEntry::new(
            "environment-summary",
            stub_factory(),
            BTreeMap::from([(
                "conex.environment/1".to_owned(),
                BTreeSet::from(["summarize".to_owned()]),
            )]),
            BTreeMap::from([("summary-default".to_owned(), json!({"scope": "workspace"}))]),
            BTreeSet::from(["environment.read".to_owned()]),
            SourceStrategy::DirectOnly,
        )
        .unwrap()
    }

    fn default_registry() -> ImplementationRegistry {
        ImplementationRegistry::new(
            vec![counter_entry(), metrics_entry(), summary_entry()],
            vec![ApprovedRemoteEndpoint::new("second-node", "agent", "conex.agent/1").unwrap()],
        )
        .unwrap()
    }

    fn default_policy() -> BTreeSet<String> {
        BTreeSet::from([
            "counter.read".to_owned(),
            "counter.add".to_owned(),
            "agent.read".to_owned(),
            "agent.interact".to_owned(),
        ])
    }

    fn resolve(
        package: &str,
        registry: &ImplementationRegistry,
        policy: &BTreeSet<String>,
    ) -> Result<ResolvedApplication, AppError> {
        resolve_application(package.as_bytes(), registry, "home-node", policy)
    }

    fn valid_package() -> String {
        format!(
            "{HEADER}\
[[modules]]
id = \"counter\"
implementation = \"counter\"
config_ref = \"counter-default\"
permissions = [\"counter.read\", \"counter.add\"]

[[modules]]
id = \"metrics\"
implementation = \"linux-metrics\"
exports = [{{ contract_id = \"conex.metrics/1\", strategy = \"relay-allowed\" }}]

[[modules]]
id = \"summary\"
implementation = \"environment-summary\"
config_ref = \"summary-default\"
local_dependencies = {{ upstream = \"counter\", env = \"metrics\" }}
exports = [{{ contract_id = \"conex.environment/1\" }}]

[[remote_bindings]]
name = \"other-agent\"
node_ref = \"second-node\"
provider_endpoint_ref = \"agent\"
contract_id = \"conex.agent/1\"
"
        )
    }

    #[test]
    fn resolves_valid_package_in_stable_kahn_order() {
        let registry = default_registry();
        let policy = default_policy();
        let resolved = resolve(&valid_package(), &registry, &policy).expect("valid package");

        assert_eq!(resolved.app_id, "workbench");
        assert_eq!(resolved.owner_node_ref, "home-node");
        // counter and metrics are both roots; the module-name tie-breaker puts
        // counter first, then metrics; summary needs both.
        let ids: Vec<&str> = resolved
            .ordered_modules
            .iter()
            .map(|module| module.id.as_str())
            .collect();
        assert_eq!(ids, ["counter", "metrics", "summary"]);

        let counter = resolved.module("counter").expect("counter");
        assert_eq!(counter.config, json!({"external": false}));
        assert_eq!(counter.permissions, ["counter.add", "counter.read"]);
        assert!(counter.local_dependencies.is_empty());
        assert_eq!(
            counter.effective_source_strategy,
            SourceStrategy::DirectOnly
        );

        let metrics = resolved.module("metrics").expect("metrics");
        assert_eq!(metrics.config, serde_json::Value::Null); // no config_ref
        assert_eq!(
            metrics.effective_source_strategy,
            SourceStrategy::RelayAllowed
        );
        assert_eq!(metrics.exports.len(), 1);
        assert_eq!(metrics.exports[0].contract_id, "conex.metrics/1");
        assert_eq!(metrics.exports[0].strategy, SourceStrategy::RelayAllowed);
        assert_eq!(
            metrics.exports[0].members,
            BTreeSet::from(["collect".to_owned()])
        );

        // Alias mapping survives into the resolved graph (env -> metrics,
        // upstream -> counter, sorted by alias).
        let summary = resolved.module("summary").expect("summary");
        assert_eq!(
            summary.local_dependencies,
            vec![
                LocalDependency {
                    alias: "env".into(),
                    target_id: "metrics".into()
                },
                LocalDependency {
                    alias: "upstream".into(),
                    target_id: "counter".into()
                },
            ]
        );
        assert_eq!(
            summary.effective_source_strategy,
            SourceStrategy::DirectOnly
        );
        assert_eq!(summary.exports[0].strategy, SourceStrategy::DirectOnly);
        assert_eq!(
            summary.exports[0].members,
            BTreeSet::from(["summarize".to_owned()])
        );

        // Bindings are approved and live outside the local topology.
        assert_eq!(resolved.remote_bindings.len(), 1);
        assert_eq!(resolved.remote_bindings[0].name, "other-agent");
        assert_eq!(resolved.remote_bindings[0].node_ref, "second-node");

        // Flattened exports sorted by (module_id, contract_id).
        let flat: Vec<(&str, &str)> = resolved
            .exports
            .iter()
            .map(|export| (export.module_id.as_str(), export.contract_id.as_str()))
            .collect();
        assert_eq!(
            flat,
            [
                ("metrics", "conex.metrics/1"),
                ("summary", "conex.environment/1")
            ]
        );
    }

    #[test]
    fn rejects_unknown_and_duplicate_keys() {
        let registry = default_registry();
        let policy = default_policy();
        let unknown =
            format!("{HEADER}\nbogus = 1\n[[modules]]\nid = \"m\"\nimplementation = \"counter\"\n");
        assert!(matches!(
            resolve(&unknown, &registry, &policy),
            Err(AppError::InvalidPackage { .. })
        ));
        let duplicate = format!(
            "{HEADER}app_id = \"other\"\n[[modules]]\nid = \"m\"\nimplementation = \"counter\"\n"
        );
        assert!(matches!(
            resolve(&duplicate, &registry, &policy),
            Err(AppError::InvalidPackage { .. })
        ));
    }

    #[test]
    fn rejects_wrong_schema() {
        let package = valid_package().replace("conex.application/1", "conex.application/2");
        assert!(matches!(
            resolve(&package, &default_registry(), &default_policy()),
            Err(AppError::InvalidPackage { .. })
        ));
    }

    #[test]
    fn rejects_oversized_package() {
        let mut bytes = valid_package().into_bytes();
        while bytes.len() <= MAX_PACKAGE_BYTES {
            bytes.push(b' ');
        }
        let error =
            resolve_application(&bytes, &default_registry(), "home-node", &default_policy())
                .expect_err("oversized");
        assert!(matches!(error, AppError::InvalidPackage { .. }));
    }

    #[test]
    fn rejects_non_utf8_package() {
        let error = resolve_application(
            &[0xff, 0xfe],
            &default_registry(),
            "home-node",
            &default_policy(),
        )
        .expect_err("non utf8");
        assert!(matches!(error, AppError::InvalidPackage { .. }));
    }

    #[test]
    fn rejects_owner_mismatch() {
        let error = resolve_application(
            valid_package().as_bytes(),
            &default_registry(),
            "other-node",
            &default_policy(),
        )
        .expect_err("owner mismatch");
        assert_eq!(
            error,
            AppError::OwnerMismatch {
                expected: "other-node".into(),
                actual: "home-node".into(),
            }
        );
    }

    #[test]
    fn rejects_unknown_implementation() {
        let package =
            valid_package().replace("implementation = \"counter\"", "implementation = \"nope\"");
        let error = resolve(&package, &default_registry(), &default_policy()).expect_err("impl");
        assert_eq!(
            error,
            AppError::UnknownImplementation {
                implementation: "nope".into(),
            }
        );
    }

    #[test]
    fn rejects_config_ref_not_allowed() {
        let package = valid_package().replace(
            "config_ref = \"counter-default\"",
            "config_ref = \"not-approved\"",
        );
        let error = resolve(&package, &default_registry(), &default_policy()).expect_err("ref");
        assert_eq!(
            error,
            AppError::ConfigRefNotAllowed {
                implementation: "counter".into(),
                config_ref: "not-approved".into(),
            }
        );
    }

    #[test]
    fn rejects_duplicate_module_id() {
        let package = format!(
            "{HEADER}\
[[modules]]
id = \"counter\"
implementation = \"counter\"

[[modules]]
id = \"counter\"
implementation = \"linux-metrics\"
"
        );
        let error = resolve(&package, &default_registry(), &default_policy()).expect_err("dup");
        assert_eq!(
            error,
            AppError::DuplicateModule {
                module: "counter".into()
            }
        );
    }

    #[test]
    fn rejects_duplicate_binding_name() {
        let package = format!(
            "{HEADER}\
[[modules]]
id = \"m\"
implementation = \"counter\"

[[remote_bindings]]
name = \"other-agent\"
node_ref = \"second-node\"
provider_endpoint_ref = \"agent\"
contract_id = \"conex.agent/1\"

[[remote_bindings]]
name = \"other-agent\"
node_ref = \"third-node\"
provider_endpoint_ref = \"agent\"
contract_id = \"conex.agent/1\"
"
        );
        let error = resolve(&package, &default_registry(), &default_policy()).expect_err("dup");
        assert_eq!(
            error,
            AppError::DuplicateBinding {
                name: "other-agent".into()
            }
        );
    }

    #[test]
    fn rejects_missing_dependency() {
        let package = valid_package().replace("env = \"metrics\"", "env = \"ghost\"");
        let error = resolve(&package, &default_registry(), &default_policy()).expect_err("miss");
        assert_eq!(
            error,
            AppError::MissingDependency {
                module: "summary".into(),
                dependency: "ghost".into(),
            }
        );
    }

    #[test]
    fn reports_concrete_cycle() {
        let package = format!(
            "{HEADER}\
[[modules]]
id = \"a\"
implementation = \"counter\"
local_dependencies = {{ x = \"b\" }}

[[modules]]
id = \"b\"
implementation = \"counter\"
local_dependencies = {{ x = \"a\" }}
"
        );
        let error = resolve(&package, &default_registry(), &default_policy()).expect_err("cycle");
        assert_eq!(
            error,
            AppError::CircularDependency {
                cycle: vec!["a".into(), "b".into()]
            }
        );
    }

    #[test]
    fn reports_self_cycle() {
        let package = format!(
            "{HEADER}\
[[modules]]
id = \"a\"
implementation = \"counter\"
local_dependencies = {{ x = \"a\" }}
"
        );
        let error = resolve(&package, &default_registry(), &default_policy()).expect_err("cycle");
        assert_eq!(
            error,
            AppError::CircularDependency {
                cycle: vec!["a".into()]
            }
        );
    }

    #[test]
    fn rejects_permission_beyond_impl_ceiling() {
        let package = valid_package().replace(
            "[\"counter.read\", \"counter.add\"]",
            "[\"counter.read\", \"counter.reconcile\"]",
        );
        let error = resolve(&package, &default_registry(), &default_policy()).expect_err("grant");
        assert_eq!(
            error,
            AppError::GrantNotAllowed {
                module: "counter".into(),
                permission: "counter.reconcile".into(),
            }
        );
    }

    #[test]
    fn rejects_permission_beyond_node_policy_despite_impl_ceiling() {
        // "agent.execute" is inside the counter implementation ceiling but
        // outside the node installation policy passed to resolve.
        let package = valid_package().replace(
            "[\"counter.read\", \"counter.add\"]",
            "[\"counter.read\", \"agent.execute\"]",
        );
        let error = resolve(&package, &default_registry(), &default_policy()).expect_err("grant");
        assert_eq!(
            error,
            AppError::GrantNotAllowed {
                module: "counter".into(),
                permission: "agent.execute".into(),
            }
        );
    }

    #[test]
    fn rejects_export_not_implemented() {
        let package = valid_package().replace("conex.environment/1", "conex.environment/9");
        let error =
            resolve(&package, &default_registry(), &default_policy()).expect_err("export impl");
        assert_eq!(
            error,
            AppError::ExportNotImplemented {
                module: "summary".into(),
                contract_id: "conex.environment/9".into(),
            }
        );
    }

    #[test]
    fn rejects_self_declared_strategy_declassification() {
        // The counter implementation is approved direct-only; a relay-allowed
        // package export cannot wash that out (package cannot raise its own
        // registry-owned source strategy).
        let package = format!(
            "{HEADER}\
[[modules]]
id = \"counter\"
implementation = \"counter\"
config_ref = \"counter-default\"
exports = [{{ contract_id = \"conex.counter/1\", strategy = \"relay-allowed\" }}]
"
        );
        let error =
            resolve(&package, &default_registry(), &default_policy()).expect_err("declassify");
        assert_eq!(
            error,
            AppError::ExportNotAllowed {
                module: "counter".into(),
                contract_id: "conex.counter/1".into(),
                strategy: SourceStrategy::RelayAllowed,
            }
        );
    }

    #[test]
    fn rejects_relay_export_washing_direct_only_upstream() {
        // summary depends on counter (effective direct-only), so its
        // relay-allowed export must be rejected.
        let package = valid_package().replace(
            "exports = [{ contract_id = \"conex.environment/1\" }]",
            "exports = [{ contract_id = \"conex.environment/1\", strategy = \"relay-allowed\" }]",
        );
        let error =
            resolve(&package, &default_registry(), &default_policy()).expect_err("wash upstream");
        assert_eq!(
            error,
            AppError::ExportNotAllowed {
                module: "summary".into(),
                contract_id: "conex.environment/1".into(),
                strategy: SourceStrategy::RelayAllowed,
            }
        );
    }

    #[test]
    fn rejects_unapproved_remote_binding() {
        let package = format!(
            "{HEADER}\
[[modules]]
id = \"m\"
implementation = \"counter\"

[[remote_bindings]]
name = \"evil\"
node_ref = \"third-node\"
provider_endpoint_ref = \"agent\"
contract_id = \"conex.agent/1\"
"
        );
        let error = resolve(&package, &default_registry(), &default_policy())
            .expect_err("binding approval");
        assert_eq!(
            error,
            AppError::ConfigRefNotAllowed {
                implementation: "remote-binding:\"evil\"".into(),
                config_ref: "third-node/agent/conex.agent/1".into(),
            }
        );
    }

    #[test]
    fn multiple_aliases_to_one_target_stay_ordered() {
        let package = format!(
            "{HEADER}\
[[modules]]
id = \"counter\"
implementation = \"counter\"

[[modules]]
id = \"fan\"
implementation = \"linux-metrics\"
local_dependencies = {{ b = \"counter\", a = \"counter\" }}
"
        );
        let resolved =
            resolve(&package, &default_registry(), &default_policy()).expect("valid fan package");
        let ids: Vec<&str> = resolved
            .ordered_modules
            .iter()
            .map(|module| module.id.as_str())
            .collect();
        assert_eq!(ids, ["counter", "fan"]);
        let fan = resolved.module("fan").expect("fan");
        assert_eq!(
            fan.local_dependencies,
            vec![
                LocalDependency {
                    alias: "a".into(),
                    target_id: "counter".into()
                },
                LocalDependency {
                    alias: "b".into(),
                    target_id: "counter".into()
                },
            ]
        );
        // A single upstream, aliased twice, still counts as one edge.
        assert_eq!(fan.effective_source_strategy, SourceStrategy::DirectOnly);
    }

    #[test]
    fn enforces_count_and_identifier_limits() {
        let registry = default_registry();
        let policy = default_policy();

        let mut many_modules = String::from(HEADER);
        for i in 0..MAX_MODULES + 1 {
            many_modules.push_str(&format!(
                "[[modules]]\nid = \"m{i}\"\nimplementation = \"counter\"\n\n"
            ));
        }
        assert!(matches!(
            resolve(&many_modules, &registry, &policy),
            Err(AppError::InvalidPackage { .. })
        ));

        let mut many_bindings = format!(
            "{HEADER}\
[[modules]]
id = \"m\"
implementation = \"counter\"

"
        );
        for i in 0..MAX_BINDINGS + 1 {
            many_bindings.push_str(&format!(
                "[[remote_bindings]]\nname = \"b{i}\"\nnode_ref = \"second-node\"\n\
                 provider_endpoint_ref = \"agent\"\ncontract_id = \"conex.agent/1\"\n\n"
            ));
        }
        assert!(matches!(
            resolve(&many_bindings, &registry, &policy),
            Err(AppError::InvalidPackage { .. })
        ));

        let deps: Vec<String> = (0..MAX_LOCAL_DEPENDENCIES + 1)
            .map(|i| format!("\"d{i}\" = \"m\""))
            .collect();
        let too_many_deps = format!(
            "{HEADER}\
[[modules]]
id = \"m\"
implementation = \"counter\"

[[modules]]
id = \"fan\"
implementation = \"linux-metrics\"
local_dependencies = {{ {} }}
",
            deps.join(", ")
        );
        assert!(matches!(
            resolve(&too_many_deps, &registry, &policy),
            Err(AppError::InvalidPackage { .. })
        ));

        let long_id = "x".repeat(MAX_IDENTIFIER_BYTES + 1);
        let long_identifier = format!(
            "{HEADER}\
[[modules]]
id = \"{long_id}\"
implementation = \"counter\"
"
        );
        assert!(matches!(
            resolve(&long_identifier, &registry, &policy),
            Err(AppError::InvalidPackage { .. })
        ));

        let empty_id = format!(
            "{HEADER}\
[[modules]]
id = \"\"
implementation = \"counter\"
"
        );
        assert!(matches!(
            resolve(&empty_id, &registry, &policy),
            Err(AppError::InvalidPackage { .. })
        ));
    }

    #[test]
    fn rejects_duplicate_permission_in_one_module() {
        let package = valid_package().replace(
            "[\"counter.read\", \"counter.add\"]",
            "[\"counter.read\", \"counter.read\"]",
        );
        assert!(matches!(
            resolve(&package, &default_registry(), &default_policy()),
            Err(AppError::InvalidPackage { .. })
        ));
    }

    #[test]
    fn rejects_empty_package_with_no_modules() {
        let package = format!("{HEADER}\n");
        assert!(matches!(
            resolve(&package, &default_registry(), &default_policy()),
            Err(AppError::InvalidPackage { .. })
        ));
    }

    #[test]
    fn registry_rejects_duplicate_implementation_key_and_bad_endpoint() {
        assert!(
            ImplementationRegistry::new(vec![counter_entry(), counter_entry()], Vec::new(),)
                .is_err()
        );
        assert!(ApprovedRemoteEndpoint::new("", "agent", "conex.agent/1").is_err());
        assert!(
            ImplementationEntry::new(
                "",
                stub_factory(),
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeSet::new(),
                SourceStrategy::DirectOnly,
            )
            .is_err()
        );
    }
}

#[cfg(test)]
mod diagnostic_tests {
    #[test]
    fn rejected_package_does_not_echo_unknown_secret_fields() {
        let secret = "private-diagnostic-canary-83f24";
        let package = format!(
            "schema='conex.application/1'\napp_id='a'\nversion='0.1.0'\nowner_node_ref='n'\nsecret='{secret}'\n"
        );
        let error = super::resolve_application(
            package.as_bytes(),
            &Default::default(),
            "n",
            &Default::default(),
        )
        .unwrap_err();
        assert!(matches!(error, super::AppError::InvalidPackage { .. }));
        assert!(!error.to_string().contains(secret));
    }
}

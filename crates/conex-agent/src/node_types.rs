//! Evaluated application inputs: order comes from application validation exactly
//! once; this layer never re-sorts or treats a dependency reference as ownership.
use conex_agent_domain::Store;
use conex_sdk::capability::HandlerFuture;
use conex_sdk::{ModuleContext, RemoteBinding, StartedModule};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone)]
pub struct ModuleRuntime {
    pub store: Store,
    pub data_dir: PathBuf,
    pub node_epoch: String,
    pub module_key: String,
    pub generation: u64,
    pub resource_id: String,
}

pub type RuntimeFactory =
    Arc<dyn Fn(ModuleRuntime, ModuleContext) -> HandlerFuture<StartedModule> + Send + Sync>;

#[derive(Clone)]
pub struct PreparedLocalModule {
    pub name: String,
    pub module_key: String,
    pub dependencies: BTreeMap<String, String>,
    pub permissions: BTreeSet<String>,
    pub relay_allowed: bool,
    pub exposed_methods: BTreeSet<String>,
    pub factory: RuntimeFactory,
}

#[derive(Clone)]
pub struct PreparedApplication {
    pub app_id: String,
    pub ordered_modules: Vec<PreparedLocalModule>,
    pub bindings: Vec<RemoteBinding>,
}

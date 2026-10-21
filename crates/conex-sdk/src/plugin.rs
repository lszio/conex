//! S2 plugin runtime (docs/contracts/s2-plugins.md): one [`Instance`] is one S1
//! link; modules compose inside the instance via process-local handles
//! ([`Deps::local`]) or explicit `ModuleRef` bindings ([`Deps::remote`]).
//!
//! Composition happens before any network frame: duplicate module names,
//! missing local/bound dependencies and local cycles are rejected with typed
//! [`PluginError`]s (never wire errors). Modules start in Kahn order of their
//! local dependencies; publication follows the same order after every module
//! has started, and mid-publication failure un-publishes in reverse.
//!
//! Derived modules are ordinary modules: same `contract_id`, own 4-tuple and
//! own revision sequence. A derived state handler MUST capture the upstream
//! snapshot BEFORE returning its [`StateLease`](crate::StateLease) — the lease
//! opening is what makes the derived subscription public, so snapshot-then-
//! updates holds on the derived boundary independently ("先快照后发布").
//!
//! `stop()` is the only resource-release path and is idempotent: reverse
//! topological unprovide → consumer/observer close → handler abort → link
//! close (the client's owner aborts and awaits its handler JoinSet). A broken
//! link walks the same path without reviving anything; reconnection and
//! instance re-creation belong to the caller.
#![forbid(unsafe_code)]

use crate::{
    Call, Client, ConnectOptions, Contract, Error, Exposure, ModuleRef, Subscription,
    capability::HandlerFuture,
};
use conex_proto as p;
use serde::Serialize;
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    mem,
    sync::{Arc, Mutex},
};
use tokio::sync::Mutex as AsyncMutex;

/// Service handle a module exposes to same-instance dependents. Shape is owned
/// by the plugin author; the SDK never inspects it (contract §5).
pub type Service = Arc<dyn Any + Send + Sync>;

/// Typed local composition error. These never cross the wire (contract §4).
#[derive(Clone, Debug, PartialEq)]
pub enum PluginError {
    /// Local dependency not in the module graph, or remote dependency without
    /// a `ModuleRef` binding.
    MissingDependency { module: String, dependency: String },
    /// Local dependency graph contains a cycle; `cycle` follows
    /// module → dependency edges and ends where it started.
    CircularDependency { cycle: Vec<String> },
    /// Same module name declared twice in one plugin.
    DuplicateModule { module: String },
    /// Connection, module start or publication failed; everything published
    /// so far has been rolled back.
    StartFailed { module: String, source: Error },
}
impl PluginError {
    fn start(module: impl Into<String>, source: Error) -> Self {
        Self::StartFailed {
            module: module.into(),
            source,
        }
    }
}
impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingDependency { module, dependency } => {
                write!(f, "module {module:?} missing dependency {dependency:?}")
            }
            Self::CircularDependency { cycle } => {
                write!(f, "dependency cycle: {}", cycle.join(" -> "))
            }
            Self::DuplicateModule { module } => write!(f, "duplicate module name {module:?}"),
            Self::StartFailed { module, source } => {
                write!(f, "module {module:?} start failed: {source}")
            }
        }
    }
}
impl std::error::Error for PluginError {}

/// Consumer handle bound to one dependency name's fixed `ModuleRef`. Calls go
/// through the instance's own S1 client with the already-verified semantics.
#[derive(Clone)]
pub struct RemoteDep {
    client: Client,
    contract: Arc<Contract>,
    pub target: ModuleRef,
}
impl RemoteDep {
    pub fn invoke(&self, method: &str, argument: impl Serialize) -> crate::Result<Call> {
        self.client
            .invoke(self.target.clone(), self.contract.clone(), method, argument)
    }
    pub fn subscribe(&self, state: &str, argument: impl Serialize) -> crate::Result<Subscription> {
        self.client
            .subscribe(self.target.clone(), self.contract.clone(), state, argument)
    }
}

/// Fixed remote binding supplied in [`InstanceConfig`].
#[derive(Clone)]
pub struct RemoteBinding {
    pub dependency: String,
    pub target: ModuleRef,
    /// Installed contract used to validate invocations/subscriptions against
    /// this binding (consumer side; no handlers required).
    pub contract: Arc<Contract>,
}

/// Per-module start input. `Deps` accessors return `None` for names not
/// resolved at composition time — which cannot happen for declared deps, so
/// `None` means the declaration and the factory disagree (author bug).
pub struct ModuleContext {
    pub deps: Deps,
}
#[derive(Clone, Default)]
pub struct Deps {
    locals: Arc<BTreeMap<String, Option<Service>>>,
    remotes: Arc<BTreeMap<String, RemoteDep>>,
}
impl Deps {
    /// Upstream module's start-time service handle, downcast to the author's
    /// type. Process-local call: no request id, no timeout envelope.
    pub fn local<T: Send + Sync + 'static>(&self, name: &str) -> Option<Arc<T>> {
        self.locals.get(name)?.clone()?.downcast::<T>().ok()
    }
    /// Remote binding consumer for the dependency name.
    pub fn remote(&self, name: &str) -> Option<RemoteDep> {
        self.remotes.get(name).cloned()
    }
}

/// What a module's start factory hands back. The contract is created inside
/// start so handlers can capture resolved dependency handles directly.
#[derive(Clone)]
pub struct StartedModule {
    pub contract: Arc<Contract>,
    /// Handle exposed to same-instance dependents via [`Deps::local`].
    pub service: Option<Service>,
}
impl StartedModule {
    pub fn new(contract: Arc<Contract>) -> Self {
        Self {
            contract,
            service: None,
        }
    }
}

pub type ModuleStart = Arc<dyn Fn(ModuleContext) -> HandlerFuture<StartedModule> + Send + Sync>;

/// Static module declaration: graph shape plus the start factory.
#[derive(Clone)]
pub struct ModuleDecl {
    pub name: String,
    /// Same-instance dependencies; they define the startup partial order.
    pub local_deps: Vec<String>,
    /// Dependency names that must be covered by [`InstanceConfig::bindings`].
    pub remote_deps: Vec<String>,
    pub start: ModuleStart,
}
impl ModuleDecl {
    pub fn new(
        name: impl Into<String>,
        local_deps: Vec<String>,
        remote_deps: Vec<String>,
        start: impl Fn(ModuleContext) -> HandlerFuture<StartedModule> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            local_deps,
            remote_deps,
            start: Arc::new(start),
        }
    }
}

/// Plugin class: installed once per process, instantiated many times.
#[derive(Clone)]
pub struct Plugin {
    pub id: String,
    modules: Arc<Vec<ModuleDecl>>,
}
impl Plugin {
    pub fn new(id: impl Into<String>, modules: Vec<ModuleDecl>) -> Self {
        Self {
            id: id.into(),
            modules: Arc::new(modules),
        }
    }
}

/// Per-instance connection and publication settings.
#[derive(Clone)]
pub struct InstanceConfig {
    pub url: String,
    pub bearer: String,
    /// Explicitly allow plaintext `ws://` on loopback (development).
    pub allow_loopback_ws: bool,
    pub space_id: String,
    pub integration_id: String,
    /// `provider_endpoint_id` per module is `{endpoint_prefix}-{module_name}`;
    /// distinct prefixes keep multiple instances separately approved.
    pub endpoint_prefix: String,
    pub bindings: Vec<RemoteBinding>,
    /// Provider-side local exposure allowlist for every published module.
    pub exposure_principals: BTreeSet<String>,
    pub exposure_tenants: BTreeSet<String>,
}

#[derive(Clone)]
pub struct Instance {
    inner: Arc<Inner>,
}
struct Inner {
    plugin: Plugin,
    config: InstanceConfig,
    /// Serializes start/stop; stop is the only release path.
    life: AsyncMutex<()>,
    phase: Mutex<Phase>,
}
enum Phase {
    Idle,
    Active(Active),
    Finished,
}
struct Active {
    client: Client,
    /// Modules in topological (publication) order.
    modules: Vec<LiveModule>,
    handle: tokio::runtime::Handle,
}
struct LiveModule {
    #[allow(dead_code)]
    name: String,
    registered: Option<ModuleRef>,
}

/// Result of [`Instance::stop`] — the terminal state is the same for every
/// call after the first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopOutcome {
    /// This call performed the release sequence.
    Stopped,
    /// Instance was already stopped/failed/never started; no-op.
    AlreadyStopped,
}

impl Instance {
    pub fn new(plugin: Plugin, config: InstanceConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                plugin,
                config,
                life: AsyncMutex::new(()),
                phase: Mutex::new(Phase::Idle),
            }),
        }
    }

    /// Composite validation → connect → start modules in Kahn order →
    /// publish in the same order. Any failure after connect rolls back every
    /// publication and closes the link (zero residue, contract §3).
    pub async fn start(&self) -> Result<(), PluginError> {
        let _life = self.inner.life.lock().await;
        let plugin = &self.inner.plugin;
        let config = &self.inner.config;
        {
            let phase = self.inner.phase.lock().unwrap();
            if !matches!(*phase, Phase::Idle) {
                return Err(PluginError::start(
                    &plugin.id,
                    Error::bad_request("instance already started or finished"),
                ));
            }
        }
        let bindings: BTreeMap<String, RemoteBinding> = config
            .bindings
            .iter()
            .map(|b| (b.dependency.clone(), b.clone()))
            .collect();
        let order = composite(&plugin.modules, &bindings)?;

        let mut connect = ConnectOptions::new(&config.url, &config.bearer);
        connect.allow_loopback_ws = config.allow_loopback_ws;
        let client = Client::connect(connect)
            .await
            .map_err(|source| PluginError::start(&plugin.id, source))?;

        let remotes: Arc<BTreeMap<String, RemoteDep>> = Arc::new(
            bindings
                .iter()
                .map(|(name, b)| {
                    (
                        name.clone(),
                        RemoteDep {
                            client: client.clone(),
                            contract: b.contract.clone(),
                            target: b.target.clone(),
                        },
                    )
                })
                .collect(),
        );

        // Start phase: every module starts before anything is published, so a
        // downstream factory always sees its upstream's service handle.
        let mut started: Vec<(String, StartedModule)> = Vec::with_capacity(order.len());
        let mut locals: Arc<BTreeMap<String, Option<Service>>> = Arc::new(BTreeMap::new());
        for &index in &order {
            let decl = &plugin.modules[index];
            let context = ModuleContext {
                deps: Deps {
                    locals: locals.clone(),
                    remotes: remotes.clone(),
                },
            };
            match (decl.start)(context).await {
                Ok(module) => {
                    locals = Arc::new({
                        let mut next = (*locals).clone();
                        next.insert(decl.name.clone(), module.service.clone());
                        next
                    });
                    started.push((decl.name.clone(), module));
                }
                Err(source) => {
                    client.close().await;
                    self.finish();
                    return Err(PluginError::start(&decl.name, source));
                }
            }
        }

        // Publication phase in the same order; reverse-order rollback keeps
        // the host registry clean when a later publish fails.
        let mut live: Vec<LiveModule> = Vec::with_capacity(started.len());
        for (name, module) in &started {
            let endpoint = format!("{}-{name}", config.endpoint_prefix);
            let exposure = Exposure {
                principals: config.exposure_principals.clone(),
                tenants: config.exposure_tenants.clone(),
                methods: module.contract.methods.keys().cloned().collect(),
                states: module.contract.states.keys().cloned().collect(),
            };
            let input = p::ModuleRegisterInput {
                space_id: config.space_id.clone(),
                integration_id: config.integration_id.clone(),
                provider_endpoint_id: endpoint,
                contract_id: module.contract.id.clone(),
            };
            match client
                .provide(input, module.contract.clone(), exposure)
                .await
            {
                Ok(registered) => live.push(LiveModule {
                    name: name.clone(),
                    registered: Some(ModuleRef::from(&registered)),
                }),
                Err(source) => {
                    rollback(&client, &live).await;
                    client.close().await;
                    self.finish();
                    return Err(PluginError::start(name, source));
                }
            }
        }

        let mut phase = self.inner.phase.lock().unwrap();
        *phase = Phase::Active(Active {
            client,
            modules: live,
            handle: tokio::runtime::Handle::current(),
        });
        Ok(())
    }

    /// Idempotent, concurrency-safe release: reverse topological unprovide →
    /// consumer/observer close → handler abort → link close. Usable on a
    /// broken link, where unprovide errors are absorbed (the host's
    /// generation cleanup already dropped the registrations).
    pub async fn stop(&self) -> StopOutcome {
        let _life = self.inner.life.lock().await;
        let taken = mem::replace(&mut *self.inner.phase.lock().unwrap(), Phase::Finished);
        match taken {
            Phase::Active(active) => {
                release(active).await;
                StopOutcome::Stopped
            }
            other => {
                *self.inner.phase.lock().unwrap() = other;
                StopOutcome::AlreadyStopped
            }
        }
    }

    fn finish(&self) {
        *self.inner.phase.lock().unwrap() = Phase::Finished;
    }
}

impl Drop for Instance {
    /// Fallback walking the same release path as [`Instance::stop`]; reuses
    /// the runtime handle captured at start, never builds a second runtime.
    fn drop(&mut self) {
        let taken = mem::replace(&mut *self.inner.phase.lock().unwrap(), Phase::Finished);
        if let Phase::Active(active) = taken {
            let handle = active.handle.clone();
            // Dropping the JoinHandle detaches the release task; the runtime
            // generation cleanup is the last-resort backstop behind it.
            drop(handle.spawn(release(active)));
        }
    }
}

async fn release(active: Active) {
    rollback(&active.client, &active.modules).await;
    // Closing aborts and awaits every provider/consumer handler task.
    active.client.close().await;
}

/// Reverse-order unprovide; errors absorbed — stop must finish even when the
/// link is already gone.
async fn rollback(client: &Client, modules: &[LiveModule]) {
    for module in modules.iter().rev() {
        if let Some(target) = &module.registered {
            let _ = client.unprovide(target).await;
        }
    }
}

/// Composite-phase validation (before any network frame): duplicate module
/// names, missing local/bound dependencies, Kahn order, cycle path.
fn composite(
    modules: &[ModuleDecl],
    bindings: &BTreeMap<String, RemoteBinding>,
) -> Result<Vec<usize>, PluginError> {
    let mut index = BTreeMap::new();
    for (i, module) in modules.iter().enumerate() {
        if index.insert(module.name.clone(), i).is_some() {
            return Err(PluginError::DuplicateModule {
                module: module.name.clone(),
            });
        }
    }
    for module in modules {
        for dependency in &module.local_deps {
            if !index.contains_key(dependency) {
                return Err(PluginError::MissingDependency {
                    module: module.name.clone(),
                    dependency: dependency.clone(),
                });
            }
        }
        for dependency in &module.remote_deps {
            if !bindings.contains_key(dependency) {
                return Err(PluginError::MissingDependency {
                    module: module.name.clone(),
                    dependency: dependency.clone(),
                });
            }
        }
    }
    let count = modules.len();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut indegree = vec![0usize; count];
    for (i, module) in modules.iter().enumerate() {
        for dependency in &module.local_deps {
            let upstream = index[dependency];
            dependents[upstream].push(i);
            indegree[i] += 1;
        }
    }
    let mut ready: BTreeSet<usize> = (0..count).filter(|&i| indegree[i] == 0).collect();
    let mut order = Vec::with_capacity(count);
    while let Some(&i) = ready.iter().next() {
        ready.remove(&i);
        order.push(i);
        for &downstream in &dependents[i] {
            indegree[downstream] -= 1;
            if indegree[downstream] == 0 {
                ready.insert(downstream);
            }
        }
    }
    if order.len() == count {
        return Ok(order);
    }
    // Every leftover node still has an unresolved dependency; following local
    // dependency edges from any leftover node revisits a node — that segment
    // is the cycle path.
    let remaining: BTreeSet<usize> = order
        .iter()
        .copied()
        .collect::<BTreeSet<usize>>()
        .symmetric_difference(&(0..count).collect::<BTreeSet<usize>>())
        .copied()
        .collect();
    let mut path = vec![*remaining.iter().next().expect("leftover nodes exist")];
    loop {
        let current = *path.last().expect("non-empty");
        let next = modules[current]
            .local_deps
            .iter()
            .map(|dependency| index[dependency])
            .find(|upstream| remaining.contains(upstream))
            .expect("leftover nodes always have a leftover dependency");
        if let Some(position) = path.iter().position(|&node| node == next) {
            path.drain(..position);
            break;
        }
        path.push(next);
    }
    Err(PluginError::CircularDependency {
        cycle: path.into_iter().map(|i| modules[i].name.clone()).collect(),
    })
}

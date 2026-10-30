//! Local business/publication runtime (private-runtime §1; MVP-P2 §5): an
//! [`Instance`] owns local business — modules composed via process-local
//! handles ([`Deps::local`]) or explicit pinned `ModuleRef` bindings
//! ([`Deps::remote`]) — while **publication is a separate lifecycle** owning
//! only the hub link and registrations.
//!
//! Composition happens before any network frame: duplicate module names,
//! missing local/bound dependencies and local cycles are rejected with typed
//! [`PluginError`]s (never wire errors). Modules start in Kahn order of their
//! local dependencies. With no required remote dependency, local startup does
//! not await hub contact: publication connects independently after the local
//! modules are active.
//!
//! A lost link ends only that link's publications and forwarding:
//! [`PublicationState`] moves to `Disconnected`, existing `StartedModule`s and
//! their service handles stay alive, and the background publication driver
//! reconnects with bounded backoff and re-registers the SAME started modules
//! under fresh registration refs. Nothing is replayed; old refs fail
//! explicitly and pinned remote bindings are never transparently rebound.
//!
//! Explicit [`Instance::stop`] / [`Instance::stop_bounded`] and start-failure
//! rollback tear down owned business in reverse topology. They revoke reachable
//! registrations and await [`ModuleCleanup`] within one shared deadline.
//! Cleanup failures remain diagnostics; only owned links are closed, never a
//! borrowed Node publication link or an external business resource.
//!
//! Clone/Drop: cloning an instance is free and NEVER tears anything down.
//! Dropping the last clone is a synchronous fallback only — it cancels the
//! publication driver (which closes its own link in its async context) and
//! releases the business handles; it never fakes async completion.
#![forbid(unsafe_code)]

use crate::{
    Call, Cancellation, Client, ConnectOptions, Contract, Error, Exposure, InvocationContext,
    ModuleRef, Subscription,
    capability::{self, HandlerFuture},
};
use conex_proto as p;
use serde::Serialize;
use serde_json::Value;
use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    mem,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, watch};

/// Service handle a module exposes to same-instance dependents. Shape is owned
/// by the plugin author; the SDK never inspects it (contract §5).
pub type Service = Arc<dyn Any + Send + Sync>;

/// Explicit asynchronous cleanup. The absolute deadline is shared by the whole
/// stop; errors and incomplete cleanup are reported rather than swallowed.
pub type ModuleCleanup = Arc<dyn Fn(tokio::time::Instant) -> HandlerFuture<()> + Send + Sync>;

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
    /// A module start failed; reverse-order cleanup was attempted for earlier
    /// modules. The source preserves the original error and cleanup diagnostics.
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

/// The instance's current publication link, shared by the publication driver
/// and by every [`RemoteDep`]. The slot is empty while the hub is unreachable
/// or the link is down; remote bindings then fail explicitly and are never
/// transparently rebound.
struct LinkSlot {
    current: Mutex<Option<Client>>,
}
impl LinkSlot {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            current: Mutex::new(None),
        })
    }
    fn set(&self, client: Client) {
        *self.current.lock().unwrap() = Some(client);
    }
    fn clear(&self) {
        *self.current.lock().unwrap() = None;
    }
    fn current(&self) -> Option<Client> {
        self.current.lock().unwrap().clone()
    }
}

/// Consumer handle bound to one dependency name's fixed `ModuleRef`. The
/// target is pinned at composition and never re-resolved; calls ride the
/// instance's current link and fail explicitly when that link is absent or the
/// pinned registration is gone (stale/换代).
#[derive(Clone)]
pub struct RemoteDep {
    slot: Arc<LinkSlot>,
    contract: Arc<Contract>,
    pub target: ModuleRef,
}
impl RemoteDep {
    fn link(&self) -> crate::Result<Client> {
        self.slot.current().ok_or_else(|| {
            Error::new(
                p::ErrorCode::Unavailable,
                "no link; pinned remote binding is not transparently rebound",
                "not_started",
                "never",
            )
        })
    }
    pub fn invoke(&self, method: &str, argument: impl Serialize) -> crate::Result<Call> {
        let client = self.link()?;
        client.invoke(self.target.clone(), self.contract.clone(), method, argument)
    }
    pub fn subscribe(&self, state: &str, argument: impl Serialize) -> crate::Result<Subscription> {
        let client = self.link()?;
        client.subscribe(self.target.clone(), self.contract.clone(), state, argument)
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
    /// Application-level alias remap (`alias -> module name`): dependency
    /// lookups by alias resolve the aliased module's local handle or remote
    /// binding, with the original names retained.
    pub fn with_aliases(&self, aliases: &BTreeMap<String, String>) -> Deps {
        let mut locals = BTreeMap::new();
        for (alias, target) in aliases {
            if let Some(entry) = self.locals.get(target) {
                locals.insert(alias.clone(), entry.clone());
            }
        }
        let remotes = self.remotes.clone();
        Deps {
            locals: Arc::new(locals),
            remotes,
        }
    }
}

/// What a module's start factory hands back. The contract is created inside
/// start so handlers can capture resolved dependency handles directly.
#[derive(Clone)]
pub struct StartedModule {
    pub contract: Arc<Contract>,
    /// Handle exposed to same-instance dependents via [`Deps::local`].
    pub service: Option<Service>,
    /// Reverse-order stop hook ([`ModuleCleanup`]); called by explicit stop
    /// and by start-failure rollback, never on a link disconnect.
    pub cleanup: Option<ModuleCleanup>,
}
impl StartedModule {
    pub fn new(contract: Arc<Contract>) -> Self {
        Self {
            contract,
            service: None,
            cleanup: None,
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
    /// Whether this module's contract is published to the hub. Modules with no
    /// exports (`false`) start and serve locally but never register.
    pub publish: bool,
    pub exposure: Option<Exposure>,
    pub start: ModuleStart,
}
impl ModuleDecl {
    #[allow(clippy::too_many_arguments)]
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
            publish: true,
            exposure: None,
            start: Arc::new(start),
        }
    }
    /// Mark the module as publishable (`true`) or local-only (`false`).
    pub fn with_publication(mut self, publish: bool) -> Self {
        self.publish = publish;
        self
    }
    pub fn with_exposure(mut self, exposure: Exposure) -> Self {
        self.exposure = Some(exposure);
        self
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

/// Per-instance link and publication settings.
#[derive(Clone)]
pub struct InstanceConfig {
    /// Hub WSS URL; empty means local-only (publication stays Unpublished).
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

/// Business lifecycle (private-runtime §1), independent of the publication
/// and link lifecycles. A link drop never changes this state; only an explicit
/// stop tears business down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceState {
    Installed,
    Starting,
    Active,
    Stopping,
    Stopped,
    Failed,
}

/// Publication lifecycle (private-runtime §1), independent of business state:
/// Unpublished → Registering → Published (fresh registration refs) →
/// Disconnected → Registering → Published; explicit stop ends in Revoked. A
/// failed publish round unwinds and returns to Unpublished, retrying on the
/// next ready link. Current refs are exposed via [`Instance::module`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationState {
    Unpublished,
    Registering,
    Published,
    Disconnected,
    Revoked,
}

#[derive(Clone)]
pub struct Instance {
    inner: Arc<Inner>,
}
struct Inner {
    plugin: Plugin,
    config: InstanceConfig,
    /// Serializes start/stop.
    life: AsyncMutex<()>,
    phase: Mutex<Phase>,
    /// Shared publication lifecycle state (written by the driver).
    pubstate: Arc<Mutex<PublicationState>>,
    /// Current link registrations, keyed by module name; cleared on link loss
    /// and on stop so old refs are never re-served.
    references: Arc<Mutex<BTreeMap<String, ModuleRef>>>,
    /// Current publication link for remote deps and hub publication.
    slot: Arc<LinkSlot>,
    owner: Mutex<Option<Owner>>,
    stopping: watch::Sender<bool>,
    stop_deadline: watch::Sender<Option<tokio::time::Instant>>,
    stop_report: Mutex<Option<StopOutcome>>,
    cleanup_errors: Mutex<Vec<String>>,
}
enum Phase {
    Installed,
    Starting,
    Active(Active),
    Stopping,
    Stopped,
    Failed,
}
struct Active {
    /// Modules in start (Kahn) order; reverse order is the stop order.
    started: Vec<StartedRecord>,
    locals: Arc<BTreeMap<String, Option<Service>>>,
}
struct StartedRecord {
    name: String,
    contract: Arc<Contract>,
    cleanup: Option<ModuleCleanup>,
}

struct Owner {
    cancel: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

/// Result of [`Instance::stop`] — the terminal state is the same for every
/// call after the first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopOutcome {
    Stopped,
    Failed(Vec<String>),
}

const DEFAULT_STOP_DEADLINE: Duration = Duration::from_secs(30);
const PUBLISH_BACKOFF_MIN: Duration = Duration::from_millis(250);
const PUBLISH_BACKOFF_MAX: Duration = Duration::from_secs(30);

#[derive(Clone)]
struct DriverConfig {
    url: String,
    bearer: String,
    allow_loopback_ws: bool,
    space_id: String,
    integration_id: String,
    endpoint_prefix: String,
}

/// A module scheduled for publication: name, installed contract and the
/// explicit local exposure ceiling.
struct PublishSpec {
    name: String,
    contract: Arc<Contract>,
    exposure: Exposure,
}

impl Instance {
    pub fn new(plugin: Plugin, config: InstanceConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                plugin,
                config,
                life: AsyncMutex::new(()),
                phase: Mutex::new(Phase::Installed),
                pubstate: Arc::new(Mutex::new(PublicationState::Unpublished)),
                references: Arc::new(Mutex::new(BTreeMap::new())),
                slot: LinkSlot::new(),
                owner: Mutex::new(None),
                stopping: watch::channel(false).0,
                stop_deadline: watch::channel(None).0,
                stop_report: Mutex::new(None),
                cleanup_errors: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Start local business in dependency order. Publication is asynchronous;
    /// callers needing a remote reference observe `publication_state`/`module`.
    pub async fn start(&self) -> Result<(), PluginError> {
        let _life = self.inner.life.lock().await;
        let plugin = &self.inner.plugin;
        let config = &self.inner.config;
        {
            let mut phase = self.inner.phase.lock().unwrap();
            if !matches!(*phase, Phase::Installed) || *self.inner.stopping.borrow() {
                return Err(PluginError::start(
                    &plugin.id,
                    Error::bad_request("instance cannot start"),
                ));
            }
            *phase = Phase::Starting;
        }
        let bindings: BTreeMap<_, _> = config
            .bindings
            .iter()
            .map(|binding| (binding.dependency.clone(), binding.clone()))
            .collect();
        let order = match composite(&plugin.modules, &bindings) {
            Ok(order) => order,
            Err(error) => {
                *self.inner.phase.lock().unwrap() = Phase::Failed;
                return Err(error);
            }
        };
        let remotes: Arc<BTreeMap<String, RemoteDep>> = Arc::new(
            bindings
                .iter()
                .map(|(name, binding)| {
                    (
                        name.clone(),
                        RemoteDep {
                            slot: self.inner.slot.clone(),
                            contract: binding.contract.clone(),
                            target: binding.target.clone(),
                        },
                    )
                })
                .collect(),
        );
        let mut first_client = None;
        let mut started: Vec<(String, StartedModule)> = Vec::with_capacity(order.len());
        let mut locals: Arc<BTreeMap<String, Option<Service>>> = Arc::new(BTreeMap::new());
        let mut specs = Vec::new();
        let mut stopping = self.inner.stopping.subscribe();
        for index in order {
            let decl = &plugin.modules[index];
            if !decl.remote_deps.is_empty() && first_client.is_none() {
                let mut options = ConnectOptions::new(&config.url, &config.bearer);
                options.allow_loopback_ws = config.allow_loopback_ws;
                let connected = tokio::select! {
                    biased;
                    _ = wait_cancelled(&mut stopping) => Err(Error::new(p::ErrorCode::Cancelled,"start cancelled","not_started","never")),
                    result = Client::connect(options) => result,
                };
                match connected {
                    Ok(client) => {
                        self.inner.slot.set(client.clone());
                        first_client = Some(client);
                    }
                    Err(error) => {
                        return Err(self
                            .fail_start(&started, first_client.as_ref(), &decl.name, error)
                            .await);
                    }
                }
            }
            let context = ModuleContext {
                deps: Deps {
                    locals: locals.clone(),
                    remotes: remotes.clone(),
                },
            };
            let result = tokio::select! {
                biased;
                _ = wait_cancelled(&mut stopping) => Err(Error::new(p::ErrorCode::Cancelled,"start cancelled","not_started","never")),
                result = (decl.start)(context) => result,
            };
            let module = match result {
                Ok(module) => module,
                Err(error) => {
                    return Err(self
                        .fail_start(&started, first_client.as_ref(), &decl.name, error)
                        .await);
                }
            };
            Arc::make_mut(&mut locals).insert(decl.name.clone(), module.service.clone());
            if decl.publish {
                specs.push(PublishSpec {
                    name: decl.name.clone(),
                    contract: fenced_contract(&module.contract, self.inner.stopping.clone()),
                    exposure: decl
                        .exposure
                        .clone()
                        .unwrap_or_else(|| exposure_for(config, &module.contract)),
                });
            }
            started.push((decl.name.clone(), module));
        }
        let needs_link = !specs.is_empty() || !bindings.is_empty();
        let driver_config = DriverConfig {
            url: if needs_link {
                config.url.clone()
            } else {
                String::new()
            },
            bearer: config.bearer.clone(),
            allow_loopback_ws: config.allow_loopback_ws,
            space_id: config.space_id.clone(),
            integration_id: config.integration_id.clone(),
            endpoint_prefix: config.endpoint_prefix.clone(),
        };
        *self.inner.phase.lock().unwrap() = Phase::Active(Active {
            started: started
                .into_iter()
                .map(|(name, module)| StartedRecord {
                    name,
                    contract: module.contract,
                    cleanup: module.cleanup,
                })
                .collect(),
            locals,
        });
        let (cancel, cancel_rx) = watch::channel(false);
        let task = tokio::spawn(driver(
            driver_config,
            specs,
            self.inner.pubstate.clone(),
            self.inner.references.clone(),
            self.inner.slot.clone(),
            first_client,
            cancel_rx,
        ));
        *self.inner.owner.lock().unwrap() = Some(Owner { cancel, task });
        Ok(())
    }

    async fn fail_start(
        &self,
        started: &[(String, StartedModule)],
        client: Option<&Client>,
        module: &str,
        mut source: Error,
    ) -> PluginError {
        self.set_stop_deadline(DEFAULT_STOP_DEADLINE);
        for (_, module) in started.iter().rev() {
            if let Some(cleanup) = &module.cleanup {
                let cutoff = self.inner.stop_deadline.borrow().unwrap();
                match self.before_deadline(cleanup(cutoff)).await {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => self
                        .inner
                        .cleanup_errors
                        .lock()
                        .unwrap()
                        .push(error.to_string()),
                    Err(()) => self
                        .inner
                        .cleanup_errors
                        .lock()
                        .unwrap()
                        .push("startup cleanup deadline".into()),
                }
            }
        }
        if let Some(client) = client {
            let _ = self.before_deadline(client.close()).await;
        }
        self.inner.slot.clear();
        *self.inner.pubstate.lock().unwrap() = PublicationState::Unpublished;
        *self.inner.phase.lock().unwrap() = Phase::Failed;
        let errors = self.inner.cleanup_errors.lock().unwrap();
        if !errors.is_empty() {
            source
                .0
                .message
                .push_str(&format!("; cleanup incomplete: {}", errors.join("; ")));
        }
        PluginError::start(module, source)
    }

    fn set_stop_deadline(&self, duration: Duration) {
        let requested = tokio::time::Instant::now() + duration.min(DEFAULT_STOP_DEADLINE);
        self.inner.stop_deadline.send_if_modified(|cutoff| {
            if cutoff.is_none_or(|old| requested < old) {
                *cutoff = Some(requested);
                true
            } else {
                false
            }
        });
    }

    async fn before_deadline<F: std::future::Future>(&self, future: F) -> Result<F::Output, ()> {
        let mut deadline = self.inner.stop_deadline.subscribe();
        tokio::pin!(future);
        loop {
            let cutoff = (*deadline.borrow_and_update())
                .unwrap_or_else(|| tokio::time::Instant::now() + DEFAULT_STOP_DEADLINE);
            tokio::select! {
                biased;
                result = &mut future => return Ok(result),
                _ = tokio::time::sleep_until(cutoff) => return Err(()),
                changed = deadline.changed() => if changed.is_err() { return Err(()); },
            }
        }
    }

    /// Idempotent, concurrency-safe release with the default 30s budget:
    /// reverse-topology unregister when the link is reachable, reverse-order
    /// module cleanup hooks, link close. Explicit stop is the only path that
    /// closes owned business.
    pub async fn stop(&self) -> StopOutcome {
        self.stop_bounded(DEFAULT_STOP_DEADLINE).await
    }

    /// Fence new admissions immediately; serialized cleanup still runs in stop.
    /// Used by a node coordinator to shorten a stop while its command is queued.
    pub fn request_stop(&self, duration: Duration) {
        self.set_stop_deadline(duration);
        self.inner.stopping.send_replace(true);
    }

    pub fn effective_stop_deadline(&self) -> Option<tokio::time::Instant> {
        *self.inner.stop_deadline.borrow()
    }

    /// Concurrent callers share one result and may only shorten its deadline.
    pub async fn stop_bounded(&self, duration: Duration) -> StopOutcome {
        self.request_stop(duration);
        let _life = self.inner.life.lock().await;
        if let Some(report) = self.inner.stop_report.lock().unwrap().as_ref() {
            return report.clone();
        }
        let active = match mem::replace(&mut *self.inner.phase.lock().unwrap(), Phase::Stopping) {
            Phase::Active(active) => Some(active),
            _ => None,
        };
        let owner = self.inner.owner.lock().unwrap().take();
        if let Some(owner) = owner {
            owner.cancel.send_replace(true);
            let mut task = owner.task;
            match self.before_deadline(&mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => self
                    .inner
                    .cleanup_errors
                    .lock()
                    .unwrap()
                    .push(format!("publication cleanup: {error}")),
                Err(()) => {
                    task.abort();
                    self.inner
                        .cleanup_errors
                        .lock()
                        .unwrap()
                        .push("publication cleanup deadline".into());
                }
            }
        }
        // An aborted publication task must not leave a live forwarding link.
        if !self.inner.config.url.is_empty()
            && let Some(client) = self.inner.slot.current()
        {
            let _ = self.before_deadline(client.close()).await;
        }
        self.inner.slot.clear();
        *self.inner.pubstate.lock().unwrap() = PublicationState::Revoked;
        self.inner.references.lock().unwrap().clear();
        if let Some(active) = active {
            for record in active.started.iter().rev() {
                if let Some(cleanup) = &record.cleanup {
                    let cutoff = self.inner.stop_deadline.borrow().unwrap();
                    match self.before_deadline(cleanup(cutoff)).await {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => self
                            .inner
                            .cleanup_errors
                            .lock()
                            .unwrap()
                            .push(error.to_string()),
                        Err(()) => self
                            .inner
                            .cleanup_errors
                            .lock()
                            .unwrap()
                            .push(format!("{} cleanup deadline", record.name)),
                    }
                }
            }
        }
        let errors = self.inner.cleanup_errors.lock().unwrap().clone();
        let outcome = if errors.is_empty() {
            StopOutcome::Stopped
        } else {
            StopOutcome::Failed(errors)
        };
        *self.inner.phase.lock().unwrap() = if outcome == StopOutcome::Stopped {
            Phase::Stopped
        } else {
            Phase::Failed
        };
        *self.inner.stop_report.lock().unwrap() = Some(outcome.clone());
        outcome
    }

    pub fn instance_state(&self) -> InstanceState {
        match &*self.inner.phase.lock().unwrap() {
            Phase::Installed => InstanceState::Installed,
            Phase::Starting => InstanceState::Starting,
            Phase::Active(_) => InstanceState::Active,
            Phase::Stopping => InstanceState::Stopping,
            Phase::Stopped => InstanceState::Stopped,
            Phase::Failed => InstanceState::Failed,
        }
    }

    pub fn publication_state(&self) -> PublicationState {
        *self.inner.pubstate.lock().unwrap()
    }

    /// The CURRENT registration of a module on the publication link. `None`
    /// while unpublished/disconnected/revoked — old refs fail explicitly and
    /// are never re-served after a link generation changes.
    pub fn module(&self, name: &str) -> Option<ModuleRef> {
        self.inner.references.lock().unwrap().get(name).cloned()
    }

    /// Borrow a node-owned link for pinned remote dependencies. Local-only
    /// instances never close this link when stopping their own business.
    pub fn attach_link(&self, client: Option<Client>) -> crate::Result<()> {
        if !self.inner.config.url.is_empty() {
            return Err(Error::bad_request("instance owns its publication link"));
        }
        if let Some(client) = client {
            if *self.inner.stopping.borrow() {
                return Err(Error::unavailable());
            }
            self.inner.slot.set(client);
        } else {
            self.inner.slot.clear();
        }
        Ok(())
    }

    /// A revocable capability reference for an external node publisher; the
    /// publisher owns no business resources and admission stays instance-fenced.
    pub fn export_module(&self, name: &str) -> crate::Result<(Arc<Contract>, Exposure)> {
        let phase = self.inner.phase.lock().unwrap();
        let Phase::Active(active) = &*phase else {
            return Err(Error::unavailable());
        };
        if *self.inner.stopping.borrow() {
            return Err(Error::unavailable());
        }
        let record = active
            .started
            .iter()
            .find(|record| record.name == name)
            .ok_or_else(Error::unavailable)?;
        let declaration = self
            .inner
            .plugin
            .modules
            .iter()
            .find(|decl| decl.name == name)
            .ok_or_else(Error::unavailable)?;
        Ok((
            fenced_contract(&record.contract, self.inner.stopping.clone()),
            declaration
                .exposure
                .clone()
                .unwrap_or_else(|| exposure_for(&self.inner.config, &record.contract)),
        ))
    }

    /// In-process service handle of a started module, downcast to the author's
    /// type. `None` while the instance is not active or the module is absent.
    pub fn service<T: Send + Sync + 'static>(&self, module: &str) -> Option<Arc<T>> {
        let phase = self.inner.phase.lock().unwrap();
        let Phase::Active(active) = &*phase else {
            return None;
        };
        active.locals.get(module)?.clone()?.downcast::<T>().ok()
    }

    /// Local invocation through the same domain boundary as hub forwards:
    /// trusted caller identity (built by the node's access layer, never from
    /// wire payloads), contract input validation, the real installed handler,
    /// output validation and uncertain-execution mapping. The DI types are not
    /// a sandbox; authorization happens before this call.
    pub async fn invoke_local(
        &self,
        module: &str,
        method: &str,
        argument: Value,
        principal_id: &str,
    ) -> crate::Result<Value> {
        if *self.inner.stopping.borrow() {
            return Err(Error::new(
                p::ErrorCode::Unavailable,
                "instance stopping",
                "not_started",
                "never",
            ));
        }
        let (handler, execution, output_schema) = {
            let phase = self.inner.phase.lock().unwrap();
            let Phase::Active(active) = &*phase else {
                return Err(Error::new(
                    p::ErrorCode::Unavailable,
                    "instance not active",
                    "not_started",
                    "never",
                ));
            };
            let record = active
                .started
                .iter()
                .find(|record| record.name == module)
                .ok_or_else(|| {
                    Error::new(
                        p::ErrorCode::BadRequest,
                        "module not started",
                        "not_started",
                        "never",
                    )
                })?;
            let descriptor = record.contract.methods.get(method).ok_or_else(|| {
                Error::new(
                    p::ErrorCode::BadRequest,
                    "method not installed",
                    "not_started",
                    "never",
                )
            })?;
            capability::validate(&descriptor.input_schema, &argument)?;
            let handler = descriptor.handler.clone().ok_or_else(|| {
                Error::new(
                    p::ErrorCode::BadRequest,
                    "method has no handler",
                    "not_started",
                    "never",
                )
            })?;
            (
                handler,
                descriptor.execution,
                descriptor.output_schema.clone(),
            )
        };
        let (hub, caller) = self.local_identity(module, principal_id);
        let (_cancel_owner, cancellation) = watch::channel(false);
        let invocation = InvocationContext {
            hub,
            caller,
            request_id: local_request_id(),
            cancellation: Cancellation(cancellation),
        };
        match handler(invocation, argument).await {
            Ok(output) => {
                capability::validate(&output_schema, &output).map_err(|error| {
                    execution.uncertain(p::ErrorCode::Unavailable, error.to_string())
                })?;
                Ok(output)
            }
            Err(mut error) => {
                // Same completion semantics as forwarded hub invocations.
                if error.0.execution.is_empty() {
                    error.0.execution = "completed".into();
                }
                if error.0.retry.is_empty() {
                    error.0.retry = "never".into();
                }
                Err(error)
            }
        }
    }

    /// Synthetic local link identity + caller context for in-process calls.
    /// Never derived from a request body; the principal comes from the node's
    /// restricted local operator authorization.
    fn local_identity(
        &self,
        module: &str,
        principal_id: &str,
    ) -> (p::LinkIdentity, p::RequestContext) {
        let config = &self.inner.config;
        let tenant = config
            .exposure_tenants
            .iter()
            .next()
            .cloned()
            .unwrap_or_default();
        let registration_id = self
            .inner
            .references
            .lock()
            .unwrap()
            .get(module)
            .map(|reference| reference.registration_id.clone())
            .unwrap_or_default();
        let hub = p::LinkIdentity {
            link_id: format!("local-{}", config.endpoint_prefix),
            peer_id: principal_id.to_owned(),
            tenant_id: tenant.clone(),
        };
        let caller = p::RequestContext {
            plane: p::Plane::Broker as i32,
            provider_endpoint_id: format!("{}-{}", config.endpoint_prefix, module),
            registration_id,
            principal_id: principal_id.to_owned(),
            tenant_id: tenant,
            space_id: config.space_id.clone(),
            ..Default::default()
        };
        (hub, caller)
    }
}

/// Sync fallback for the last clone: cancels the publication driver (which
/// closes its own link in its async context) and releases the business
/// handles held here. Nothing is awaited or faked as completed.
impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.get_mut().ok().and_then(Option::take) {
            let _ = owner.cancel.send_replace(true);
            // JoinHandle dropped -> detached; the driver tears down its link.
        }
    }
}

fn exposure_for(config: &InstanceConfig, contract: &Contract) -> Exposure {
    Exposure {
        principals: config.exposure_principals.clone(),
        tenants: config.exposure_tenants.clone(),
        methods: contract.methods.keys().cloned().collect(),
        states: contract.states.keys().cloned().collect(),
    }
}

fn grow_backoff(backoff: Duration) -> Duration {
    if backoff.is_zero() {
        PUBLISH_BACKOFF_MIN
    } else {
        backoff.saturating_mul(2).min(PUBLISH_BACKOFF_MAX)
    }
}

async fn wait_cancelled(cancelled: &mut watch::Receiver<bool>) {
    while !*cancelled.borrow_and_update() {
        if cancelled.changed().await.is_err() {
            return;
        }
    }
}

/// Register every publishable module on `client`; on failure unwind this
/// round's fresh registrations in reverse (mid-round rollback). Returned refs
/// land in the shared references map. `Err` leaves the caller to decide
/// Unpublished (live link) vs Disconnected (link died mid-round).
async fn publish_round(
    client: &Client,
    config: &DriverConfig,
    specs: &[PublishSpec],
    references: &Mutex<BTreeMap<String, ModuleRef>>,
) -> Result<(), ()> {
    let mut fresh: Vec<(String, ModuleRef)> = Vec::new();
    for spec in specs {
        let input = p::ModuleRegisterInput {
            space_id: config.space_id.clone(),
            integration_id: config.integration_id.clone(),
            provider_endpoint_id: format!("{}-{}", config.endpoint_prefix, spec.name),
            contract_id: spec.contract.id.clone(),
        };
        match client
            .provide(input, spec.contract.clone(), spec.exposure.clone())
            .await
        {
            Ok(registered) => {
                let reference = ModuleRef::from(&registered);
                references
                    .lock()
                    .unwrap()
                    .insert(spec.name.clone(), reference.clone());
                fresh.push((spec.name.clone(), reference));
            }
            Err(_) => {
                for (name, reference) in fresh.iter().rev() {
                    references.lock().unwrap().remove(name);
                    let _ = client.unprovide(reference).await;
                }
                return Err(());
            }
        }
    }
    Ok(())
}

/// Background publication coordinator. Owns the hub link after start: waits
/// out a live link, ends only this link's refs/forwarding on loss, reconnects
/// with bounded backoff, republishes the SAME started modules under fresh refs
/// and never replays mutations. Cancellation (explicit stop / last-clone drop)
/// unwinds live registrations and closes the link.
#[allow(clippy::too_many_arguments)]
async fn driver(
    config: DriverConfig,
    specs: Vec<PublishSpec>,
    pubstate: Arc<Mutex<PublicationState>>,
    references: Arc<Mutex<BTreeMap<String, ModuleRef>>>,
    slot: Arc<LinkSlot>,
    first: Option<Client>,
    mut cancelled: watch::Receiver<bool>,
) {
    let mut client = first;
    let mut needs_publish = true;
    let mut backoff = Duration::ZERO;
    loop {
        if *cancelled.borrow() {
            teardown(client.as_ref(), &pubstate, &slot, &references, &specs).await;
            return;
        }
        if let Some(current) = &client {
            if needs_publish {
                *pubstate.lock().unwrap() = PublicationState::Registering;
                let round = tokio::select! {
                    biased;
                    _ = wait_cancelled(&mut cancelled) => continue,
                    round = publish_round(current, &config, &specs, &references) => round,
                };
                if round.is_ok() {
                    *pubstate.lock().unwrap() = PublicationState::Published;
                } else {
                    references.lock().unwrap().clear();
                    *pubstate.lock().unwrap() = PublicationState::Unpublished;
                }
                needs_publish = false;
            }
            tokio::select! {
                biased;
                _ = wait_cancelled(&mut cancelled) => {}
                _ = current.wait_closed() => {
                    references.lock().unwrap().clear();
                    slot.clear();
                    *pubstate.lock().unwrap() = PublicationState::Disconnected;
                    client = None;
                    backoff = PUBLISH_BACKOFF_MIN;
                }
            }
        } else {
            if config.url.is_empty() {
                wait_cancelled(&mut cancelled).await;
                continue;
            }
            tokio::select! {
                biased;
                _ = wait_cancelled(&mut cancelled) => continue,
                _ = tokio::time::sleep(backoff) => {}
            }
            let mut options = ConnectOptions::new(&config.url, &config.bearer);
            options.allow_loopback_ws = config.allow_loopback_ws;
            let connected = tokio::select! {
                biased;
                _ = wait_cancelled(&mut cancelled) => continue,
                result = Client::connect(options) => result,
            };
            match connected {
                Err(_) => {
                    *pubstate.lock().unwrap() = PublicationState::Disconnected;
                    backoff = grow_backoff(backoff);
                }
                Ok(connected) => {
                    slot.set(connected.clone());
                    client = Some(connected);
                    needs_publish = true;
                    backoff = PUBLISH_BACKOFF_MIN;
                }
            }
        }
    }
}

/// Best-effort reverse unregister of live refs, link close, and the terminal
/// Revoked state. Absorbs link errors: the S1 generation cleanup is the
/// backstop behind an already-dead link.
async fn teardown(
    client: Option<&Client>,
    pubstate: &Mutex<PublicationState>,
    slot: &LinkSlot,
    references: &Mutex<BTreeMap<String, ModuleRef>>,
    specs: &[PublishSpec],
) {
    let client = client.cloned();
    if let Some(client) = &client {
        let current: Vec<ModuleRef> = {
            let refs = references.lock().unwrap();
            specs
                .iter()
                .rev()
                .filter_map(|spec| refs.get(&spec.name).cloned())
                .collect()
        };
        for reference in &current {
            let _ = client.unprovide(reference).await;
        }
        client.close().await;
    }
    slot.clear();
    references.lock().unwrap().clear();
    *pubstate.lock().unwrap() = PublicationState::Revoked;
}

fn local_request_id() -> String {
    static NEXT: Mutex<u64> = Mutex::new(0);
    let mut next = NEXT.lock().unwrap();
    *next += 1;
    format!("local-{}-{next}", std::process::id())
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

fn fenced_contract(contract: &Contract, stopping: watch::Sender<bool>) -> Arc<Contract> {
    let mut contract = contract.clone();
    for method in contract.methods.values_mut() {
        if let Some(handler) = method.handler.take() {
            let stopping = stopping.clone();
            method.handler = Some(Arc::new(move |context, argument| -> HandlerFuture<Value> {
                if *stopping.borrow() {
                    Box::pin(async {
                        Err(Error::new(
                            p::ErrorCode::Unavailable,
                            "instance stopping",
                            "not_started",
                            "never",
                        ))
                    })
                } else {
                    handler(context, argument)
                }
            }));
        }
    }
    for state in contract.states.values_mut() {
        if let Some(handler) = state.handler.take() {
            let stopping = stopping.clone();
            state.handler = Some(Arc::new(
                move |context, argument, observer| -> HandlerFuture<crate::StateLease> {
                    if *stopping.borrow() {
                        Box::pin(async {
                            Err(Error::new(
                                p::ErrorCode::Unavailable,
                                "instance stopping",
                                "not_started",
                                "never",
                            ))
                        })
                    } else {
                        handler(context, argument, observer)
                    }
                },
            ));
        }
    }
    Arc::new(contract)
}

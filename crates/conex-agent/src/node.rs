//! Node owns local instances; publication owns only their remote visibility.
use crate::access::TrustedCaller;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch};

pub type NodeResult = Result<Value, conex_sdk::Error>;
type Reply = oneshot::Sender<NodeResult>;

pub fn failure(reason: &str) -> conex_sdk::Error {
    use conex_proto::ErrorCode;
    let code = match reason {
        "forbidden" => ErrorCode::Forbidden,
        "stale_revision" => ErrorCode::StaleRevision,
        "unavailable" => ErrorCode::Unavailable,
        "unsupported_capability" => ErrorCode::UnsupportedCapability,
        "quota_exceeded" => ErrorCode::QuotaExceeded,
        "conflict" => ErrorCode::Conflict,
        _ => ErrorCode::BadRequest,
    };
    conex_sdk::Error::new(code, reason, "not_started", "never")
}
enum Request {
    Snapshot(Reply),
    Start(String, Reply),
    Stop(String, tokio::time::Instant, Reply),
    Invoke(TrustedCaller, String, String, u64, String, Value, Reply),
    Shutdown(tokio::time::Instant, Reply),
}

#[derive(Clone)]
pub struct NodeHandle {
    tx: mpsc::Sender<Request>,
    stop_targets: Arc<std::sync::Mutex<BTreeMap<String, Instance>>>,
}
impl NodeHandle {
    async fn send(&self, make: impl FnOnce(Reply) -> Request) -> NodeResult {
        let (reply, wait) = oneshot::channel();
        self.tx
            .try_send(make(reply))
            .map_err(|_| failure("quota_exceeded"))?;
        wait.await.map_err(|_| failure("unavailable"))?
    }
    pub async fn snapshot(&self) -> NodeResult {
        self.send(Request::Snapshot).await
    }
    pub async fn start_package(&self, app: String) -> NodeResult {
        self.send(|reply| Request::Start(app, reply)).await
    }
    pub async fn stop_instance(&self, id: String, deadline: Duration) -> NodeResult {
        let deadline = tokio::time::Instant::now() + deadline.min(Duration::from_secs(30));
        let (reply, wait) = oneshot::channel();
        self.tx
            .try_send(Request::Stop(id.clone(), deadline, reply))
            .map_err(|_| failure("quota_exceeded"))?;
        if let Some(sdk) = self.stop_targets.lock().unwrap().get(&id) {
            sdk.request_stop(deadline.saturating_duration_since(tokio::time::Instant::now()));
        }
        wait.await.map_err(|_| failure("unavailable"))?
    }
    pub async fn invoke(
        &self,
        caller: TrustedCaller,
        instance: String,
        module: String,
        generation: u64,
        method: String,
        argument: Value,
    ) -> NodeResult {
        self.send(|reply| {
            Request::Invoke(
                caller, instance, module, generation, method, argument, reply,
            )
        })
        .await
    }
    pub async fn stop_all(&self, deadline: Duration) -> NodeResult {
        let deadline = tokio::time::Instant::now() + deadline.min(Duration::from_secs(30));
        let (reply, wait) = oneshot::channel();
        self.tx
            .try_send(Request::Shutdown(deadline, reply))
            .map_err(|_| failure("quota_exceeded"))?;
        for sdk in self.stop_targets.lock().unwrap().values() {
            sdk.request_stop(deadline.saturating_duration_since(tokio::time::Instant::now()));
        }
        wait.await.map_err(|_| failure("unavailable"))?
    }
}

use crate::builtins::CounterService;
use crate::node_config::NodeConfig;
use crate::node_types::{ModuleRuntime, PreparedApplication};
use conex_agent_domain::{ResourceState, Store, new_ulid};
use conex_sdk::{Instance, ModuleContext, ModuleDecl, Plugin, StartedModule};
use serde_json::json;

struct LiveModule {
    name: String,
    module_key: String,
    generation: u64,
    permissions: std::collections::BTreeSet<String>,
    started: Arc<std::sync::Mutex<Option<StartedModule>>>,
    exported: bool,
}
struct LiveInstance {
    id: String,
    app_id: String,
    state: &'static str,
    sdk: Instance,
    modules: Vec<LiveModule>,
    stop_report: Option<Value>,
}

pub struct Node {
    config: NodeConfig,
    store: Store,
    epoch: String,
    packages: BTreeMap<String, PreparedApplication>,
    instances: BTreeMap<String, LiveInstance>,
    start_order: Vec<String>,
    startup_errors: Vec<Value>,
    stop_targets: Arc<std::sync::Mutex<BTreeMap<String, Instance>>>,
    publication: crate::publication::Publication,
    publication_task: Option<tokio::task::JoinHandle<()>>,
    recovery_pending_at_boot: usize,
}

impl Node {
    pub async fn boot(
        config: NodeConfig,
        store: Store,
        applications: Vec<PreparedApplication>,
    ) -> Result<Self, String> {
        let epoch = store
            .begin_node_boot(&config.node_id)
            .await
            .map_err(|error| error.to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "node clock unavailable")?
            .as_millis();
        let now = u64::try_from(now).map_err(|_| "node clock out of range")?;
        let recovery_pending_at_boot = store
            .recover_unfinished(now)
            .await
            .map_err(|error| error.to_string())?
            .len();
        let mut packages = BTreeMap::new();
        for application in applications {
            if packages
                .insert(application.app_id.clone(), application)
                .is_some()
            {
                return Err("duplicate installed application".into());
            }
        }
        let (publication, publication_task) =
            crate::publication::Publication::spawn(config.publication.clone());
        let mut node = Self {
            config,
            store,
            epoch,
            packages,
            instances: BTreeMap::new(),
            start_order: Vec::new(),
            startup_errors: Vec::new(),
            stop_targets: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            publication,
            publication_task: Some(publication_task),
            recovery_pending_at_boot,
        };
        node.reconcile_resources().await?;
        Ok(node)
    }

    pub fn record_startup_failure(&mut self, app: &str, error: &conex_sdk::Error) {
        self.startup_errors
            .push(json!({"appId":app,"code":error.code(),"message":error.0.message}));
    }

    pub fn spawn(
        mut self,
    ) -> (
        NodeHandle,
        watch::Receiver<bool>,
        tokio::task::JoinHandle<()>,
    ) {
        let (tx, mut rx) = mpsc::channel(64);
        let (shutdown, closed) = watch::channel(false);
        let handle = NodeHandle {
            tx,
            stop_targets: self.stop_targets.clone(),
        };
        let task = tokio::spawn(async move {
            while let Some(request) = rx.recv().await {
                match request {
                    Request::Snapshot(reply) => {
                        let _ = reply.send(self.snapshot().await);
                    }
                    Request::Start(app, reply) => {
                        let result = self.start_package(&app).await;
                        let _ = reply.send(result);
                    }
                    Request::Stop(id, deadline, reply) => {
                        let result = self.stop_instance(&id, deadline).await;
                        let _ = reply.send(result);
                    }
                    Request::Invoke(caller, id, module, generation, method, argument, reply) => {
                        let result = self
                            .invoke(&caller, &id, &module, generation, &method, argument)
                            .await;
                        let _ = reply.send(result);
                    }
                    Request::Shutdown(deadline, reply) => {
                        let business = self.stop_all(deadline).await;
                        let publication = self.close_publication(deadline).await;
                        let result = match (business, publication) {
                            (Ok(report), Ok(())) => Ok(report),
                            (Ok(report), Err(error)) => {
                                // A consumed stop deadline can make publication
                                // cleanup fail instantly; the business report is
                                // then the only carrier of per-instance
                                // diagnostics and must not be discarded.
                                let mut message = error.0.message;
                                message.push_str(&format!("; stop_report={report}"));
                                let code = conex_proto::ErrorCode::try_from(error.0.code)
                                    .unwrap_or(conex_proto::ErrorCode::Unspecified);
                                Err(conex_sdk::Error::new(code, &message, "unknown", "never"))
                            }
                            (Err(error), Ok(())) | (Err(error), Err(_)) => Err(error),
                        };
                        let _ = reply.send(result);
                        let _ = shutdown.send(true);
                        return;
                    }
                }
            }
            let _ = self
                .stop_all(tokio::time::Instant::now() + Duration::from_secs(30))
                .await;
            let _ = self
                .close_publication(tokio::time::Instant::now() + Duration::from_secs(30))
                .await;
            let _ = shutdown.send(true);
        });
        (handle, closed, task)
    }

    pub async fn start_package(&mut self, app_id: &str) -> NodeResult {
        if let Some(instance) = self.instances.values().find(|instance| {
            instance.app_id == app_id
                && matches!(instance.state, "starting" | "active" | "stopping")
        }) {
            return Ok(json!({"instanceId":instance.id,"state":instance.state}));
        }
        let application = self
            .packages
            .get(app_id)
            .cloned()
            .ok_or_else(|| failure("bad_request"))?;
        let resources = self
            .store
            .list_unreconciled_resources()
            .await
            .map_err(|error| failure(&error.to_string()))?;
        for module in &application.ordered_modules {
            if resources
                .iter()
                .any(|resource| resource.module_key == module.module_key)
            {
                return Err(failure("conflict"));
            }
        }
        let id = new_ulid();
        let mut declarations = Vec::new();
        let mut modules = Vec::new();
        for module in application.ordered_modules {
            let generation = self
                .store
                .next_module_generation(&module.module_key)
                .await
                .map_err(|error| failure(&error.to_string()))?;
            let runtime = ModuleRuntime {
                store: self.store.clone(),
                data_dir: self.config.data_dir.clone(),
                node_epoch: self.epoch.clone(),
                module_key: module.module_key.clone(),
                generation,
                resource_id: new_ulid(),
            };
            let slot = Arc::new(std::sync::Mutex::new(None));
            let capture = slot.clone();
            let factory = module.factory.clone();
            let aliases = module.dependencies.clone();
            let decl = ModuleDecl::new(
                module.name.clone(),
                module.dependencies.values().cloned().collect(),
                Vec::new(),
                move |context: ModuleContext| {
                    let factory = factory.clone();
                    let runtime = runtime.clone();
                    let capture = capture.clone();
                    let aliases = aliases.clone();
                    Box::pin(async move {
                        let context = ModuleContext {
                            deps: context.deps.with_aliases(&aliases),
                        };
                        let started = factory(runtime, context).await?;
                        *capture.lock().unwrap() = Some(started.clone());
                        Ok(started)
                    })
                },
            )
            .with_publication(false)
            .with_exposure(conex_sdk::Exposure {
                principals: self.config.publication.exposure_principals.clone(),
                tenants: self.config.publication.exposure_tenants.clone(),
                methods: module.exposed_methods,
                states: Default::default(),
            });
            declarations.push(decl);
            modules.push(LiveModule {
                name: module.name,
                module_key: module.module_key,
                generation,
                permissions: module.permissions,
                started: slot,
                exported: module.relay_allowed,
            });
        }
        let remote_bindings = !application.bindings.is_empty();
        let mut config = crate::publication::instance_config(&self.config.publication, app_id);
        config.bindings = application.bindings;
        let sdk = Instance::new(Plugin::new(app_id, declarations), config);
        self.stop_targets
            .lock()
            .unwrap()
            .insert(id.clone(), sdk.clone());
        let result = sdk.start().await;
        let state = if result.is_ok() { "active" } else { "failed" };
        if result.is_ok() {
            let mut exports = Vec::new();
            for module in modules.iter().filter(|module| module.exported) {
                let (contract, exposure) = match sdk.export_module(&module.name) {
                    Ok(export) => export,
                    Err(error) => {
                        self.startup_errors.push(
                            json!({"appId":app_id,"phase":"publication","code":error.code()}),
                        );
                        continue;
                    }
                };
                exports.push(crate::publication::Export {
                    endpoint: format!(
                        "{}-{app_id}-{}",
                        self.config.publication.endpoint_prefix, module.name
                    ),
                    lease_id: id.clone(),
                    contract,
                    exposure,
                });
            }
            self.publication
                .install(id.clone(), sdk.clone(), remote_bindings, exports);
        }
        self.instances.insert(
            id.clone(),
            LiveInstance {
                id: id.clone(),
                app_id: app_id.into(),
                state,
                sdk,
                modules,
                stop_report: None,
            },
        );
        self.start_order.push(id.clone());
        if let Err(error) = result {
            let report = self
                .stop_instance(&id, tokio::time::Instant::now() + Duration::from_secs(30))
                .await;
            let mut source = match error {
                conex_sdk::PluginError::StartFailed { source, .. } => source,
                error => failure(&error.to_string()),
            };
            source.0.message.push_str(&format!("; cleanup={report:?}"));
            return Err(source);
        }
        Ok(json!({"instanceId":id,"state":state}))
    }

    async fn invoke(
        &self,
        caller: &TrustedCaller,
        id: &str,
        name: &str,
        generation: u64,
        method: &str,
        argument: Value,
    ) -> NodeResult {
        let instance = self
            .instances
            .get(id)
            .ok_or_else(|| failure("unavailable"))?;
        let module = instance
            .modules
            .iter()
            .find(|module| module.name == name)
            .ok_or_else(|| failure("forbidden"))?;
        let started = module
            .started
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| failure("unavailable"))?;
        let methods = started
            .contract
            .methods
            .keys()
            .filter(|method| module.permissions.contains(&method.replace('/', ".")))
            .cloned()
            .collect();
        let scope = crate::access::authorize(
            caller,
            crate::access::Target {
                node_id: &self.config.node_id,
                active: instance.state == "active",
                generation: module.generation,
                expected_generation: generation,
                methods: &methods,
            },
            method,
            None,
            0,
        )
        .map_err(|reason| failure(&reason))?;
        instance
            .sdk
            .invoke_local(name, method, argument, &scope.principal_id)
            .await
    }

    async fn stop_instance(&mut self, id: &str, deadline: tokio::time::Instant) -> NodeResult {
        let instance = self
            .instances
            .get_mut(id)
            .ok_or_else(|| failure("bad_request"))?;
        if let Some(report) = &instance.stop_report {
            return Ok(report.clone());
        }
        instance.state = "stopping";
        self.publication.revoke(id);
        let mut diagnostics = Vec::new();
        // Fence SDK routes immediately; owned process cleanup remains Node's
        // responsibility even if publication is down or asynchronous cleanup fails.
        if let conex_sdk::StopOutcome::Failed(errors) = instance
            .sdk
            .stop_bounded(deadline.saturating_duration_since(tokio::time::Instant::now()))
            .await
        {
            diagnostics.extend(errors);
        }
        let deadline = instance
            .sdk
            .effective_stop_deadline()
            .map_or(deadline, |cutoff| cutoff.min(deadline));
        for module in instance.modules.iter().rev() {
            let service = module
                .started
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|started| started.service.clone())
                .and_then(|service| service.downcast::<CounterService>().ok());
            if let Some(service) = service
                && let Err(error) = service.stop(deadline).await
            {
                diagnostics.push(error);
            }
        }
        let resources = match self.store.list_unreconciled_resources().await {
            Ok(resources) => resources,
            Err(error) => {
                diagnostics.push(format!("ownership inventory: {error}"));
                Vec::new()
            }
        };
        for resource in resources.into_iter().filter(|resource| {
            resource.owner_epoch == self.epoch
                && instance.modules.iter().any(|module| {
                    resource.module_key == module.module_key
                        && resource.generation == module.generation
                })
        }) {
            let released = serde_json::from_slice::<crate::process::ControlIdentity>(
                &resource.control_identity,
            )
            .is_ok_and(|identity| crate::process::known_exited(&identity));
            let state = if released {
                ResourceState::Released
            } else {
                diagnostics.push(format!("resource {} remains unknown", resource.id));
                ResourceState::Unknown
            };
            if let Err(error) = self
                .store
                .record_resource_reconciled(&resource.id, state)
                .await
            {
                diagnostics.push(format!("ownership record {}: {error}", resource.id));
            }
        }
        instance.state = if diagnostics.is_empty() {
            "stopped"
        } else {
            "failed"
        };
        let report = json!({"instanceId":id,"state":instance.state,"diagnostics":diagnostics});
        instance.stop_report = Some(report.clone());
        Ok(report)
    }

    async fn stop_all(&mut self, deadline: tokio::time::Instant) -> NodeResult {
        let mut reports = Vec::new();
        for id in self.start_order.clone().into_iter().rev() {
            match self.stop_instance(&id, deadline).await {
                Ok(report) => reports.push(report),
                Err(error) => reports.push(
                    json!({"instanceId":id,"state":"failed","diagnostics":[error.to_string()]}),
                ),
            }
        }
        Ok(json!({"reports":reports}))
    }

    async fn reconcile_resources(&mut self) -> Result<(), String> {
        for resource in self
            .store
            .list_unreconciled_resources()
            .await
            .map_err(|error| error.to_string())?
        {
            let released = if resource.control_kind == "counter-unix" {
                match serde_json::from_slice::<crate::process::ControlIdentity>(
                    &resource.control_identity,
                ) {
                    Ok(identity) => crate::process::reconcile(&identity).await,
                    Err(_) => false,
                }
            } else {
                false
            };
            self.store
                .record_resource_reconciled(
                    &resource.id,
                    if released {
                        ResourceState::Released
                    } else {
                        ResourceState::Unknown
                    },
                )
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    async fn snapshot(&self) -> NodeResult {
        let instances: Vec<_> = self.instances.values().map(|instance| {
            let modules: Vec<_> = instance.modules.iter().map(|module| {
                let process = module.started.lock().unwrap().as_ref().and_then(|started| started.service.clone())
                    .and_then(|service| service.downcast::<CounterService>().ok()).map(|service| service.identity.pid);
                let (publication, reference) = if module.exported && instance.state == "active" {
                    self.publication.module(&format!("{}-{}-{}",self.config.publication.endpoint_prefix,instance.app_id,module.name))
                } else { (if instance.state == "active" { "unpublished" } else { "revoked" }, None) };
                let reference = reference.map(|reference| json!({
                    "spaceId":reference.space_id,"providerEndpointId":reference.provider_endpoint_id,
                    "registrationId":reference.registration_id,"contractId":reference.contract_id,
                }));
                json!({"name":module.name,"moduleKey":module.module_key,"generation":module.generation.to_string(),"publication":publication,"processId":process,"reference":reference})
            }).collect();
            json!({"id":instance.id,"appId":instance.app_id,"state":instance.state,"modules":modules})
        }).collect();
        let resources = self
            .store
            .list_unreconciled_resources()
            .await
            .map_err(|error| failure(&error.to_string()))?;
        let resources: Vec<_> = resources.into_iter().map(|resource| json!({
            "id":resource.id,"moduleKey":resource.module_key,"generation":resource.generation.to_string(),"state":resource.state.as_str(),
        })).collect();
        Ok(
            json!({"nodeId":self.config.node_id,"nodeRuntimeEpoch":self.epoch,"instances":instances,
            "installedApplications":self.packages.keys().collect::<Vec<_>>(),"startupErrors":self.startup_errors,"resources":resources,"publication":self.publication.diagnostics(),"recoveryPendingAtBoot":self.recovery_pending_at_boot}),
        )
    }

    async fn close_publication(
        &mut self,
        deadline: tokio::time::Instant,
    ) -> Result<(), conex_sdk::Error> {
        self.publication.shutdown();
        if let Some(mut task) = self.publication_task.take() {
            match tokio::time::timeout_at(deadline, &mut task).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Err(failure("publication cleanup failed")),
                Err(_) => {
                    task.abort();
                    return Err(conex_sdk::Error::new(
                        conex_proto::ErrorCode::Timeout,
                        "publication cleanup deadline",
                        "unknown",
                        "never",
                    ));
                }
            }
        }
        Ok(())
    }
}

//! One authenticated publication link per node. S1 replaces all registrations
//! when a node opens a new generation, so applications MUST share this link.
//! Business instances and their owned resources never belong to the driver.
use crate::node_config::PublicationConfig;
use conex_sdk::{Client, ConnectOptions, Contract, Exposure, Instance, InstanceConfig, ModuleRef};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

#[derive(Clone)]
pub struct Export {
    pub endpoint: String,
    pub lease_id: String,
    pub contract: Arc<Contract>,
    pub exposure: Exposure,
}
#[derive(Clone)]
struct Application {
    instance: Instance,
    remote_bindings: bool,
    exports: Vec<Arc<Export>>,
}
struct State {
    applications: BTreeMap<String, Application>,
    references: BTreeMap<String, (String, ModuleRef)>,
    client: Option<Client>,
    status: &'static str,
    error: Option<String>,
}
#[derive(Clone)]
pub struct Publication {
    shared: Arc<Mutex<State>>,
    changed: watch::Sender<()>,
    cancel: watch::Sender<bool>,
}

impl Publication {
    pub fn spawn(config: PublicationConfig) -> (Self, tokio::task::JoinHandle<()>) {
        let shared = Arc::new(Mutex::new(State {
            applications: BTreeMap::new(),
            references: BTreeMap::new(),
            client: None,
            status: "unpublished",
            error: None,
        }));
        let (changed, changes) = watch::channel(());
        let (cancel, cancelled) = watch::channel(false);
        let handle = Self {
            shared,
            changed,
            cancel,
        };
        let driver = tokio::spawn(run(config, handle.shared.clone(), changes, cancelled));
        (handle, driver)
    }

    pub fn install(
        &self,
        id: String,
        instance: Instance,
        remote_bindings: bool,
        exports: Vec<Export>,
    ) {
        let mut state = self.shared.lock().unwrap();
        if let Some(client) = &state.client {
            let _ = instance.attach_link(Some(client.clone()));
        }
        state.applications.insert(
            id,
            Application {
                instance,
                remote_bindings,
                exports: exports.into_iter().map(Arc::new).collect(),
            },
        );
        drop(state);
        self.changed.send_replace(());
    }

    pub fn revoke(&self, id: &str) {
        let mut state = self.shared.lock().unwrap();
        if let Some(application) = state.applications.remove(id) {
            let _ = application.instance.attach_link(None);
            for export in application.exports {
                if state
                    .references
                    .get(&export.endpoint)
                    .is_some_and(|(lease, _)| *lease == export.lease_id)
                {
                    state.references.remove(&export.endpoint);
                }
            }
        }
        drop(state);
        self.changed.send_replace(());
    }

    pub fn module(&self, endpoint: &str) -> (&'static str, Option<ModuleRef>) {
        let state = self.shared.lock().unwrap();
        let reference = state
            .references
            .get(endpoint)
            .map(|(_, reference)| reference.clone());
        let status = if reference.is_some() {
            "published"
        } else if state.status == "published" {
            "unpublished"
        } else {
            state.status
        };
        (status, reference)
    }

    pub fn diagnostics(&self) -> Value {
        let state = self.shared.lock().unwrap();
        json!({"state":state.status,"error":state.error})
    }

    pub fn shutdown(&self) {
        self.cancel.send_replace(true);
    }
}

/// A local-only SDK Instance borrows the node's shared link for pinned remote
/// dependencies. It owns neither that link nor its siblings' publications.
pub fn instance_config(config: &PublicationConfig, app_id: &str) -> InstanceConfig {
    InstanceConfig {
        url: String::new(),
        bearer: String::new(),
        allow_loopback_ws: false,
        space_id: config.space_id.clone(),
        integration_id: config.integration_id.clone(),
        endpoint_prefix: format!("{}-{app_id}", config.endpoint_prefix),
        bindings: Vec::new(),
        exposure_principals: config.exposure_principals.clone(),
        exposure_tenants: config.exposure_tenants.clone(),
    }
}

fn desired(shared: &Mutex<State>) -> BTreeMap<String, Arc<Export>> {
    let state = shared.lock().unwrap();
    let mut exports = BTreeMap::new();
    for application in state.applications.values() {
        for export in &application.exports {
            exports.insert(export.endpoint.clone(), export.clone());
        }
    }
    exports
}

fn detach(shared: &Mutex<State>, status: &'static str, error: Option<String>) {
    let mut state = shared.lock().unwrap();
    state.client = None;
    state.references.clear();
    state.status = status;
    state.error = error;
    for application in state.applications.values() {
        let _ = application.instance.attach_link(None);
    }
}

async fn reconcile(
    client: &Client,
    config: &PublicationConfig,
    shared: &Mutex<State>,
    live: &mut BTreeMap<String, (String, ModuleRef)>,
) -> Result<(), String> {
    let wanted = desired(shared);
    let obsolete: Vec<_> = live
        .iter()
        .filter(|(endpoint, (lease, _))| {
            wanted
                .get(*endpoint)
                .is_none_or(|export| export.lease_id != *lease)
        })
        .map(|(endpoint, (_, reference))| (endpoint.clone(), reference.clone()))
        .collect();
    for (endpoint, reference) in obsolete {
        let _ = client.unprovide(&reference).await;
        live.remove(&endpoint);
    }
    for (endpoint, export) in wanted {
        if live
            .get(&endpoint)
            .is_some_and(|(lease, _)| *lease == export.lease_id)
        {
            continue;
        }
        let registered = client
            .provide(
                conex_proto::ModuleRegisterInput {
                    space_id: config.space_id.clone(),
                    integration_id: config.integration_id.clone(),
                    provider_endpoint_id: endpoint.clone(),
                    contract_id: export.contract.id.clone(),
                },
                export.contract.clone(),
                export.exposure.clone(),
            )
            .await
            .map_err(|error| format!("publication rejected: code {}", error.code()))?;
        let reference = ModuleRef::from(&registered);
        // A stop or replacement can happen while registration is in flight.
        // The old reply never installs a reference for the new lease.
        let valid = {
            let mut state = shared.lock().unwrap();
            let valid = state.applications.values().any(|application| {
                application.exports.iter().any(|current| {
                    current.endpoint == endpoint && current.lease_id == export.lease_id
                })
            });
            if valid {
                state.references.insert(
                    endpoint.clone(),
                    (export.lease_id.clone(), reference.clone()),
                );
            }
            valid
        };
        if valid {
            live.insert(endpoint, (export.lease_id.clone(), reference));
        } else {
            let _ = client.unprovide(&reference).await;
        }
    }
    Ok(())
}

async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    while !*cancel.borrow_and_update() {
        if cancel.changed().await.is_err() {
            return;
        }
    }
}

async fn run(
    config: PublicationConfig,
    shared: Arc<Mutex<State>>,
    mut changes: watch::Receiver<()>,
    mut cancel: watch::Receiver<bool>,
) {
    let mut client: Option<Client> = None;
    let mut live = BTreeMap::new();
    let mut dirty = true;
    let mut backoff = Duration::ZERO;
    loop {
        if *cancel.borrow() {
            break;
        }
        let needs_link = shared
            .lock()
            .unwrap()
            .applications
            .values()
            .any(|application| application.remote_bindings || !application.exports.is_empty());
        if !needs_link {
            if let Some(connection) = client.take() {
                connection.close().await;
            }
            live.clear();
            detach(&shared, "unpublished", None);
            tokio::select! {
                biased;
                _ = cancelled(&mut cancel) => break,
                changed = changes.changed() => { if changed.is_err() { break; } dirty = true; }
            }
            continue;
        }
        if client.is_none() {
            tokio::select! {
                biased;
                _ = cancelled(&mut cancel) => break,
                _ = changes.changed() => {},
                _ = tokio::time::sleep(backoff) => {},
            }
            if *cancel.borrow() {
                break;
            }
            let token = match crate::node_config::read_secret(&config.token_backend) {
                Ok(token) => token,
                Err(_) => {
                    detach(
                        &shared,
                        "disconnected",
                        Some("publication credential unavailable".into()),
                    );
                    backoff = if backoff.is_zero() {
                        Duration::from_millis(250)
                    } else {
                        (backoff * 2).min(Duration::from_secs(30))
                    };
                    tokio::select! { _ = cancelled(&mut cancel) => break, _ = tokio::time::sleep(backoff) => {} }
                    continue;
                }
            };
            shared.lock().unwrap().status = "registering";
            let mut options = ConnectOptions::new(&config.url, &token);
            options.allow_loopback_ws = config.allow_loopback_ws;
            let connected = tokio::select! {
                biased;
                _ = cancelled(&mut cancel) => break,
                result = Client::connect(options) => result,
            };
            match connected {
                Err(_) => {
                    detach(
                        &shared,
                        "disconnected",
                        Some("publication link unavailable".into()),
                    );
                    backoff = if backoff.is_zero() {
                        Duration::from_millis(250)
                    } else {
                        (backoff * 2).min(Duration::from_secs(30))
                    };
                    continue;
                }
                Ok(connection) => {
                    let mut state = shared.lock().unwrap();
                    state.client = Some(connection.clone());
                    for application in state.applications.values() {
                        let _ = application.instance.attach_link(Some(connection.clone()));
                    }
                    client = Some(connection);
                    live.clear();
                    dirty = true;
                    backoff = Duration::from_millis(250);
                }
            }
        }
        let connection = client.as_ref().unwrap();
        if dirty {
            shared.lock().unwrap().status = "registering";
            let result = tokio::select! {
                biased;
                _ = cancelled(&mut cancel) => break,
                result = reconcile(connection, &config, &shared, &mut live) => result,
            };
            let retry = {
                let mut state = shared.lock().unwrap();
                state.status = if result.is_ok() {
                    "published"
                } else {
                    "unpublished"
                };
                state.error = result.err();
                state.error.is_some()
            };
            if retry {
                // A live link with a rejected `provide` must keep retrying like
                // the connect path; otherwise the publication path stays
                // silently unregistered until an unrelated change.
                backoff = if backoff.is_zero() {
                    Duration::from_millis(250)
                } else {
                    (backoff * 2).min(Duration::from_secs(30))
                };
                tokio::select! {
                    biased;
                    _ = cancelled(&mut cancel) => break,
                    changed = changes.changed() => { if changed.is_err() { break; } }
                    _ = tokio::time::sleep(backoff) => {},
                }
                if *cancel.borrow() {
                    break;
                }
                continue;
            }
            dirty = false;
            backoff = Duration::from_millis(250);
        }
        tokio::select! {
            biased;
            _ = cancelled(&mut cancel) => break,
            changed = changes.changed() => { if changed.is_err() { break; } dirty = true; }
            _ = connection.wait_closed() => {
                detach(&shared, "disconnected", None);
                client = None;
                live.clear();
            }
        }
    }
    detach(&shared, "revoked", None);
    if let Some(client) = client {
        client.close().await;
    }
}

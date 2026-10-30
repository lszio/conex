//! Approved, compiled-in implementations. No application-supplied executable path.
use crate::process::{self, ControlIdentity, OwnedProcess};
use conex_proto as p;
use conex_sdk::{Contract, Error, Execution, Method};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;

pub struct CounterService {
    pub identity: ControlIdentity,
    owned: Mutex<Option<OwnedProcess>>,
    closed: AtomicBool,
}

impl CounterService {
    pub async fn spawn(socket: PathBuf, token: String) -> Result<Arc<Self>, process::SpawnFailure> {
        let owned = OwnedProcess::spawn(socket, token).await?;
        Ok(Arc::new(Self {
            identity: owned.identity.clone(),
            owned: Mutex::new(Some(owned)),
            closed: AtomicBool::new(false),
        }))
    }

    pub async fn external(socket: PathBuf, token: String) -> Result<Arc<Self>, String> {
        let mut identity = ControlIdentity {
            socket,
            token,
            pid: 0,
            start_ticks: String::new(),
            boot_id: String::new(),
        };
        let reply = process::request(&identity, "inspect", 0).await?;
        identity.pid = reply
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or("external controller lacks process identity")?;
        let (ticks, boot) = process::process_identity(identity.pid)?;
        identity.start_ticks = ticks;
        identity.boot_id = boot;
        Ok(Arc::new(Self {
            identity,
            owned: Mutex::new(None),
            closed: AtomicBool::new(false),
        }))
    }

    async fn call(&self, method: &str, delta: i32) -> conex_sdk::Result<p::CounterResult> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::new(
                p::ErrorCode::Unavailable,
                "local module stopped",
                "not_started",
                "never",
            ));
        }
        let value = process::request(&self.identity, method, delta)
            .await
            .map_err(|_| {
                if method == "add" {
                    Error::new(
                        p::ErrorCode::OutcomeUnknown,
                        "worker outcome unknown",
                        "unknown",
                        "never",
                    )
                } else {
                    Error::new(
                        p::ErrorCode::Unavailable,
                        "worker unavailable",
                        "not_started",
                        "safe",
                    )
                }
            })?;
        let count = value
            .get("count")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok())
            .ok_or_else(|| {
                Error::new(
                    p::ErrorCode::OutcomeUnknown,
                    "invalid worker count",
                    "unknown",
                    "never",
                )
            })?;
        let revision = value
            .get("revision")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| {
                Error::new(
                    p::ErrorCode::OutcomeUnknown,
                    "invalid worker revision",
                    "unknown",
                    "never",
                )
            })?;
        Ok(p::CounterResult { count, revision })
    }
    pub fn contract(self: &Arc<Self>) -> conex_sdk::Result<Arc<Contract>> {
        let read = self.clone();
        let add = self.clone();
        Ok(Arc::new(Contract::new(
            "conex.counter",
            BTreeMap::from([
                (
                    "counter/read".into(),
                    Method::new(
                        "conex.CounterReadInput",
                        "conex.CounterResult",
                        Execution::ReadOnly,
                    )
                    .with_handler(move |_, _: p::CounterReadInput| {
                        let read = read.clone();
                        async move {
                            let result = read.call("inspect", 0).await?;
                            Ok(json!({"count":result.count,"revision":result.revision.to_string()}))
                        }
                    }),
                ),
                (
                    "counter/add".into(),
                    Method::new(
                        "conex.CounterAddInput",
                        "conex.CounterResult",
                        Execution::NonReplayable,
                    )
                    .with_handler(move |_, input: p::CounterAddInput| {
                        let add = add.clone();
                        async move {
                            let result = add.call("add", input.delta).await?;
                            Ok(json!({"count":result.count,"revision":result.revision.to_string()}))
                        }
                    }),
                ),
            ]),
            BTreeMap::new(),
        )?))
    }

    pub async fn stop(&self, deadline: tokio::time::Instant) -> Result<(), String> {
        self.closed.store(true, Ordering::Release);
        let mut owned = self.owned.lock().await;
        if let Some(child) = owned.as_mut() {
            child.stop(deadline).await?;
            *owned = None;
        }
        // External references are disconnected, never sent stop or terminated.
        Ok(())
    }
}

pub fn factory(
    config: crate::node_config::ApprovedCounterConfig,
    external: bool,
) -> crate::node_types::RuntimeFactory {
    Arc::new(move |runtime, _context| {
        let config = config.clone();
        Box::pin(async move {
            let service = if external {
                let socket = config
                    .external_socket
                    .ok_or_else(|| crate::node::failure("invalid approved external config"))?;
                let token =
                    crate::node_config::read_secret(&config.external_token_backend.ok_or_else(
                        || crate::node::failure("missing external credential source"),
                    )?)
                    .map_err(|error| crate::node::failure(&error))?;
                CounterService::external(socket, token)
                    .await
                    .map_err(|error| crate::node::failure(&error))?
            } else {
                use conex_agent_domain::{OwnedResource, ResourceState};
                let socket = runtime
                    .data_dir
                    .join(format!("w-{}.sock", runtime.resource_id));
                let token =
                    process::random_token().map_err(|error| crate::node::failure(&error))?;
                let reservation = ControlIdentity {
                    socket: socket.clone(),
                    token: token.clone(),
                    pid: 0,
                    start_ticks: String::new(),
                    boot_id: String::new(),
                };
                runtime
                    .store
                    .record_owned_resource(OwnedResource {
                        id: runtime.resource_id.clone(),
                        module_key: runtime.module_key.clone(),
                        generation: runtime.generation,
                        owner_epoch: runtime.node_epoch.clone(),
                        control_kind: "counter-unix".into(),
                        control_identity: serde_json::to_vec(&reservation)
                            .map_err(|error| crate::node::failure(&error.to_string()))?,
                        state: ResourceState::Created,
                    })
                    .await
                    .map_err(|error| crate::node::failure(&error.to_string()))?;
                let service = match CounterService::spawn(socket, token).await {
                    Ok(service) => service,
                    Err(error) => {
                        let state = if error.confirmed_exit {
                            ResourceState::Released
                        } else {
                            ResourceState::Unknown
                        };
                        runtime
                            .store
                            .record_resource_reconciled(&runtime.resource_id, state)
                            .await
                            .map_err(|record_error| {
                                crate::node::failure(&format!(
                                    "{}; ownership: {record_error}",
                                    error.message
                                ))
                            })?;
                        return Err(crate::node::failure(&error.message));
                    }
                };
                if let Err(error) = runtime
                    .store
                    .record_resource_ready(
                        &runtime.resource_id,
                        &runtime.node_epoch,
                        serde_json::to_vec(&service.identity)
                            .map_err(|error| crate::node::failure(&error.to_string()))?,
                    )
                    .await
                {
                    let diagnostic = match service.stop(tokio::time::Instant::now()).await {
                        Ok(()) => error.to_string(),
                        Err(cleanup) => format!("{error}; cleanup: {cleanup}"),
                    };
                    return Err(crate::node::failure(&diagnostic));
                }
                service
            };
            let mut started = conex_sdk::StartedModule::new(service.contract()?);
            started.service = Some(service.clone());
            started.cleanup = Some(Arc::new(move |deadline| {
                let service = service.clone();
                Box::pin(async move {
                    service
                        .stop(deadline)
                        .await
                        .map_err(|error| crate::node::failure(&error))
                })
            }));
            Ok(started)
        })
    })
}

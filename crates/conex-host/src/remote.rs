//! Remote source routes backed by authenticated reverse-connected agents.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::ws::Message;
use conex_core::{
    CallContext, CallError, CallResult, ExecutionIo, Handler, Installation, Route,
};
use conex_proto;
use conex_proto::wire::encode_wire;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::Instant;

use crate::agent::AgentError;
use crate::ws_transport::{PendingRequests, enqueue_message};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct RemoteLink {
    pub generation: u64,
    tx: mpsc::Sender<(Message, usize)>,
    queued_bytes: Arc<AtomicUsize>,
    pending: Arc<PendingRequests>,
    max_queued: usize,
    ready: Arc<AtomicBool>,
    close: Option<UnboundedSender<()>>,
}


impl RemoteLink {
    pub fn new(
        generation: u64,
        tx: mpsc::Sender<(Message, usize)>,
        queued_bytes: Arc<AtomicUsize>,
        pending: Arc<PendingRequests>,
        max_queued: usize,
    ) -> Self {
        Self::with_close(generation, tx, queued_bytes, pending, max_queued, None)
    }

    pub fn with_close(
        generation: u64,
        tx: mpsc::Sender<(Message, usize)>,
        queued_bytes: Arc<AtomicUsize>,
        pending: Arc<PendingRequests>,
        max_queued: usize,
        close: Option<UnboundedSender<()>>,
    ) -> Self {
        Self {
            generation,
            tx,
            queued_bytes,
            pending,
            max_queued,
            ready: Arc::new(AtomicBool::new(false)),
            close,
        }
    }

    fn close(&self) {
        if let Some(close) = &self.close {
            let _ = close.send(());
        }
    }

    pub fn activate(&self) {
        self.ready.store(true, Ordering::Release);
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }

    async fn request(
        &self,
        endpoint_id: &str,
        method: &str,
        input: Value,
        deadline: Instant,
    ) -> CallResult<Value> {
        if !self.is_ready() {
            return Err(unavailable("remote agent is offline"));
        }
        if Instant::now() >= deadline {
            return Err(timeout());
        }
        let request_id = format!(
            "01ARZ3NDEKTSV4RR{:010X}",
            NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
        );
        let receiver = self.pending.register(&request_id).await?;
        let message = conex_proto::Message {
            body: Some(conex_proto::message::Body::Request(conex_proto::Request {
                request_id: request_id.clone(),
                method: method.to_owned(),
                params: Some(conex_proto::CallParams {
                    context: Some(conex_proto::RequestContext {
                        provider_endpoint_id: endpoint_id.to_owned(),
                        plane: conex_proto::Plane::Broker as i32,
                        binding_id: None,
                    }),
                    timeout_budget_ms: deadline
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                        .min(u128::from(u32::MAX)) as u32,
                    input: Some(
                        serde_json::from_value(input)
                            .map_err(|_| bad("remote input cannot be encoded"))?,
                    ),
                }),
            })),
        };
        let bytes = match encode_wire(&message) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.pending.cancel(self.generation, &request_id).await;
                return Err(CallError::new(conex_proto::ErrorCode::Internal, error.message));
            }
        };
        let text = String::from_utf8(bytes).map_err(|error| {
            CallError::new(conex_proto::ErrorCode::Internal, format!("remote envelope is not UTF-8: {error}"))
        })?;
        if !enqueue_message(
            &self.tx,
            &self.queued_bytes,
            self.max_queued,
            Message::Text(text.into()),
        ) {
            self.pending.cancel(self.generation, &request_id).await;
            return Err(CallError::new(conex_proto::ErrorCode::QuotaExceeded, "remote link queue is full"));
        }
        match tokio::time::timeout_at(deadline, receiver).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(unavailable("remote agent link closed")),
            Err(_) => {
                self.pending.cancel(self.generation, &request_id).await;
                Err(timeout())
            }
        }
    }
}

#[derive(Default)]
pub struct RemoteConnections {
    links: Mutex<HashMap<String, RemoteLink>>,
}

impl RemoteConnections {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn prepare(&self, agent_id: &str, link: RemoteLink) {
        let old = self.links.lock().await.insert(agent_id.to_owned(), link);
        if let Some(old) = old {
            old.close();
            old.pending.fail_all().await;
        }
    }

    pub async fn activate(&self, agent_id: &str, generation: u64) -> Result<(), AgentError> {
        let links = self.links.lock().await;
        let link = links.get(agent_id).ok_or(AgentError::Unknown)?;
        if link.generation != generation {
            return Err(AgentError::StaleGeneration);
        }
        link.activate();
        Ok(())
    }
    pub async fn is_ready(&self, agent_id: &str) -> bool {
        self.links
            .lock()
            .await
            .get(agent_id)
            .is_some_and(RemoteLink::is_ready)
    }


    pub async fn remove(&self, agent_id: &str, generation: u64) {
        let old = {
            let mut links = self.links.lock().await;
            if links
                .get(agent_id)
                .is_some_and(|link| link.generation == generation)
            {
                links.remove(agent_id)
            } else {
                None
            }
        };
        if let Some(link) = old {
            link.pending.fail_all().await;
        }
    }

    pub async fn request(
        &self,
        agent_id: &str,
        endpoint_id: &str,
        method: &str,
        input: Value,
        deadline: Instant,
    ) -> CallResult<Value> {
        let link = self
            .links
            .lock()
            .await
            .get(agent_id)
            .cloned()
            .ok_or_else(|| unavailable("remote agent is offline"))?;
        link.request(endpoint_id, method, input, deadline).await
    }
}

pub struct RemoteHandler {
    pub agent_id: String,
    pub endpoint_id: String,
    pub root: String,
    pub connections: Arc<RemoteConnections>,
}

#[async_trait]
impl Handler for RemoteHandler {
    async fn execute(
        &self,
        ctx: &CallContext,
        input: Value,
        io: ExecutionIo,
    ) -> CallResult<Value> {
        if io.connection.is_some() || io.secret.is_some() {
            return Err(CallError::new(conex_proto::ErrorCode::Internal, "remote source received unexpected outbound HTTP state"));
        }
        if !conex_source::within(&self.root, &ctx.claim.resource_id) {
            return Err(CallError::new(conex_proto::ErrorCode::Forbidden, "resource is outside remote endpoint root"));
        }
        self.connections
            .request(
                &self.agent_id,
                &self.endpoint_id,
                &ctx.method,
                input,
                ctx.deadline,
            )
            .await
    }
}

pub fn routes(
    installation: &Installation,
    agent_id: &str,
    root: &str,
    connections: Arc<RemoteConnections>,
) -> CallResult<Vec<Route>> {
    let mut routes = Vec::new();
    for (method, contract) in conex_source::contracts() {
        if !installation.endpoint.provides.iter().any(|provided| provided == method) {
            continue;
        }
        routes.push(Route {
            protocol: installation.factory.protocol.clone(),
            version: installation.factory.version,
            endpoint: installation.endpoint.clone(),
            method: method.to_owned(),
            contract,
            handler: Arc::new(RemoteHandler {
                agent_id: agent_id.to_owned(),
                endpoint_id: installation.endpoint.id.clone(),
                root: root.to_owned(),
                connections: connections.clone(),
            }),
            target: None,
            credential: None,
        });
    }
    if routes.is_empty() {
        return Err(bad("source-remote endpoint advertises no supported source method"));
    }
    Ok(routes)
}

fn bad(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::BadRequest, message)
}

fn unavailable(message: impl Into<String>) -> CallError {
    CallError::new(conex_proto::ErrorCode::Unavailable, message)
}

fn timeout() -> CallError {
    CallError::new(conex_proto::ErrorCode::Timeout, "remote agent exceeded the deadline")
}

//! Remote source routes backed by authenticated reverse-connected agents.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::extract::ws::Message;
use conex_core::{CallContext, CallError, CallResult, ExecutionIo, Handler, Installation, Route};
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
        caller: &conex_core::Caller,
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
                        // Authenticated identity forwarded so the agent can
                        // bind snapshot cursors per principal (plan M2).
                        principal_id: caller.principal_id.clone(),
                        tenant_id: caller.tenant_id.clone(),
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
        let bytes = {
            use prost::Message as _;
            message.encode_to_vec()
        };
        if !enqueue_message(
            &self.tx,
            &self.queued_bytes,
            self.max_queued,
            Message::Binary(bytes.into()),
        ) {
            self.pending.cancel(self.generation, &request_id).await;
            return Err(CallError::new(
                conex_proto::ErrorCode::QuotaExceeded,
                "remote link queue is full",
            ));
        }
        match tokio::time::timeout_at(deadline, receiver).await {
            Ok(Ok(Ok(crate::ws_transport::AgentReply::Value(value)))) => Ok(value),
            Ok(Ok(Ok(crate::ws_transport::AgentReply::Chunk(_)))) => Err(CallError::new(
                conex_proto::ErrorCode::Internal,
                "unexpected binary reply on the json plane",
            )),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err(unavailable("remote agent link closed")),
            Err(_) => {
                self.pending.cancel(self.generation, &request_id).await;
                Err(timeout())
            }
        }
    }

    /// M3: fetch one bounded file slice from the agent over the protobuf
    /// binary profile. `input` is the canonical blob/get remote JSON (no
    /// bytes on the control plane); the reply carries raw chunk bytes.
    pub async fn request_chunk(
        &self,
        endpoint_id: &str,
        caller: &conex_core::Caller,
        input: Value,
        deadline: Instant,
    ) -> CallResult<conex_proto::DataChunk> {
        use prost::Message as _;
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
                method: "blob/get".to_owned(),
                params: Some(conex_proto::CallParams {
                    context: Some(conex_proto::RequestContext {
                        provider_endpoint_id: endpoint_id.to_owned(),
                        plane: conex_proto::Plane::Broker as i32,
                        binding_id: None,
                        principal_id: caller.principal_id.clone(),
                        tenant_id: caller.tenant_id.clone(),
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
        let bytes = message.encode_to_vec();
        if !enqueue_message(
            &self.tx,
            &self.queued_bytes,
            self.max_queued,
            Message::Binary(bytes.into()),
        ) {
            self.pending.cancel(self.generation, &request_id).await;
            return Err(CallError::new(
                conex_proto::ErrorCode::QuotaExceeded,
                "remote link queue is full",
            ));
        }
        match tokio::time::timeout_at(deadline, receiver).await {
            Ok(Ok(Ok(crate::ws_transport::AgentReply::Chunk(chunk)))) => Ok(chunk),
            Ok(Ok(Ok(crate::ws_transport::AgentReply::Value(_)))) => Err(CallError::new(
                conex_proto::ErrorCode::Internal,
                "expected binary chunk reply on the protobuf link",
            )),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err(unavailable("remote agent link closed")),
            Err(_) => {
                self.pending.cancel(self.generation, &request_id).await;
                Err(timeout())
            }
        }
    }
}

#[derive(Default)]
struct ConnectionTable {
    links: HashMap<String, RemoteLink>,
    /// Upgraded sockets awaiting successful registration (plan M2): they are
    /// NOT serving until `activate` swaps them in, so a failed registration
    /// never disturbs the healthy current link.
    staged: HashMap<(String, u64), RemoteLink>,
    /// Endpoints accepted in the current generation per agent; drives the
    /// per-endpoint ready projection.
    accepted: HashMap<String, std::collections::HashSet<String>>,
}

#[derive(Default)]
pub struct RemoteConnections {
    state: Mutex<ConnectionTable>,
}

impl RemoteConnections {
    pub fn new() -> Self {
        Self::default()
    }

    /// Park an upgraded link until its registration completes.
    pub async fn stage(&self, agent_id: &str, generation: u64, link: RemoteLink) {
        self.state
            .lock()
            .await
            .staged
            .insert((agent_id.to_owned(), generation), link);
    }

    /// Swap the staged link in as the current one after successful
    /// registration; the replaced link is closed and its pending requests
    /// failed. Only now does the old connection die (plan M2).
    pub async fn activate(
        &self,
        agent_id: &str,
        generation: u64,
        accepted_endpoints: &[String],
    ) -> Result<(), AgentError> {
        let mut state = self.state.lock().await;
        let link = state
            .staged
            .remove(&(agent_id.to_owned(), generation))
            .ok_or(AgentError::Unknown)?;
        if link.generation != generation {
            return Err(AgentError::StaleGeneration);
        }
        if let Some(old) = state.links.insert(agent_id.to_owned(), link) {
            old.close();
            old.pending.fail_all().await;
        }
        state.accepted.insert(
            agent_id.to_owned(),
            accepted_endpoints.iter().cloned().collect(),
        );
        let link = state.links.get(agent_id).expect("just inserted");
        link.activate();
        Ok(())
    }

    /// Drop a staged link whose registration failed; the current link stays
    /// untouched (plan M2 "失败注册不踢掉健康 Agent").
    pub async fn discard(&self, agent_id: &str, generation: u64) {
        let link = self
            .state
            .lock()
            .await
            .staged
            .remove(&(agent_id.to_owned(), generation));
        if let Some(link) = link {
            link.pending.fail_all().await;
        }
    }

    pub async fn is_ready(&self, agent_id: &str) -> bool {
        self.state
            .lock()
            .await
            .links
            .get(agent_id)
            .is_some_and(RemoteLink::is_ready)
    }

    /// Ready means: the agent's current link is active AND this endpoint was
    /// accepted in the current generation (plan M2).
    pub async fn is_ready_endpoint(&self, agent_id: &str, endpoint_id: &str) -> bool {
        let state = self.state.lock().await;
        state.links.get(agent_id).is_some_and(RemoteLink::is_ready)
            && state
                .accepted
                .get(agent_id)
                .is_some_and(|endpoints| endpoints.contains(endpoint_id))
    }

    pub async fn remove(&self, agent_id: &str, generation: u64) {
        let current = {
            let mut state = self.state.lock().await;
            state.accepted.remove(agent_id);
            let staged = state.staged.remove(&(agent_id.to_owned(), generation));
            let current = if state
                .links
                .get(agent_id)
                .is_some_and(|link| link.generation == generation)
            {
                state.links.remove(agent_id)
            } else {
                None
            };
            current.or(staged)
        };
        if let Some(link) = current {
            link.pending.fail_all().await;
        }
    }

    pub async fn request(
        &self,
        agent_id: &str,
        endpoint_id: &str,
        caller: &conex_core::Caller,
        method: &str,
        input: Value,
        deadline: Instant,
    ) -> CallResult<Value> {
        let link = self
            .state
            .lock()
            .await
            .links
            .get(agent_id)
            .cloned()
            .ok_or_else(|| unavailable("remote agent is offline"))?;
        link.request(endpoint_id, caller, method, input, deadline)
            .await
    }

    /// M3: bounded slice fetch; the endpoint must be accepted on the current
    /// generation (per-endpoint ready, plan M2) for the read to be served.
    pub async fn request_chunk(
        &self,
        agent_id: &str,
        endpoint_id: &str,
        caller: &conex_core::Caller,
        input: Value,
        deadline: Instant,
    ) -> CallResult<conex_proto::DataChunk> {
        let link = self
            .state
            .lock()
            .await
            .links
            .get(agent_id)
            .cloned()
            .ok_or_else(|| unavailable("remote agent is offline"))?;
        link.request_chunk(endpoint_id, caller, input, deadline)
            .await
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
    async fn execute(&self, ctx: &CallContext, input: Value, io: ExecutionIo) -> CallResult<Value> {
        if io.connection.is_some() || io.secret.is_some() {
            return Err(CallError::new(
                conex_proto::ErrorCode::Internal,
                "remote source received unexpected outbound HTTP state",
            ));
        }
        if !conex_source::within(&self.root, &ctx.claim.resource_id) {
            return Err(CallError::new(
                conex_proto::ErrorCode::Forbidden,
                "resource is outside remote endpoint root",
            ));
        }
        self.connections
            .request(
                &self.agent_id,
                &self.endpoint_id,
                &ctx.caller,
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
        if !installation
            .endpoint
            .provides
            .iter()
            .any(|provided| provided == method)
        {
            continue;
        }
        routes.push(Route {
            range_reader: None,
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
        return Err(bad(
            "source-remote endpoint advertises no supported source method",
        ));
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
    CallError::new(
        conex_proto::ErrorCode::Timeout,
        "remote agent exceeded the deadline",
    )
}

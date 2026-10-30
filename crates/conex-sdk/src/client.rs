use crate::{
    Error, Result,
    capability::{
        self, Cancellation, Contract, Execution, Exposure, InvocationContext, ModuleRef, Published,
    },
    state::{ConsumerState, Sequence, StateEvent, StateLease, StateObserver, Subscription},
};
use conex_proto::{self as p, capability as wire, message::Body};
use futures::{SinkExt, StreamExt};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot, watch},
    task::JoinSet,
    time::Instant,
};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{
        Message as WsMessage,
        client::IntoClientRequest,
        http::{HeaderValue, Uri},
        protocol::WebSocketConfig,
    },
};

const PROFILE: &str = "conex-jsonrpc2-wss";
const REQUESTS: [&str; 7] = [
    "module/register",
    "module/unregister",
    "module/list",
    "module/invoke",
    "module/cancel",
    "state/subscribe",
    "state/unsubscribe",
];
const PROVIDES: [&str; 4] = [
    "module/invoke",
    "module/cancel",
    "state/subscribe",
    "state/unsubscribe",
];

pub struct ConnectOptions {
    pub url: String,
    pub bearer: String,
    pub limits: p::Limits,
    pub allow_loopback_ws: bool,
}
impl ConnectOptions {
    pub fn new(url: impl Into<String>, bearer: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            bearer: bearer.into(),
            limits: p::Limits {
                max_frame_bytes: 262144,
                max_inflight: 32,
                max_queued_bytes: 1048576,
                timeout_ms: 8000,
                max_subscriptions: 64,
                max_modules: 64,
            },
            allow_loopback_ws: false,
        }
    }
}
fn valid_limits(l: &p::Limits) -> Result<()> {
    wire::validate_limits(l).map_err(Error::protocol)?;
    if l.max_frame_bytes == 0
        || l.max_inflight < 2
        || l.timeout_ms == 0
        || u64::from(l.max_queued_bytes) < u64::from(l.max_frame_bytes) * 2
    {
        return Err(Error::bad_request(
            "invalid six-field S1 limits or missing control reserve",
        ));
    }
    Ok(())
}
fn lower(a: p::Limits, b: p::Limits) -> p::Limits {
    p::Limits {
        max_frame_bytes: a.max_frame_bytes.min(b.max_frame_bytes),
        max_inflight: a.max_inflight.min(b.max_inflight),
        max_queued_bytes: a.max_queued_bytes.min(b.max_queued_bytes),
        timeout_ms: a.timeout_ms.min(b.timeout_ms),
        max_subscriptions: a.max_subscriptions.min(b.max_subscriptions),
        max_modules: a.max_modules.min(b.max_modules),
    }
}
fn typed<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(Error::decode)
}
fn json_value(value: impl Serialize) -> Result<Value> {
    serde_json::to_value(value).map_err(Error::decode)
}

pub(crate) struct Queued<T> {
    pub value: T,
    _total: OwnedSemaphorePermit,
    _data: Option<OwnedSemaphorePermit>,
}
pub(crate) struct Budget {
    total: Arc<Semaphore>,
    data: Arc<Semaphore>,
    frame: usize,
}
impl Budget {
    fn new(l: &p::Limits) -> Arc<Self> {
        Arc::new(Self {
            total: Arc::new(Semaphore::new(l.max_queued_bytes as usize)),
            data: Arc::new(Semaphore::new(
                (l.max_queued_bytes - l.max_frame_bytes) as usize,
            )),
            frame: l.max_frame_bytes as usize,
        })
    }
    pub(crate) fn reserve<T>(&self, value: T, bytes: usize, control: bool) -> Result<Queued<T>> {
        if bytes > self.frame {
            return Err(Error::new(
                p::ErrorCode::PayloadTooLarge,
                "frame limit",
                "not_started",
                "never",
            ));
        }
        let bytes = u32::try_from(bytes.max(1)).map_err(Error::decode)?;
        let data = if control {
            None
        } else {
            Some(
                self.data
                    .clone()
                    .try_acquire_many_owned(bytes)
                    .map_err(|_| Error::quota())?,
            )
        };
        let total = self
            .total
            .clone()
            .try_acquire_many_owned(bytes)
            .map_err(|_| Error::quota())?;
        Ok(Queued {
            value,
            _total: total,
            _data: data,
        })
    }
}
struct Frame {
    bytes: Vec<u8>,
    request: Option<String>,
    fence: Option<Arc<AtomicBool>>,
}
struct Pending {
    method: &'static str,
    kind: PendingKind,
    context: p::RequestContext,
    deadline: Instant,
    sent: bool,
    tx: oneshot::Sender<Result<Value>>,
    _slot: OwnedSemaphorePermit,
}
enum PendingKind {
    Rpc,
    Invoke {
        execution: Execution,
        output: String,
    },
    Register {
        input: p::ModuleRegisterInput,
        contract: Arc<Contract>,
        exposure: Exposure,
    },
    Subscribe(String),
}
struct Running {
    context: p::RequestContext,
    cancel: watch::Sender<bool>,
    abort: tokio::task::AbortHandle,
}
struct ProviderState {
    context: p::RequestContext,
    cancel: watch::Sender<bool>,
    abort: tokio::task::AbortHandle,
    stopped: Arc<AtomicBool>,
    opening: Option<String>,
}
#[derive(Default)]
struct Tables {
    pending: HashMap<String, Pending>,
    published: HashMap<String, Published>,
    running: HashMap<String, Running>,
    providers: HashMap<String, ProviderState>,
    consumers: HashMap<String, ConsumerState>,
}
pub(crate) struct Core {
    limits: p::Limits,
    hub: p::LinkIdentity,
    budget: Arc<Budget>,
    tables: Mutex<Tables>,
    data: mpsc::Sender<Queued<Frame>>,
    control: mpsc::Sender<Queued<Frame>>,
    slots: Arc<Semaphore>,
    control_slot: Arc<Semaphore>,
    closed: watch::Sender<bool>,
    finished: watch::Sender<bool>,
}
struct Life(Arc<Core>);
impl Drop for Life {
    fn drop(&mut self) {
        self.0.shutdown();
    }
}
#[derive(Clone)]
pub struct Client {
    life: Arc<Life>,
}

/// A cancellation acceptance request and the original result have independent lifetimes.
#[derive(Clone)]
pub struct CallControl {
    pub request_id: String,
    core: Weak<Core>,
    target: ModuleRef,
}
impl CallControl {
    pub async fn cancel(&self) -> Result<p::ModuleCancelResult> {
        let core = self.core.upgrade().ok_or_else(Error::unavailable)?;
        {
            let mut tables = core.tables.lock().unwrap();
            let Some(pending) = tables.pending.get(&self.request_id) else {
                return Ok(p::ModuleCancelResult { requested: false });
            };
            if !pending.sent {
                let pending = tables.pending.remove(&self.request_id).expect("present");
                let _ = pending.tx.send(Err(Error::new(
                    p::ErrorCode::Cancelled,
                    "cancelled before transmission",
                    "not_started",
                    "never",
                )));
                return Ok(p::ModuleCancelResult { requested: true });
            }
        }
        let input = json_value(p::ModuleCancelInput {
            request_id: self.request_id.clone(),
        })?;
        let call = core.request(
            "module/cancel",
            self.target.context(),
            input,
            PendingKind::Rpc,
            true,
        )?;
        call.result().await
    }
}
pub struct Call {
    pub control: CallControl,
    rx: oneshot::Receiver<Result<Value>>,
}
impl Call {
    pub async fn result<T: DeserializeOwned>(self) -> Result<T> {
        let value = self.rx.await.map_err(|_| Error::unavailable())??;
        typed(value)
    }
}

impl Client {
    pub async fn connect(options: ConnectOptions) -> Result<Self> {
        valid_limits(&options.limits)?;
        let uri: Uri = options.url.parse().map_err(Error::decode)?;
        let host = uri.host().unwrap_or("").trim_matches(['[', ']']);
        let loopback =
            host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback());
        if uri.query().is_some()
            || options.bearer.is_empty()
            || !(uri.scheme_str() == Some("wss")
                || (uri.scheme_str() == Some("ws") && options.allow_loopback_ws && loopback))
        {
            return Err(Error::bad_request(
                "WSS and bearer header required; plaintext only on explicitly enabled loopback; URL query forbidden",
            ));
        }
        let mut request = options.url.into_client_request().map_err(Error::decode)?;
        request.headers_mut().insert(
            "authorization",
            HeaderValue::from_str(&format!("Bearer {}", options.bearer)).map_err(Error::decode)?,
        );
        request
            .headers_mut()
            .insert("sec-websocket-protocol", HeaderValue::from_static(PROFILE));
        let config = WebSocketConfig::default()
            .max_message_size(Some(options.limits.max_frame_bytes as usize))
            .max_frame_size(Some(options.limits.max_frame_bytes as usize));
        let timeout = Duration::from_millis(options.limits.timeout_ms.into());
        let (mut ws, response) = tokio::time::timeout(
            timeout,
            connect_async_with_config(request, Some(config), false),
        )
        .await
        .map_err(|_| Error::unavailable())?
        .map_err(|_| Error::unavailable())?;
        if response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|s| s.to_str().ok())
            != Some(PROFILE)
        {
            return Err(Error::new(
                p::ErrorCode::UnsupportedCapability,
                "JSON WSS subprotocol not selected",
                "not_started",
                "never",
            ));
        }
        let hello = p::HelloRequest {
            profile_id: PROFILE.into(),
            plane: p::Plane::Broker as i32,
            provides: PROVIDES.map(str::to_owned).to_vec(),
            requires: REQUESTS.map(str::to_owned).to_vec(),
        };
        let result = handshake(&mut ws, "conex/hello", json_value(hello)?, timeout).await?;
        let hello: p::HelloResponse = typed(result)?;
        if hello.profile_id != PROFILE
            || hello.plane != p::Plane::Broker as i32
            || !p::wire::is_ulid(&hello.binding_id)
            || REQUESTS
                .iter()
                .any(|m| !hello.provides.iter().any(|x| x == m))
            || !hello.rejected_capabilities.is_empty()
        {
            return Err(Error::bad_request("S1 negotiation rejected or incomplete"));
        }
        let remote = hello
            .limits
            .ok_or_else(|| Error::bad_request("missing S1 limits"))?;
        valid_limits(&remote)?;
        let limits = lower(options.limits, remote);
        valid_limits(&limits)?;
        let ready = p::ReadyRequest {
            negotiation_id: hello.binding_id.clone(),
            profile_id: PROFILE.into(),
            plane: p::Plane::Broker as i32,
            provides: PROVIDES.map(str::to_owned).to_vec(),
            limits: Some(limits),
        };
        let result = handshake(&mut ws, "conex/ready", json_value(ready)?, timeout).await?;
        let ready: p::ReadyResponse = typed(result)?;
        if ready.negotiation_id != hello.binding_id
            || ready.profile_id != PROFILE
            || ready.plane != p::Plane::Broker as i32
            || REQUESTS
                .iter()
                .any(|m| !ready.provides.iter().any(|x| x == m))
        {
            return Err(Error::bad_request("ready does not match negotiation"));
        }
        let final_limits = ready
            .limits
            .ok_or_else(|| Error::bad_request("missing ready limits"))?;
        valid_limits(&final_limits)?;
        if lower(limits, final_limits) != final_limits {
            return Err(Error::bad_request("ready raised negotiated limits"));
        }
        let hub = ready
            .link_identity
            .filter(|h| {
                p::wire::is_ulid(&h.link_id) && !h.peer_id.is_empty() && !h.tenant_id.is_empty()
            })
            .ok_or_else(|| Error::bad_request("missing authenticated hub identity"))?;
        let capacity = final_limits.max_queued_bytes as usize;
        // Byte permits bound payload storage; channel slots also bound tiny frames.
        let (data, data_rx) = mpsc::channel(capacity.min(4096));
        let (control, control_rx) = mpsc::channel((final_limits.max_frame_bytes as usize).min(256));
        let core = Arc::new(Core {
            limits: final_limits,
            hub,
            budget: Budget::new(&final_limits),
            tables: Mutex::new(Tables::default()),
            data,
            control,
            slots: Arc::new(Semaphore::new((final_limits.max_inflight - 1) as usize)),
            control_slot: Arc::new(Semaphore::new(1)),
            closed: watch::channel(false).0,
            finished: watch::channel(false).0,
        });
        tokio::spawn(owner(ws, core.clone(), data_rx, control_rx));
        Ok(Self {
            life: Arc::new(Life(core)),
        })
    }
    fn core(&self) -> &Arc<Core> {
        &self.life.0
    }
    pub fn limits(&self) -> p::Limits {
        self.core().limits
    }
    pub fn hub_identity(&self) -> &p::LinkIdentity {
        &self.core().hub
    }
    pub async fn list_modules(&self, space_id: impl Into<String>) -> Result<p::ModuleListResult> {
        self.core()
            .request(
                "module/list",
                broker(),
                json_value(p::ModuleListInput {
                    space_id: space_id.into(),
                })?,
                PendingKind::Rpc,
                false,
            )?
            .result()
            .await
    }
    pub async fn provide(
        &self,
        input: p::ModuleRegisterInput,
        contract: Arc<Contract>,
        exposure: Exposure,
    ) -> Result<p::ModuleRegisterResult> {
        if input.contract_id != contract.id
            || contract.methods.values().any(|m| m.handler.is_none())
            || contract.states.values().any(|s| s.handler.is_none())
            || contract
                .methods
                .keys()
                .any(|m| !exposure.methods.contains(m))
            || contract.states.keys().any(|s| !exposure.states.contains(s))
            || exposure.principals.is_empty()
            || exposure.tenants.is_empty()
        {
            return Err(Error::bad_request(
                "publication needs complete handlers and explicit local exposure",
            ));
        }
        self.core()
            .request(
                "module/register",
                broker(),
                json_value(&input)?,
                PendingKind::Register {
                    input,
                    contract,
                    exposure,
                },
                false,
            )?
            .result()
            .await
    }
    pub async fn unprovide(&self, target: &ModuleRef) -> Result<p::ModuleUnregisterResult> {
        self.core().unpublish_local(target);
        self.core()
            .request(
                "module/unregister",
                broker(),
                json_value(p::ModuleUnregisterInput {
                    provider_endpoint_id: target.provider_endpoint_id.clone(),
                    registration_id: target.registration_id.clone(),
                })?,
                PendingKind::Rpc,
                true,
            )?
            .result()
            .await
    }
    pub fn invoke(
        &self,
        target: ModuleRef,
        contract: Arc<Contract>,
        method: &str,
        argument: impl Serialize,
    ) -> Result<Call> {
        if target.contract_id != contract.id {
            return Err(Error::bad_request("contract binding mismatch"));
        }
        let descriptor = contract
            .methods
            .get(method)
            .ok_or_else(|| Error::bad_request("method not installed"))?;
        let argument = json_value(argument)?;
        capability::validate(&descriptor.input_schema, &argument)?;
        let input = json!({"method":method,"argument":argument});
        let mut call = self.core().request(
            "module/invoke",
            target.context(),
            input,
            PendingKind::Invoke {
                execution: descriptor.execution,
                output: descriptor.output_schema.clone(),
            },
            false,
        )?;
        call.control.target = target;
        Ok(call)
    }
    /// Opening is installed before enqueue. Snapshot arrives as the first event.
    pub fn subscribe(
        &self,
        target: ModuleRef,
        contract: Arc<Contract>,
        state: &str,
        argument: impl Serialize,
    ) -> Result<Subscription> {
        if target.contract_id != contract.id {
            return Err(Error::bad_request("contract binding mismatch"));
        }
        let descriptor = contract
            .states
            .get(state)
            .ok_or_else(|| Error::bad_request("state not installed"))?;
        let argument = json_value(argument)?;
        capability::validate(&descriptor.input_schema, &argument)?;
        let id = new_id();
        let (events, rx) = mpsc::channel(64);
        let (terminal, terminal_rx) = watch::channel(None);
        let stopped = Arc::new(AtomicBool::new(false));
        {
            let mut tables = self.core().tables.lock().unwrap();
            if tables.consumers.len() + tables.providers.len()
                >= self.core().limits.max_subscriptions as usize
            {
                return Err(Error::quota());
            }
            tables.consumers.insert(
                id.clone(),
                ConsumerState {
                    target: target.clone(),
                    schema: descriptor.value_schema.clone(),
                    events,
                    terminal,
                    stopped: stopped.clone(),
                    sequence: None,
                },
            );
        }
        let result = self.core().request(
            "state/subscribe",
            target.context(),
            json!({"subscriptionId":id,"state":state,"argument":argument}),
            PendingKind::Subscribe(id.clone()),
            false,
        );
        if let Err(error) = result {
            self.core().end_consumer(&id, error.clone());
            return Err(error);
        }
        Ok(Subscription {
            id,
            core: Arc::downgrade(self.core()),
            target,
            events: rx,
            terminal: terminal_rx,
            stopped,
            ended: false,
        })
    }
    pub async fn close(&self) {
        self.core().shutdown();
        let mut finished = self.core().finished.subscribe();
        while !*finished.borrow_and_update() {
            if finished.changed().await.is_err() {
                break;
            }
        }
    }
    /// True once the link is lost or explicitly closed; no request will be
    /// transmitted and pending ones have settled with link-lost errors.
    pub fn is_closed(&self) -> bool {
        self.core().is_closed()
    }
    /// Completes when this link is lost or explicitly closed. Lost links never
    /// replay pending requests — they settled on settle_closed. Used by the
    /// plugin publication coordinator to detect breakage without polling.
    pub async fn wait_closed(&self) {
        let mut closed = self.core().closed.subscribe();
        while !*closed.borrow_and_update() {
            if closed.changed().await.is_err() {
                return;
            }
        }
    }
}

fn broker() -> p::RequestContext {
    p::RequestContext {
        plane: p::Plane::Broker as i32,
        ..Default::default()
    }
}
fn new_id() -> String {
    // One process-wide monotonic counter: IDs are never reused on a link. Not an idempotency key.
    static NEXT: Mutex<u128> = Mutex::new(0);
    let mut next = NEXT.lock().unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        << 80;
    *next = now.max(next.checked_add(1).expect("ULID space exhausted"));
    let mut value = *next;
    let mut out = [b'0'; 26];
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    for ch in out.iter_mut().rev() {
        *ch = ALPHABET[(value & 31) as usize];
        value >>= 5;
    }
    String::from_utf8(out.to_vec()).expect("ASCII ULID")
}
// JSON-plane envelope mapping mirrors `conex_proto::wire`, but round-trips raw
// JSON instead of `pbjson_types::Value`: protobuf Value stores every number as
// f64, and the generated `NumberDeserialize<u32|i32|…>` would then reject
// `3.0`/`262144.0` for a JSON integer on the wire (S1 vectors/contract require
// int32 as a plain JSON number and uint64 as a decimal string). The TS SDK
// mirrors the envelope the same way, so both languages stay byte-compatible.
fn plane_lower(plane: i32) -> &'static str {
    match p::Plane::try_from(plane) {
        Ok(p::Plane::Broker) => "broker",
        Ok(p::Plane::Relay) => "relay",
        Ok(p::Plane::Unspecified) => "unspecified",
        Err(_) => "unspecified",
    }
}
fn context_json(context: &p::RequestContext, method: Option<&str>) -> Value {
    let mut ctx = serde_json::Map::new();
    if !matches!(
        method,
        Some("module/register" | "module/unregister" | "module/list")
    ) {
        ctx.insert(
            "providerEndpointId".into(),
            Value::String(context.provider_endpoint_id.clone()),
        );
    }
    ctx.insert(
        "plane".into(),
        Value::String(plane_lower(context.plane).into()),
    );
    if let Some(binding) = &context.binding_id {
        ctx.insert("bindingId".into(), Value::String(binding.clone()));
    }
    if !context.principal_id.is_empty() {
        ctx.insert(
            "principalId".into(),
            Value::String(context.principal_id.clone()),
        );
    }
    if !context.tenant_id.is_empty() {
        ctx.insert("tenantId".into(), Value::String(context.tenant_id.clone()));
    }
    if !context.space_id.is_empty() {
        ctx.insert("spaceId".into(), Value::String(context.space_id.clone()));
    }
    if !context.registration_id.is_empty() {
        ctx.insert(
            "registrationId".into(),
            Value::String(context.registration_id.clone()),
        );
    }
    Value::Object(ctx)
}
fn params_json(
    method: Option<&str>,
    context: &p::RequestContext,
    budget: u32,
    input: Option<Value>,
) -> Value {
    let mut params = serde_json::Map::new();
    params.insert("context".into(), context_json(context, method));
    if !method.is_some_and(p::capability::is_s1_notification) {
        params.insert("timeoutBudgetMs".into(), Value::Number(budget.into()));
    }
    if let Some(input) = input {
        params.insert("input".into(), input);
    }
    Value::Object(params)
}
fn request_envelope(
    id: &str,
    method: &str,
    context: p::RequestContext,
    input: Value,
    budget: u32,
) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params_json(Some(method), &context, budget, Some(input))})
}
fn error_json(error: &p::Error) -> Result<Value> {
    let mut data = serde_json::Map::new();
    data.insert(
        "code".into(),
        Value::String(p::wire::error_code_name(error.code).into()),
    );
    if !error.diagnostic_id.is_empty() {
        data.insert(
            "diagnosticId".into(),
            Value::String(error.diagnostic_id.clone()),
        );
    }
    if !error.execution.is_empty() {
        data.insert("execution".into(), Value::String(error.execution.clone()));
    }
    if !error.retry.is_empty() {
        data.insert("retry".into(), Value::String(error.retry.clone()));
    }
    if let Some(details) = &error.details {
        data.insert("details".into(), json_value(details)?);
    }
    Ok(json!({"code":error.code,"message":error.message,"data":data}))
}
fn response_envelope(id: &str, result: Result<Value>) -> Value {
    match result {
        Ok(value) => json!({"jsonrpc":"2.0","id":id,"result":value}),
        Err(error) => match error_json(&error.0) {
            Ok(error) => json!({"jsonrpc":"2.0","id":id,"error":error}),
            Err(_) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":p::ErrorCode::Internal as i32,"message":"error details encode failure","data":{"code":"internal","execution":"not_started","retry":"never"}}})
            }
        },
    }
}
fn validate_client_envelope(bytes: &[u8]) -> Result<()> {
    let message = p::wire::decode_wire_for_boundary(bytes, wire::Boundary::Client)
        .map_err(Error::protocol)?;
    p::wire::validate_message(&message).map_err(Error::protocol)?;
    wire::validate_frame(&message, wire::Boundary::Client).map_err(Error::protocol)
}

struct StateDelivery {
    updates: mpsc::Receiver<Queued<p::StateSample>>,
    terminal: watch::Receiver<Option<Error>>,
    stopped: Arc<AtomicBool>,
    slot: OwnedSemaphorePermit,
}
async fn handshake<S>(
    ws: &mut tokio_tungstenite::WebSocketStream<S>,
    method: &str,
    input: Value,
    timeout: Duration,
) -> Result<Value>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let id = new_id();
    let envelope = request_envelope(&id, method, broker(), input, timeout.as_millis() as u32);
    let bytes = serde_json::to_vec(&envelope).map_err(Error::decode)?;
    tokio::time::timeout(timeout, async {
        ws.send(WsMessage::Text(
            String::from_utf8(bytes).map_err(Error::decode)?.into(),
        ))
        .await
        .map_err(|_| Error::unavailable())?;
        loop {
            match ws.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    let message = p::wire::decode_wire(text.as_bytes()).map_err(Error::protocol)?;
                    let raw: Value = serde_json::from_str(&text).map_err(Error::decode)?;
                    return match message.body {
                        Some(Body::Success(s)) if s.request_id == id => {
                            Ok(raw.get("result").cloned().unwrap_or(Value::Null))
                        }
                        Some(Body::Failure(f)) if f.request_id.as_deref() == Some(id.as_str()) => {
                            Err(Error(Box::new(
                                f.error.ok_or_else(|| Error::bad_request("missing error"))?,
                            )))
                        }
                        _ => Err(Error::bad_request("unexpected handshake frame")),
                    };
                }
                Some(Ok(WsMessage::Ping(_))) | Some(Ok(WsMessage::Pong(_))) => continue,
                _ => return Err(Error::unavailable()),
            }
        }
    })
    .await
    .map_err(|_| Error::unavailable())?
}

impl Core {
    pub(crate) fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }
    pub(crate) fn shutdown(&self) {
        self.closed.send_replace(true);
    }
    fn enqueue(
        &self,
        envelope: Value,
        control: bool,
        ordered: bool,
        request: Option<String>,
        fence: Option<Arc<AtomicBool>>,
    ) -> Result<()> {
        if self.is_closed() {
            return Err(Error::unavailable());
        }
        let bytes = serde_json::to_vec(&envelope).map_err(Error::decode)?;
        validate_client_envelope(&bytes)?;
        let len = bytes.len();
        let queued = self.budget.reserve(
            Frame {
                bytes,
                request,
                fence,
            },
            len,
            control,
        )?;
        let tx = if control && !ordered {
            &self.control
        } else {
            &self.data
        };
        tx.try_send(queued).map_err(|_| Error::quota())
    }
    fn request(
        self: &Arc<Self>,
        method: &'static str,
        context: p::RequestContext,
        input: Value,
        kind: PendingKind,
        control: bool,
    ) -> Result<Call> {
        if self.is_closed() {
            return Err(Error::unavailable());
        }
        let slot = if control {
            &self.control_slot
        } else {
            &self.slots
        }
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::quota())?;
        let id = new_id();
        let envelope =
            request_envelope(&id, method, context.clone(), input, self.limits.timeout_ms);
        let (tx, rx) = oneshot::channel();
        let mut tables = self.tables.lock().unwrap();
        if let PendingKind::Register { .. } = &kind {
            let openings = tables
                .pending
                .values()
                .filter(|p| matches!(p.kind, PendingKind::Register { .. }))
                .count();
            if tables.published.len() + openings >= self.limits.max_modules as usize {
                return Err(Error::quota());
            }
        }
        tables.pending.insert(
            id.clone(),
            Pending {
                method,
                kind,
                context: context.clone(),
                deadline: Instant::now() + Duration::from_millis(self.limits.timeout_ms.into()),
                sent: false,
                tx,
                _slot: slot,
            },
        );
        // Keep the lock through enqueue: reader cannot complete before opening registration.
        if let Err(error) = self.enqueue(envelope, control, false, Some(id.clone()), None) {
            tables.pending.remove(&id);
            return Err(error);
        }
        Ok(Call {
            control: CallControl {
                request_id: id,
                core: Arc::downgrade(self),
                target: ModuleRef {
                    space_id: context.space_id,
                    provider_endpoint_id: context.provider_endpoint_id,
                    registration_id: context.registration_id,
                    contract_id: String::new(),
                },
            },
            rx,
        })
    }
    pub(crate) fn end_consumer(&self, id: &str, error: Error) {
        let mut tables = self.tables.lock().unwrap();
        if let Some(state) = tables.consumers.remove(id) {
            state.end(error.clone());
        }
        let opening = tables.pending.iter().find_map(|(key, pending)| {
            if matches!(&pending.kind, PendingKind::Subscribe(sub) if sub == id) {
                Some(key.clone())
            } else {
                None
            }
        });
        if let Some(key) = opening
            && let Some(pending) = tables.pending.remove(&key)
        {
            let _ = pending.tx.send(Err(error));
        }
    }
    pub(crate) fn begin_unsubscribe(
        self: &Arc<Self>,
        target: ModuleRef,
        id: String,
    ) -> Result<Call> {
        self.request(
            "state/unsubscribe",
            target.context(),
            json!({"subscriptionId":id}),
            PendingKind::Rpc,
            true,
        )
    }
    pub(crate) fn unsubscribe_detached(self: &Arc<Self>, target: ModuleRef, id: String) {
        // The bounded pending/control slot owns cleanup; no detached task or retry.
        let result = self.request(
            "state/unsubscribe",
            target.context(),
            json!({"subscriptionId":id}),
            PendingKind::Rpc,
            true,
        );
        if result.is_err() {
            self.shutdown();
        }
    }
    fn unpublish_local(&self, target: &ModuleRef) {
        let mut tables = self.tables.lock().unwrap();
        if tables
            .published
            .get(&target.registration_id)
            .is_some_and(|p| p.target == *target)
        {
            tables.published.remove(&target.registration_id);
        }
        tables.running.retain(|_, run| {
            if target.matches(&run.context) {
                run.cancel.send_replace(true);
            }
            true // Invoke awaits a definite result or its original deadline.
        });
        tables.providers.retain(|_, state| {
            if target.matches(&state.context) {
                state.stopped.store(true, Ordering::Release);
                state.cancel.send_replace(true);
                state.abort.abort();
                false
            } else {
                true
            }
        });
    }
    fn pending_error(pending: &Pending, code: p::ErrorCode, message: &str) -> Error {
        match &pending.kind {
            PendingKind::Invoke { execution, .. } if pending.sent => {
                execution.uncertain(code, message)
            }
            _ if pending.sent => Error::new(code, message, "unknown", "never"),
            _ => Error::new(
                code,
                message,
                "not_started",
                if code == p::ErrorCode::Timeout {
                    "safe"
                } else {
                    "never"
                },
            ),
        }
    }
    fn expire(self: &Arc<Self>) {
        let now = Instant::now();
        let expired: Vec<_> = {
            let mut tables = self.tables.lock().unwrap();
            let ids: Vec<_> = tables
                .pending
                .iter()
                .filter(|(_, p)| p.deadline <= now)
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| tables.pending.remove(&id).map(|pending| (id, pending)))
                .collect()
        };
        for (id, pending) in expired {
            let error = Self::pending_error(&pending, p::ErrorCode::Timeout, "request deadline");
            if let PendingKind::Subscribe(id) = &pending.kind {
                let target = {
                    let mut tables = self.tables.lock().unwrap();
                    tables.consumers.remove(id).map(|s| {
                        s.end(error.clone());
                        s.target
                    })
                };
                if let Some(target) = target {
                    self.unsubscribe_detached(target, id.clone());
                }
            }
            if matches!(&pending.kind, PendingKind::Register { .. }) {
                self.shutdown();
            }
            if pending.sent && matches!(&pending.kind, PendingKind::Invoke { .. }) {
                let cleanup = self.request(
                    "module/cancel",
                    pending.context.clone(),
                    json!({"requestId":id}),
                    PendingKind::Rpc,
                    true,
                );
                if cleanup.is_err() {
                    self.shutdown();
                }
            }
            let _ = pending.tx.send(Err(error));
        }
    }
    fn settle_closed(&self) {
        let mut tables = self.tables.lock().unwrap();
        for (_, pending) in tables.pending.drain() {
            let error = Self::pending_error(
                &pending,
                p::ErrorCode::Unavailable,
                "link lost; no requests replayed",
            );
            let _ = pending.tx.send(Err(error));
        }
        for (_, state) in tables.consumers.drain() {
            state.end(Error::ended(p::ErrorCode::Unavailable, "link lost"));
        }
        for (_, run) in tables.running.drain() {
            run.cancel.send_replace(true);
            run.abort.abort();
        }
        for (_, state) in tables.providers.drain() {
            state.stopped.store(true, Ordering::Release);
            state.cancel.send_replace(true);
            state.abort.abort();
        }
        tables.published.clear();
    }
    fn complete(self: &Arc<Self>, id: String, result: Result<Value>) -> Result<()> {
        let mut tables = self.tables.lock().unwrap();
        let Some(pending) = tables.pending.remove(&id) else {
            return Ok(());
        }; // Late reply is fenced to this link.
        let success_frame = result.is_ok();
        let checked = result.and_then(|value| {
            wire::validate_result(pending.method, &value).map_err(Error::protocol)?;
            match &pending.kind {
                PendingKind::Invoke { output, .. } => capability::validate(output, &value)?,
                PendingKind::Register {
                    input,
                    contract,
                    exposure,
                } => {
                    let registration: p::ModuleRegisterResult = typed(value.clone())?;
                    if registration.space_id != input.space_id
                        || registration.provider_endpoint_id != input.provider_endpoint_id
                        || registration.integration_id != input.integration_id
                        || registration.contract_id != input.contract_id
                    {
                        return Err(Error::bad_request("registration result binding mismatch"));
                    }
                    tables.published.insert(
                        registration.registration_id.clone(),
                        Published {
                            target: ModuleRef::from(&registration),
                            contract: contract.clone(),
                            exposure: exposure.clone(),
                        },
                    );
                }
                PendingKind::Subscribe(id) => {
                    if let Some(state) = tables.consumers.get_mut(id) {
                        if value["subscriptionId"].as_str() != Some(id.as_str())
                            || value["registrationId"].as_str()
                                != Some(state.target.registration_id.as_str())
                        {
                            return Err(Error::bad_request("snapshot binding mismatch"));
                        }
                        let sample = wire::state_sample_from_json(&value["snapshot"])
                            .map_err(Error::protocol)?;
                        let sample_value = json_value(&sample.value)?;
                        capability::validate(&state.schema, &sample_value)?;
                        state.sequence = Some(Sequence::new(sample.revision));
                        let bytes = serde_json::to_vec(&value).map_err(Error::decode)?.len();
                        let item =
                            self.budget
                                .reserve(StateEvent::Snapshot(sample), bytes, false)?;
                        state.events.try_send(item).map_err(|_| {
                            Error::ended(p::ErrorCode::SlowConsumer, "snapshot queue full")
                        })?;
                    }
                }
                _ => {}
            }
            Ok(value)
        });
        let checked = checked.map_err(|error| {
            // A valid remote failure is already definite; malformed success cannot establish it.
            if success_frame
                && pending.sent
                && let PendingKind::Invoke { execution, .. } = &pending.kind
            {
                return execution.uncertain(
                    p::ErrorCode::Unavailable,
                    format!("untrusted invoke result: {}", error.0.message),
                );
            }
            error
        });
        let mut cleanup = None;
        if let PendingKind::Subscribe(id) = &pending.kind
            && let Err(error) = &checked
            && let Some(state) = tables.consumers.remove(id)
        {
            state.end(error.clone());
            cleanup = Some((state.target, id.clone()));
        }
        if matches!(&pending.kind, PendingKind::Register { .. }) && checked.is_err() {
            self.shutdown();
        }
        let _ = pending.tx.send(checked);
        drop(tables);
        if let Some((target, id)) = cleanup {
            self.unsubscribe_detached(target, id);
        }
        Ok(())
    }
    fn notification(
        self: &Arc<Self>,
        method: String,
        context: p::RequestContext,
        input: Value,
    ) -> Result<()> {
        let id = input["subscriptionId"]
            .as_str()
            .ok_or_else(|| Error::bad_request("subscription ID"))?;
        let mut tables = self.tables.lock().unwrap();
        let Some(state) = tables.consumers.get_mut(id) else {
            return Ok(());
        };
        if !state.target.matches(&context) {
            return Err(Error::bad_request("state registration scope mismatch"));
        }
        if method == "state/end" {
            let error = Error(Box::new(
                wire::parse_state_end_error(&input["error"]).map_err(Error::protocol)?,
            ));
            if let Some(state) = tables.consumers.remove(id) {
                state.end(error);
            }
            return Ok(());
        }
        if method != "state/update" {
            return Err(Error::bad_request("unexpected notification"));
        }
        let check: Result<()> = (|| {
            let seq: u64 = input["seq"]
                .as_str()
                .ok_or_else(|| Error::bad_request("seq"))?
                .parse()
                .map_err(Error::decode)?;
            let sample = wire::state_sample_from_json(&input["sample"]).map_err(Error::protocol)?;
            capability::validate(&state.schema, &json_value(&sample.value)?)?;
            state
                .sequence
                .as_mut()
                .ok_or_else(|| Error::ended(p::ErrorCode::SessionLost, "update before snapshot"))?
                .accept(seq, sample.revision)?;
            let bytes = serde_json::to_vec(&input).map_err(Error::decode)?.len();
            let item = self
                .budget
                .reserve(StateEvent::Update { seq, sample }, bytes, false)
                .map_err(|_| Error::ended(p::ErrorCode::SlowConsumer, "consumer byte budget"))?;
            state.events.try_send(item).map_err(|_| {
                Error::ended(p::ErrorCode::SlowConsumer, "consumer event queue full")
            })?;
            Ok(())
        })();
        if let Err(error) = check {
            let state = tables.consumers.remove(id).expect("present");
            let target = state.target.clone();
            state.end(if error.code() == p::ErrorCode::SlowConsumer as i32 {
                error
            } else {
                Error::ended(p::ErrorCode::SessionLost, error.to_string())
            });
            drop(tables);
            self.unsubscribe_detached(target, id.to_owned());
        }
        Ok(())
    }
}

async fn owner<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    core: Arc<Core>,
    mut data: mpsc::Receiver<Queued<Frame>>,
    mut control: mpsc::Receiver<Queued<Frame>>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    {
        let (mut sink, mut stream) = ws.split();
        // Both halves are driven by this ONE task. A blocked sink never prevents reads.
        let writer_core = core.clone();
        let writer = async {
            let mut closed = writer_core.closed.subscribe();
            let mut flush = tokio::time::interval(Duration::from_millis(50));
            while !*closed.borrow_and_update() {
                let mut queued = tokio::select! {
                    biased;
                    _ = closed.changed() => break,
                    _ = flush.tick() => {
                        tokio::select! {
                            _ = closed.changed() => break,
                            result = sink.flush() => if result.is_err() { break; },
                        }
                        continue;
                    }
                    item = control.recv() => match item { Some(item) => item, None => break },
                    item = data.recv() => match item { Some(item) => item, None => break },
                };
                if queued
                    .value
                    .fence
                    .as_ref()
                    .is_some_and(|f| f.load(Ordering::Acquire))
                {
                    continue;
                }
                if let Some(id) = &queued.value.request {
                    let mut tables = writer_core.tables.lock().unwrap();
                    let Some(pending) = tables.pending.get_mut(id) else {
                        continue;
                    };
                    let now = Instant::now();
                    if pending.deadline <= now {
                        continue;
                    }
                    let remaining = (pending.deadline - now).as_millis().max(1) as u32;
                    let mut envelope: Value = match serde_json::from_slice(&queued.value.bytes) {
                        Ok(value) => value,
                        Err(_) => break,
                    };
                    if let Some(params) = envelope.get_mut("params").and_then(Value::as_object_mut)
                    {
                        params.insert("timeoutBudgetMs".into(), Value::Number(remaining.into()));
                    }
                    queued.value.bytes = match serde_json::to_vec(&envelope) {
                        Ok(bytes) => bytes,
                        Err(_) => break,
                    };
                    pending.sent = true; // Conservative possible-transmission boundary, before sink.send.
                }
                let text = match String::from_utf8(queued.value.bytes) {
                    Ok(text) => text,
                    Err(_) => break,
                };
                tokio::select! {
                    _ = closed.changed() => break,
                    result = sink.send(WsMessage::Text(text.into())) => if result.is_err() { break; },
                }
                // Byte permits include the frame being flushed.
                drop(queued._total);
                drop(queued._data);
            }
        };
        let reader_core = core.clone();
        let reader = async {
            let mut closed = reader_core.closed.subscribe();
            let mut tasks = JoinSet::new();
            let mut timer = tokio::time::interval(Duration::from_millis(10));
            while !*closed.borrow_and_update() {
                tokio::select! {
                    biased;
                    _ = closed.changed() => break,
                    _ = timer.tick() => reader_core.expire(),
                    Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
                    incoming = stream.next() => {
                        let text = match incoming {
                            Some(Ok(WsMessage::Text(text))) if text.len() <= reader_core.limits.max_frame_bytes as usize => text,
                            Some(Ok(WsMessage::Ping(_))) | Some(Ok(WsMessage::Pong(_))) => continue,
                            _ => break,
                        };
                        let message = match p::wire::decode_wire(text.as_bytes()) { Ok(m) => m, Err(_) => break };
                        if p::wire::validate_message(&message).is_err() || wire::validate_frame(&message, wire::Boundary::Host).is_err() { break; }
                        // Dispatch copy: raw JSON keeps integer numbers; decode_wire's
                        // protobuf Value would coerce them to f64. Duplicate keys were
                        // already rejected by the strict decode above.
                        let raw: Value = match serde_json::from_str(&text) { Ok(value) => value, Err(_) => break };
                        let result = if raw.get("method").is_some() {
                            let input = raw.pointer("/params/input").cloned().unwrap_or(Value::Null);
                            match message.body {
                                Some(Body::Request(r)) => reader_core.forwarded(r, input, &mut tasks),
                                Some(Body::Notification(n)) => reader_core.notification(n.method.clone(), n.params.as_ref().and_then(|p| p.context.clone()).unwrap_or_default(), input),
                                _ => Err(Error::bad_request("unsupported frame")),
                            }
                        } else {
                            match message.body {
                                Some(Body::Success(s)) => reader_core.complete(s.request_id, Ok(raw.get("result").cloned().unwrap_or(Value::Null))),
                                Some(Body::Failure(f)) => match (f.request_id, f.error) { (Some(id), Some(error)) => reader_core.complete(id, Err(Error(Box::new(error)))), _ => Err(Error::bad_request("unassociated failure")) },
                                _ => Err(Error::bad_request("unsupported frame")),
                            }
                        };
                        if result.is_err() { break; }
                    }
                }
            }
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        };
        tokio::pin!(writer, reader);
        let writer_first = tokio::select! { _ = &mut writer => true, _ = &mut reader => false };
        core.shutdown();
        core.settle_closed();
        if writer_first {
            reader.await;
        } else {
            writer.await;
        }
    }
    data.close();
    control.close();
    while data.try_recv().is_ok() {}
    while control.try_recv().is_ok() {}
    core.finished.send_replace(true);
}

fn same_scope(a: &p::RequestContext, b: &p::RequestContext) -> bool {
    a.space_id == b.space_id
        && a.provider_endpoint_id == b.provider_endpoint_id
        && a.registration_id == b.registration_id
        && a.principal_id == b.principal_id
        && a.tenant_id == b.tenant_id
}
struct WorkerGuard {
    core: Weak<Core>,
    id: String,
    state: bool,
    completed: bool,
}
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        if let Some(core) = self.core.upgrade() {
            let mut tables = core.tables.lock().unwrap();
            let removed = if self.state {
                tables.providers.remove(&self.id).is_some()
            } else {
                tables.running.remove(&self.id).is_some()
            };
            if removed && !self.completed && !core.is_closed() {
                core.shutdown();
            }
        }
    }
}
impl Core {
    fn reply(&self, id: String, result: Result<Value>, control: bool) -> Result<()> {
        self.enqueue(response_envelope(&id, result), control, false, None, None)
    }
    fn notify(
        &self,
        method: &str,
        context: &p::RequestContext,
        input: Value,
        control: bool,
        fence: Option<Arc<AtomicBool>>,
    ) -> Result<()> {
        let mut context = context.clone();
        context.principal_id.clear();
        context.tenant_id.clear();
        let envelope = json!({"jsonrpc":"2.0","method":method,"params":params_json(Some(method), &context, 0, Some(input))});
        // end has reserved byte capacity but shares the data FIFO to preserve causality.
        self.enqueue(envelope, control, true, None, fence)
    }
    fn forwarded(
        self: &Arc<Self>,
        request: p::Request,
        input: Value,
        tasks: &mut JoinSet<()>,
    ) -> Result<()> {
        let params = request
            .params
            .ok_or_else(|| Error::bad_request("forwarded params"))?;
        let context = params
            .context
            .ok_or_else(|| Error::bad_request("forwarded context"))?;
        if context.principal_id.is_empty()
            || context.tenant_id.is_empty()
            || params.timeout_budget_ms == 0
            || params.timeout_budget_ms > self.limits.timeout_ms
        {
            return self.reply(
                request.request_id,
                Err(Error::new(
                    p::ErrorCode::Forbidden,
                    "missing trusted hub context or invalid budget",
                    "not_started",
                    "never",
                )),
                true,
            );
        }
        match request.method.as_str() {
            "module/cancel" => {
                let input: p::ModuleCancelInput = typed(input)?;
                let tables = self.tables.lock().unwrap();
                let requested = tables.running.get(&input.request_id).is_some_and(|run| {
                    if same_scope(&context, &run.context) {
                        run.cancel.send_replace(true);
                        true
                    } else {
                        false
                    }
                });
                self.reply(
                    request.request_id,
                    Ok(json_value(p::ModuleCancelResult { requested })?),
                    true,
                )
            }
            "state/unsubscribe" => {
                let input: p::StateUnsubscribeInput = typed(input)?;
                let mut tables = self.tables.lock().unwrap();
                let removed = tables
                    .providers
                    .get(&input.subscription_id)
                    .is_some_and(|state| same_scope(&state.context, &context));
                if removed {
                    let state = tables
                        .providers
                        .remove(&input.subscription_id)
                        .expect("present");
                    state.stopped.store(true, Ordering::Release);
                    state.cancel.send_replace(true);
                    state.abort.abort();
                    if let Some(opening) = state.opening {
                        self.reply(
                            opening,
                            Err(Error::new(
                                p::ErrorCode::Cancelled,
                                "opening unsubscribed",
                                "not_started",
                                "never",
                            )),
                            true,
                        )?;
                    }
                }
                self.reply(
                    request.request_id,
                    Ok(json_value(p::StateUnsubscribeResult { removed })?),
                    true,
                )
            }
            "module/invoke" | "state/subscribe" => {
                let mut tables = self.tables.lock().unwrap();
                let Some(published) = tables
                    .published
                    .get(&context.registration_id)
                    .filter(|p| p.target.matches(&context))
                    .cloned()
                else {
                    return self.reply(
                        request.request_id,
                        Err(Error::new(
                            p::ErrorCode::StaleRevision,
                            "registration not locally exposed",
                            "not_started",
                            "never",
                        )),
                        true,
                    );
                };
                let state = request.method == "state/subscribe";
                let member = input[if state { "state" } else { "method" }]
                    .as_str()
                    .ok_or_else(|| Error::bad_request("missing member"))?
                    .to_owned();
                if !published.exposure.allows(&context, &member, state) {
                    return self.reply(
                        request.request_id,
                        Err(Error::new(
                            p::ErrorCode::Forbidden,
                            "local exposure denied",
                            "not_started",
                            "never",
                        )),
                        true,
                    );
                }
                let argument = input["argument"].clone();
                let (cancel, receiver) = watch::channel(false);
                let invocation = InvocationContext {
                    hub: self.hub.clone(),
                    caller: context.clone(),
                    request_id: request.request_id.clone(),
                    cancellation: Cancellation(receiver.clone()),
                };
                let slot = match self.slots.clone().try_acquire_owned() {
                    Ok(slot) => slot,
                    Err(_) => return self.reply(request.request_id, Err(Error::quota()), true),
                };
                let deadline = Duration::from_millis(params.timeout_budget_ms.into());
                if !state {
                    if tables.running.contains_key(&request.request_id) {
                        return self.reply(
                            request.request_id,
                            Err(Error::new(
                                p::ErrorCode::Conflict,
                                "duplicate forwarded request",
                                "not_started",
                                "never",
                            )),
                            true,
                        );
                    }
                    let Some(method) = published.contract.methods.get(&member).cloned() else {
                        return self.reply(
                            request.request_id,
                            Err(Error::bad_request("method not installed")),
                            true,
                        );
                    };
                    if let Err(error) = capability::validate(&method.input_schema, &argument) {
                        return self.reply(request.request_id, Err(error), true);
                    }
                    let handler = method
                        .handler
                        .ok_or_else(|| Error::bad_request("method has no handler"))?;
                    let core = self.clone();
                    let id = request.request_id.clone();
                    let abort = tasks.spawn(async move {
                        let mut guard = WorkerGuard {
                            core: Arc::downgrade(&core),
                            id: id.clone(),
                            state: false,
                            completed: false,
                        };
                        let result =
                            match tokio::time::timeout(deadline, handler(invocation, argument))
                                .await
                            {
                                Ok(Ok(output)) => {
                                    capability::validate(&method.output_schema, &output)
                                        .map(|_| output)
                                        .map_err(|e| {
                                            method
                                                .execution
                                                .uncertain(p::ErrorCode::Unavailable, e.to_string())
                                        })
                                }
                                Ok(Err(mut error)) => {
                                    // Only the handler can confirm cancellation/completion.
                                    if error.0.execution.is_empty() {
                                        error.0.execution = "completed".into();
                                    }
                                    if error.0.retry.is_empty() {
                                        error.0.retry = "never".into();
                                    }
                                    Err(error)
                                }
                                Err(_) => Err(method
                                    .execution
                                    .uncertain(p::ErrorCode::Timeout, "provider handler deadline")),
                            };
                        // Remove before reply: a subsequent cancel cannot overwrite this result.
                        core.tables.lock().unwrap().running.remove(&id);
                        if core.reply(id, result, false).is_err() {
                            core.shutdown();
                        }
                        guard.completed = true;
                        drop(slot);
                    });
                    tables.running.insert(
                        request.request_id,
                        Running {
                            context,
                            cancel,
                            abort,
                        },
                    );
                } else {
                    if tables.providers.len() + tables.consumers.len()
                        >= self.limits.max_subscriptions as usize
                    {
                        return self.reply(request.request_id, Err(Error::quota()), true);
                    }
                    let id = input["subscriptionId"]
                        .as_str()
                        .ok_or_else(|| Error::bad_request("subscription ID"))?
                        .to_owned();
                    if tables.providers.contains_key(&id) {
                        return self.reply(
                            request.request_id,
                            Err(Error::new(
                                p::ErrorCode::Conflict,
                                "duplicate subscription",
                                "not_started",
                                "never",
                            )),
                            true,
                        );
                    }
                    let Some(descriptor) = published.contract.states.get(&member).cloned() else {
                        return self.reply(
                            request.request_id,
                            Err(Error::bad_request("state not installed")),
                            true,
                        );
                    };
                    if let Err(error) = capability::validate(&descriptor.input_schema, &argument) {
                        return self.reply(request.request_id, Err(error), true);
                    }
                    let handler = descriptor
                        .handler
                        .ok_or_else(|| Error::bad_request("state has no handler"))?;
                    let (updates, updates_rx) = mpsc::channel(64);
                    let (terminal, terminal_rx) = watch::channel(None);
                    let stopped = Arc::new(AtomicBool::new(false));
                    let observer = StateObserver {
                        tx: updates,
                        terminal,
                        stopped: stopped.clone(),
                        budget: self.budget.clone(),
                        schema: descriptor.value_schema.clone(),
                    };
                    let core = self.clone();
                    let sub_id = id.clone();
                    let ctx = context.clone();
                    let req_id = request.request_id.clone();
                    let stop = stopped.clone();
                    let abort = tasks.spawn(async move {
                        let mut guard = WorkerGuard {
                            core: Arc::downgrade(&core),
                            id: sub_id.clone(),
                            state: true,
                            completed: false,
                        };
                        let lease =
                            tokio::time::timeout(deadline, handler(invocation, argument, observer))
                                .await;
                        match lease {
                            Ok(Ok(lease)) => {
                                let result = core
                                    .serve_state(
                                        &req_id,
                                        &sub_id,
                                        &ctx,
                                        &descriptor.value_schema,
                                        lease,
                                        StateDelivery {
                                            updates: updates_rx,
                                            terminal: terminal_rx,
                                            stopped: stop,
                                            slot,
                                        },
                                    )
                                    .await;
                                if result.is_err() {
                                    core.shutdown();
                                }
                            }
                            failure => {
                                let error = match failure {
                                    Ok(Err(error)) => error,
                                    _ => Error::new(
                                        p::ErrorCode::Timeout,
                                        "state opening deadline",
                                        "not_started",
                                        "safe",
                                    ),
                                };
                                if core.reply(req_id, Err(error), false).is_err() {
                                    core.shutdown();
                                }
                            }
                        }
                        guard.completed = true;
                    });
                    tables.providers.insert(
                        id,
                        ProviderState {
                            context,
                            cancel,
                            abort,
                            stopped,
                            opening: Some(request.request_id),
                        },
                    );
                }
                Ok(())
            }
            _ => self.reply(
                request.request_id,
                Err(Error::new(
                    p::ErrorCode::UnknownMethod,
                    "not implemented by SDK",
                    "not_started",
                    "never",
                )),
                true,
            ),
        }
    }
    async fn serve_state(
        self: &Arc<Self>,
        request_id: &str,
        id: &str,
        context: &p::RequestContext,
        schema: &str,
        lease: StateLease,
        delivery: StateDelivery,
    ) -> Result<()> {
        let StateDelivery {
            mut updates,
            mut terminal,
            stopped,
            slot,
        } = delivery;
        let snapshot = wire::state_sample_json(&lease.snapshot).map_err(Error::protocol)?;
        capability::validate(schema, &snapshot["value"])?;
        let mut sequence = Sequence::new(lease.snapshot.revision);
        {
            let mut tables = self.tables.lock().unwrap();
            let Some(state) = tables.providers.get_mut(id) else {
                return Ok(());
            };
            if let Some(error) = terminal.borrow().clone() {
                self.reply(request_id.to_owned(), Err(error), false)?;
                return Ok(());
            }
            self.reply(request_id.to_owned(), Ok(json!({"subscriptionId":id,"registrationId":context.registration_id,"snapshot":snapshot})), false)?;
            state.opening = None;
        }
        drop(slot); // Establishment budget/slot does not limit active subscription lifetime.
        let mut seq = 0u64;
        let error = loop {
            if let Some(error) = terminal.borrow().clone() {
                break error;
            }
            tokio::select! {
                biased;
                changed = terminal.changed() => {
                    if changed.is_err() { break Error::ended(p::ErrorCode::Unavailable, "state source closed"); }
                }
                item = updates.recv() => {
                    let Some(item) = item else { break Error::ended(p::ErrorCode::Unavailable, "state source closed"); };
                    let Some(next) = seq.checked_add(1) else { break Error::ended(p::ErrorCode::SessionLost, "seq overflow; re-register"); };
                    if let Err(error) = sequence.accept(next, item.value.revision) { break error; }
                    let input = wire::state_update_input(id, next, &item.value).map_err(Error::protocol)?;
                    // Transfer from domain queue to wire queue; no unbounded intermediary.
                    drop(item);
                    if let Err(error) = self.notify("state/update", context, input, false, Some(stopped.clone())) {
                        break Error::ended(p::ErrorCode::SlowConsumer, error.to_string());
                    }
                    seq = next;
                }
            }
        };
        stopped.store(true, Ordering::Release); // Writer drops any queued updates before end.
        self.notify(
            "state/end",
            context,
            wire::state_end_input(id, &error.0).map_err(Error::protocol)?,
            true,
            None,
        )?;
        drop(lease); // Always unhook the domain observer before the worker finishes.
        Ok(())
    }
}

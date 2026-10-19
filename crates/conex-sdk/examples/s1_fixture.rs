//! S1-T08 controlled Rust counter fixture
//! (docs/plans/2026-10-17-conex-s1.md T08; docs/contracts/s1-capabilities.md).
//!
//! A test-only process that connects and publishes `conex.counter` through the
//! real conex-sdk, then executes a JSONL command protocol on stdin/stdout so
//! the S1 e2e harness can drive deterministic races (commit/snapshot barriers)
//! and observe SDK behavior. No production SDK/host code is modified.
//!
//! STDOUT is JSONL protocol only; diagnostics go to STDERR. The token is never
//! printed. Deterministic gates use channel/Notify only, never sleeps.
//!
//! Env:
//!   CONEX_URL               ws://127.0.0.1:PORT/wss
//!   CONEX_TOKEN             bearer matching a host [[tokens]] digest
//!   CONEX_SPACE             space id (dev)
//!   CONEX_ENDPOINT          own provider endpoint id (counter-rust)
//!   CONEX_INTEGRATION       default "integration"
//!   CONEX_INITIAL_REVISION  default 0; u64, tests revisions > 2^53 exactly
//!
//! Startup emits `{"event":"ready","reference":<ModuleRegisterResult>}` after
//! the module is published. Local exposure accepts the own endpoint plus every
//! `conex.counter` provider endpoint visible in a short pre-registration scan
//! (the host.toml convention names principals == endpoints); the host approval
//! config remains authoritative.
//!
//! Requests (JSONL on stdin): `{"id":<any>,"op":<op>,...}` responds
//! `{"id":<echo>,"ok":true,"value":...}` or `{"id":<echo>,"ok":false,"error":{code,message,execution,retry}}`.
//!
//! Ops:
//! - list {spaceId?}                              -> {modules:[ModuleInfo]}, refreshes the target cache
//! - invoke {callId,target,method,argument,timeoutMs?} -> terminal result; emits
//!   `{"event":"call_started","callId","requestId"}` once the cancel control is
//!   stored and before the result future is awaited
//! - cancel {callId}                              -> {requested}; races the pending result, never settles it itself
//! - subscribe {subscriptionKey,target,state,argument} -> {snapshot,subscriptionId}; then forwards
//!   `{"event":"update","subscriptionKey",seq,sample}` and `{"event":"end","subscriptionKey",error}`
//! - unsubscribe {subscriptionKey}                -> {removed}
//! - arm {point:"before_commit"|"after_commit"|"snapshot"}  single-shot gate on the next provider op
//! - release {point}                              -> unblocks the armed gate
//!   on reach: `{"event":"barrier","point","requestId"?,"count","revision"}`
//! - mutate {delta}   commits through the same atomic path as counter/add
//! - inspect          -> {count,revision,executions,observers}
//! - close            closes the SDK link; inspect still works
//! - shutdown         clean close + exit 0

use conex_sdk::proto as p;
use conex_sdk::{
    CallControl, Client, ConnectOptions, Contract, Error as SdkError, Execution, Exposure,
    InvocationContext, Method, ModuleRef, State, StateEvent, StateLease, StateObserver,
    Subscription,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{Notify, mpsc, oneshot};

const CONTRACT_ID: &str = "conex.counter";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        eprintln!("missing {name}");
        std::process::exit(2);
    })
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Deserialize)]
struct AddInput {
    delta: i32,
}

#[derive(Serialize, Deserialize, Debug)]
struct CounterResult {
    count: i32,
    revision: String,
}

/// Domain state. Every add and every snapshot registration take this lock, so
/// the committed mutation and the state broadcast share one serialization
/// boundary; a slow observer is ended by `StateObserver::publish`, never rolls
/// back the commit. `executions` counts only committing adds (delta = 0 also
/// commits and bumps the revision).
struct Domain {
    count: i32,
    revision: u64,
    executions: u64,
    next_observer: u64,
    observers: Vec<(u64, StateObserver)>,
}

impl Domain {
    fn new(initial_revision: u64) -> Self {
        Self {
            count: 0,
            revision: initial_revision,
            executions: 0,
            next_observer: 0,
            observers: Vec::new(),
        }
    }
    fn result(&self) -> CounterResult {
        CounterResult {
            count: self.count,
            revision: self.revision.to_string(),
        }
    }
    fn sample(&self) -> p::StateSample {
        p::StateSample {
            revision: self.revision,
            value: Some(
                serde_json::from_value(json!({"count": self.count})).expect("counter value json"),
            ),
            sampled_at_ms: now_ms(),
            quality: "current".into(),
        }
    }
    /// Returns the committed result plus the post-commit revision.
    fn add(&mut self, delta: i32) -> Result<(CounterResult, u64), SdkError> {
        // int32 range check before any mutation; on overflow count and
        // revision stay unchanged.
        let next = self.count.checked_add(delta).ok_or_else(|| {
            SdkError::new(
                p::ErrorCode::BadRequest,
                "int32 overflow",
                "not_started",
                "never",
            )
        })?;
        self.revision = self.revision.checked_add(1).ok_or_else(|| {
            SdkError::new(
                p::ErrorCode::BadRequest,
                "revision overflow",
                "not_started",
                "never",
            )
        })?;
        self.count = next;
        self.executions += 1; // delta = 0 also advances revison/executions
        let sample = self.sample();
        self.observers
            .retain(|(_, observer)| observer.publish(sample.clone()).is_ok());
        Ok((self.result(), self.revision))
    }
}

type Writer = mpsc::UnboundedSender<(String, Option<oneshot::Sender<()>>)>;

fn send_line(writer: &Writer, line: String) {
    let _ = writer.send((line, None));
}

/// Single-shot deterministic gate: armed -> the next operation that reaches it
/// emits the barrier event and blocks until `release`. Notify stores the permit
/// when no waiter is registered yet, so ordering is race-free.
struct Gate {
    armed: Mutex<bool>,
    notify: Notify,
}

impl Gate {
    fn new() -> Self {
        Self {
            armed: Mutex::new(false),
            notify: Notify::new(),
        }
    }
    fn arm(&self) {
        *self.armed.lock().unwrap() = true;
    }
    fn release(&self) {
        self.notify.notify_one();
    }
    async fn wait(
        &self,
        point: &str,
        count: i32,
        revision: u64,
        request_id: Option<&str>,
        writer: &Writer,
        node: &str,
    ) {
        if !*self.armed.lock().unwrap() {
            return;
        }
        *self.armed.lock().unwrap() = false;
        let notified = self.notify.notified();
        let mut fields = Map::new();
        fields.insert("point".into(), point.into());
        fields.insert("count".into(), json!(count));
        fields.insert("revision".into(), json!(revision.to_string()));
        if let Some(request_id) = request_id {
            fields.insert("requestId".into(), Value::String(request_id.to_string()));
        }
        emit(writer, node, "barrier", Value::Object(fields));
        notified.await;
    }
}

fn emit(writer: &Writer, node: &str, event: &str, fields: Value) {
    let mut map = match fields {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    map.insert("event".into(), event.into());
    map.insert("node".into(), node.into());
    send_line(writer, Value::Object(map).to_string());
}

fn response_error(error: &SdkError) -> Value {
    json!({
        "code": error.0.code,
        "message": error.0.message,
        "execution": error.0.execution,
        "retry": error.0.retry,
    })
}

fn respond_ok(writer: &Writer, id: &str, value: Value) {
    send_line(
        writer,
        json!({"id": id, "ok": true, "value": value}).to_string(),
    );
}

fn respond_err(writer: &Writer, id: &str, error: &SdkError) {
    send_line(
        writer,
        json!({"id": id, "ok": false, "error": response_error(error)}).to_string(),
    );
}

fn bad_request(message: impl Into<String>) -> SdkError {
    SdkError::new(p::ErrorCode::BadRequest, message, "not_started", "never")
}

fn require_str(req: &Value, field: &str) -> Result<String, SdkError> {
    req.get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| bad_request(format!("missing string field {field}")))
}

/// Resolve a target provider endpoint id to the current ModuleRef, refreshing
/// the discovery cache from a fresh list when the id is not cached yet.
async fn resolve_target(
    client: &Client,
    discovered: &Arc<Mutex<BTreeMap<String, p::ModuleInfo>>>,
    space: &str,
    endpoint_id: &str,
) -> Result<ModuleRef, SdkError> {
    {
        let cache = discovered.lock().unwrap();
        if let Some(info) = cache.get(endpoint_id) {
            return Ok(ModuleRef::from(info));
        }
    }
    if let Ok(list) = client.list_modules(space.to_string()).await {
        let mut cache = discovered.lock().unwrap();
        for info in &list.modules {
            if info.contract_id == CONTRACT_ID {
                cache.insert(info.provider_endpoint_id.clone(), info.clone());
            }
        }
        if let Some(info) = cache.get(endpoint_id) {
            return Ok(ModuleRef::from(info));
        }
    }
    Err(SdkError::new(
        p::ErrorCode::UnknownProvider,
        format!("target {endpoint_id} not visible in space {space}"),
        "not_started",
        "never",
    ))
}

/// `target` accepts either an endpoint id string (current directory) or a
/// pinned ModuleRef object — needed to address stale registration instances.
async fn resolve_target_value(
    client: &Client,
    discovered: &Arc<Mutex<BTreeMap<String, p::ModuleInfo>>>,
    space: &str,
    target: &Value,
) -> Result<ModuleRef, SdkError> {
    if let Some(object) = target.as_object() {
        let field = |key: &str| {
            object
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        return Ok(ModuleRef {
            space_id: field("spaceId"),
            provider_endpoint_id: field("providerEndpointId"),
            registration_id: field("registrationId"),
            contract_id: field("contractId"),
        });
    }
    let endpoint = target.as_str().unwrap_or_default();
    resolve_target(client, discovered, space, endpoint).await
}

async fn writer_task(mut rx: mpsc::UnboundedReceiver<(String, Option<oneshot::Sender<()>>)>) {
    let mut out = std::io::BufWriter::new(std::io::stdout());
    while let Some((line, ack)) = rx.recv().await {
        let flushed = writeln!(out, "{line}").is_ok() && out.flush().is_ok();
        if let Some(ack) = ack {
            let _ = ack.send(());
        }
        if !flushed {
            break;
        }
    }
}

#[tokio::main]
async fn main() {
    let url = env("CONEX_URL");
    let token = env("CONEX_TOKEN");
    let space = env("CONEX_SPACE");
    let endpoint = env("CONEX_ENDPOINT");
    let integration = env_or("CONEX_INTEGRATION", "integration");
    let initial_revision: u64 = env_or("CONEX_INITIAL_REVISION", "0")
        .parse()
        .unwrap_or_else(|_| {
            eprintln!("invalid CONEX_INITIAL_REVISION");
            std::process::exit(2);
        });

    let (writer, writer_rx) = mpsc::unbounded_channel::<(String, Option<oneshot::Sender<()>>)>();
    tokio::spawn(writer_task(writer_rx));
    emit(
        &writer,
        &endpoint,
        "start",
        json!({"space": space, "integration": integration}),
    );

    let domain = Arc::new(Mutex::new(Domain::new(initial_revision)));
    let before_commit = Arc::new(Gate::new());
    let after_commit = Arc::new(Gate::new());
    let snapshot_gate = Arc::new(Gate::new());

    let read = Method::new(
        "conex.CounterReadInput",
        "conex.CounterResult",
        Execution::ReadOnly,
    )
    .with_handler({
        let domain = domain.clone();
        move |_context: InvocationContext, _input: Value| {
            let domain = domain.clone();
            async move {
                let d = domain.lock().unwrap();
                Ok(d.result())
            }
        }
    });

    let add = Method::new(
        "conex.CounterAddInput",
        "conex.CounterResult",
        Execution::NonReplayable,
    )
    .with_handler({
        let domain = domain.clone();
        let before = before_commit.clone();
        let after = after_commit.clone();
        let writer = writer.clone();
        let node = endpoint.clone();
        move |mut context: InvocationContext, input: AddInput| {
            let domain = domain.clone();
            let before = before.clone();
            let after = after.clone();
            let writer = writer.clone();
            let node = node.clone();
            async move {
                let (count, revision) = {
                    let d = domain.lock().unwrap();
                    (d.count, d.revision)
                };
                tokio::select! {
                    _ = context.cancellation.cancelled() => {}
                    _ = before.wait("before_commit", count, revision, None, &writer, &node) => {}
                }
                // Confirmed stop before the commit: the handler observes the
                // cancel signal and settles cancelled/not_started without
                // mutating the domain (contract A05/A06).
                if context.cancellation.is_cancelled() {
                    return Err(SdkError::new(
                        p::ErrorCode::Cancelled,
                        "add cancelled before commit",
                        "not_started",
                        "never",
                    ));
                }
                // One lock scope: commit and observer broadcast are atomic.
                let outcome = {
                    let mut d = domain.lock().unwrap();
                    d.add(input.delta)
                };
                match outcome {
                    Ok((result, revision)) => {
                        after
                            .wait("after_commit", result.count, revision, None, &writer, &node)
                            .await;
                        Ok(result)
                    }
                    Err(error) => Err(error),
                }
            }
        }
    });

    // Snapshot capture + observer registration are atomic under the domain
    // lock; the "snapshot" gate holds the lease (and thus the subscribe
    // response) after attaching, so updates published meanwhile queue in the
    // SDK and are only delivered after the snapshot response.
    let value_state =
        State::new("conex.CounterReadInput", "conex.CounterStateValue").with_handler({
            let domain = domain.clone();
            let snapshot_gate = snapshot_gate.clone();
            let writer = writer.clone();
            let node = endpoint.clone();
            move |_context: InvocationContext, _input: Value, observer: StateObserver| {
                let domain = domain.clone();
                let snapshot_gate = snapshot_gate.clone();
                let writer = writer.clone();
                let node = node.clone();
                async move {
                    let (sample, id, count, revision) = {
                        let mut d = domain.lock().unwrap();
                        let sample = d.sample();
                        let id = d.next_observer;
                        d.next_observer += 1;
                        d.observers.push((id, observer));
                        (sample, id, d.count, d.revision)
                    };
                    snapshot_gate
                        .wait("snapshot", count, revision, None, &writer, &node)
                        .await;
                    let cleanup_domain = domain.clone();
                    Ok(StateLease::new(sample, move || {
                        let mut d = cleanup_domain.lock().unwrap();
                        d.observers.retain(|(registered, _)| *registered != id);
                    }))
                }
            }
        });

    let contract = Arc::new(
        Contract::new(
            CONTRACT_ID,
            BTreeMap::from([
                ("counter/read".to_string(), read),
                ("counter/add".to_string(), add),
            ]),
            BTreeMap::from([("counter/value".to_string(), value_state)]),
        )
        .expect("contract installs"),
    );

    let mut connect = ConnectOptions::new(&url, &token);
    connect.allow_loopback_ws = true;
    let client = match Client::connect(connect).await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("connect error: {error}");
            std::process::exit(2);
        }
    };

    // Optional pre-registration scan: seed the discovery cache and the local
    // exposure principal set from the provider endpoints currently visible.
    let discovered: Arc<Mutex<BTreeMap<String, p::ModuleInfo>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let scan_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(list) = client.list_modules(space.clone()).await {
            let mut cache = discovered.lock().unwrap();
            for info in &list.modules {
                if info.contract_id == CONTRACT_ID {
                    cache.insert(info.provider_endpoint_id.clone(), info.clone());
                }
            }
            if !cache.is_empty() || Instant::now() >= scan_deadline {
                break;
            }
        } else if Instant::now() >= scan_deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let mut principals: BTreeSet<String> = discovered.lock().unwrap().keys().cloned().collect();
    principals.insert(endpoint.clone());
    let exposure = Exposure {
        principals,
        tenants: BTreeSet::from([space.clone()]),
        methods: BTreeSet::from(["counter/read".into(), "counter/add".into()]),
        states: BTreeSet::from(["counter/value".into()]),
    };

    let registered = match client
        .provide(
            p::ModuleRegisterInput {
                space_id: space.clone(),
                integration_id: integration.clone(),
                provider_endpoint_id: endpoint.clone(),
                contract_id: CONTRACT_ID.into(),
            },
            contract.clone(),
            exposure,
        )
        .await
    {
        Ok(registered) => registered,
        Err(error) => {
            eprintln!("publish error: {error}");
            std::process::exit(2);
        }
    };
    let reference = serde_json::to_value(&registered).unwrap_or(Value::Null);
    emit(&writer, &endpoint, "ready", json!({"reference": reference}));

    let active_calls: Arc<Mutex<BTreeMap<String, CallControl>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    type UnsubscribeReply = oneshot::Sender<conex_sdk::Result<p::StateUnsubscribeResult>>;
    let subs: Arc<Mutex<BTreeMap<String, mpsc::UnboundedSender<UnsubscribeReply>>>> =
        Arc::new(Mutex::new(BTreeMap::new()));

    // Dedicated std reader thread: tokio has no io-std; the main loop stays
    // free to serve cancel/unsubscribe while long-running ops are spawned.
    let (stdin_tx, mut stdin_rx) = mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.lock().read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim().to_string();
                    if !trimmed.is_empty() {
                        let _ = stdin_tx.send(trimmed);
                    }
                }
                Err(_) => break,
            }
        }
    });

    while let Some(line) = stdin_rx.recv().await {
        let req: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                eprintln!("fixture: bad JSONL: {error}");
                continue;
            }
        };
        let id = req
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let Some(op) = req.get("op").and_then(Value::as_str) else {
            respond_err(&writer, &id, &bad_request("missing op"));
            continue;
        };
        match op {
            "list" => {
                let space_id = req
                    .get("spaceId")
                    .and_then(Value::as_str)
                    .unwrap_or(&space)
                    .to_string();
                match client.list_modules(space_id.clone()).await {
                    Ok(list) => {
                        let mut cache = discovered.lock().unwrap();
                        for info in &list.modules {
                            if info.contract_id == CONTRACT_ID {
                                cache.insert(info.provider_endpoint_id.clone(), info.clone());
                            }
                        }
                        drop(cache);
                        let modules: Result<Vec<Value>, _> =
                            list.modules.iter().map(serde_json::to_value).collect();
                        match modules {
                            Ok(modules) => respond_ok(&writer, &id, json!({"modules": modules})),
                            Err(_) => respond_err(
                                &writer,
                                &id,
                                &bad_request("list result does not encode"),
                            ),
                        }
                    }
                    Err(error) => respond_err(&writer, &id, &error),
                }
            }
            "invoke" => {
                let call_id = match require_str(&req, "callId") {
                    Ok(v) => v,
                    Err(error) => {
                        respond_err(&writer, &id, &error);
                        continue;
                    }
                };
                let Some(target) = req.get("target").cloned() else {
                    respond_err(&writer, &id, &bad_request("missing field target"));
                    continue;
                };
                let method = match require_str(&req, "method") {
                    Ok(v) => v,
                    Err(error) => {
                        respond_err(&writer, &id, &error);
                        continue;
                    }
                };
                let argument = req.get("argument").cloned().unwrap_or(Value::Null);
                let timeout_ms = req
                    .get("timeoutMs")
                    .and_then(Value::as_u64)
                    .filter(|ms| *ms > 0);
                let execution = contract
                    .methods
                    .get(&method)
                    .map(|m| m.execution)
                    .unwrap_or(Execution::NonReplayable);
                match resolve_target_value(&client, &discovered, &space, &target).await {
                    Ok(module) => {
                        match client.invoke(module, contract.clone(), &method, argument) {
                            Ok(call) => {
                                let control = call.control.clone();
                                let request_id = control.request_id.clone();
                                active_calls
                                    .lock()
                                    .unwrap()
                                    .insert(call_id.clone(), control);
                                emit(
                                    &writer,
                                    &endpoint,
                                    "call_started",
                                    json!({"callId": call_id, "requestId": request_id}),
                                );
                                let active = active_calls.clone();
                                let writer = writer.clone();
                                let reply_id = id;
                                let call_id_for_task = call_id.clone();
                                tokio::spawn(async move {
                                    let outcome = if let Some(ms) = timeout_ms {
                                        match tokio::time::timeout(
                                            Duration::from_millis(ms),
                                            call.result::<Value>(),
                                        )
                                        .await
                                        {
                                            Ok(result) => result,
                                            Err(_) => Err(SdkError::new(
                                                p::ErrorCode::Timeout,
                                                "invoke timeout",
                                                "unknown",
                                                if matches!(execution, Execution::ReadOnly) {
                                                    "safe"
                                                } else {
                                                    "never"
                                                },
                                            )),
                                        }
                                    } else {
                                        call.result::<Value>().await
                                    };
                                    active.lock().unwrap().remove(&call_id_for_task);
                                    match outcome {
                                        Ok(value) => respond_ok(&writer, &reply_id, value),
                                        Err(error) => respond_err(&writer, &reply_id, &error),
                                    }
                                });
                            }
                            Err(error) => respond_err(&writer, &id, &error),
                        }
                    }
                    Err(error) => respond_err(&writer, &id, &error),
                }
            }
            "cancel" => {
                let call_id = match require_str(&req, "callId") {
                    Ok(v) => v,
                    Err(error) => {
                        respond_err(&writer, &id, &error);
                        continue;
                    }
                };
                let control = active_calls.lock().unwrap().get(&call_id).cloned();
                match control {
                    Some(control) => match control.cancel().await {
                        Ok(result) => {
                            respond_ok(&writer, &id, json!({"requested": result.requested}))
                        }
                        Err(error) => respond_err(&writer, &id, &error),
                    },
                    None => respond_ok(&writer, &id, json!({"requested": false})),
                }
            }
            "subscribe" => {
                let sub_key = match require_str(&req, "subscriptionKey") {
                    Ok(v) => v,
                    Err(error) => {
                        respond_err(&writer, &id, &error);
                        continue;
                    }
                };
                let Some(target) = req.get("target").cloned() else {
                    respond_err(&writer, &id, &bad_request("missing field target"));
                    continue;
                };
                let state = req
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("counter/value")
                    .to_string();
                let argument = req.get("argument").cloned().unwrap_or(Value::Null);
                match resolve_target_value(&client, &discovered, &space, &target).await {
                    Ok(module) => {
                        match client.subscribe(module, contract.clone(), &state, argument) {
                            Ok(subscription) => {
                                let subscription_id = subscription.id.clone();
                                let (handle_tx, mut handle_rx) = mpsc::unbounded_channel();
                                subs.lock().unwrap().insert(sub_key.clone(), handle_tx);
                                let writer = writer.clone();
                                let writer_events = writer.clone();
                                let node = endpoint.clone();
                                let reply_id = id;
                                let event_key = sub_key.clone();
                                tokio::spawn(async move {
                                    let mut subscription: Subscription = subscription;
                                    let snapshot = match subscription.next().await {
                                        Some(StateEvent::Snapshot(sample)) => sample,
                                        Some(StateEvent::End(error)) => {
                                            respond_err(
                                                &writer,
                                                &reply_id,
                                                &bad_request(format!(
                                                    "subscribe ended before snapshot: {error}"
                                                )),
                                            );
                                            return;
                                        }
                                        other => {
                                            respond_err(
                                                &writer,
                                                &reply_id,
                                                &bad_request(format!(
                                                    "expected snapshot, got {other:?}"
                                                )),
                                            );
                                            return;
                                        }
                                    };
                                    let snapshot_json = p::capability::state_sample_json(&snapshot)
                                        .expect("snapshot encodes");
                                    respond_ok(
                                        &writer,
                                        &reply_id,
                                        json!({"snapshot": snapshot_json, "subscriptionId": subscription_id}),
                                    );
                                    loop {
                                        tokio::select! {
                                            event = subscription.next() => match event {
                                                Some(StateEvent::Update { seq, sample }) => {
                                                    let sample_json =
                                                        p::capability::state_sample_json(&sample)
                                                            .expect("sample encodes");
                                                    let mut fields = Map::new();
                                                    fields.insert("subscriptionKey".into(), Value::String(event_key.clone()));
                                                    fields.insert("seq".into(), json!(seq));
                                                    fields.insert("sample".into(), sample_json);
                                                    emit(&writer_events, &node, "update", Value::Object(fields));
                                                }
                                                Some(StateEvent::End(error)) => {
                                                    let mut fields = Map::new();
                                                    fields.insert("subscriptionKey".into(), Value::String(event_key.clone()));
                                                    fields.insert("error".into(), response_error(&error));
                                                    emit(&writer_events, &node, "end", Value::Object(fields));
                                                    break;
                                                }
                                                Some(StateEvent::Snapshot(_)) => {}
                                                None => {
                                                    let closed = SdkError::new(
                                                        p::ErrorCode::Unavailable,
                                                        "subscription closed",
                                                        "completed",
                                                        "safe",
                                                    );
                                                    let mut fields = Map::new();
                                                    fields.insert("subscriptionKey".into(), Value::String(event_key.clone()));
                                                    fields.insert("error".into(), response_error(&closed));
                                                    emit(&writer_events, &node, "end", Value::Object(fields));
                                                    break;
                                                }
                                            },
                                            request = handle_rx.recv() => match request {
                                                Some(reply) => {
                                                    let _ = reply.send(subscription.unsubscribe().await);
                                                    break;
                                                }
                                                None => break,
                                            },
                                        }
                                    }
                                });
                            }
                            Err(error) => respond_err(&writer, &id, &error),
                        }
                    }
                    Err(error) => respond_err(&writer, &id, &error),
                }
            }
            "unsubscribe" => {
                let sub_key = match require_str(&req, "subscriptionKey") {
                    Ok(v) => v,
                    Err(error) => {
                        respond_err(&writer, &id, &error);
                        continue;
                    }
                };
                let sender = subs.lock().unwrap().remove(&sub_key);
                match sender {
                    Some(sender) => {
                        let (reply_tx, reply_rx) = oneshot::channel();
                        if sender.send(reply_tx).is_err() {
                            respond_ok(&writer, &id, json!({"removed": false}));
                        } else {
                            match reply_rx.await {
                                Ok(Ok(result)) => {
                                    respond_ok(&writer, &id, json!({"removed": result.removed}))
                                }
                                Ok(Err(error)) => respond_err(&writer, &id, &error),
                                Err(_) => respond_ok(&writer, &id, json!({"removed": false})),
                            }
                        }
                    }
                    None => respond_ok(&writer, &id, json!({"removed": false})),
                }
            }
            "arm" => {
                let point = match require_str(&req, "point") {
                    Ok(v) => v,
                    Err(error) => {
                        respond_err(&writer, &id, &error);
                        continue;
                    }
                };
                match point.as_str() {
                    "before_commit" => before_commit.arm(),
                    "after_commit" => after_commit.arm(),
                    "snapshot" => snapshot_gate.arm(),
                    _ => {
                        respond_err(&writer, &id, &bad_request("unknown barrier point"));
                        continue;
                    }
                }
                respond_ok(&writer, &id, json!({"armed": point}));
            }
            "release" => {
                let point = match require_str(&req, "point") {
                    Ok(v) => v,
                    Err(error) => {
                        respond_err(&writer, &id, &error);
                        continue;
                    }
                };
                match point.as_str() {
                    "before_commit" => before_commit.release(),
                    "after_commit" => after_commit.release(),
                    "snapshot" => snapshot_gate.release(),
                    _ => {
                        respond_err(&writer, &id, &bad_request("unknown barrier point"));
                        continue;
                    }
                }
                respond_ok(&writer, &id, json!({"released": point}));
            }
            "mutate" => {
                let delta_i64 = req.get("delta").and_then(Value::as_i64).unwrap_or(0);
                let Some(delta) = i32::try_from(delta_i64).ok() else {
                    respond_err(&writer, &id, &bad_request("delta outside int32 range"));
                    continue;
                };
                // Spawned like invoke: a gated mutate must not block the main
                // loop, or the release command could never be read.
                let domain = domain.clone();
                let before = before_commit.clone();
                let after = after_commit.clone();
                let writer = writer.clone();
                let node = endpoint.clone();
                let reply_id = id;
                tokio::spawn(async move {
                    let (count, revision) = {
                        let d = domain.lock().unwrap();
                        (d.count, d.revision)
                    };
                    before
                        .wait(
                            "before_commit",
                            count,
                            revision,
                            Some(&reply_id),
                            &writer,
                            &node,
                        )
                        .await;
                    let outcome = {
                        let mut d = domain.lock().unwrap();
                        d.add(delta)
                    };
                    match outcome {
                        Ok((result, revision)) => {
                            after
                                .wait(
                                    "after_commit",
                                    result.count,
                                    revision,
                                    Some(&reply_id),
                                    &writer,
                                    &node,
                                )
                                .await;
                            respond_ok(&writer, &reply_id, json!(result));
                        }
                        Err(error) => respond_err(&writer, &reply_id, &error),
                    }
                });
            }
            "inspect" => {
                let d = domain.lock().unwrap();
                respond_ok(
                    &writer,
                    &id,
                    json!({
                        "count": d.count,
                        "revision": d.revision.to_string(),
                        "executions": d.executions,
                        "observers": d.observers.len(),
                    }),
                );
            }
            "close" => {
                client.close().await;
                respond_ok(&writer, &id, Value::Bool(true));
            }
            "shutdown" => {
                // The last response is acknowledged by the writer task before
                // the runtime exits, so its delivery is deterministic.
                let line = json!({"id": id, "ok": true, "value": true}).to_string();
                let (ack_tx, ack_rx) = oneshot::channel();
                let _ = writer.send((line, Some(ack_tx)));
                let _ = ack_rx.await;
                client.close().await;
                std::process::exit(0);
            }
            other => {
                respond_err(&writer, &id, &bad_request(format!("unknown op {other}")));
            }
        }
    }
    // stdin EOF: clean close and exit.
    client.close().await;
    std::process::exit(0);
}

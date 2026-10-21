//! Real Rust plugin instance (instance A) for the S2 dual-instance sample
//! (docs/contracts/s2-plugins.md §5–§7; plan 2026-10-19-conex-s2.md §6 U03).
//!
//! Same plugin design as examples/s2/plugin-node.ts: module `counter` (S1
//! counter semantics: int32 overflow rejected before mutation, delta=0 still
//! advances the revision) plus module `derived-counter` (same contract
//! republished under its own 4-tuple: an ×2 view of its upstream with its own
//! revision sequence, upstream snapshot captured before anything is published).
//!
//! This process runs the derived-counter in LOCAL mode: its upstream is the
//! same instance's counter service handle via `Deps::local` — a process-local
//! call, zero hub frames. The remote-mode branch (deps.remote bound to a peer
//! ModuleRef) exists for symmetry and is what the TS instance exercises.
//!
//! Emits structured one-line JSON events; never prints the token.
//!
//! Env:
//!   CONEX_URL            ws://127.0.0.1:PORT/wss
//!   CONEX_TOKEN          bearer matching the host [[tokens]] digest (s2-rust)
//!   CONEX_SPACE          space id (dev)
//!   CONEX_ENDPOINT       own endpoint prefix (s2-rust)
//!   CONEX_PEER_ENDPOINT  peer endpoint prefix (s2-ts)

use conex_sdk::proto as p;
use conex_sdk::{
    Client, ConnectOptions, Contract, Error as SdkError, Execution, Instance, InstanceConfig,
    InvocationContext, Method, ModuleContext, ModuleDecl, ModuleRef, Plugin, StartedModule, State,
    StateEvent, StateLease, StateObserver, Subscription,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::watch;

const CONTRACT_ID: &str = "conex.counter";
const INTEGRATION_ID: &str = "integration";
const DERIVED_TARGET: &str = "counter/value";
const READ_METHOD: &str = "counter/read";
const ADD_METHOD: &str = "counter/add";

#[derive(Deserialize)]
struct AddInput {
    delta: i32,
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        eprintln!("missing {name}");
        std::process::exit(2);
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn emit(event: &str, node: &str, fields: Value) {
    let mut line = match fields {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };
    line.insert("event".into(), event.into());
    line.insert("node".into(), node.into());
    println!("{}", Value::Object(line));
}

/// ×2 view with the derived's own int32 ceiling; never silently wraps.
fn doubled(count: i32) -> Result<i32, SdkError> {
    count.checked_mul(2).ok_or_else(|| {
        SdkError::new(
            p::ErrorCode::BadRequest,
            "derived ×2 view overflows int32",
            "not_started",
            "never",
        )
    })
}

fn sample(count: i32, revision: u64) -> p::StateSample {
    p::StateSample {
        revision,
        value: Some(serde_json::from_value(json!({"count": count})).expect("counter value")),
        sampled_at_ms: now_ms(),
        quality: "current".into(),
    }
}

fn sample_fields(sample: &p::StateSample) -> (String, i64) {
    let json = p::capability::state_sample_json(sample).expect("sample encodes");
    let revision = json["revision"].as_str().unwrap_or("?").to_string();
    let count = json["value"]["count"].as_i64().unwrap_or(-1);
    (revision, count)
}

fn result_json(count: i32, revision: u64) -> Value {
    json!({"count": count, "revision": revision.to_string()})
}

async fn next_event(subscription: &mut Subscription, what: &str) -> Result<StateEvent, SdkError> {
    match tokio::time::timeout(Duration::from_secs(10), subscription.next()).await {
        Ok(Some(event)) => Ok(event),
        Ok(None) => Err(SdkError::new(
            p::ErrorCode::Unavailable,
            format!("subscription closed while waiting for {what}"),
            "completed",
            "safe",
        )),
        Err(_) => Err(SdkError::new(
            p::ErrorCode::Timeout,
            format!("timed out waiting for {what}"),
            "unknown",
            "safe",
        )),
    }
}

// ---- counter module --------------------------------------------------------

/// Counter domain: committed (count, revision) plus a watch fan-out so the
/// derived module observes through a process-local handle.
struct CounterDomain {
    state: Mutex<(i32, u64)>,
    tx: watch::Sender<(i32, u64)>,
}

impl CounterDomain {
    fn new() -> Arc<Self> {
        let (tx, _) = watch::channel((0, 0));
        Arc::new(Self {
            state: Mutex::new((0, 0)),
            tx,
        })
    }
    fn read(&self) -> (i32, u64) {
        *self.state.lock().unwrap()
    }
    /// Snapshot and watch subscription are taken under the SAME lock the add
    /// path commits under, so a snapshot never misses a committed update.
    fn subscribe(&self) -> (watch::Receiver<(i32, u64)>, p::StateSample) {
        let guard = self.state.lock().unwrap();
        let rx = self.tx.subscribe();
        let (count, revision) = *guard;
        (rx, sample(count, revision))
    }
    fn add(&self, delta: i32) -> Result<(i32, u64), SdkError> {
        let mut guard = self.state.lock().unwrap();
        let count = guard.0.checked_add(delta).ok_or_else(|| {
            SdkError::new(
                p::ErrorCode::BadRequest,
                "int32 overflow",
                "not_started",
                "never",
            )
        })?;
        *guard = (count, guard.1 + 1); // delta = 0 also advances the revision
        let value = *guard;
        self.tx.send_replace(value);
        Ok(value)
    }
}

/// Provider-side forwarder for the counter's own state.
async fn forward(mut rx: watch::Receiver<(i32, u64)>, observer: StateObserver) {
    loop {
        if rx.changed().await.is_err() {
            return;
        }
        let (count, revision) = *rx.borrow_and_update();
        let _ = observer.publish(sample(count, revision));
    }
}

fn counter_module() -> ModuleDecl {
    ModuleDecl::new("counter", vec![], vec![], |_| {
        Box::pin(async {
            let domain = CounterDomain::new();
            let (d_read, d_add, d_state) = (domain.clone(), domain.clone(), domain.clone());
            let read = Method::new(
                "conex.CounterReadInput",
                "conex.CounterResult",
                Execution::ReadOnly,
            )
            .with_handler(move |_: InvocationContext, _: Value| {
                let domain = d_read.clone();
                async move {
                    let (count, revision) = domain.read();
                    Ok(result_json(count, revision))
                }
            });
            let add = Method::new(
                "conex.CounterAddInput",
                "conex.CounterResult",
                Execution::NonReplayable,
            )
            .with_handler(move |_: InvocationContext, input: AddInput| {
                let domain = d_add.clone();
                async move {
                    let (count, revision) = domain.add(input.delta)?;
                    Ok(result_json(count, revision))
                }
            });
            let value = State::new("conex.CounterReadInput", "conex.CounterStateValue")
                .with_handler(
                    move |_: InvocationContext, _: Value, observer: StateObserver| {
                        let domain = d_state.clone();
                        async move {
                            // Snapshot and watch attach share the domain lock; the
                            // lease opening is the subscription's publish point.
                            let (rx, snapshot) = domain.subscribe();
                            let task = tokio::spawn(forward(rx, observer));
                            Ok(StateLease::new(snapshot, move || task.abort()))
                        }
                    },
                );
            let contract = Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::from([
                    (READ_METHOD.to_string(), read),
                    (ADD_METHOD.to_string(), add),
                ]),
                BTreeMap::from([(DERIVED_TARGET.to_string(), value)]),
            )?);
            Ok(StartedModule {
                contract,
                service: Some(domain),
            })
        })
    })
}

// ---- derived-counter module (local mode) ------------------------------------

/// Derived ×2 view: own revision sequence, own observer fan-out. The upstream
/// snapshot is committed in the start factory (before any lease), so the
/// derived boundary keeps snapshot-then-updates independently (先快照后发布).
struct DerivedState {
    inner: Mutex<DerivedInner>,
}
struct DerivedInner {
    count: i32,
    revision: u64,
    upstream_seen: u64,
    next_observer: u64,
    observers: Vec<(u64, StateObserver)>,
}

impl DerivedState {
    fn new(upstream_count: i32, upstream_revision: u64) -> Result<Arc<Self>, SdkError> {
        Ok(Arc::new(Self {
            inner: Mutex::new(DerivedInner {
                count: doubled(upstream_count)?,
                revision: 1,
                upstream_seen: upstream_revision,
                next_observer: 0,
                observers: Vec::new(),
            }),
        }))
    }
    fn on_upstream(self: &Arc<Self>, upstream: (i32, u64)) {
        let count = match doubled(upstream.0) {
            Ok(count) => count,
            Err(_) => return,
        };
        let mut d = self.inner.lock().unwrap();
        if upstream.1 <= d.upstream_seen {
            return;
        }
        d.upstream_seen = upstream.1;
        d.count = count;
        d.revision += 1;
        let s = sample(d.count, d.revision);
        d.observers
            .retain(|(_, observer)| observer.publish(s.clone()).is_ok());
    }
    fn view(&self) -> (i32, u64) {
        let d = self.inner.lock().unwrap();
        (d.count, d.revision)
    }
    /// Snapshot and observer attach share one lock: no observer can miss an
    /// update committed after the snapshot it received.
    fn subscribe(self: &Arc<Self>, observer: StateObserver) -> (p::StateSample, u64) {
        let mut d = self.inner.lock().unwrap();
        let snapshot = sample(d.count, d.revision);
        let id = d.next_observer;
        d.next_observer += 1;
        d.observers.push((id, observer));
        (snapshot, id)
    }
    fn detach(&self, id: u64) {
        self.inner
            .lock()
            .unwrap()
            .observers
            .retain(|(registered, _)| *registered != id);
    }
}

async fn forward_derived(mut rx: watch::Receiver<(i32, u64)>, derived: Arc<DerivedState>) {
    loop {
        if rx.changed().await.is_err() {
            return;
        }
        derived.on_upstream(*rx.borrow_and_update());
    }
}

fn derived_module() -> ModuleDecl {
    ModuleDecl::new(
        "derived-counter",
        vec!["counter".into()],
        vec![],
        |context: ModuleContext| {
            Box::pin(async move {
                let counter = context
                    .deps
                    .local::<CounterDomain>("counter")
                    .ok_or_else(|| {
                        SdkError::new(
                            p::ErrorCode::BadRequest,
                            "counter service handle missing",
                            "not_started",
                            "never",
                        )
                    })?;
                // 先快照后发布: capture the upstream snapshot before this module
                // can serve any state subscription; the derived revision sequence
                // starts independently at 1.
                let (rx, upstream) = counter.subscribe();
                let (_, upstream_count) = sample_fields(&upstream);
                let upstream_count = i32::try_from(upstream_count).map_err(|_| {
                    SdkError::new(
                        p::ErrorCode::BadRequest,
                        "upstream count out of int32 range",
                        "not_started",
                        "never",
                    )
                })?;
                let derived = DerivedState::new(upstream_count, upstream.revision)?;
                tokio::spawn(forward_derived(rx, derived.clone()));
                let (d_read, d_add, d_state) = (derived.clone(), counter.clone(), derived.clone());
                let read = Method::new(
                    "conex.CounterReadInput",
                    "conex.CounterResult",
                    Execution::ReadOnly,
                )
                .with_handler(move |_: InvocationContext, _: Value| {
                    let derived = d_read.clone();
                    async move {
                        let (count, revision) = derived.view();
                        Ok(result_json(count, revision))
                    }
                });
                // add is FORWARDED to the upstream counter — a process-local call
                // in local mode (zero frames); the ×2 view follows via the watch.
                let add = Method::new(
                    "conex.CounterAddInput",
                    "conex.CounterResult",
                    Execution::NonReplayable,
                )
                .with_handler(move |_: InvocationContext, input: AddInput| {
                    let counter = d_add.clone();
                    async move {
                        let (count, revision) = counter.add(input.delta)?;
                        Ok(result_json(count, revision))
                    }
                });
                let value = State::new("conex.CounterReadInput", "conex.CounterStateValue")
                    .with_handler(
                        move |_: InvocationContext, _: Value, observer: StateObserver| {
                            let derived = d_state.clone();
                            async move {
                                let (snapshot, observer_id) = derived.subscribe(observer);
                                let cleanup = derived.clone();
                                Ok(StateLease::new(snapshot, move || {
                                    cleanup.detach(observer_id)
                                }))
                            }
                        },
                    );
                let contract = Arc::new(Contract::new(
                    CONTRACT_ID,
                    BTreeMap::from([
                        (READ_METHOD.to_string(), read),
                        (ADD_METHOD.to_string(), add),
                    ]),
                    BTreeMap::from([(DERIVED_TARGET.to_string(), value)]),
                )?);
                Ok(StartedModule {
                    contract,
                    service: None,
                })
            })
        },
    )
}

// ---- consumer helpers -------------------------------------------------------

/// Consumer-side contract: descriptors only, no handlers.
fn consumer_contract() -> Arc<Contract> {
    Arc::new(
        Contract::new(
            CONTRACT_ID,
            BTreeMap::from([
                (
                    READ_METHOD.to_string(),
                    Method::new(
                        "conex.CounterReadInput",
                        "conex.CounterResult",
                        Execution::ReadOnly,
                    ),
                ),
                (
                    ADD_METHOD.to_string(),
                    Method::new(
                        "conex.CounterAddInput",
                        "conex.CounterResult",
                        Execution::NonReplayable,
                    ),
                ),
            ]),
            BTreeMap::from([(
                DERIVED_TARGET.to_string(),
                State::new("conex.CounterReadInput", "conex.CounterStateValue"),
            )]),
        )
        .expect("consumer contract"),
    )
}

async fn ref_of(client: &Client, space: &str, endpoint: &str) -> Result<ModuleRef, SdkError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(list) = client.list_modules(space.to_string()).await
            && let Some(info) = list.modules.iter().find(|module| {
                module.provider_endpoint_id == endpoint && module.contract_id == CONTRACT_ID
            })
        {
            return Ok(ModuleRef::from(info));
        }
        if Instant::now() >= deadline {
            return Err(SdkError::new(
                p::ErrorCode::Unavailable,
                format!("endpoint {endpoint} not registered within 10s"),
                "not_started",
                "safe",
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn read_via(client: &Client, target: &ModuleRef) -> Result<Value, SdkError> {
    client
        .invoke(target.clone(), consumer_contract(), READ_METHOD, json!({}))?
        .result::<Value>()
        .await
}

async fn add_via(client: &Client, target: &ModuleRef, delta: i32) -> Result<Value, SdkError> {
    client
        .invoke(
            target.clone(),
            consumer_contract(),
            ADD_METHOD,
            json!({"delta": delta}),
        )?
        .result::<Value>()
        .await
}

fn count_of_value(value: &Value) -> i64 {
    value["count"].as_i64().unwrap_or(i64::MIN)
}

/// Cross-process phase gate: poll a counter read until `count >= min`.
async fn wait_count(
    client: &Client,
    target: &ModuleRef,
    min: i64,
    what: &str,
) -> Result<Value, SdkError> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let value = read_via(client, target).await?;
        if count_of_value(&value) >= min {
            return Ok(value);
        }
        if Instant::now() >= deadline {
            return Err(SdkError::new(
                p::ErrorCode::Timeout,
                format!("timed out waiting for {what}"),
                "unknown",
                "safe",
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits until every `peer` endpoint has left the hub (the TS instance
/// stopped and unpublished) — the observable start of the stop-isolation check.
async fn wait_peer_gone(client: &Client, space: &str, peer: &str) -> Result<(), SdkError> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let mut gone = false;
        if let Ok(list) = client.list_modules(space.to_string()).await {
            gone = !list
                .modules
                .iter()
                .any(|module| module.provider_endpoint_id.starts_with(peer));
        }
        if gone {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(SdkError::new(
                p::ErrorCode::Timeout,
                format!("peer {peer} endpoints still registered after 60s"),
                "unknown",
                "safe",
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

// ---- main --------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let url = env("CONEX_URL");
    let token = env("CONEX_TOKEN");
    let space = env("CONEX_SPACE");
    let endpoint = env("CONEX_ENDPOINT");
    let peer = env("CONEX_PEER_ENDPOINT");

    emit(
        "start",
        &endpoint,
        json!({"space": space, "peer": peer, "derivedMode": "local"}),
    );

    let plugin = Plugin::new("counter-plugin", vec![counter_module(), derived_module()]);
    let instance = Instance::new(
        plugin,
        InstanceConfig {
            url: url.clone(),
            bearer: token.clone(),
            allow_loopback_ws: true,
            space_id: space.clone(),
            integration_id: INTEGRATION_ID.into(),
            endpoint_prefix: endpoint.clone(),
            bindings: Vec::new(),
            // Local exposure ceiling: the two sample principals.
            exposure_principals: BTreeSet::from([endpoint.clone(), peer.clone()]),
            exposure_tenants: BTreeSet::from([space.clone()]),
        },
    );
    // A graceful signal walks the same stop path (idempotent with main's stop);
    // the task holds its own clone so main keeps driving the instance.
    let signal_instance = instance.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
        let _ = signal_instance.stop().await;
        std::process::exit(0);
    });

    instance.start().await?;
    emit(
        "publish",
        &endpoint,
        json!({"providerEndpointId": format!("{endpoint}-counter")}),
    );
    emit(
        "publish",
        &endpoint,
        json!({"providerEndpointId": format!("{endpoint}-derived-counter")}),
    );

    let mut options = ConnectOptions::new(&url, &token);
    options.allow_loopback_ws = true;
    let consumer = Client::connect(options).await?;
    emit("ready", &endpoint, json!({}));

    // ---- A02: local derivation with zero hub frames ---------------------------
    let own_counter = ref_of(&consumer, &space, &format!("{endpoint}-counter")).await?;
    let own_derived = ref_of(&consumer, &space, &format!("{endpoint}-derived-counter")).await?;
    let mut derived_sub = consumer.subscribe(
        own_derived.clone(),
        consumer_contract(),
        DERIVED_TARGET,
        json!({}),
    )?;
    let event = next_event(&mut derived_sub, "own derived snapshot").await?;
    match &event {
        StateEvent::Snapshot(s) => {
            let (revision, count) = sample_fields(s);
            emit(
                "snapshot",
                &endpoint,
                json!({"target": format!("{endpoint}-derived-counter"), "revision": revision, "count": count}),
            );
        }
        other => {
            emit(
                "error",
                &endpoint,
                json!({"message": format!("expected snapshot, got {other:?}")}),
            );
            std::process::exit(1);
        }
    }

    let added = add_via(&consumer, &own_counter, 5).await?;
    emit(
        "add",
        &endpoint,
        json!({"target": format!("{endpoint}-counter"), "delta": 5, "count": added["count"], "revision": added["revision"]}),
    );
    // The ×2 view advanced through the in-process handle, not the hub (A02).
    let event = next_event(&mut derived_sub, "own derived update").await?;
    match &event {
        StateEvent::Update { seq, sample } => {
            let (revision, count) = sample_fields(sample);
            emit(
                "update",
                &endpoint,
                json!({"target": format!("{endpoint}-derived-counter"), "seq": seq, "revision": revision, "count": count}),
            );
        }
        other => {
            emit(
                "error",
                &endpoint,
                json!({"message": format!("expected update, got {other:?}")}),
            );
            std::process::exit(1);
        }
    }
    let view = read_via(&consumer, &own_derived).await?;
    emit(
        "read",
        &endpoint,
        json!({"target": format!("{endpoint}-derived-counter"), "count": view["count"], "revision": view["revision"]}),
    );
    emit(
        "phase-done",
        &endpoint,
        json!({"phase": "A02-local-derivation"}),
    );

    // ---- A01: two instances, independent counters and revisions ---------------
    let peer_counter = ref_of(&consumer, &space, &format!("{peer}-counter")).await?;
    let peer_read = read_via(&consumer, &peer_counter).await?;
    emit(
        "read",
        &endpoint,
        json!({"target": format!("{peer}-counter"), "count": peer_read["count"], "revision": peer_read["revision"]}),
    );
    match add_via(&consumer, &own_counter, i32::MAX).await {
        Ok(result) => {
            emit(
                "error",
                &endpoint,
                json!({"message": format!("overflow add unexpectedly succeeded: {result}")}),
            );
            std::process::exit(1);
        }
        Err(error) => emit(
            "overflow-rejected",
            &endpoint,
            json!({
                "target": format!("{endpoint}-counter"),
                "code": error.code(),
                "execution": error.execution(),
                "retry": error.retry(),
                "message": error.to_string(),
            }),
        ),
    }
    let unchanged = read_via(&consumer, &own_counter).await?;
    emit(
        "read",
        &endpoint,
        json!({"target": format!("{endpoint}-counter"), "count": unchanged["count"], "revision": unchanged["revision"]}),
    );
    emit(
        "phase-done",
        &endpoint,
        json!({"phase": "A01-instance-independence"}),
    );

    // ---- A07: Rust consumes the TS instance's published derived module --------
    let peer_after_ts = wait_count(&consumer, &own_counter, 8, "ts binding add(+3)").await?;
    emit(
        "read",
        &endpoint,
        json!({"target": format!("{endpoint}-counter"), "count": peer_after_ts["count"], "revision": peer_after_ts["revision"], "gate": "ts add(+3) landed"}),
    );
    let peer_derived = ref_of(&consumer, &space, &format!("{peer}-derived-counter")).await?;
    let mut peer_sub = consumer.subscribe(
        peer_derived.clone(),
        consumer_contract(),
        DERIVED_TARGET,
        json!({}),
    )?;
    let event = next_event(&mut peer_sub, "peer derived snapshot").await?;
    match &event {
        StateEvent::Snapshot(s) => {
            let (revision, count) = sample_fields(s);
            emit(
                "snapshot",
                &endpoint,
                json!({"target": format!("{peer}-derived-counter"), "revision": revision, "count": count}),
            );
        }
        other => {
            emit(
                "error",
                &endpoint,
                json!({"message": format!("expected snapshot, got {other:?}")}),
            );
            std::process::exit(1);
        }
    }
    let forwarded = add_via(&consumer, &peer_derived, 4).await?;
    emit(
        "add",
        &endpoint,
        json!({"target": format!("{peer}-derived-counter"), "delta": 4, "forwardedCount": forwarded["count"], "forwardedRevision": forwarded["revision"]}),
    );
    loop {
        let event = next_event(&mut peer_sub, "peer derived update >= 24").await?;
        let StateEvent::Update { seq, sample } = &event else {
            emit(
                "error",
                &endpoint,
                json!({"message": format!("expected update, got {event:?}")}),
            );
            std::process::exit(1);
        };
        let (revision, count) = sample_fields(sample);
        emit(
            "update",
            &endpoint,
            json!({"target": format!("{peer}-derived-counter"), "seq": seq, "revision": revision, "count": count}),
        );
        if count >= 24 {
            break;
        }
    }
    let final_view = read_via(&consumer, &peer_derived).await?;
    emit(
        "read",
        &endpoint,
        json!({"target": format!("{peer}-derived-counter"), "count": final_view["count"], "revision": final_view["revision"]}),
    );
    emit(
        "phase-done",
        &endpoint,
        json!({"phase": "A07-rust-consumes-ts-derived"}),
    );

    // ---- stop isolation: the TS instance goes away, this one keeps serving ----
    emit("idle", &endpoint, json!({}));
    wait_peer_gone(&consumer, &space, &peer).await?;
    emit("peer-gone", &endpoint, json!({"peer": peer}));
    let survivor = read_via(&consumer, &own_counter).await?;
    emit(
        "read",
        &endpoint,
        json!({"target": format!("{endpoint}-counter"), "count": survivor["count"], "revision": survivor["revision"], "note": "peer stopped; still serving"}),
    );
    let bumped = add_via(&consumer, &own_counter, 1).await?;
    emit(
        "add",
        &endpoint,
        json!({"target": format!("{endpoint}-counter"), "delta": 1, "count": bumped["count"], "revision": bumped["revision"], "note": "peer stopped; still serving"}),
    );
    emit("stopping", &endpoint, json!({}));
    instance.stop().await;
    consumer.close().await;
    emit("done", &endpoint, json!({}));
    Ok(())
}

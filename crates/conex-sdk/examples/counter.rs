//! Real Rust counter node for the S1 two-process sample
//! (docs/plans/2026-10-17-conex-s1.md T07; docs/contracts/s1-capabilities.md).
//!
//! Provides `conex.counter` (counter/read, counter/add, state counter/value)
//! at CONEX_ENDPOINT and consumes the peer at CONEX_PEER_ENDPOINT over a
//! single conex-sdk connection. Emits structured one-line JSON events on
//! stdout for read/add/snapshot/update/termination; never prints the token.
//!
//! Env:
//!   CONEX_URL           ws://127.0.0.1:PORT/wss
//!   CONEX_TOKEN         bearer matching the host [[tokens]] digest
//!   CONEX_SPACE         space id (dev)
//!   CONEX_ENDPOINT      own provider endpoint id (counter-rust)
//!   CONEX_PEER_ENDPOINT peer endpoint id to consume (counter-ts)

use conex_sdk::proto as p;
use conex_sdk::{
    Client, ConnectOptions, Contract, Error as SdkError, Execution, Exposure, InvocationContext,
    Method, ModuleRef, State, StateEvent, StateLease, StateObserver, Subscription,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CONTRACT_ID: &str = "conex.counter";
const INTEGRATION_ID: &str = "integration";

#[derive(Deserialize)]
struct AddInput {
    delta: i32,
}

#[derive(Serialize, Deserialize, Debug)]
struct CounterResult {
    count: i32,
    revision: String,
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

/// Domain state. Every add and every snapshot registration take this lock, so
/// the committed mutation and the state broadcast share one serialization
/// boundary; a slow observer is ended by `StateObserver::publish`, never rolls
/// back the commit.
struct Domain {
    count: i32,
    revision: u64,
    next_observer: u64,
    observers: Vec<(u64, StateObserver)>,
}

impl Domain {
    fn new() -> Self {
        Self {
            count: 0,
            revision: 0,
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
    fn add(&mut self, delta: i32) -> Result<CounterResult, SdkError> {
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
        self.count = next;
        self.revision += 1; // delta = 0 also advances the revision
        let sample = self.sample();
        self.observers
            .retain(|(_, observer)| observer.publish(sample.clone()).is_ok());
        Ok(self.result())
    }
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

fn sample_fields(sample: &p::StateSample) -> (String, i64) {
    let json = p::capability::state_sample_json(sample).expect("sample encodes");
    let revision = json["revision"].as_str().unwrap_or("?").to_string();
    let count = json["value"]["count"].as_i64().unwrap_or(-1);
    (revision, count)
}

async fn next_update(subscription: &mut Subscription) -> Result<StateEvent, SdkError> {
    match tokio::time::timeout(Duration::from_secs(5), subscription.next()).await {
        Ok(Some(event)) => Ok(event),
        Ok(None) => Err(SdkError::new(
            p::ErrorCode::Unavailable,
            "subscription closed",
            "completed",
            "safe",
        )),
        Err(_) => Err(SdkError::new(
            p::ErrorCode::Timeout,
            "update timeout",
            "unknown",
            "safe",
        )),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let url = env("CONEX_URL");
    let token = env("CONEX_TOKEN");
    let space = env("CONEX_SPACE");
    let endpoint = env("CONEX_ENDPOINT");
    let peer = env("CONEX_PEER_ENDPOINT");

    emit("start", &endpoint, json!({"space": space, "peer": peer}));

    let domain = Arc::new(Mutex::new(Domain::new()));

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
                let d = domain.lock().expect("domain poisoned");
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
        move |_context: InvocationContext, input: AddInput| {
            let domain = domain.clone();
            async move {
                let mut d = domain.lock().expect("domain poisoned");
                d.add(input.delta)
            }
        }
    });

    // Snapshot is captured and the observer registered under the same domain
    // lock; the lease removes the observer on drop.
    let value_state =
        State::new("conex.CounterReadInput", "conex.CounterStateValue").with_handler({
            let domain = domain.clone();
            move |_context: InvocationContext, _input: Value, observer: StateObserver| {
                let domain = domain.clone();
                async move {
                    let mut d = domain.lock().expect("domain poisoned");
                    let sample = d.sample();
                    let id = d.next_observer;
                    d.next_observer += 1;
                    d.observers.push((id, observer));
                    let cleanup_domain = domain.clone();
                    Ok(StateLease::new(sample, move || {
                        let mut d = cleanup_domain.lock().expect("domain poisoned");
                        d.observers.retain(|(registered, _)| *registered != id);
                    }))
                }
            }
        });

    let contract = Arc::new(Contract::new(
        CONTRACT_ID,
        BTreeMap::from([
            ("counter/read".to_string(), read),
            ("counter/add".to_string(), add),
        ]),
        BTreeMap::from([("counter/value".to_string(), value_state)]),
    )?);

    // Local exposure ceiling: the two statically approved service principals
    // (this node and the peer) may reach the whole contract.
    let exposure = Exposure {
        principals: BTreeSet::from([endpoint.clone(), peer.clone()]),
        tenants: BTreeSet::from([space.clone()]),
        methods: BTreeSet::from(["counter/read".into(), "counter/add".into()]),
        states: BTreeSet::from(["counter/value".into()]),
    };

    let mut connect = ConnectOptions::new(&url, &token);
    connect.allow_loopback_ws = true;
    let client = Client::connect(connect).await?;
    emit("ready", &endpoint, json!({}));

    let registered = client
        .provide(
            p::ModuleRegisterInput {
                space_id: space.clone(),
                integration_id: INTEGRATION_ID.into(),
                provider_endpoint_id: endpoint.clone(),
                contract_id: CONTRACT_ID.into(),
            },
            contract.clone(),
            exposure,
        )
        .await?;
    emit(
        "publish",
        &endpoint,
        json!({"registrationId": registered.registration_id, "providerEndpointId": registered.provider_endpoint_id}),
    );

    let target = {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut found = None;
        while Instant::now() < deadline {
            if let Ok(list) = client.list_modules(space.clone()).await
                && let Some(info) = list
                    .modules
                    .iter()
                    .find(|m| m.provider_endpoint_id == peer && m.contract_id == CONTRACT_ID)
            {
                found = Some(ModuleRef::from(info));
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        found.ok_or_else(|| {
            SdkError::new(
                p::ErrorCode::Unavailable,
                format!("peer {peer} not registered within 10s"),
                "not_started",
                "safe",
            )
        })?
    };
    emit(
        "discover",
        &endpoint,
        json!({"target": target.provider_endpoint_id, "registrationId": target.registration_id}),
    );

    // Subscribe before mutating: the snapshot is delivered as the first event.
    let mut subscription =
        client.subscribe(target.clone(), contract.clone(), "counter/value", json!({}))?;
    let snapshot = match subscription
        .next()
        .await
        .expect("subscription delivers the snapshot")
    {
        StateEvent::Snapshot(sample) => sample,
        other => {
            emit(
                "error",
                &endpoint,
                json!({"message": format!("expected snapshot, got {other:?}")}),
            );
            std::process::exit(1);
        }
    };
    let (revision, count) = sample_fields(&snapshot);
    emit(
        "snapshot",
        &endpoint,
        json!({"target": peer, "revision": revision, "count": count}),
    );

    let invoke = |method: &str, argument: Value| {
        client
            .invoke(target.clone(), contract.clone(), method, argument)
            .map(|call| call.result::<CounterResult>())
    };

    let read0 = invoke("counter/read", json!({}))?.await?;
    emit(
        "read",
        &endpoint,
        json!({"target": peer, "count": read0.count, "revision": read0.revision}),
    );

    let added = invoke("counter/add", json!({"delta": 3}))?.await?;
    emit(
        "add",
        &endpoint,
        json!({"target": peer, "delta": 3, "count": added.count, "revision": added.revision}),
    );
    let update1 = next_update(&mut subscription).await?;
    match update1 {
        StateEvent::Update { seq, sample } => {
            let (revision, count) = sample_fields(&sample);
            emit(
                "update-seen",
                &endpoint,
                json!({"target": peer, "seq": seq, "revision": revision, "count": count}),
            );
        }
        other => emit(
            "error",
            &endpoint,
            json!({"message": format!("expected update, got {other:?}")}),
        ),
    }

    let read1 = invoke("counter/read", json!({}))?.await?;
    emit(
        "read",
        &endpoint,
        json!({"target": peer, "count": read1.count, "revision": read1.revision}),
    );

    // delta = 0 still advances the revision and publishes a new sample.
    let zero = invoke("counter/add", json!({"delta": 0}))?.await?;
    emit(
        "add",
        &endpoint,
        json!({"target": peer, "delta": 0, "count": zero.count, "revision": zero.revision}),
    );
    let update2 = next_update(&mut subscription).await?;
    match update2 {
        StateEvent::Update { seq, sample } => {
            let (revision, count) = sample_fields(&sample);
            emit(
                "update-seen",
                &endpoint,
                json!({"target": peer, "seq": seq, "revision": revision, "count": count}),
            );
        }
        other => emit(
            "error",
            &endpoint,
            json!({"message": format!("expected update, got {other:?}")}),
        ),
    }

    // int32 overflow is rejected before any mutation: revision unchanged, and
    // no state update is published.
    match invoke("counter/add", json!({"delta": i32::MAX}))?.await {
        Ok(result) => {
            emit(
                "error",
                &endpoint,
                json!({"message": format!("overflow add unexpectedly succeeded: {result:?}")}),
            );
            std::process::exit(1);
        }
        Err(error) => emit(
            "overflow-rejected",
            &endpoint,
            json!({
                "code": error.code(),
                "execution": error.execution(),
                "retry": error.retry(),
                "message": error.to_string(),
            }),
        ),
    }

    let read2 = invoke("counter/read", json!({}))?.await?;
    emit(
        "read",
        &endpoint,
        json!({"target": peer, "count": read2.count, "revision": read2.revision}),
    );

    let removed = subscription.unsubscribe().await?;
    emit(
        "termination",
        &endpoint,
        json!({"target": peer, "removed": removed.removed}),
    );
    emit("done", &endpoint, json!({}));
    // Consumer completion does not withdraw this node's published provider.
    tokio::select! {
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
    }
    client.close().await;
    Ok(())
}

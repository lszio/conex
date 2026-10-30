//! S2 plugin runtime behavior tests (docs/contracts/private-runtime.md §1, §5
//! P2.4; plan 2026-10-29-conex-mvp-p2.md §5). Coverage: A01/A02/A04/A05/A06/
//! A08 from the S2 acceptance list plus the P2.4 lifecycle cutover — local
//! start without a hub, independent publication with reconnect, clone/Drop
//! semantics, reverse-order cleanup hooks and the local authorized invoke
//! path. Every scenario runs against a real in-process `conex-host` loopback
//! server — no mocks, no compile-existence checks.

use conex_host::config::HostConfig;
use conex_sdk::{
    Client, ConnectOptions, Contract, Execution, Instance, InstanceConfig, InstanceState, Method,
    ModuleDecl, ModuleRef, Plugin, PluginError, PublicationState, RemoteBinding, StartedModule,
    State, StateEvent, StateLease, StateObserver, StopOutcome,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;

const CONTRACT_ID: &str = "conex.counter";
const SPACE: &str = "dev";
const TENANT: &str = "dev";
const AUDIENCE: &str = "s2.test";
const PRINCIPALS: [&str; 3] = ["s2-a", "s2-b", "s2-consumer"];
const ENDPOINTS: [&str; 4] = ["a-counter", "a-derived", "b-counter", "b-derived"];

fn token_of(principal: &str) -> String {
    format!("{principal}-token")
}

fn sha256_hex(value: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.as_bytes()))
}

/// Explicit trusted local caller for the node-facing local invoke path.
fn local_caller() -> String {
    "s2-a".to_string()
}

// ---- real host fixture ----------------------------------------------------

/// A free loopback port that is released immediately (listener dropped).
async fn free_port() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    listener.local_addr().expect("addr").port()
}

/// A host has a dedicated runtime so shutdown closes its accepted WebSockets,
/// not just the outer listener future. Aborting `serve` alone leaves its
/// independently spawned connection tasks alive.
struct HostRuntime {
    shutdown: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}
impl HostRuntime {
    fn abort(&self) {
        if let Some(shutdown) = self.shutdown.lock().unwrap().take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
    fn is_finished(&self) -> bool {
        self.thread
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
    }
}
impl Drop for HostRuntime {
    fn drop(&mut self) {
        self.abort();
    }
}

/// Real `conex-host::serve` on a fresh loopback port, with the S1 counter
/// contract preinstalled and static publish/consume approvals for two plugin
/// instances (prefixes a/b) plus one raw consumer principal.
async fn spawn_host() -> (String, tempfile::TempDir, HostRuntime) {
    let port = free_port().await;
    spawn_host_on_with(port, &ENDPOINTS).await
}

async fn spawn_host_on(port: u16) -> (String, tempfile::TempDir, HostRuntime) {
    spawn_host_on_with(port, &ENDPOINTS).await
}

/// Like [`spawn_host_on`] but with an explicit publish/consume endpoint
/// allowlist (used by the remote-binding scenario).
async fn spawn_host_on_with(
    port: u16,
    endpoints: &[&str],
) -> (String, tempfile::TempDir, HostRuntime) {
    for _ in 0..5 {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["content", "session", "operation"] {
            std::fs::create_dir(dir.path().join(name)).expect("mkdir");
        }
        let path = dir.path().join("host.toml");
        std::fs::write(&path, host_toml_with(port, dir.path(), endpoints)).expect("write config");
        let config = HostConfig::load(&path).expect("load config");
        let (shutdown, stopping) = tokio::sync::oneshot::channel();
        let thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                tokio::select! {
                    _ = conex_host::serve::serve(config) => {}
                    _ = stopping => {}
                }
            });
            runtime.shutdown_timeout(Duration::from_secs(1));
        });
        let host = HostRuntime {
            shutdown: Mutex::new(Some(shutdown)),
            thread: Mutex::new(Some(thread)),
        };
        let url = format!("ws://127.0.0.1:{port}/wss");
        for _ in 0..100 {
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
                // One quiet beat: a port thief would have crashed the serve
                // task by now.
                tokio::time::sleep(Duration::from_millis(50)).await;
                if !host.is_finished() {
                    return (url, dir, host);
                }
                break;
            }
            if host.is_finished() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
    panic!("conex-host could not claim port {port} after 5 attempts");
}

fn host_toml_with(port: u16, root: &std::path::Path, endpoints: &[&str]) -> String {
    let mut toml = format!(
        "listen = \"127.0.0.1:{port}\"\n\
allow_loopback_http = true\n\
audience = \"{AUDIENCE}\"\n\
content_root = \"{}/content\"\n\
session_root = \"{}/session\"\n\
operation_root = \"{}/operation\"\n",
        root.join("content").display(),
        root.join("session").display(),
        root.join("operation").display(),
    );
    for principal in PRINCIPALS {
        toml.push_str(&format!(
            "\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"{principal}\"\ntenant_id = \"{TENANT}\"\naudience = \"{AUDIENCE}\"\nrole = \"service\"\n",
            sha256_hex(&token_of(principal))
        ));
    }
    toml.push_str("\n\n[capabilities]\n");
    toml.push_str(&format!(
        "\n[[capabilities.contracts]]\ncontract_id = \"{CONTRACT_ID}\"\n\
\n[[capabilities.contracts.methods]]\nname = \"counter/read\"\ninput_schema = \"conex.CounterReadInput\"\noutput_schema = \"conex.CounterResult\"\naction = \"counter.read\"\nexecution = \"read_only\"\n\
\n[[capabilities.contracts.methods]]\nname = \"counter/add\"\ninput_schema = \"conex.CounterAddInput\"\noutput_schema = \"conex.CounterResult\"\naction = \"counter.add\"\nexecution = \"non_replayable\"\n\
\n[[capabilities.contracts.methods]]\nname = \"counter/slow\"\ninput_schema = \"conex.CounterAddInput\"\noutput_schema = \"conex.CounterResult\"\naction = \"counter.slow\"\nexecution = \"non_replayable\"\n\
\n[[capabilities.contracts.states]]\nname = \"counter/value\"\nvalue_schema = \"conex.CounterStateValue\"\naction = \"counter.value\"\n"
    ));
    for endpoint in endpoints {
        let node = if endpoint.starts_with("a-") {
            "s2-a"
        } else {
            "s2-b"
        };
        toml.push_str(&format!(
            "\n[[capabilities.publish]]\nnode_id = \"{node}\"\nspace_id = \"{SPACE}\"\nintegration_id = \"integration\"\nprovider_endpoint_id = \"{endpoint}\"\ncontract_id = \"{CONTRACT_ID}\"\n"
        ));
    }
    for principal in PRINCIPALS {
        for endpoint in endpoints {
            toml.push_str(&format!(
                "\n[[capabilities.consume]]\nprincipal_id = \"{principal}\"\nspace_id = \"{SPACE}\"\nprovider_endpoint_id = \"{endpoint}\"\ncontract_id = \"{CONTRACT_ID}\"\n"
            ));
        }
    }
    toml
}

// ---- plugin under test ----------------------------------------------------

/// Counter domain: committed (count, revision) plus a watch fan-out so the
/// derived module can observe through a process-local handle. Shared as Arc.
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
    /// Snapshot and watch subscription are taken under the SAME lock the add
    /// path commits under, so a snapshot never misses a committed update.
    fn subscribe(&self) -> (watch::Receiver<(i32, u64)>, conex_proto::StateSample) {
        let guard = self.state.lock().unwrap();
        let rx = self.tx.subscribe();
        let (count, revision) = *guard;
        (rx, sample(count, revision))
    }
    fn add(&self, delta: i32) -> (i32, u64) {
        let mut guard = self.state.lock().unwrap();
        guard.0 += delta;
        guard.1 += 1;
        let value = *guard;
        self.tx.send_replace(value);
        value
    }
}

fn sample(count: i32, revision: u64) -> conex_proto::StateSample {
    conex_proto::StateSample {
        revision,
        value: Some(serde_json::from_value(json!({"count": count})).expect("counter value")),
        sampled_at_ms: 0,
        quality: "current".into(),
    }
}

fn count_of(event_sample: &conex_proto::StateSample) -> i64 {
    let raw = serde_json::to_value(event_sample.value.clone().expect("sample value"))
        .expect("sample json");
    raw["count"]
        .as_f64()
        .map(|count| count as i64)
        .unwrap_or_else(|| panic!("count field missing in {raw}"))
}

/// Provider-side forwarder for the counter's own state: same revision
/// sequence as the committed domain.
async fn forward(mut rx: watch::Receiver<(i32, u64)>, observer: StateObserver) {
    loop {
        if rx.changed().await.is_err() {
            return;
        }
        let (count, revision) = *rx.borrow_and_update();
        let _ = observer.publish(sample(count, revision));
    }
}

/// Derived re-publishing uses its OWN revision sequence (contract §6): the
/// upstream revision value is never inherited.
async fn forward_derived(
    mut rx: watch::Receiver<(i32, u64)>,
    observer: StateObserver,
    revision: Arc<Mutex<u64>>,
) {
    loop {
        if rx.changed().await.is_err() {
            return;
        }
        let (count, _) = *rx.borrow_and_update();
        let revision = {
            let mut guard = revision.lock().unwrap();
            *guard += 1;
            *guard
        };
        let _ = observer.publish(sample(count, revision));
    }
}

fn counter_module(slow_started: Arc<AtomicUsize>) -> ModuleDecl {
    ModuleDecl::new("counter", vec![], vec![], move |_| {
        let slow_flag = slow_started.clone();
        Box::pin(async move {
            let domain = CounterDomain::new();
            let (d_read, d_add, d_state, slow_flag) =
                (domain.clone(), domain.clone(), domain.clone(), slow_flag);
            let read = Method::new(
                "conex.CounterReadInput",
                "conex.CounterResult",
                Execution::ReadOnly,
            )
            .with_handler(move |_, _: Value| {
                let domain = d_read.clone();
                async move {
                    let (count, revision) = *domain.state.lock().unwrap();
                    Ok(json!({"count": count, "revision": revision.to_string()}))
                }
            });
            let add = Method::new(
                "conex.CounterAddInput",
                "conex.CounterResult",
                Execution::NonReplayable,
            )
            .with_handler(move |_, input: Value| {
                let domain = d_add.clone();
                async move {
                    let delta = input["delta"].as_i64().unwrap_or(0) as i32;
                    let (count, revision) = domain.add(delta);
                    Ok(json!({"count": count, "revision": revision.to_string()}))
                }
            });
            let slow = Method::new(
                "conex.CounterAddInput",
                "conex.CounterResult",
                Execution::NonReplayable,
            )
            .with_handler(move |_, _: Value| {
                let slow_started = slow_flag.clone();
                async move {
                    slow_started.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Ok(json!({"count": 0, "revision": "0"}))
                }
            });
            let value = State::new("conex.CounterReadInput", "conex.CounterStateValue")
                .with_handler(move |_, _: Value, observer| {
                    let domain = d_state.clone();
                    async move {
                        // Snapshot and watch attach share the domain lock; the
                        // lease opening is the subscription's publish point.
                        let (rx, snapshot) = domain.subscribe();
                        let task = tokio::spawn(forward(rx, observer));
                        Ok(StateLease::new(snapshot, move || task.abort()))
                    }
                });
            let contract = Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::from([
                    ("counter/read".to_string(), read),
                    ("counter/add".to_string(), add),
                    ("counter/slow".to_string(), slow),
                ]),
                BTreeMap::from([("counter/value".to_string(), value)]),
            )?);
            Ok(StartedModule {
                contract,
                service: Some(domain),
                cleanup: None,
            })
        })
    })
}

fn derived_module() -> ModuleDecl {
    ModuleDecl::new("derived", vec!["counter".into()], vec![], |context| {
        Box::pin(async move {
            let counter = context
                .deps
                .local::<CounterDomain>("counter")
                .ok_or_else(|| {
                    conex_sdk::Error::new(
                        conex_proto::ErrorCode::BadRequest,
                        "counter service handle missing",
                        "not_started",
                        "never",
                    )
                })?;
            let value = State::new("conex.CounterReadInput", "conex.CounterStateValue")
                .with_handler(move |_, _: Value, observer| {
                    let counter = counter.clone();
                    async move {
                        // 先快照后发布: the upstream snapshot is captured before
                        // the lease (the publish point) is returned, and the
                        // derived revision sequence starts independently at 0.
                        let (rx, snapshot) = counter.subscribe();
                        let revision = Arc::new(Mutex::new(0u64));
                        let task = tokio::spawn(forward_derived(rx, observer, revision));
                        Ok(StateLease::new(snapshot, move || task.abort()))
                    }
                });
            let contract = Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::new(),
                BTreeMap::from([("counter/value".to_string(), value)]),
            )?);
            Ok(StartedModule {
                contract,
                service: None,
                cleanup: None,
            })
        })
    })
}

fn counter_plugin(slow_started: Arc<AtomicUsize>) -> Plugin {
    Plugin::new(
        "s2-counter",
        vec![counter_module(slow_started), derived_module()],
    )
}

/// Placeholder start factory for composition-failure plugins; never executed
/// because validation rejects before any start.
fn placeholder_module(name: &str, local_deps: Vec<String>, remote_deps: Vec<String>) -> ModuleDecl {
    ModuleDecl::new(
        name,
        local_deps,
        remote_deps,
        |_: conex_sdk::ModuleContext| {
            Box::pin(async {
                let contract = Arc::new(
                    Contract::new(
                        CONTRACT_ID,
                        BTreeMap::new(),
                        BTreeMap::from([(
                            "counter/value".to_string(),
                            State::new("conex.CounterReadInput", "conex.CounterStateValue"),
                        )]),
                    )
                    .expect("placeholder contract"),
                );
                Ok(StartedModule {
                    contract,
                    service: None,
                    cleanup: None,
                })
            })
        },
    )
}

/// Local-only module with a synchronous reverse-order cleanup hook that
/// records its invocation for order assertions.
fn cleanup_module(
    name: &str,
    local_deps: Vec<String>,
    events: Arc<Mutex<Vec<String>>>,
    label: &str,
) -> ModuleDecl {
    let label = label.to_string();
    ModuleDecl::new(name, local_deps, vec![], move |_| {
        let events = events.clone();
        let label = label.clone();
        Box::pin(async move {
            let contract = Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::from([(
                    "counter/read".into(),
                    Method::new(
                        "conex.CounterReadInput",
                        "conex.CounterResult",
                        Execution::ReadOnly,
                    ),
                )]),
                BTreeMap::new(),
            )?);
            Ok(StartedModule {
                contract,
                service: None,
                cleanup: Some(Arc::new(move |_deadline| {
                    let events = events.clone();
                    let label = label.clone();
                    Box::pin(async move {
                        events.lock().unwrap().push(label);
                        Ok(())
                    })
                })),
            })
        })
    })
    .with_publication(false)
}

fn instance_config(url: &str, prefix: &str, bindings: Vec<RemoteBinding>) -> InstanceConfig {
    InstanceConfig {
        url: url.to_string(),
        bearer: token_of(&format!("s2-{prefix}")),
        allow_loopback_ws: true,
        space_id: SPACE.into(),
        integration_id: "integration".into(),
        endpoint_prefix: prefix.into(),
        bindings,
        exposure_principals: PRINCIPALS.iter().map(|p| p.to_string()).collect(),
        exposure_tenants: BTreeSet::from([TENANT.to_string()]),
    }
}

async fn consumer(url: &str) -> Client {
    let mut options = ConnectOptions::new(url, token_of("s2-consumer"));
    options.allow_loopback_ws = true;
    Client::connect(options).await.expect("consumer link")
}

/// Consumer-side contract: descriptors only, no handlers.
fn consumer_contract() -> Arc<Contract> {
    let method = |name: &str, execution| {
        (
            name.to_string(),
            Method::new(
                if name == "counter/read" {
                    "conex.CounterReadInput"
                } else {
                    "conex.CounterAddInput"
                },
                "conex.CounterResult",
                execution,
            ),
        )
    };
    Arc::new(
        Contract::new(
            CONTRACT_ID,
            BTreeMap::from([
                method("counter/read", Execution::ReadOnly),
                method("counter/add", Execution::NonReplayable),
                method("counter/slow", Execution::NonReplayable),
            ]),
            BTreeMap::from([(
                "counter/value".to_string(),
                State::new("conex.CounterReadInput", "conex.CounterStateValue"),
            )]),
        )
        .expect("consumer contract"),
    )
}

async fn ref_of(client: &Client, endpoint: &str) -> ModuleRef {
    let list = client.list_modules(SPACE).await.expect("list");
    list.modules
        .iter()
        .find(|module| module.provider_endpoint_id == endpoint)
        .map(ModuleRef::from)
        .unwrap_or_else(|| panic!("endpoint {endpoint} not registered"))
}

async fn add_via(client: &Client, target: &ModuleRef, delta: i32) -> Value {
    client
        .invoke(
            target.clone(),
            consumer_contract(),
            "counter/add",
            json!({"delta": delta}),
        )
        .expect("invoke queued")
        .result::<Value>()
        .await
        .expect("invoke result")
}

async fn read_via(client: &Client, target: &ModuleRef) -> Value {
    client
        .invoke(
            target.clone(),
            consumer_contract(),
            "counter/read",
            json!({}),
        )
        .expect("invoke queued")
        .result::<Value>()
        .await
        .expect("invoke result")
}

async fn wait_for(predicate: impl Fn() -> bool, what: &str) {
    wait_for_secs(predicate, 3, what).await;
}

async fn wait_for_secs(predicate: impl Fn() -> bool, secs: u64, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// The instance reached the hub and holds current registrations.
async fn wait_published(instance: &Instance) {
    wait_for_secs(
        || instance.publication_state() == PublicationState::Published,
        5,
        "instance to reach Published",
    )
    .await;
}

// ---- S2-A01: one plugin class, two instances, fully independent -----------

#[tokio::test]
async fn s2_a01_two_instances_are_independent() {
    let (url, _dir, host) = spawn_host().await;
    let plugin = counter_plugin(Arc::new(AtomicUsize::new(0)));
    let a = Instance::new(plugin.clone(), instance_config(&url, "a", vec![]));
    let b = Instance::new(plugin, instance_config(&url, "b", vec![]));
    a.start().await.expect("instance a");
    b.start().await.expect("instance b");
    wait_published(&a).await;
    wait_published(&b).await;

    let consumer = consumer(&url).await;
    let list = consumer.list_modules(SPACE).await.expect("list");
    let endpoints: BTreeSet<&str> = list
        .modules
        .iter()
        .map(|module| module.provider_endpoint_id.as_str())
        .collect();
    assert_eq!(
        endpoints,
        ENDPOINTS.into_iter().collect::<BTreeSet<&str>>(),
        "both instances publish their own 4-tuples"
    );

    let a_counter = ref_of(&consumer, "a-counter").await;
    let b_counter = ref_of(&consumer, "b-counter").await;
    assert_ne!(
        a_counter.registration_id, b_counter.registration_id,
        "independent registration ids"
    );
    assert_eq!(add_via(&consumer, &a_counter, 5).await["count"], 5);
    let a_read = read_via(&consumer, &a_counter).await;
    let b_read = read_via(&consumer, &b_counter).await;
    assert_eq!(a_read["count"], 5, "instance a committed its own add");
    assert_eq!(a_read["revision"], "1");
    assert_eq!(b_read["count"], 0, "instance b untouched by a's add");
    assert_eq!(b_read["revision"], "0", "independent revision sequences");

    a.stop().await;
    b.stop().await;
    host.abort();
}

// ---- S2-A02: local dependency derived module, in-process handle -----------

#[tokio::test]
async fn s2_a02_local_dependency_derived_module() {
    let (url, _dir, host) = spawn_host().await;
    let a = Instance::new(
        counter_plugin(Arc::new(AtomicUsize::new(0))),
        instance_config(&url, "a", vec![]),
    );
    a.start().await.expect("instance a");
    wait_published(&a).await;

    let consumer = consumer(&url).await;
    let derived = ref_of(&consumer, "a-derived").await;
    let mut subscription = consumer
        .subscribe(derived, consumer_contract(), "counter/value", json!({}))
        .expect("subscribe derived");

    let counter = ref_of(&consumer, "a-counter").await;
    add_via(&consumer, &counter, 3).await;
    add_via(&consumer, &counter, 4).await;

    // Snapshot before updates; the derived module re-publishes each commit
    // through the process-local handle with its own revision sequence.
    let event = subscription.next().await.expect("snapshot event");
    let StateEvent::Snapshot(snapshot) = event else {
        panic!("expected snapshot, got {event:?}");
    };
    assert_eq!(snapshot.revision, 0);
    assert_eq!(count_of(&snapshot), 0);

    let event = subscription.next().await.expect("first update");
    let StateEvent::Update { seq, sample } = event else {
        panic!("expected update, got {event:?}");
    };
    assert_eq!((seq, sample.revision), (1, 1));
    assert_eq!(count_of(&sample), 3);

    let event = subscription.next().await.expect("second update");
    let StateEvent::Update { seq, sample } = event else {
        panic!("expected update, got {event:?}");
    };
    assert_eq!((seq, sample.revision), (2, 2));
    assert_eq!(count_of(&sample), 7);

    a.stop().await;
    host.abort();
}

// ---- S2-A04: missing local dependency and missing remote binding ----------

#[tokio::test]
async fn s2_a04_missing_dependencies_are_rejected() {
    let (url, _dir, host) = spawn_host().await;

    let missing_local = Plugin::new(
        "missing-local",
        vec![placeholder_module("solo", vec!["ghost".into()], vec![])],
    );
    let instance = Instance::new(missing_local, instance_config(&url, "a", vec![]));
    let error = instance.start().await.expect_err("missing local dep");
    assert_eq!(
        error,
        PluginError::MissingDependency {
            module: "solo".into(),
            dependency: "ghost".into()
        }
    );
    assert_eq!(instance.instance_state(), InstanceState::Failed);

    let needs_remote = Plugin::new(
        "needs-remote",
        vec![placeholder_module("solo", vec![], vec!["peer".into()])],
    );
    let instance = Instance::new(needs_remote, instance_config(&url, "a", vec![]));
    let error = instance.start().await.expect_err("missing binding");
    assert_eq!(
        error,
        PluginError::MissingDependency {
            module: "solo".into(),
            dependency: "peer".into()
        }
    );
    assert_eq!(instance.instance_state(), InstanceState::Failed);

    // Rejected before any network frame: zero publications, zero links.
    let consumer = consumer(&url).await;
    let list = consumer.list_modules(SPACE).await.expect("list");
    assert!(list.modules.is_empty(), "no module was published");

    host.abort();
}

// ---- S2-A05: circular local dependency rejected with the cycle path -------

#[tokio::test]
async fn s2_a05_circular_dependency_is_rejected() {
    let (url, _dir, host) = spawn_host().await;
    let cyclic = Plugin::new(
        "cyclic",
        vec![
            placeholder_module("a", vec!["b".into()], vec![]),
            placeholder_module("b", vec!["a".into()], vec![]),
            // An independent acyclic module must not hide the cycle.
            placeholder_module("c", vec![], vec![]),
        ],
    );
    let instance = Instance::new(cyclic, instance_config(&url, "a", vec![]));
    let error = instance.start().await.expect_err("cycle rejected");
    let PluginError::CircularDependency { cycle } = error else {
        panic!("expected CircularDependency, got {error:?}");
    };
    assert_eq!(cycle, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(instance.instance_state(), InstanceState::Failed);

    let consumer = consumer(&url).await;
    let list = consumer.list_modules(SPACE).await.expect("list");
    assert!(list.modules.is_empty(), "cycle rejected before any publish");

    host.abort();
}

// ---- S2-A06: stop releases everything; other instances unaffected ---------

#[tokio::test]
async fn s2_a06_stop_releases_everything() {
    let (url, _dir, host) = spawn_host().await;
    let slow_started = Arc::new(AtomicUsize::new(0));
    let a = Instance::new(
        counter_plugin(slow_started.clone()),
        instance_config(&url, "a", vec![]),
    );
    let b = Instance::new(
        counter_plugin(Arc::new(AtomicUsize::new(0))),
        instance_config(&url, "b", vec![]),
    );
    a.start().await.expect("instance a");
    b.start().await.expect("instance b");
    wait_published(&a).await;
    wait_published(&b).await;

    let consumer = consumer(&url).await;
    let a_counter = ref_of(&consumer, "a-counter").await;
    let mut subscription = consumer
        .subscribe(
            a_counter.clone(),
            consumer_contract(),
            "counter/value",
            json!({}),
        )
        .expect("subscribe");
    assert!(matches!(
        subscription.next().await,
        Some(StateEvent::Snapshot(_))
    ));

    let slow = consumer
        .invoke(
            a_counter.clone(),
            consumer_contract(),
            "counter/slow",
            json!({"delta": 1}),
        )
        .expect("invoke queued");
    let slow_task = tokio::spawn(async move { slow.result::<Value>().await });
    wait_for(
        || slow_started.load(Ordering::SeqCst) == 1,
        "slow handler to start",
    )
    .await;

    let outcome = a.stop().await;
    assert_eq!(outcome, StopOutcome::Stopped);
    assert_eq!(a.instance_state(), InstanceState::Stopped);
    // stop() aborts and awaits handlers instead of waiting out the deadline.
    let slow_result = tokio::time::timeout(Duration::from_secs(5), slow_task)
        .await
        .expect("slow result settles promptly after abort")
        .expect("slow task join");
    assert!(slow_result.is_err(), "aborted handler reports an error");

    // Active subscription receives end.
    assert!(matches!(
        subscription.next().await,
        Some(StateEvent::End(_))
    ));

    // Everything from instance a is unregistered.
    let list = consumer.list_modules(SPACE).await.expect("list");
    assert!(
        list.modules
            .iter()
            .all(|module| !module.provider_endpoint_id.starts_with("a-")),
        "instance a fully unpublished: {:?}",
        list.modules
    );

    // Instance b keeps serving.
    let b_counter = ref_of(&consumer, "b-counter").await;
    let b_read = read_via(&consumer, &b_counter).await;
    assert_eq!(b_read["count"], 0);

    // stop() is idempotent with the same terminal state.
    assert_eq!(a.stop().await, StopOutcome::Stopped);
    assert_eq!(a.stop().await, StopOutcome::Stopped);

    host.abort();
}

// ---- S2-A08: no revival; new instance is a new generation -----------------

#[tokio::test]
async fn s2_a08_no_revival_new_generation() {
    let (url, _dir, host) = spawn_host().await;
    let plugin = counter_plugin(Arc::new(AtomicUsize::new(0)));
    let a = Instance::new(plugin.clone(), instance_config(&url, "a", vec![]));
    a.start().await.expect("instance a");
    wait_published(&a).await;
    let consumer = consumer(&url).await;
    let stale = ref_of(&consumer, "a-counter").await;

    a.stop().await;

    // Old ModuleRef fails explicitly; nothing revives it.
    let call = consumer
        .invoke(
            stale.clone(),
            consumer_contract(),
            "counter/read",
            json!({}),
        )
        .expect("invoke queued on stale ref");
    assert!(
        call.result::<Value>().await.is_err(),
        "stale ModuleRef must fail"
    );
    let list = consumer.list_modules(SPACE).await.expect("list");
    assert!(list.modules.is_empty(), "no module revived after stop");

    // A fresh instance of the same plugin class is a new generation.
    let fresh_instance = Instance::new(plugin, instance_config(&url, "a", vec![]));
    fresh_instance.start().await.expect("fresh instance");
    wait_published(&fresh_instance).await;
    let fresh = ref_of(&consumer, "a-counter").await;
    assert_ne!(
        fresh.registration_id, stale.registration_id,
        "new instance obtains a new registration id"
    );
    // Subscribe on the stale ref cannot resurrect the old registration: S1
    // establishes asynchronously, so the explicit failure is the End event.
    let mut stale_subscription = consumer
        .subscribe(stale, consumer_contract(), "counter/value", json!({}))
        .expect("stale subscribe queued");
    assert!(matches!(
        stale_subscription.next().await,
        Some(StateEvent::End(_))
    ));

    host.abort();
}

// ---- last-clone Drop: sync cancel of publication, never a fake completion --

#[tokio::test]
async fn drop_without_stop_releases_modules() {
    let (url, _dir, host) = spawn_host().await;
    let consumer = consumer(&url).await;
    {
        let a = Instance::new(
            counter_plugin(Arc::new(AtomicUsize::new(0))),
            instance_config(&url, "a", vec![]),
        );
        a.start().await.expect("instance a");
        wait_published(&a).await;
        ref_of(&consumer, "a-counter").await;
        // Dropping the last clone cancels the publication driver, which
        // unwinds registrations in its own async context (no fake completion).
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    loop {
        let list = consumer.list_modules(SPACE).await.expect("list");
        if list.modules.is_empty() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "dropped instance did not release its modules: {:?}",
            list.modules
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    host.abort();
}

// ---- P2.4: local start succeeds without a hub -----------------------------

#[tokio::test]
async fn p24_local_start_without_hub() {
    // Port 1 refuses connections instantly; the instance must still start its
    // local modules and serve local invocations (private-runtime §1).
    let plugin = counter_plugin(Arc::new(AtomicUsize::new(0)));
    let instance = Instance::new(plugin, instance_config("ws://127.0.0.1:1/wss", "a", vec![]));
    instance
        .start()
        .await
        .expect("local start succeeds without hub");

    assert_eq!(instance.instance_state(), InstanceState::Active);
    let caller = local_caller();
    let added = instance
        .invoke_local("counter", "counter/add", json!({"delta": 5}), &caller)
        .await
        .expect("local add");
    assert_eq!(added["count"], 5);
    let read = instance
        .invoke_local("counter", "counter/read", json!({}), &caller)
        .await
        .expect("local read");
    assert_eq!(read["revision"], "1");

    // The in-process DI handle is exposed for the owner (node) layer.
    let domain = instance
        .service::<CounterDomain>("counter")
        .expect("service handle");
    assert_eq!(*domain.state.lock().unwrap(), (5, 1));

    // Publication never succeeds against a refused hub; no registration exists.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_ne!(
        instance.publication_state(),
        PublicationState::Published,
        "refused hub must not publish"
    );
    assert_eq!(instance.module("counter"), None);

    // Explicit stop is the only teardown; a finished instance cannot restart.
    assert_eq!(instance.stop().await, StopOutcome::Stopped);
    assert_eq!(instance.instance_state(), InstanceState::Stopped);
    assert_eq!(instance.publication_state(), PublicationState::Revoked);
    assert!(
        instance.start().await.is_err(),
        "finished instance cannot restart"
    );
}

// ---- P2.4: a dropped clone never tears business down ----------------------

#[tokio::test]
async fn p24_clone_drop_keeps_business_alive() {
    let (url, _dir, host) = spawn_host().await;
    let instance = Instance::new(
        counter_plugin(Arc::new(AtomicUsize::new(0))),
        instance_config(&url, "a", vec![]),
    );
    instance.start().await.expect("instance a");
    wait_published(&instance).await;

    let consumer = consumer(&url).await;
    let counter = ref_of(&consumer, "a-counter").await;
    add_via(&consumer, &counter, 3).await;

    // Dropping any clone must not tear down business.
    {
        let clone = instance.clone();
        drop(clone);
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(instance.instance_state(), InstanceState::Active);
    assert_eq!(instance.publication_state(), PublicationState::Published);
    let read = read_via(&consumer, &counter).await;
    assert_eq!(read["count"], 3, "business untouched by a dropped clone");

    instance.stop().await;
    host.abort();
}

// ---- P2.4: reconnect keeps business, fresh registration, old ref fails -----

#[tokio::test]
async fn p24_reconnect_keeps_business_new_registration() {
    let port = free_port().await;
    let (url, _dir, host) = spawn_host_on(port).await;
    let instance = Instance::new(
        counter_plugin(Arc::new(AtomicUsize::new(0))),
        instance_config(&url, "a", vec![]),
    );
    instance.start().await.expect("instance a");
    wait_published(&instance).await;

    let consumer = consumer(&url).await;
    let stale = ref_of(&consumer, "a-counter").await;
    add_via(&consumer, &stale, 5).await;

    // Stop the hub: publication disconnects, business keeps counting locally.
    host.abort();
    wait_for_secs(
        || instance.publication_state() == PublicationState::Disconnected,
        8,
        "disconnected publication after hub stop",
    )
    .await;
    assert_eq!(instance.instance_state(), InstanceState::Active);
    let caller = local_caller();
    let bumped = instance
        .invoke_local("counter", "counter/add", json!({"delta": 2}), &caller)
        .await
        .expect("local add while disconnected");
    assert_eq!(bumped["count"], 7);

    // Restart the hub on the same port; the SAME business re-publishes under
    // a fresh registration (no rebuild, no replay).
    let (url2, _dir2, host2) = spawn_host_on(port).await;
    assert_eq!(url, url2);
    wait_for_secs(
        || {
            instance.publication_state() == PublicationState::Published
                && instance.module("counter").is_some()
        },
        8,
        "republished after hub restart",
    )
    .await;
    let fresh = instance.module("counter").expect("fresh registration");
    assert_ne!(
        fresh.registration_id, stale.registration_id,
        "reconnect registers a new generation"
    );

    let consumer2 = self::consumer(&url2).await;
    let read = read_via(&consumer2, &fresh).await;
    assert_eq!(
        read["count"], 7,
        "business survived the hub outage without rebuild"
    );

    // The pre-restart ref died with the old link; it fails explicitly.
    let call = consumer2
        .invoke(
            stale.clone(),
            consumer_contract(),
            "counter/read",
            json!({}),
        )
        .expect("queued on stale ref");
    assert!(
        call.result::<Value>().await.is_err(),
        "old registration must fail after reconnect"
    );

    instance.stop().await;
    host2.abort();
}

// ---- P2.4: remote binding rides the instance link and never rebinds --------

#[tokio::test]
async fn p24_remote_binding_uses_instance_link_and_never_rebinds() {
    let port = free_port().await;
    let endpoints = ["a-counter", "a-derived", "b-proxy"];
    let (url, _dir, host) = spawn_host_on_with(port, &endpoints).await;
    let a = Instance::new(
        counter_plugin(Arc::new(AtomicUsize::new(0))),
        instance_config(&url, "a", vec![]),
    );
    a.start().await.expect("instance a");
    wait_published(&a).await;
    let consumer = consumer(&url).await;
    let a_counter = ref_of(&consumer, "a-counter").await;
    add_via(&consumer, &a_counter, 4).await;

    // B binds its remote dependency to A's CURRENT registration.
    let binding = RemoteBinding {
        dependency: "peer".into(),
        target: a_counter.clone(),
        contract: consumer_contract(),
    };
    let proxy = ModuleDecl::new("proxy", vec![], vec!["peer".into()], |context| {
        Box::pin(async move {
            let remote = context.deps.remote("peer").expect("peer binding present");
            let read = Method::new(
                "conex.CounterReadInput",
                "conex.CounterResult",
                Execution::ReadOnly,
            )
            .with_handler(move |_, _: Value| {
                let remote = remote.clone();
                async move {
                    remote
                        .invoke("counter/read", json!({}))?
                        .result::<Value>()
                        .await
                }
            });
            let contract = Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::from([("counter/read".to_string(), read)]),
                BTreeMap::new(),
            )?);
            Ok(StartedModule {
                contract,
                service: None,
                cleanup: None,
            })
        })
    });
    let b = Instance::new(
        Plugin::new("s2-proxy", vec![proxy]),
        instance_config(&url, "b", vec![binding]),
    );
    b.start().await.expect("instance b");
    wait_published(&b).await;

    // Calling B's proxy is routed through B's link to A's pinned ref.
    let b_proxy = ref_of(&consumer, "b-proxy").await;
    let read = read_via(&consumer, &b_proxy).await;
    assert_eq!(read["count"], 4, "remote binding reached the pinned module");

    // A's registration is revoked: the pinned binding fails explicitly and is
    // never transparently rebound to anything else.
    a.stop().await;
    let call = consumer
        .invoke(
            b_proxy.clone(),
            consumer_contract(),
            "counter/read",
            json!({}),
        )
        .expect("queued");
    assert!(
        call.result::<Value>().await.is_err(),
        "pinned binding fails after the peer stopped"
    );

    b.stop().await;
    host.abort();
}

// ---- P2.4: publication flags control which modules are registered ----------

#[tokio::test]
async fn p24_publication_flags_control_registration() {
    let (url, _dir, host) = spawn_host().await;
    let plugin = Plugin::new(
        "s2-mixed",
        vec![
            counter_module(Arc::new(AtomicUsize::new(0))),
            derived_module().with_publication(false),
        ],
    );
    let instance = Instance::new(plugin, instance_config(&url, "a", vec![]));
    instance.start().await.expect("instance");
    wait_published(&instance).await;

    let consumer = consumer(&url).await;
    let list = consumer.list_modules(SPACE).await.expect("list");
    let endpoints: BTreeSet<&str> = list
        .modules
        .iter()
        .map(|module| module.provider_endpoint_id.as_str())
        .collect();
    assert!(endpoints.contains("a-counter"));
    assert!(
        !endpoints.contains("a-derived"),
        "non-published module must stay local-only"
    );
    assert!(instance.module("counter").is_some());
    assert_eq!(instance.module("derived"), None);

    // The local-only module still works through the local DI path.
    let domain = instance
        .service::<CounterDomain>("counter")
        .expect("service");
    domain.add(2);
    instance.stop().await;
    host.abort();
}

// ---- P2.4: partial-start rollback runs cleanup hooks in reverse ------------

#[tokio::test]
async fn p24_partial_start_rollback_runs_cleanup() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let first = cleanup_module("first", vec![], events.clone(), "first");
    let failing = ModuleDecl::new("second", vec!["first".into()], vec![], |_| {
        Box::pin(async {
            Err(conex_sdk::Error::new(
                conex_proto::ErrorCode::BadRequest,
                "boom",
                "not_started",
                "never",
            ))
        })
    });
    let instance = Instance::new(
        Plugin::new("s2-rollback", vec![first, failing]),
        instance_config("ws://127.0.0.1:1/wss", "a", vec![]),
    );
    let error = instance.start().await.expect_err("second start fails");
    let PluginError::StartFailed { module, .. } = error else {
        panic!("expected StartFailed, got {error:?}");
    };
    assert_eq!(module, "second");
    assert_eq!(instance.instance_state(), InstanceState::Failed);
    assert_eq!(
        *events.lock().unwrap(),
        vec!["first".to_string()],
        "already-started module rolled back exactly once"
    );
    assert_eq!(instance.publication_state(), PublicationState::Unpublished);
    assert_eq!(instance.stop().await, StopOutcome::Stopped);
}

// ---- P2.4: explicit stop runs cleanup hooks in reverse topological order ---

#[tokio::test]
async fn p24_stop_runs_cleanup_reverse_order() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let base = cleanup_module("base", vec![], events.clone(), "base");
    let top = cleanup_module("top", vec!["base".into()], events.clone(), "top");
    let instance = Instance::new(
        Plugin::new("s2-cleanup", vec![base, top]),
        instance_config("ws://127.0.0.1:1/wss", "a", vec![]),
    );
    instance.start().await.expect("start");
    assert_eq!(instance.instance_state(), InstanceState::Active);
    instance.stop().await;
    assert_eq!(
        *events.lock().unwrap(),
        vec!["top".to_string(), "base".to_string()],
        "reverse topological cleanup"
    );
    assert_eq!(instance.instance_state(), InstanceState::Stopped);
}

// ---- P2.4: local invoke enforces the same domain boundary ------------------

#[tokio::test]
async fn p24_local_invoke_domain_errors() {
    let instance = Instance::new(
        counter_plugin(Arc::new(AtomicUsize::new(0))),
        instance_config("ws://127.0.0.1:1/wss", "a", vec![]),
    );
    instance.start().await.expect("start");
    let caller = local_caller();
    assert!(
        instance
            .invoke_local("ghost", "counter/read", json!({}), &caller)
            .await
            .is_err(),
        "unknown module rejected"
    );
    assert!(
        instance
            .invoke_local("counter", "ghost/method", json!({}), &caller)
            .await
            .is_err(),
        "unknown method rejected"
    );
    assert!(
        instance
            .invoke_local(
                "counter",
                "counter/add",
                json!({"delta": "not-a-number"}),
                &caller
            )
            .await
            .is_err(),
        "schema-invalid input rejected on the local path"
    );
    instance.stop().await;
    assert!(
        instance
            .invoke_local("counter", "counter/read", json!({}), &caller)
            .await
            .is_err(),
        "stopped instance denies local invoke"
    );
}

#[tokio::test]
async fn local_start_does_not_wait_for_a_stalled_network_handshake() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/wss", listener.local_addr().unwrap());
    let instance = Instance::new(
        counter_plugin(Arc::new(AtomicUsize::new(0))),
        instance_config(&url, "a", vec![]),
    );
    // Real socket/clock integration: the listening peer never speaks WebSocket.
    tokio::time::timeout(Duration::from_millis(100), instance.start())
        .await
        .expect("local start must not await handshake")
        .unwrap();
    assert_eq!(instance.instance_state(), InstanceState::Active);
    let value = instance
        .invoke_local("counter", "counter/read", json!({}), &local_caller())
        .await
        .unwrap();
    assert_eq!(value["count"], 0);
    assert_eq!(instance.stop().await, StopOutcome::Stopped);
}

#[tokio::test]
async fn cleanup_failure_is_a_stable_failed_stop_result() {
    let mut module = cleanup_module("owned", vec![], Arc::new(Mutex::new(Vec::new())), "owned");
    module.start = Arc::new(|_| {
        Box::pin(async {
            let mut started = StartedModule::new(Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::from([(
                    "counter/read".into(),
                    Method::new(
                        "conex.CounterReadInput",
                        "conex.CounterResult",
                        Execution::ReadOnly,
                    ),
                )]),
                BTreeMap::new(),
            )?));
            started.cleanup = Some(Arc::new(|_| {
                Box::pin(async {
                    Err(conex_sdk::Error::new(
                        conex_sdk::proto::ErrorCode::Unavailable,
                        "cleanup refused",
                        "unknown",
                        "never",
                    ))
                })
            }));
            Ok(started)
        })
    });
    let instance = Instance::new(
        Plugin::new("cleanup", vec![module]),
        instance_config("", "a", vec![]),
    );
    instance.start().await.unwrap();
    let first = instance.stop().await;
    assert!(
        matches!(&first,StopOutcome::Failed(errors) if errors.iter().any(|error|error.contains("cleanup refused")))
    );
    assert_eq!(instance.instance_state(), InstanceState::Failed);
    assert_eq!(instance.stop().await, first);
}

#[tokio::test]
async fn concurrent_stop_shortens_a_pending_cleanup_deadline() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let signal = entered.clone();
    let module = ModuleDecl::new("owned", vec![], vec![], move |_| {
        let signal = signal.clone();
        Box::pin(async move {
            let mut started = StartedModule::new(Arc::new(Contract::new(
                CONTRACT_ID,
                BTreeMap::from([(
                    "counter/read".into(),
                    Method::new(
                        "conex.CounterReadInput",
                        "conex.CounterResult",
                        Execution::ReadOnly,
                    ),
                )]),
                BTreeMap::new(),
            )?));
            started.cleanup = Some(Arc::new(move |_| {
                let signal = signal.clone();
                Box::pin(async move {
                    signal.notify_one();
                    std::future::pending::<conex_sdk::Result<()>>().await
                })
            }));
            Ok(started)
        })
    })
    .with_publication(false);
    let instance = Instance::new(
        Plugin::new("deadline", vec![module]),
        instance_config("", "a", vec![]),
    );
    instance.start().await.unwrap();
    let first = instance.clone();
    let first = tokio::spawn(async move { first.stop_bounded(Duration::from_secs(30)).await });
    entered.notified().await;
    // Real total-stop deadline, ordered by the cleanup notification, not sleeps.
    let second = tokio::time::timeout(
        Duration::from_secs(1),
        instance.stop_bounded(Duration::from_millis(1)),
    )
    .await
    .expect("shortened stop");
    assert!(matches!(second, StopOutcome::Failed(_)));
    assert_eq!(first.await.unwrap(), second);
}

//! S2 plugin runtime behavior tests (docs/contracts/s2-plugins.md §8; U02 scope:
//! A01/A02/A04/A05/A06/A08). Every scenario runs against a real in-process
//! `conex-host` serve loopback — no mocks, no compile-existence checks.

use conex_host::config::HostConfig;
use conex_sdk::{
    Client, ConnectOptions, Contract, Execution, Instance, InstanceConfig, Method, ModuleDecl,
    ModuleRef, Plugin, PluginError, RemoteBinding, StartedModule, State, StateEvent, StateLease,
    StateObserver, StopOutcome,
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
use tokio::task::JoinHandle;

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

// ---- real host fixture ----------------------------------------------------

/// Real `conex-host::serve` on a loopback port, with the S1 counter contract
/// preinstalled and static publish/consume approvals for two plugin instances
/// (prefixes a/b) plus one raw consumer principal.
async fn spawn_host() -> (String, tempfile::TempDir, JoinHandle<()>) {
    for _ in 0..5 {
        let dir = tempfile::tempdir().expect("tempdir");
        for name in ["content", "session", "operation"] {
            std::fs::create_dir(dir.path().join(name)).expect("mkdir");
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        let path = dir.path().join("host.toml");
        std::fs::write(&path, host_toml(port, dir.path())).expect("write config");
        let config = HostConfig::load(&path).expect("load config");
        let host = tokio::spawn(async move {
            let _ = conex_host::serve::serve(config).await;
        });
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
    panic!("conex-host could not claim a free port after 5 attempts");
}

fn host_toml(port: u16, root: &std::path::Path) -> String {
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
    for endpoint in ENDPOINTS {
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
        for endpoint in ENDPOINTS {
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
            Ok(StartedModule::new(contract))
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
                Ok(StartedModule::new(contract))
            })
        },
    )
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
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
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
    assert_eq!(a.stop().await, StopOutcome::AlreadyStopped);
    assert_eq!(a.stop().await, StopOutcome::AlreadyStopped);

    host.abort();
}

// ---- S2-A08: no revival; new instance is a new generation -----------------

#[tokio::test]
async fn s2_a08_no_revival_new_generation() {
    let (url, _dir, host) = spawn_host().await;
    let plugin = counter_plugin(Arc::new(AtomicUsize::new(0)));
    let a = Instance::new(plugin.clone(), instance_config(&url, "a", vec![]));
    a.start().await.expect("instance a");
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

// ---- stop() fallback: Drop walks the same release path --------------------

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
        ref_of(&consumer, "a-counter").await;
        // a dropped here: Drop must walk the stop path on the captured handle.
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

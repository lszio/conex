//! S1 cross-process capability acceptance (docs/plans/2026-10-17-conex-s1.md T08;
//! docs/contracts/s1-capabilities.md §12: A01–A12).
//!
//! Live Host, Rust and TS fixture processes exchange real loopback WSS/HTTP
//! frames. Deterministic host regressions supplement queue and late-frame
//! boundaries as explicitly labeled SUPPORT evidence.
//! No mocks, no sleeps as ordering: every wait has a deadline, every barrier
//! is driven by explicit arm/release commands. On any exit path (success,
//! error, panic) a guard kills all children and drops the tempdir.
//!
//! Fixture JSONL protocol (from the fixture headers):
//!   -> {"id": string, "op": string, ...}
//!   <- {"id", "ok": true, "value": ...} | {"id", "ok": false,
//!      "error": {code, message, execution, retry}}
//!   events: {"event":"ready"|"barrier"|"update"|"end"|"snapshot"|...}
//! Rust fixture events carry "node"; responses echo the command id.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const TIMEOUT: Duration = Duration::from_secs(25);
const SPACE: &str = "dev";
const INTEGRATION: &str = "integration";
const CONTRACT: &str = "conex.counter";
const HOST_READY: Duration = Duration::from_secs(15);

/// Registry of implemented scenarios, run in order; the first failure stops
/// the suite (never continue silently past a failure).
type Scenario = (&'static str, fn(&mut Env) -> Result<Outcome>);
static SCENARIOS: &[Scenario] = &[
    ("A01", a01),
    ("A02", a02),
    ("A03", a03),
    ("A05", a05),
    ("A06", a06),
    ("A07", a07),
    ("A04", a04),
    ("A08", a08),
    ("A09", a09),
    ("A10", a10),
    ("A11", a11),
    ("A12", a12),
];

pub fn run() -> Result<()> {
    let mut env = Env::start()?;
    for (name, scenario) in SCENARIOS {
        match scenario(&mut env) {
            Ok(outcome) => println!(
                "{name} expected={} actual={} ok",
                outcome.expected, outcome.actual
            ),
            Err(err) => {
                println!("{name} expected=scenario-detail actual=FAIL");
                eprintln!("  {err:#}");
                return Err(err).with_context(|| format!("scenario {name} failed"));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Process plumbing
// ---------------------------------------------------------------------------

struct Env {
    root: PathBuf,
    port: u16,
    token_ts: String,
    token_rust: String,
    token_big: String,
    host: Mutex<Child>,
    fixtures: Vec<Fixture>,
    /// camelCase module reference per fixture label.
    refs: HashMap<String, Value>,
    _tmp: tempfile::TempDir,
}

impl Drop for Env {
    fn drop(&mut self) {
        // Kill/wait on every exit path (success, error, panic unwind).
        for fixture in &self.fixtures {
            let _ = fixture.kill();
        }
        let _ = self.host.lock().map(|mut host| host.kill());
        let _ = self.host.lock().map(|mut host| host.wait());
    }
}

impl Env {
    fn start() -> Result<Env> {
        let root = repo_root()?;
        let host_build = Command::new("cargo")
            .args(["build", "-p", "conex-host"])
            .current_dir(&root)
            .status()
            .context("build conex-host")?;
        if !host_build.success() {
            bail!("cargo build -p conex-host failed");
        }
        let sdk_build = Command::new("cargo")
            .args(["build", "-p", "conex-sdk", "--example", "s1_fixture"])
            .current_dir(&root)
            .status()
            .context("build s1_fixture")?;
        if !sdk_build.success() {
            bail!("cargo build -p conex-sdk --example s1_fixture failed");
        }

        let port = free_port()?;
        let tmp = tempfile::tempdir().context("create s1 e2e temp dir")?;
        for name in ["content", "session", "operation"] {
            std::fs::create_dir_all(tmp.path().join(name)).context("mkdir s1 root")?;
        }
        let token_ts = random_token();
        let token_rust = random_token();
        let token_big = random_token();
        let config = host_toml(port, tmp.path(), &token_ts, &token_rust, &token_big);
        let config_path = tmp.path().join("host.toml");
        std::fs::write(&config_path, &config).context("write host config")?;

        let binary = root.join("target/debug/conex-host");
        let mut host = Command::new(&binary)
            .arg(&config_path)
            .current_dir(&root)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| format!("spawn {}", binary.display()))?;
        if !wait_ready(port, HOST_READY) {
            let _ = host.kill();
            let _ = host.wait();
            bail!("conex-host did not become ready on port {port}");
        }

        let mut env = Env {
            root,
            port,
            token_ts,
            token_rust,
            token_big,
            host: Mutex::new(host),
            fixtures: Vec::new(),
            refs: HashMap::new(),
            _tmp: tmp,
        };
        // TS first: the Rust fixture seeds its local exposure principals from
        // a pre-registration discovery scan, so counter-ts must already be
        // registered or the TS consumer would be denied by the provider-side
        // exposure check.
        env.fixture("ts", "counter-ts", env.token_ts.clone(), &[])?;
        env.fixture("rust", "counter-rust", env.token_rust.clone(), &[])?;
        Ok(env)
    }

    /// Spawn a fixture, wait for its `ready` event, record its module
    /// reference under `label`.
    fn fixture(
        &mut self,
        label: &'static str,
        endpoint: &str,
        token: String,
        extra: &[(&str, &str)],
    ) -> Result<Value> {
        let fixture = spawn_fixture(&self.root, label, self.port, endpoint, &token, extra)?;
        let ready = fixture
            .wait_event(|event| event.get("event").and_then(Value::as_str) == Some("ready"))?;
        let reference = normalize_ref(&ready.get("reference").cloned().unwrap_or(Value::Null));
        self.refs.insert(label.to_string(), reference.clone());
        self.fixtures.push(fixture);
        Ok(reference)
    }

    /// Spawn a JSONL child without a ready event (raw WSS / HTTP probes).
    /// It participates in cleanup like any fixture.
    fn plain(&mut self, label: &'static str, token: String, extra: &[(&str, &str)]) -> Result<()> {
        let probe = spawn_plain(&self.root, label, self.port, &token, extra)?;
        self.fixtures.push(probe);
        Ok(())
    }

    fn node(&self, label: &str) -> &Fixture {
        self.fixtures
            .iter()
            .find(|fixture| fixture.label == label)
            .unwrap_or_else(|| panic!("unknown fixture label {label}"))
    }
}

struct Fixture {
    label: &'static str,
    child: Mutex<Child>,
    stdin: Mutex<std::process::ChildStdin>,
    events: Arc<Mutex<Vec<Value>>>,
    pending: Arc<Mutex<HashMap<String, Sender<Value>>>>,
    reader: Option<JoinHandle<()>>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.kill();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl Fixture {
    fn send(&self, value: Value) -> Result<Receiver<Value>> {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .with_context(|| format!("{}: command needs a string id", self.label))?
            .to_string();
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let mut text = serde_json::to_string(&value)?;
        text.push('\n');
        let mut stdin = self.stdin.lock().unwrap();
        stdin
            .write_all(text.as_bytes())
            .with_context(|| format!("{}: write command", self.label))?;
        stdin
            .flush()
            .with_context(|| format!("{}: flush command", self.label))?;
        Ok(rx)
    }

    fn recv(&self, rx: Receiver<Value>) -> Result<Value> {
        rx.recv_timeout(TIMEOUT)
            .with_context(|| format!("{}: no response within {TIMEOUT:?}", self.label))
    }

    /// Full roundtrip: send and wait for that id's response.
    fn roundtrip(&self, value: Value) -> Result<Value> {
        let rx = self.send(value)?;
        self.recv(rx)
    }

    fn cmd_ok(&self, value: Value) -> Result<Value> {
        let response = self.roundtrip(value)?;
        if response.get("ok").and_then(Value::as_bool) != Some(true) {
            bail!("{}: expected ok response, got {}", self.label, response);
        }
        Ok(response.get("value").cloned().unwrap_or(Value::Null))
    }

    fn cmd_err(&self, value: Value) -> Result<Value> {
        let response = self.roundtrip(value)?;
        if response.get("ok").and_then(Value::as_bool) == Some(true) {
            bail!("{}: expected error response, got {}", self.label, response);
        }
        Ok(response.get("error").cloned().unwrap_or(Value::Null))
    }

    /// First event matching the predicate, buffered from the JSONL stream.
    /// Waits purely by deadline; responses are routed to their pending ids by
    /// the reader thread and never land in this log.
    fn wait_event(&self, predicate: impl Fn(&Value) -> bool) -> Result<Value> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            {
                let events = self.events.lock().unwrap();
                if let Some(event) = events.iter().find(|event| predicate(event)) {
                    return Ok(event.clone());
                }
            }
            if Instant::now() >= deadline {
                bail!("{}: event not seen within {TIMEOUT:?}", self.label);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Current event count; used as a marker so later waits only see events
    /// emitted after the marker (single-shot gates otherwise match stale
    /// barriers from earlier scenarios).
    fn events_len(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    fn wait_event_after(&self, marker: usize, predicate: impl Fn(&Value) -> bool) -> Result<Value> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            {
                let events = self.events.lock().unwrap();
                if let Some(event) = events.iter().skip(marker).find(|event| predicate(event)) {
                    return Ok(event.clone());
                }
            }
            if Instant::now() >= deadline {
                bail!("{}: event not seen within {TIMEOUT:?}", self.label);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn kill(&self) -> Result<()> {
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }
}

fn spawn_reader(
    stdout: std::process::ChildStdout,
    label: &'static str,
    events: Arc<Mutex<Vec<Value>>>,
    pending: Arc<Mutex<HashMap<String, Sender<Value>>>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                eprintln!("{label}: bad JSONL line: {line}");
                continue;
            };
            // Responses (id, no event) go to their pending waiter; everything
            // else is buffered as an event.
            if value.get("event").is_none()
                && let Some(id) = value.get("id").and_then(Value::as_str)
                && let Some(tx) = pending.lock().unwrap().remove(id)
            {
                let _ = tx.send(value);
                continue;
            }
            events.lock().unwrap().push(value);
        }
    })
}

/// Spawn a probe-style JSONL child (no `ready` handshake, no domain).
fn spawn_plain(
    root: &Path,
    label: &'static str,
    port: u16,
    token: &str,
    extra: &[(&str, &str)],
) -> Result<Fixture> {
    let script = if label.starts_with("probe") {
        "sdk/typescript/tests/s1_wire_probe.ts"
    } else if label.starts_with("http") {
        "sdk/typescript/tests/s1_http_probe.ts"
    } else {
        bail!("unknown probe label {label}");
    };
    let mut command = Command::new("bun");
    command.args(["run", script]).current_dir(root);
    command
        .env("CONEX_URL", format!("ws://127.0.0.1:{port}/wss"))
        .env("CONEX_HTTP_URL", format!("http://127.0.0.1:{port}/rpc"))
        .env("CONEX_TOKEN", token)
        .env("CONEX_SPACE", SPACE)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    for (key, value) in extra {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn probe {label}"))?;
    let stdin = child.stdin.take().expect("probe stdin piped");
    let stdout = child.stdout.take().expect("probe stdout piped");
    let events = Arc::new(Mutex::new(Vec::new()));
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let _reader = spawn_reader(stdout, label, events.clone(), pending.clone());
    Ok(Fixture {
        label,
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        events,
        pending,
        reader: Some(_reader),
    })
}

fn spawn_fixture(
    root: &Path,
    label: &'static str,
    port: u16,
    endpoint: &str,
    token: &str,
    extra: &[(&str, &str)],
) -> Result<Fixture> {
    let url = format!("ws://127.0.0.1:{port}/wss");
    let mut command = if label.starts_with("rust") {
        let mut command = Command::new(root.join("target/debug/examples/s1_fixture"));
        command.current_dir(root);
        command
    } else {
        let mut command = Command::new("bun");
        command
            .args(["run", "sdk/typescript/tests/fixtures/s1-node.ts"])
            .current_dir(root);
        command
    };
    command
        .env("CONEX_URL", &url)
        .env("CONEX_TOKEN", token)
        .env("CONEX_SPACE", SPACE)
        .env("CONEX_ENDPOINT", endpoint)
        .env("CONEX_INTEGRATION", INTEGRATION)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    for (key, value) in extra {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn fixture {label}"))?;
    let stdin = child.stdin.take().expect("fixture stdin piped");
    let stdout = child.stdout.take().expect("fixture stdout piped");
    let events = Arc::new(Mutex::new(Vec::new()));
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let _reader = spawn_reader(stdout, label, events.clone(), pending.clone());
    Ok(Fixture {
        label,
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        events,
        pending,
        reader: Some(_reader),
    })
}

fn repo_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest.join("..").canonicalize()?)
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn wait_ready(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn random_token() -> String {
    let mut bytes = [0u8; 16];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        let _ = file.read_exact(&mut bytes);
    }
    hex::encode(bytes)
}

/// Static approval config modeled on examples/s1/host.toml: three service
/// principals (counter-ts, counter-rust, counter-rust-big), one preinstalled
/// contract, publish approvals per node and bidirectional consume approvals
/// with the TS counter for both Rust counters.
fn host_toml(port: u16, dir: &Path, token_ts: &str, token_rust: &str, token_big: &str) -> String {
    let digest = |token: &str| hex::encode(Sha256::digest(token.as_bytes()));
    let token = |principal: &str, hash: String| {
        format!(
            "[[tokens]]\ntoken_hash = \"{hash}\"\nprincipal_id = \"{principal}\"\ntenant_id = \"dev\"\naudience = \"conex.dev\"\nrole = \"service\"\n"
        )
    };
    let publish = |node: &str, endpoint: &str| {
        format!(
            "[[capabilities.publish]]\nnode_id = \"{node}\"\nspace_id = \"dev\"\nintegration_id = \"integration\"\nprovider_endpoint_id = \"{endpoint}\"\ncontract_id = \"{CONTRACT}\"\n"
        )
    };
    let consume = |principal: &str, endpoint: &str| {
        format!(
            "[[capabilities.consume]]\nprincipal_id = \"{principal}\"\nspace_id = \"dev\"\nprovider_endpoint_id = \"{endpoint}\"\ncontract_id = \"{CONTRACT}\"\n"
        )
    };
    format!(
        concat!(
            "listen = \"127.0.0.1:{port}\"\n",
            "allow_loopback_http = true\n",
            "audience = \"conex.dev\"\n",
            "content_root = \"{content}\"\n",
            "session_root = \"{session}\"\n",
            "operation_root = \"{operation}\"\n",
            "\n",
            "{token_ts}",
            "{token_rust}",
            "{token_big}",
            "\n",
            "[capabilities]\n",
            "\n",
            "[[capabilities.contracts]]\n",
            "contract_id = \"{CONTRACT}\"\n",
            "\n",
            "[[capabilities.contracts.methods]]\n",
            "name = \"counter/read\"\n",
            "input_schema = \"conex.CounterReadInput\"\n",
            "output_schema = \"conex.CounterResult\"\n",
            "action = \"counter.read\"\n",
            "execution = \"read_only\"\n",
            "\n",
            "[[capabilities.contracts.methods]]\n",
            "name = \"counter/add\"\n",
            "input_schema = \"conex.CounterAddInput\"\n",
            "output_schema = \"conex.CounterResult\"\n",
            "action = \"counter.add\"\n",
            "execution = \"non_replayable\"\n",
            "\n",
            "[[capabilities.contracts.states]]\n",
            "name = \"counter/value\"\n",
            "value_schema = \"conex.CounterStateValue\"\n",
            "action = \"counter.value\"\n",
            "\n",
            "{publish_ts}",
            "{publish_rust}",
            "{publish_big}",
            "\n",
            "{consume_ts_rust}",
            "{consume_rust_ts}",
            "{consume_big_ts}",
            "{consume_ts_big}",
        ),
        port = port,
        CONTRACT = CONTRACT,
        content = dir.join("content").display(),
        session = dir.join("session").display(),
        operation = dir.join("operation").display(),
        token_ts = token("counter-ts", digest(token_ts)),
        token_rust = token("counter-rust", digest(token_rust)),
        token_big = token("counter-rust-big", digest(token_big)),
        publish_ts = publish("counter-ts", "counter-ts"),
        publish_rust = publish("counter-rust", "counter-rust"),
        publish_big = publish("counter-rust-big", "counter-rust-big"),
        consume_ts_rust = consume("counter-ts", "counter-rust"),
        consume_rust_ts = consume("counter-rust", "counter-ts"),
        consume_big_ts = consume("counter-rust-big", "counter-ts"),
        consume_ts_big = consume("counter-ts", "counter-rust-big"),
    )
}

// ---------------------------------------------------------------------------
// JSON helpers
// ---------------------------------------------------------------------------

fn jstr(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn jnum(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(-1)
}

fn jbool(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// Seq arrives as a decimal string from real frames; accept number-or-string.
fn seq_of(value: &Value) -> String {
    match value.get("seq") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number.to_string(),
        _ => String::new(),
    }
}

fn count_of(value: &Value) -> i64 {
    jnum(value, "count")
}

/// Count inside a sample payload (`value.count`), as delivered by snapshots
/// and updates (unlike flat `{count,revision}` results).
fn sample_count(value: &Value) -> i64 {
    value
        .get("value")
        .and_then(|value| value.get("count"))
        .and_then(Value::as_i64)
        .unwrap_or(-1)
}

fn rev_of(value: &Value) -> String {
    jstr(value, "revision")
}

/// Normalize a module reference to camelCase regardless of the producing
/// fixture's casing.
fn normalize_ref(reference: &Value) -> Value {
    let Some(object) = reference.as_object() else {
        return Value::Null;
    };
    let pick = |names: &[&str]| -> Option<Value> {
        names.iter().find_map(|name| object.get(*name).cloned())
    };
    json!({
        "registrationId": pick(&["registrationId", "registration_id"]).unwrap_or(Value::Null),
        "providerEndpointId": pick(&["providerEndpointId", "provider_endpoint_id"]).unwrap_or(Value::Null),
        "spaceId": pick(&["spaceId", "space_id"]).unwrap_or(Value::Null),
        "integrationId": pick(&["integrationId", "integration_id"]).unwrap_or(Value::Null),
        "contractId": pick(&["contractId", "contract_id"]).unwrap_or(Value::Null),
        "methods": pick(&["methods"]).unwrap_or_else(|| json!([])),
        "states": pick(&["states"]).unwrap_or_else(|| json!([])),
    })
}

fn reg_of(reference: &Value) -> String {
    jstr(reference, "registrationId")
}

fn endpoint_id(module: &Value) -> String {
    jstr(module, "providerEndpointId")
}

/// Rust fixture list returns {"modules":[...]}, the TS fixture list returns
/// the bare array. Accept both.
fn modules_of(list: &Value) -> Vec<Value> {
    list.get("modules")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| list.as_array().cloned())
        .unwrap_or_default()
}

fn find_module(list: &Value, provider_endpoint_id: &str) -> Option<Value> {
    modules_of(list)
        .into_iter()
        .find(|module| endpoint_id(module) == provider_endpoint_id)
        .map(|module| normalize_ref(&module))
}

struct Outcome {
    expected: String,
    actual: String,
}

impl Outcome {
    fn new(expected: impl Into<String>, actual: impl Into<String>) -> Self {
        Self {
            expected: expected.into(),
            actual: actual.into(),
        }
    }
}

/// Consumer-projected reference for an endpoint, as seen by `node`. Subscribe
/// needs methods/states and the host needs the full key fields, so targets
/// always come from discovery rather than the publish-ready reference.
fn discovered_ref(node: &Fixture, _id: &str, endpoint: &str) -> Result<Value> {
    static DISC: AtomicU64 = AtomicU64::new(1);
    let id = format!("disc-{}", DISC.fetch_add(1, Ordering::Relaxed));
    let list = node.cmd_ok(json!({ "id": id, "op": "list" }))?;
    find_module(&list, endpoint).with_context(|| format!("list misses {endpoint}"))
}

fn inspect(node: &Fixture) -> Result<Value> {
    node.cmd_ok(json!({"id": "ctl", "op": "inspect"}))
}

/// TS-fixture invoke command; returns on the terminal result.
fn ts_invoke(
    ts: &Fixture,
    id: &str,
    call_id: &str,
    target: &Value,
    method: &str,
    argument: Value,
) -> Result<Value> {
    ts.cmd_ok(json!({
        "id": id,
        "op": "invoke",
        "callId": call_id,
        "target": target,
        "method": method,
        "argument": argument,
    }))
}

/// Rust-fixture invoke command; target is a provider endpoint id.
fn rust_invoke(
    rust: &Fixture,
    id: &str,
    call_id: &str,
    target: &str,
    method: &str,
    argument: Value,
) -> Result<Value> {
    rust.cmd_ok(json!({
        "id": id,
        "op": "invoke",
        "callId": call_id,
        "target": target,
        "method": method,
        "argument": argument,
    }))
}

fn arm(rust: &Fixture, point: &str) -> Result<Value> {
    rust.cmd_ok(json!({"id": "ctl", "op": "arm", "point": point}))
}

fn release(rust: &Fixture, point: &str) -> Result<Value> {
    rust.cmd_ok(json!({"id": "ctl", "op": "release", "point": point}))
}

fn mutate(rust: &Fixture, delta: i64) -> Result<Value> {
    rust.cmd_ok(json!({"id": "ctl", "op": "mutate", "delta": delta}))
}

fn assert_count(result: &Value, expected: i64, what: &str) -> Result<()> {
    let actual = count_of(result);
    if actual != expected {
        bail!("{what}: expected count {expected}, got {actual} ({result})");
    }
    Ok(())
}

fn assert_rev(result: &Value, expected: &str, what: &str) -> Result<()> {
    let actual = rev_of(result);
    if actual != expected {
        bail!("{what}: expected revision \"{expected}\", got \"{actual}\" ({result})");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// A01: bidirectional provide/consume with isolated domains.
fn a01(env: &mut Env) -> Result<Outcome> {
    let rust = env.node("rust");
    let ts = env.node("ts");

    // Discovery both directions.
    let rust_list = rust.cmd_ok(json!({"id": "a01", "op": "list"}))?;
    let ts_list = ts.cmd_ok(json!({"id": "a01", "op": "list"}))?;
    let ts_ref =
        find_module(&rust_list, "counter-ts").with_context(|| "rust list misses counter-ts")?;
    let rust_ref =
        find_module(&ts_list, "counter-rust").with_context(|| "ts list misses counter-rust")?;
    if reg_of(&ts_ref).is_empty() || reg_of(&rust_ref).is_empty() {
        bail!("A01 discovery returned refs without registrationId");
    }

    // TS invokes Rust: read=0, add(3)=3, read=3.
    assert_count(
        &ts_invoke(ts, "a01-1", "a01-1", &rust_ref, "counter/read", json!({}))?,
        0,
        "ts read rust",
    )?;
    assert_count(
        &ts_invoke(
            ts,
            "a01-2",
            "a01-2",
            &rust_ref,
            "counter/add",
            json!({"delta": 3}),
        )?,
        3,
        "ts add rust",
    )?;
    assert_count(
        &ts_invoke(ts, "a01-3", "a01-3", &rust_ref, "counter/read", json!({}))?,
        3,
        "ts read rust after add",
    )?;

    // Rust invokes TS: read=0, add(3)=3, read=3.
    assert_count(
        &rust_invoke(
            rust,
            "a01-4",
            "a01-4",
            "counter-ts",
            "counter/read",
            json!({}),
        )?,
        0,
        "rust read ts",
    )?;
    assert_count(
        &rust_invoke(
            rust,
            "a01-5",
            "a01-5",
            "counter-ts",
            "counter/add",
            json!({"delta": 3}),
        )?,
        3,
        "rust add ts",
    )?;
    assert_count(
        &rust_invoke(
            rust,
            "a01-6",
            "a01-6",
            "counter-ts",
            "counter/read",
            json!({}),
        )?,
        3,
        "rust read ts after add",
    )?;

    // Isolation: separate domains, both at 3.
    let rust_inspect = inspect(rust)?;
    let ts_inspect = inspect(ts)?;
    assert_count(&rust_inspect, 3, "rust inspect")?;
    assert_count(&ts_inspect, 3, "ts inspect")?;
    if rev_of(&rust_inspect) != "1" || rev_of(&ts_inspect) != "1" {
        bail!(
            "A01 isolation: expected both revisions 1, rust={} ts={}",
            rev_of(&rust_inspect),
            rev_of(&ts_inspect)
        );
    }

    // TS's own add must not leak into Rust's counter.
    let ts_mut = ts.cmd_ok(json!({"id": "a01-7", "op": "mutate", "delta": 1}))?;
    assert_count(&ts_mut, 4, "ts own mutate")?;
    let after_ts_add = ts_invoke(ts, "a01-8", "a01-8", &rust_ref, "counter/read", json!({}))?;
    assert_count(&after_ts_add, 3, "rust read after ts own add")?;
    assert_rev(&after_ts_add, "1", "rust revision after ts own add")?;
    if jnum(&inspect(rust)?, "executions") != 1 {
        bail!("A01 isolation: rust executions must stay 1");
    }

    Ok(Outcome::new(
        "both directions list, read=0 add(3)=3 read=3, isolated 3/3 domains, rust reads own 3 after ts add",
        format!(
            "rust=3 rev={} ts=3 rev={} rust-after-ts-add=3 executions=1",
            rev_of(&rust_inspect),
            rev_of(&ts_inspect)
        ),
    ))
}

/// A02: snapshot window and contiguous updates.
fn a02(env: &mut Env) -> Result<Outcome> {
    let rust = env.node("rust");
    let ts = env.node("ts");

    // Subscription targets need the consumer-projected reference (methods/
    // states); the publish-ready reference is provider-side only.
    let ts_list = ts.cmd_ok(json!({"id": "a02-list", "op": "list"}))?;
    let rust_ref =
        find_module(&ts_list, "counter-rust").with_context(|| "ts list misses counter-rust")?;

    // Rust commits delta 2; chained scenarios share the node, so all counts
    // and revisions are asserted relative to the pre-scenario state.
    let before = inspect(rust)?;
    let c0 = count_of(&before);
    let r0: u64 = rev_of(&before).parse().unwrap_or(0);
    let e0 = jnum(&before, "executions");
    let m2 = mutate(rust, 2)?;
    if count_of(&m2) != c0 + 2 || rev_of(&m2) != (r0 + 1).to_string() {
        bail!(
            "A02 rust mutate 2 expected count {} revision {}, got count {} revision \"{}\"",
            c0 + 2,
            r0 + 1,
            count_of(&m2),
            rev_of(&m2)
        );
    }

    // Arm the snapshot gate, subscribe from TS; the subscribe is held.
    let mark = rust.events_len();
    arm(rust, "snapshot")?;
    let subscribe_rx = ts.send(json!({
        "id": "a02-sub",
        "op": "subscribe",
        "subscriptionKey": "a02",
        "target": rust_ref,
        "state": "counter/value",
        "argument": {},
    }))?;

    // The provider reads the snapshot and blocks at the gate.
    let snapshot_barrier = rust.wait_event_after(mark, |event| {
        event.get("event").and_then(Value::as_str) == Some("barrier")
            && event.get("point").and_then(Value::as_str) == Some("snapshot")
    })?;
    if count_of(&snapshot_barrier) != c0 + 2 || rev_of(&snapshot_barrier) != (r0 + 1).to_string() {
        bail!(
            "A02 snapshot barrier: expected count={} revision={}, got {} / {}",
            c0 + 2,
            r0 + 1,
            jnum(&snapshot_barrier, "count"),
            rev_of(&snapshot_barrier)
        );
    }

    // While held, a commit on Rust must complete (TS-invoked add(5)).
    let add5 = ts_invoke(
        ts,
        "a02-5",
        "a02-5",
        &rust_ref,
        "counter/add",
        json!({"delta": 5}),
    )?;
    if count_of(&add5) != c0 + 7 || rev_of(&add5) != (r0 + 2).to_string() {
        bail!(
            "A02 ts add rust while held must complete count={} rev={}, got {} / {}",
            c0 + 7,
            r0 + 2,
            count_of(&add5),
            rev_of(&add5)
        );
    }

    release(rust, "snapshot")?;

    // Snapshot first: the value captured before the held commit.
    let subscribe = ts.recv(subscribe_rx)?;
    if !jbool(&subscribe, "ok") {
        bail!("A02 subscribe failed: {subscribe}");
    }
    let snapshot = subscribe
        .get("value")
        .and_then(|v| v.get("snapshot"))
        .cloned()
        .unwrap_or(Value::Null);
    let snapshot_count = snapshot
        .get("value")
        .and_then(|v| v.get("count"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if snapshot_count != c0 + 2 || rev_of(&snapshot) != (r0 + 1).to_string() {
        bail!(
            "A02 snapshot: expected count={} revision=\"{}\", got count={} revision=\"{}\"",
            c0 + 2,
            r0 + 1,
            snapshot_count,
            rev_of(&snapshot)
        );
    }

    // Then the buffered update: seq="1", no missing seq.
    let update = ts.wait_event(|event| {
        event.get("event").and_then(Value::as_str) == Some("update")
            && event.get("subscriptionKey").and_then(Value::as_str) == Some("a02")
    })?;
    let seq = jstr(&update, "seq");
    if seq != "1" {
        bail!("A02 first update seq must be 1, got {seq}");
    }
    let sample = update.get("sample").cloned().unwrap_or(Value::Null);
    let sample_count = sample
        .get("value")
        .and_then(|v| v.get("count"))
        .and_then(Value::as_i64)
        .unwrap_or(-1);
    if sample_count != c0 + 7 || rev_of(&sample) != (r0 + 2).to_string() {
        bail!(
            "A02 first update sample: expected count={} revision=\"{}\", got count={} revision=\"{}\"",
            c0 + 7,
            r0 + 2,
            sample_count,
            rev_of(&sample)
        );
    }

    let rust_inspect = inspect(rust)?;
    if count_of(&rust_inspect) != c0 + 7
        || jnum(&rust_inspect, "executions") != e0 + 2
        || rev_of(&rust_inspect) != (r0 + 2).to_string()
    {
        bail!("A02 rust inspect after update: {}", rust_inspect);
    }

    Ok(Outcome::new(
        "snapshot before held commit, then seq=1 update with the committed value, no gap",
        format!(
            "snapshot={}/{} update=seq {}/{}/{} rust={} rev={} executions={}",
            snapshot_count,
            rev_of(&snapshot),
            seq,
            sample_count,
            rev_of(&sample),
            count_of(&rust_inspect),
            rev_of(&rust_inspect),
            e0 + 2
        ),
    ))
}

/// A03: registration generation replacement; old references stay stale.
fn a03(env: &mut Env) -> Result<Outcome> {
    // Capture the current (old) registration through discovery so the pinned
    // reference carries the full consumer-view fields.
    let rust = env.node("rust");
    let pre_list = rust.cmd_ok(json!({"id": "a03-0", "op": "list"}))?;
    let old_ref = find_module(&pre_list, "counter-ts")
        .with_context(|| "A03: counter-ts missing before new generation")?;
    let old_reg = reg_of(&old_ref);
    if old_reg.is_empty() {
        bail!("A03: no registrationId for the original ts fixture");
    }

    // A second TS fixture with identical endpoint/token registers a new
    // generation.
    let new_ref = env.fixture("ts2", "counter-ts", env.token_ts.clone(), &[])?;
    let new_reg = reg_of(&new_ref);
    if new_reg == old_reg {
        bail!("A03: new generation reused registrationId {new_reg}");
    }

    // Invoking the old reference yields stale_revision (-32013). The caller
    // must be an approved consumer of counter-ts (rust), not ts itself.
    let ts1 = env.node("ts");
    let rust_caller = env.node("rust");
    let err = rust_caller.cmd_err(json!({
        "id": "a03-1",
        "op": "invoke",
        "callId": "a03-1",
        "target": old_ref,
        "method": "counter/read",
        "argument": {},
    }))?;
    let code = jnum(&err, "code");
    if code != -32013 {
        bail!("A03: old reference invoke expected stale_revision (-32013), got code {code}: {err}");
    }

    // Close the old fixture; the new registration must survive.
    ts1.cmd_ok(json!({"id": "a03-2", "op": "close"}))?;

    let rust = env.node("rust");
    let rust_list = rust.cmd_ok(json!({"id": "a03-3", "op": "list"}))?;
    let listed = find_module(&rust_list, "counter-ts")
        .with_context(|| "A03: counter-ts missing from rust list after old close")?;
    if reg_of(&listed) != new_reg {
        bail!(
            "A03: rust list shows counter-ts registration {} instead of new {}",
            reg_of(&listed),
            new_reg
        );
    }

    Ok(Outcome::new(
        "old ref -> stale_revision; close old fixture; list keeps new registrationId",
        format!("old={old_reg} new={new_reg} invoke-code={code} list-alive"),
    ))
}

/// A05: cancel a before-commit-barriered invoke; contract expects no commit.
/// NOTE: the current Rust fixture handler never observes cancellation (its
/// `add` handler ignores the SDK cancel signal), so this scenario may expose a
/// fixture/brief gap by failing with the observed settle.
fn a05(env: &mut Env) -> Result<Outcome> {
    let rust = env.node("rust");
    let ts = env.node("ts2");
    let rust_ref = discovered_ref(ts, "a05-disc", "counter-rust")?;

    let before = inspect(rust)?;
    let count_before = count_of(&before);
    let executions_before = jnum(&before, "executions");

    let mark = rust.events_len();
    arm(rust, "before_commit")?;
    let invoke_rx = ts.send(json!({
        "id": "a05-inv",
        "op": "invoke",
        "callId": "a05-inv",
        "target": rust_ref,
        "method": "counter/add",
        "argument": {"delta": 4},
    }))?;

    let barrier = rust.wait_event_after(mark, |event| {
        event.get("event").and_then(Value::as_str) == Some("barrier")
            && event.get("point").and_then(Value::as_str) == Some("before_commit")
    })?;
    if count_of(&barrier) != count_before || rev_of(&barrier) != rev_of(&before) {
        bail!(
            "A05 before_commit barrier must show pre-commit state {} / {}, got {} / {}",
            count_before,
            rev_of(&before),
            jnum(&barrier, "count"),
            rev_of(&barrier)
        );
    }

    let cancel = ts.cmd_ok(json!({"id": "a05-c", "op": "cancel", "callId": "a05-inv"}))?;
    if !jbool(&cancel, "requested") {
        bail!("A05 cancel expected requested=true: {cancel}");
    }

    // Acceptance is not confirmation. Keep commit blocked until the provider
    // observes the signal and settles the original invocation itself.

    let settle = ts.recv(invoke_rx)?;
    let settle_ok = jbool(&settle, "ok");
    let after = inspect(rust)?;
    let count_after = count_of(&after);
    let executions_after = jnum(&after, "executions");

    let settled_cancelled = !settle_ok
        && jnum(&settle["error"], "code") == -32011
        && jstr(&settle["error"], "execution") == "not_started"
        && jstr(&settle["error"], "retry") == "never";
    if !settled_cancelled || count_after != count_before || executions_after != executions_before {
        let what = if settle_ok {
            format!("success {}", settle["value"])
        } else {
            format!("error {}", settle["error"])
        };
        bail!(
            "A05 fixture gap: expected cancelled/not_started settle with count {count_before} \
             and executions {executions_before} unchanged (brief), but observed settle={what}, \
             count {count_before}->{count_after}, executions {executions_before}->{executions_after}"
        );
    }
    support_test(
        &env.root,
        &[
            "test",
            "-p",
            "conex-host",
            "--test",
            "s1_calls",
            "queued_budget_exhaustion_settles_not_started_and_never_sends",
        ],
    )?;

    Ok(Outcome::new(
        "cancel requested -> invoked settle cancelled/not_started, count+executions unchanged",
        format!(
            "cancel=true settle={} count={count_after} executions={executions_after}",
            settle
        ),
    ))
}

/// A06: cancel after commit; the committed side effect wins, no rollback.
fn a06(env: &mut Env) -> Result<Outcome> {
    let rust = env.node("rust");
    let ts = env.node("ts2");
    let rust_ref = discovered_ref(ts, "a05-disc", "counter-rust")?;

    let before = inspect(rust)?;
    let count_before = count_of(&before);
    let executions_before = jnum(&before, "executions");

    let mark = rust.events_len();
    arm(rust, "after_commit")?;
    let invoke_rx = ts.send(json!({
        "id": "a06-inv",
        "op": "invoke",
        "callId": "a06-inv",
        "target": rust_ref,
        "method": "counter/add",
        "argument": {"delta": 4},
    }))?;

    // Barrier reached after the commit: count is already +4.
    let barrier = rust.wait_event_after(mark, |event| {
        event.get("event").and_then(Value::as_str) == Some("barrier")
            && event.get("point").and_then(Value::as_str) == Some("after_commit")
    })?;
    let committed_count = jnum(&barrier, "count");
    let committed_rev = rev_of(&barrier);
    if committed_count != count_before + 4
        || committed_rev
            != rev_of(&before)
                .parse::<u64>()
                .map(|r| (r + 1).to_string())
                .unwrap_or_default()
    {
        bail!(
            "A06 after_commit barrier expected count {} rev {}, got {} / {}",
            count_before + 4,
            rev_of(&before)
                .parse::<u64>()
                .map(|r| (r + 1).to_string())
                .unwrap_or_default(),
            committed_count,
            committed_rev
        );
    }

    let cancel = ts.cmd_ok(json!({"id": "a06-c", "op": "cancel", "callId": "a06-inv"}))?;
    if !jbool(&cancel, "requested") {
        bail!("A06 cancel expected requested=true: {cancel}");
    }

    release(rust, "after_commit")?;

    // Exactly one terminal: the committed success; cancel never settles it.
    let settle = ts.recv(invoke_rx)?;
    if !jbool(&settle, "ok")
        || count_of(settle.get("value").unwrap_or(&Value::Null)) != committed_count
    {
        bail!("A06 invoke must settle success with committed count {committed_count}: {settle}");
    }

    let after = inspect(rust)?;
    if count_of(&after) != committed_count || jnum(&after, "executions") != executions_before + 1 {
        bail!(
            "A06 rust inspect: count {committed_count} executions {} expected: {after}",
            executions_before + 1
        );
    }

    Ok(Outcome::new(
        "cancel after commit -> invoke success with committed count, executions +1, no rollback",
        format!(
            "barrier-count={committed_count} cancel=true settle=ok({}) executions {}->{}",
            settle["value"],
            executions_before,
            executions_before + 1
        ),
    ))
}

/// A07: disconnect after commit -> outcome_unknown, no replay after reconnect.
fn a07(env: &mut Env) -> Result<Outcome> {
    let rust = env.node("rust");
    let ts = env.node("ts2");
    let rust_ref = discovered_ref(ts, "a05-disc", "counter-rust")?;

    let before = inspect(rust)?;
    let executions_before = jnum(&before, "executions");

    let mark = rust.events_len();
    arm(rust, "after_commit")?;
    let invoke_rx = ts.send(json!({
        "id": "a07-inv",
        "op": "invoke",
        "callId": "a07-inv",
        "target": rust_ref,
        "method": "counter/add",
        "argument": {"delta": 6},
    }))?;

    let barrier = rust.wait_event_after(mark, |event| {
        event.get("event").and_then(Value::as_str) == Some("barrier")
            && event.get("point").and_then(Value::as_str) == Some("after_commit")
    })?;
    let committed_count = jnum(&barrier, "count");
    let committed_rev = rev_of(&barrier);
    if committed_count != count_of(&before) + 6 {
        bail!(
            "A07 after_commit barrier expected count {}, got {}",
            count_of(&before) + 6,
            committed_count
        );
    }

    // The consuming TS link closes while the provider is held: the pending
    // non_replayable invoke must settle outcome_unknown, retry never.
    ts.cmd_ok(json!({"id": "a07-c", "op": "close"}))?;
    let settle = ts.recv(invoke_rx)?;
    let error = settle.get("error").cloned().unwrap_or(Value::Null);
    let code = jnum(&error, "code");
    if jbool(&settle, "ok") || code != -32012 {
        bail!("A07 invoke after consumer close must settle -32012 outcome_unknown, got: {settle}");
    }
    if jstr(&error, "execution") != "unknown" || jstr(&error, "retry") != "never" {
        bail!("A07 settle must be execution=unknown retry=never, got: {error}");
    }

    // The commit happened exactly once; no rollback on disconnect.
    let after = inspect(rust)?;
    if count_of(&after) != committed_count || jnum(&after, "executions") != executions_before + 1 {
        bail!(
            "A07 rust must hold committed count {committed_count} exactly once (executions {}): {after}",
            executions_before + 1
        );
    }

    release(rust, "after_commit")?;
    // Fresh TS fixture reconnects: reading Rust must show the single commit
    // (no replay), and Rust's execution count stays +1.
    env.fixture("ts3", "counter-ts", env.token_ts.clone(), &[])?;
    let fresh = env.node("ts3");
    let rust = env.node("rust");
    let read = ts_invoke(
        fresh,
        "a07-r",
        "a07-r",
        &rust_ref,
        "counter/read",
        json!({}),
    )?;
    if count_of(&read) != committed_count || rev_of(&read) != committed_rev {
        bail!(
            "A07 no replay: fresh ts read must be {} / {}, got {} / {}",
            committed_count,
            committed_rev,
            count_of(&read),
            rev_of(&read)
        );
    }
    let final_inspect = inspect(rust)?;
    if jnum(&final_inspect, "executions") != executions_before + 1
        || count_of(&final_inspect) != committed_count
    {
        bail!(
            "A07 no replay: rust executions must stay {}: {final_inspect}",
            executions_before + 1
        );
    }

    Ok(Outcome::new(
        "after_commit + consumer close -> -32012 unknown/never, single commit, no replay on reconnect",
        format!(
            "settle={code}/unknown/never count={committed_count} rev={committed_rev} executions={} re-read={committed_count}",
            executions_before + 1
        ),
    ))
}

/// A12: int32 overflow, malformed provider output, exact uint64 revisions.
fn a12(env: &mut Env) -> Result<Outcome> {
    // Spawned up front so the &mut Env borrow does not overlap the shared
    // fixture handles used by parts 1-2. A10 replaced the TS provider, so
    // this scenario uses its latest live generation.
    env.fixture(
        "rust-big",
        "counter-rust-big",
        env.token_big.clone(),
        &[("CONEX_INITIAL_REVISION", "9007199254740993")],
    )?;
    let rust = env.node("rust2");
    let ts = env.node("ts5");
    let rust_ref = discovered_ref(ts, "a05-disc", "counter-rust")?;

    // --- Part 1: int32 overflow rejects before mutation.
    let before = inspect(rust)?;
    let count_before = count_of(&before);
    let executions_before = jnum(&before, "executions");
    let to_max = 2_147_483_647_i64 - count_before;
    let m = mutate(rust, to_max)?;
    if count_of(&m) != 2_147_483_647 {
        bail!("A12 setup mutate to i32::MAX failed: {m}");
    }
    let rev_max = rev_of(&m);

    let overflow = ts.cmd_err(json!({
        "id": "a12-1",
        "op": "invoke",
        "callId": "a12-1",
        "target": rust_ref,
        "method": "counter/add",
        "argument": {"delta": 1},
    }))?;
    let overflow_code = jnum(&overflow, "code");
    if overflow_code != -32602 {
        bail!("A12 int32 overflow expected -32602, got {overflow_code}: {overflow}");
    }
    if jstr(&overflow, "execution") != "not_started" || jstr(&overflow, "retry") != "never" {
        bail!("A12 overflow settle must be not_started/never: {overflow}");
    }
    let after_overflow = inspect(rust)?;
    if count_of(&after_overflow) != 2_147_483_647
        || rev_of(&after_overflow) != rev_max
        || jnum(&after_overflow, "executions") != executions_before + 1
    {
        bail!(
            "A12 overflow must leave count/revision unchanged (count=2147483647 rev={rev_max}): {after_overflow}"
        );
    }

    // --- Part 2: malformed provider output -> outcome_unknown on the
    // consumer while the TS commit still happened.
    let malformed_code = {
        let provider = env.node("ts5");
        let provider_before = inspect(provider)?;
        // Resolve the current provider generation through discovery.
        let ts_target = discovered_ref(rust, "a12-disc", "counter-ts")?;
        provider.cmd_ok(json!({"id": "a12-2", "op": "fault", "kind": "malformed_output"}))?;
        let malformed = rust.cmd_err(json!({
            "id": "a12-3",
            "op": "invoke",
            "callId": "a12-3",
            "target": ts_target,
            "method": "counter/add",
            "argument": {"delta": 1},
        }))?;
        let code = jnum(&malformed, "code");
        if code != -32012 {
            bail!("A12 malformed output expected -32012 on consumer, got {code}: {malformed}");
        }
        if jstr(&malformed, "execution") != "unknown" || jstr(&malformed, "retry") != "never" {
            bail!("A12 malformed settle must be unknown/never: {malformed}");
        }
        let provider_after = inspect(provider)?;
        if count_of(&provider_after) != count_of(&provider_before) + 1
            || jnum(&provider_after, "executions") != jnum(&provider_before, "executions") + 1
        {
            bail!(
                "A12 exactly one TS commit despite corrupted reply: before={provider_before} after={provider_after}"
            );
        }
        code
    };

    // --- Part 3: exact uint64 revisions beyond 2^53.
    let big_ref = discovered_ref(ts, "disc-rust-big", "counter-rust-big")?;
    let big = env.node("rust-big");
    let ts3 = ts;

    let read_big = ts_invoke(ts3, "a12-4", "a12-4", &big_ref, "counter/read", json!({}))?;
    if count_of(&read_big) != 0 || rev_of(&read_big) != "9007199254740993" {
        bail!("A12 big read expected count=0 revision=\"9007199254740993\", got {read_big}");
    }

    let sub_rx = ts3.send(json!({
        "id": "a12-sub",
        "op": "subscribe",
        "subscriptionKey": "a12big",
        "target": big_ref,
        "state": "counter/value",
        "argument": {},
    }))?;
    let subscribe = ts3.recv(sub_rx)?;
    if !jbool(&subscribe, "ok") {
        bail!("A12 big subscribe failed: {subscribe}");
    }
    let snapshot = subscribe
        .get("value")
        .and_then(|v| v.get("snapshot"))
        .cloned()
        .unwrap_or(Value::Null);
    if rev_of(&snapshot) != "9007199254740993" || sample_count(&snapshot) != 0 {
        bail!(
            "A12 big snapshot must be exactly revision=\"9007199254740993\" count=0, got {snapshot}"
        );
    }

    let big_mut = big.cmd_ok(json!({"id": "a12-5", "op": "mutate", "delta": 1}))?;
    if count_of(&big_mut) != 1 || rev_of(&big_mut) != "9007199254740994" {
        bail!("A12 big mutate must yield revision=\"9007199254740994\", got {big_mut}");
    }
    let update =
        ts.wait_event(|event| event["event"] == "update" && event["subscriptionKey"] == "a12big")?;
    if seq_of(&update) != "1"
        || rev_of(&update["sample"]) != "9007199254740994"
        || sample_count(&update["sample"]) != 1
    {
        bail!("A12 consumer lost uint64 precision in update: {update}");
    }

    Ok(Outcome::new(
        "add overflow -> -32602 not_started/never unchanged; malformed ts output -> -32012 unknown/never with commit; revisions 9007199254740993 -> 9007199254740994 exact",
        format!(
            "overflow={overflow_code}/not_started/never count@max unchanged; malformed={malformed_code}/unknown/never ts-commit=+1/executions+1; big=9007199254740993 -> {}/1",
            rev_of(&big_mut)
        ),
    ))
}

/// Run an existing deterministic host regression as labeled SUPPORT evidence.
fn support_test(root: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .output()
        .context("spawn support test")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    print!("{stdout}");
    std::io::stderr().write_all(&output.stderr)?;
    if !output.status.success() {
        bail!("support test failed: {args:?}");
    }
    if args.first() == Some(&"test") {
        let name = args.last().context("missing support test name")?;
        if !stdout.contains(&format!("test {name} ... ok")) {
            bail!("support test did not execute its named case: {name}");
        }
    }
    Ok(())
}

/// A04: request-id isolation across links; forged identity and unauthorized
/// targets never reach business handlers.
fn a04(env: &mut Env) -> Result<Outcome> {
    env.plain("probe-a04", env.token_ts.clone(), &[])?;
    let rust = env.node("rust");
    let ts = env.node("ts3");
    let rust_ref = discovered_ref(ts, "a04-disc", "counter-rust")?;
    let registration = jstr(&rust_ref, "registrationId");
    if registration.is_empty() {
        bail!("A04: rust registrationId missing");
    }
    let before = inspect(rust)?;
    let count_before = count_of(&before);
    let executions_before = jnum(&before, "executions");

    let probe = env.node("probe-a04");
    probe.cmd_ok(json!({"id": "a04-hello", "op": "hello"}))?;

    // Same wire request id from two different links must resolve
    // independently: both calls commit and neither crosses.
    let dup_id = "01ARZ3NDEKTSV4RRFFQ69G5FD9";
    let mark = rust.events_len();
    arm(rust, "after_commit")?;
    let sdk_pending = ts.send(json!({
        "id": "a04-dup-1", "op": "invoke", "callId": "a04-dup-1",
        "wireRequestId": dup_id, "target": rust_ref,
        "method": "counter/add", "argument": {"delta": 1},
    }))?;
    rust.wait_event_after(mark, |event| {
        event["event"] == "barrier" && event["point"] == "after_commit"
    })?;
    let raw_side = probe.cmd_ok(json!({
        "id": "a04-dup-2", "op": "invoke", "requestId": dup_id,
        "target": {"providerEndpointId": "counter-rust", "registrationId": registration},
        "method": "counter/add", "argument": {"delta": 1},
    }))?;
    release(rust, "after_commit")?;
    let sdk_side = ts.recv(sdk_pending)?;
    if sdk_side["ok"] != true
        || count_of(&sdk_side["value"]) != count_before + 1
        || count_of(&raw_side) != count_before + 2
    {
        bail!("A04 overlapping same-id calls crossed: SDK={sdk_side} raw={raw_side}");
    }

    // Require an observed boundary rejection, not merely an elapsed timeout.
    // Provider inspection below independently proves no business execution.
    let forged = probe.cmd_ok(json!({
        "id": "a04-forged",
        "op": "forged_identity",
        "target": {"providerEndpointId": "counter-rust", "registrationId": registration},
        "method": "counter/add",
        "argument": {"delta": 99},
        "fakePrincipal": "counter-rust",
    }))?;
    let rejected = forged["value"]["error"]["code"].as_i64() == Some(-32602)
        || forged["wireErrors"].as_array().is_some_and(|errors| {
            errors
                .iter()
                .any(|error| error["code"].as_i64() == Some(-32602))
        });
    if !rejected {
        bail!("A04 forged identity must be rejected: {forged}");
    }

    // Unauthorized target: the probe principal has no consume approval for
    // counter-ts; host must answer forbidden without forwarding. Executions
    // below prove zero business frames.
    // ts3's own principal is not an approved consumer of counter-ts, so the
    // reference must come from an approved viewer (rust).
    let ts_ref = discovered_ref(rust, "a04-disc-ts", "counter-ts")?;
    let ts_before = inspect(ts)?;
    let unauthorized = probe
        .cmd_ok(json!({
            "id": "a04-forbidden",
            "op": "forbidden_target",
            "target": {"providerEndpointId": "counter-ts", "registrationId": jstr(&ts_ref, "registrationId")},
            "method": "counter/add",
            "argument": {"delta": 99},
        }))?;
    let forbidden_code = unauthorized
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_i64);
    if forbidden_code != Some(-32002) {
        bail!("A04 unauthorized target must be forbidden (-32002): {unauthorized}");
    }
    let ts_after = inspect(ts)?;
    if count_of(&ts_after) != count_of(&ts_before)
        || jnum(&ts_after, "executions") != jnum(&ts_before, "executions")
    {
        bail!("A04 forbidden invocation reached TS provider: {ts_before} -> {ts_after}");
    }

    let after = inspect(rust)?;
    if count_of(&after) != count_before + 2 || jnum(&after, "executions") != executions_before + 2 {
        bail!(
            "A04 forged/unauthorized must not execute business: {}",
            after
        );
    }

    Ok(Outcome::new(
        "same id two links independent; forged identity + unauthorized target rejected, zero extra executions",
        format!(
            "dup=+1/+1 forged={} executions={}->{}",
            forged
                .get("rejected")
                .and_then(Value::as_str)
                .unwrap_or("?"),
            executions_before,
            executions_before + 2
        ),
    ))
}

/// A08: provider link close ends the subscription; a fresh generation
/// re-subscribes with a new snapshot. Registration-end variant is SUPPORT.
fn a08(env: &mut Env) -> Result<Outcome> {
    // Phase 1: subscribe to the current generation, close it, observe the
    // end. Phase 2 (after the &mut spawn below) resubscribes on the new
    // generation.
    let end_code = {
        let ts3 = env.node("ts3");
        let rust_ref = discovered_ref(ts3, "a08-disc", "counter-rust")?;
        let sub_rx = ts3.send(json!({
            "id": "a08-sub", "op": "subscribe", "subscriptionKey": "a08",
            "target": rust_ref, "state": "counter/value", "argument": {},
        }))?;
        let snapshot = ts3.recv(sub_rx)?;
        if !jbool(&snapshot, "ok") {
            bail!("A08 subscribe failed: {snapshot}");
        }

        env.node("rust")
            .cmd_ok(json!({"id": "a08-close", "op": "close"}))?;
        let end = ts3.wait_event(|event| {
            event.get("event").and_then(Value::as_str) == Some("end")
                && event.get("subscriptionKey").and_then(Value::as_str) == Some("a08")
        })?;
        let code = jnum(&end.get("error").cloned().unwrap_or(Value::Null), "code");
        if code != -32005 {
            bail!("A08 provider close must end -32005, got {end}");
        }
        code
    };

    // New generation of the same endpoint: fresh subscription, new snapshot.
    env.fixture("rust2", "counter-rust", env.token_rust.clone(), &[])?;
    {
        let ts3 = env.node("ts3");
        let rust_ref2 = discovered_ref(ts3, "a08-disc2", "counter-rust")?;
        let sub_rx2 = ts3.send(json!({
            "id": "a08-sub2", "op": "subscribe", "subscriptionKey": "a08b",
            "target": rust_ref2, "state": "counter/value", "argument": {},
        }))?;
        let snapshot2 = ts3.recv(sub_rx2)?;
        if !jbool(&snapshot2, "ok") {
            bail!("A08 resubscribe after new generation failed: {snapshot2}");
        }
    }

    support_test(
        &env.root,
        &[
            "test",
            "-p",
            "conex-host",
            "--test",
            "s1_state",
            "registration_end_ends_subscriptions",
        ],
    )?;

    Ok(Outcome::new(
        "provider close ends -32005; new generation resubscribes with snapshot; registration-end SUPPORT",
        format!("end={} resubscribe=ok", end_code),
    ))
}

/// A09: another node keeps operating while subscriptions are live; the queue
/// ceiling itself is deterministic host regression territory (SUPPORT).
fn a09(env: &mut Env) -> Result<Outcome> {
    env.plain("probe-a09", env.token_ts.clone(), &[])?;
    let ts3 = env.node("ts3");
    let rust_ref = discovered_ref(ts3, "a09-disc", "counter-rust")?;

    let sub_rx = ts3.send(json!({
        "id": "a09-sub", "op": "subscribe", "subscriptionKey": "a09",
        "target": rust_ref, "state": "counter/value", "argument": {},
    }))?;
    let snapshot = ts3.recv(sub_rx)?;
    if !jbool(&snapshot, "ok") {
        bail!("A09 subscribe failed: {snapshot}");
    }

    // A different real link (the probe's) still reads while the subscriber
    // is attached.
    let probe = env.node("probe-a09");
    probe.cmd_ok(json!({"id": "a09-hello", "op": "hello"}))?;
    let read = probe.cmd_ok(json!({
        "id": "a09-read",
        "op": "invoke",
        "target": {"providerEndpointId": "counter-rust", "registrationId": jstr(&rust_ref, "registrationId")},
        "method": "counter/read",
        "argument": {},
    }))?;
    if count_of(&read) < 0 {
        bail!("A09 concurrent read failed: {read}");
    }

    support_test(
        &env.root,
        &[
            "test",
            "-p",
            "conex-host",
            "--test",
            "s1_state",
            "control_reserve_and_full_control_space_close_consumer",
        ],
    )?;
    support_test(
        &env.root,
        &[
            "test",
            "-p",
            "conex-host",
            "--test",
            "s1_state",
            "grey_window_overflow_delivers_end_after_snapshot",
        ],
    )?;

    Ok(Outcome::new(
        "subscription active while another link reads; queue ceiling SUPPORT (control reserve)",
        format!("read={} support=control_reserve ok", count_of(&read)),
    ))
}

/// A10: injected sequence gap / revision rollback / bad quality each end the
/// affected subscription with session_lost while another live subscription
/// keeps its own clean update; forged subscription ids create nothing.
/// Each fault phase runs against a fresh provider generation so the next
/// outgoing frame is unambiguously the scenario target's.
fn a10(env: &mut Env) -> Result<Outcome> {
    let outcome = (|env: &mut Env| -> Result<(String, String, String)> {
        let rust = env.node("rust2");
        let ts = env.node("ts3");
        let ts_ref = discovered_ref(rust, "a10-disc", "counter-ts")?;

        let subscribe = |rust: &Fixture, key: &str, target: &Value| -> Result<()> {
            let rx = rust.send(json!({
                "id": format!("a10-sub-{key}"), "op": "subscribe", "subscriptionKey": key,
                "target": target, "state": "counter/value", "argument": {},
            }))?;
            let response = rust.recv(rx)?;
            if !jbool(&response, "ok") {
                bail!("A10 subscribe {key} failed: {response}");
            }
            Ok(())
        };
        let mark = rust.events_len();

        // Phase 1 (seq gap): two subscribers; the gap ends exactly one (the
        // first outgoing frame), the survivor keeps its own clean update.
        subscribe(rust, "a10-a", &ts_ref)?;
        subscribe(rust, "a10-b", &ts_ref)?;
        ts.cmd_ok(json!({"id": "a10-f1", "op": "fault", "kind": "seq_gap"}))?;
        ts.cmd_ok(json!({"id": "a10-m1", "op": "mutate", "delta": 1}))?;
        let end = rust
            .wait_event_after(mark, |event| {
                event.get("event").and_then(Value::as_str) == Some("end")
            })
            .map_err(|error| {
                let tail: Vec<Value> = {
                    let events = rust.events.lock().unwrap();
                    events.iter().skip(mark).cloned().collect()
                };
                anyhow::anyhow!("{error}; rust2 events after mark: {tail:?}")
            })?;
        let gap_code = jnum(&end.get("error").cloned().unwrap_or(Value::Null), "code");
        let gap_key = jstr(&end, "subscriptionKey");
        if gap_code != -32017 {
            bail!("A10 seq gap must end session_lost: {end}");
        }
        let survivor = if gap_key == "a10-a" { "a10-b" } else { "a10-a" };
        let update = rust.wait_event_after(mark, |event| {
            event.get("event").and_then(Value::as_str) == Some("update")
                && event.get("subscriptionKey").and_then(Value::as_str) == Some(survivor)
        })?;
        if seq_of(&update) != "1" {
            bail!("A10 sibling update must keep its own seq, got {update}");
        }
        Ok((gap_key, survivor.to_string(), "phase1".to_string()))
    })(env)?;

    // Phase 2 (revision rollback) on a fresh provider generation.
    env.fixture("ts4", "counter-ts", env.token_ts.clone(), &[])?;
    (|env: &mut Env| -> Result<()> {
        let rust = env.node("rust2");
        let ts4 = env.node("ts4");
        let ts_ref = discovered_ref(rust, "a10-disc-ts4", "counter-ts")?;
        let subscribe = |key: &str| -> Result<()> {
            let rx = rust.send(json!({
                "id": format!("a10-sub-{key}"), "op": "subscribe", "subscriptionKey": key,
                "target": ts_ref, "state": "counter/value", "argument": {},
            }))?;
            let response = rust.recv(rx)?;
            if !jbool(&response, "ok") {
                bail!("A10 subscribe {key} failed: {response}");
            }
            Ok(())
        };
        let mark = rust.events_len();
        subscribe("a10-c")?;
        ts4.cmd_ok(json!({"id": "a10-f2", "op": "fault", "kind": "revision_rollback"}))?;
        ts4.cmd_ok(json!({"id": "a10-m2", "op": "mutate", "delta": 1}))?;
        let end = rust
            .wait_event_after(mark, |event| {
                event.get("event").and_then(Value::as_str) == Some("end")
            })
            .context("A10 rollback end")?;
        if jstr(&end, "subscriptionKey") != "a10-c"
            || jnum(&end.get("error").cloned().unwrap_or(Value::Null), "code") != -32017
        {
            bail!("A10 revision rollback must end a10-c session_lost: {end}");
        }
        Ok(())
    })(env)?;

    // Phase 3 (bad quality) on another fresh generation, plus forged
    // subscription ids from a non-provider link creating nothing.
    env.fixture("ts5", "counter-ts", env.token_ts.clone(), &[])?;
    let gap_key = outcome.0.clone();
    {
        let rust = env.node("rust2");
        let ts5 = env.node("ts5");
        let ts_ref = discovered_ref(rust, "a10-disc-ts5", "counter-ts")?;
        let rx = rust.send(json!({
            "id": "a10-sub-live", "op": "subscribe", "subscriptionKey": "a10-live",
            "target": ts_ref, "state": "counter/value", "argument": {},
        }))?;
        let response = rust.recv(rx)?;
        if !jbool(&response, "ok") {
            bail!("A10 subscribe a10-live failed: {response}");
        }
        let mark = rust.events_len();
        ts5.cmd_ok(json!({"id": "a10-f3", "op": "fault", "kind": "bad_quality"}))?;
        ts5.cmd_ok(json!({"id": "a10-m3", "op": "mutate", "delta": 1}))?;
        let end = rust
            .wait_event_after(mark, |event| {
                event.get("event").and_then(Value::as_str) == Some("end")
            })
            .context("A10 bad quality end")?;
        if jstr(&end, "subscriptionKey") != "a10-live"
            || jnum(&end.get("error").cloned().unwrap_or(Value::Null), "code") != -32017
        {
            bail!("A10 bad quality must end a10-live session_lost: {end}");
        }
    }
    env.plain("probe-a10", env.token_rust.clone(), &[])?;
    let probe = env.node("probe-a10");
    probe.cmd_ok(json!({"id": "a10-hello", "op": "hello"}))?;
    // Forged subscription ids from a non-provider link create nothing and do
    // not disturb the (re-subscribed) live subscription.
    {
        let rust = env.node("rust2");
        let ts5 = env.node("ts5");
        let ts_ref = discovered_ref(rust, "a10-disc-ts5b", "counter-ts")?;
        let rx = rust.send(json!({
            "id": "a10-sub-live2", "op": "subscribe", "subscriptionKey": "a10-live2",
            "target": ts_ref, "state": "counter/value", "argument": {},
        }))?;
        let response = rust.recv(rx)?;
        if !jbool(&response, "ok") {
            bail!("A10 subscribe a10-live2 failed: {response}");
        }
        probe.cmd_ok(json!({
            "id": "a10-forged", "op": "forged_sub",
            "target": {"providerEndpointId": "counter-ts", "registrationId": jstr(&ts_ref, "registrationId")},
        }))?;
        let live_mark = rust.events_len();
        ts5.cmd_ok(json!({"id": "a10-m4", "op": "mutate", "delta": 1}))?;
        let update = rust
            .wait_event_after(live_mark, |event| {
                event.get("event").and_then(Value::as_str) == Some("update")
                    && event.get("subscriptionKey").and_then(Value::as_str) == Some("a10-live2")
            })
            .context("A10 forged-id sibling update")?;
        if seq_of(&update) != "1" {
            bail!("A10 forged-id sibling sequence changed: {update}");
        }
    }

    support_test(
        &env.root,
        &[
            "test",
            "-p",
            "conex-host",
            "--test",
            "s1_state",
            "active_seq_must_start_at_one_and_be_contiguous",
        ],
    )?;
    support_test(
        &env.root,
        &[
            "test",
            "-p",
            "conex-host",
            "--test",
            "s1_state",
            "opening_timeout_cleans_up_and_late_snapshot_does_not_revive",
        ],
    )?;
    Ok(Outcome::new(
        "seq gap / rollback / quality end session_lost (-32017); sibling update clean; forged ids ignored",
        format!("gap={gap_key} survivor={} phases=3", outcome.1),
    ))
}

fn a11(env: &mut Env) -> Result<Outcome> {
    env.plain("http-a11", env.token_ts.clone(), &[])?;
    let rust_ref = discovered_ref(env.node("ts5"), "a11-disc", "counter-rust")?;
    let http = env.node("http-a11");
    http.wait_event(|event| event["event"] == "ready")?;
    http.cmd_ok(json!({"id": "a11-hello", "op": "list"}))?;

    let read = http.cmd_ok(json!({
        "id": "a11-read", "op": "invoke",
        "callId": "a11-read", "target": rust_ref,
        "method": "counter/read", "argument": {},
    }))?;

    // Cancel while the provider is gated: requested=true, then the released
    // invoke still settles with its own committed result.
    let rust = env.node("rust2");
    let mark = rust.events_len();
    arm(rust, "after_commit")?;
    let before = inspect(rust)?;
    let invoke = http.send(json!({
        "id": "a11-add", "op": "invoke", "callId": "a11-add",
        "target": rust_ref, "method": "counter/add", "argument": {"delta": 2},
    }))?;
    rust.wait_event_after(mark, |event| {
        event["event"] == "barrier" && event["point"] == "after_commit"
    })?;
    let cancelled = http.cmd_ok(json!({
        "id": "a11-cancel", "op": "cancel", "callId": "a11-add",
    }))?;
    release(rust, "after_commit")?;
    let settled = http.recv(invoke)?;
    let result_count = count_of(&settled["value"]);
    if cancelled["requested"] != true
        || settled["ok"] != true
        || result_count != count_of(&before) + 2
    {
        bail!("A11 http cancel/invoke mismatch: cancel={cancelled} invoke={settled}");
    }
    let after = inspect(rust)?;
    if count_of(&after) != count_of(&before) + 2
        || jnum(&after, "executions") != jnum(&before, "executions") + 1
    {
        bail!("A11 http cancel must preserve exactly one commit: {after}");
    }

    // HTTP must refuse provider/state methods explicitly.
    let subscribe =
        http.cmd_ok(json!({"id": "a11-sub", "op": "forbidden", "method": "state/subscribe"}))?;
    let register =
        http.cmd_ok(json!({"id": "a11-reg", "op": "forbidden", "method": "module/register"}))?;
    let unsupported = |value: &Value| value.get("code").and_then(Value::as_i64) == Some(-32004);
    if !unsupported(&subscribe) || !unsupported(&register) {
        bail!(
            "A11 http must reject provider/state with unsupported (-32004): {subscribe} / {register}"
        );
    }

    support_test(
        &env.root,
        &[
            "xtask",
            "conformance",
            "--vectors",
            "conformance/vectors/s1",
        ],
    )?;

    Ok(Outcome::new(
        "http list/invoke/cancel with binding auth; subscribe/register unsupported (-32004); vectors SUPPORT",
        format!(
            "read={} cancel=requested commit={} sub={} reg={}",
            count_of(&read),
            result_count,
            subscribe.get("code").and_then(Value::as_i64).unwrap_or(0),
            register.get("code").and_then(Value::as_i64).unwrap_or(0)
        ),
    ))
}

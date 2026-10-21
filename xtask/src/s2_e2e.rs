//! S2 plugin-runtime acceptance (docs/contracts/s2-plugins.md §8: S2-A01–A08;
//! docs/plans/2026-10-19-conex-s2.md §7 U04).
//!
//! Real Host + the real Rust sample instance (crates/conex-sdk/examples/
//! plugin.rs) + the real TS sample instance (examples/s2/plugin-node.ts) run
//! their published dual-instance choreography. This driver observes their
//! structured stdout events and drives an SDK consumer probe (JSONL) for
//! independent reads/adds/subscriptions, so every item cites central state,
//! not the samples' own claims. Composition rejections (A04/A05) run as
//! plugin-variant processes built on the same TS SDK.
//!
//! Ordering comes from central-state polls (module/list) and explicit stdout
//! phase events; the only sleep is a bounded negative-observation window in
//! A08 (proving absence, not sequencing). On any exit path a guard kills all
//! children and drops the tempdir. Scenario functions run in choreography
//! order, not numeric order; every output line carries its S2-A number.

use std::collections::HashMap;
use std::io::{BufRead, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const TIMEOUT: Duration = Duration::from_secs(25);
const SPACE: &str = "dev";
const CONTRACT: &str = "conex.counter";
const HOST_READY: Duration = Duration::from_secs(15);
/// A08: bounded "still absent" window after both instances stopped. This is a
/// negative observation (no revival), not an ordering barrier.
const ABSENCE_WINDOW_POLLS: usize = 5;
const ABSENCE_WINDOW_STEP: Duration = Duration::from_millis(100);

/// Scenarios in choreography order (the samples gate each other through hub
/// state); the printed name carries the contract §8 numbering.
type Scenario = (&'static str, fn(&mut Env) -> Result<Outcome>);
static SCENARIOS: &[Scenario] = &[
    ("A04", a04),
    ("A05", a05),
    ("A02", a02),
    ("A03", a03),
    ("A07", a07),
    ("A01", a01),
    ("A06", a06),
    ("A08", a08),
];

pub fn run() -> Result<()> {
    let mut env = Env::start()?;
    for (name, scenario) in SCENARIOS {
        match scenario(&mut env) {
            Ok(outcome) => println!(
                "S2-{name} expected={} actual={} ok",
                outcome.expected, outcome.actual
            ),
            Err(err) => {
                println!("S2-{name} expected=scenario-detail actual=FAIL");
                eprintln!("  {err:#}");
                return Err(err).with_context(|| format!("scenario {name} failed"));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Process plumbing (same conventions as s1_e2e)
// ---------------------------------------------------------------------------

struct Env {
    root: PathBuf,
    port: u16,
    token_rust: String,
    token_ts: String,
    host: Mutex<Child>,
    fixtures: Vec<Fixture>,
    /// Script cache: label -> absolute path of a generated TS fixture.
    scripts: HashMap<&'static str, PathBuf>,
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
            .args(["build", "-p", "conex-sdk", "--example", "plugin"])
            .current_dir(&root)
            .status()
            .context("build conex-sdk example plugin")?;
        if !sdk_build.success() {
            bail!("cargo build -p conex-sdk --example plugin failed");
        }
        if Command::new("bun")
            .arg("--version")
            .stdout(Stdio::null())
            .output()
            .is_err()
        {
            bail!("bun is required for the TS plugin instance and probe");
        }

        let port = free_port()?;
        let tmp = tempfile::tempdir().context("create s2 e2e temp dir")?;
        for name in ["content", "session", "operation"] {
            std::fs::create_dir_all(tmp.path().join(name)).context("mkdir s2 root")?;
        }
        let token_rust = random_token();
        let token_ts = random_token();
        let config = host_toml(port, tmp.path(), &token_rust, &token_ts);
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

        let mut scripts = HashMap::new();
        for (label, source) in [("s2_probe", PROBE_TS), ("s2_variant", VARIANT_TS)] {
            let path = tmp.path().join(format!("{label}.ts"));
            std::fs::write(&path, source).with_context(|| format!("write {label}.ts"))?;
            scripts.insert(label, path);
        }

        let mut env = Env {
            root,
            port,
            token_rust,
            token_ts,
            host: Mutex::new(host),
            fixtures: Vec::new(),
            scripts,
            _tmp: tmp,
        };

        // Consumer probe on the rust principal: consume approvals cover all
        // four endpoints and the Rust sample's local exposure ceiling.
        let script = env.scripts["s2_probe"].clone();
        let probe_root = env.root.clone();
        let url = format!("ws://127.0.0.1:{}/wss", env.port);
        let sdk_src = env.root.join("sdk/typescript/src").display().to_string();
        let token = env.token_rust.clone();
        let probe = spawn_fixture(
            "probe",
            "bun",
            |command| {
                command.arg("run").arg(script).current_dir(probe_root);
            },
            &[
                ("CONEX_URL".to_owned(), url),
                ("CONEX_TOKEN".to_owned(), token),
                ("CONEX_SPACE".to_owned(), SPACE.to_owned()),
                ("S2_TS_SDK".to_owned(), sdk_src),
            ],
        )?;
        probe.cmd_ok(json!({"id": "ctl", "op": "connect"}))?;
        env.fixtures.push(probe);
        Ok(env)
    }

    fn node(&self, label: &str) -> &Fixture {
        self.fixtures
            .iter()
            .find(|fixture| fixture.label == label)
            .unwrap_or_else(|| panic!("unknown fixture label {label}"))
    }

    fn adopt(&mut self, fixture: Fixture) {
        self.fixtures.push(fixture);
    }

    /// Central-state barrier: poll the probe's module/list until every named
    /// provider endpoint is registered and every `absent` name is gone.
    fn wait_modules(&self, present: &[&str], absent: &[&str]) -> Result<Value> {
        let probe = self.node("probe");
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let last = probe.cmd_ok(json!({"id": "ctl", "op": "list"}))?;
            let modules = modules_of(&last);
            let has = |name: &str| modules.iter().any(|m| endpoint_id(m) == name);
            if present.iter().all(|name| has(name)) && absent.iter().all(|name| !has(name)) {
                return Ok(last);
            }
            if Instant::now() >= deadline {
                bail!("module/list barrier not reached within {TIMEOUT:?}: {last}");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Start the two real sample instances in the deterministic order: Rust
    /// first (its +5 commit must precede the TS bind snapshot), then TS.
    fn start_samples(&mut self) -> Result<()> {
        let url = format!("ws://127.0.0.1:{}/wss", self.port);
        let rust_binary = self.root.join("target/debug/examples/plugin");
        let ts_sample = self.root.join("examples/s2/plugin-node.ts");
        let token_rust = self.token_rust.clone();
        let token_ts = self.token_ts.clone();

        let rust = spawn_fixture(
            "rust-sample",
            &rust_binary.display().to_string(),
            |command| {
                command.current_dir(&self.root);
            },
            &[
                ("CONEX_URL".to_owned(), url.clone()),
                ("CONEX_TOKEN".to_owned(), token_rust),
                ("CONEX_SPACE".to_owned(), SPACE.to_owned()),
                ("CONEX_ENDPOINT".to_owned(), "s2-rust".to_owned()),
                ("CONEX_PEER_ENDPOINT".to_owned(), "s2-ts".to_owned()),
            ],
        )?;
        self.adopt(rust);

        // Publish barrier from central state, not from sample stdout claims.
        self.wait_modules(&["s2-rust-counter", "s2-rust-derived-counter"], &[])?;
        // The Rust instance commits +5 during its A02 phase; waiting for that
        // phase event makes the TS bind snapshot deterministic (5/rev1).
        sample_phase(self.node("rust-sample"), "A02-local-derivation")?;

        let ts = spawn_fixture(
            "ts-sample",
            "bun",
            |command| {
                command.arg("run").arg(ts_sample).current_dir(&self.root);
            },
            &[
                ("CONEX_URL".to_owned(), url),
                ("CONEX_TOKEN".to_owned(), token_ts),
                ("CONEX_SPACE".to_owned(), SPACE.to_owned()),
                ("CONEX_ENDPOINT".to_owned(), "s2-ts".to_owned()),
                ("CONEX_PEER_ENDPOINT".to_owned(), "s2-rust".to_owned()),
            ],
        )?;
        self.adopt(ts);

        self.wait_modules(&["s2-ts-counter", "s2-ts-derived-counter"], &[])?;
        Ok(())
    }

    /// Spawn a plugin-variant process on the TS SDK (composition rejections,
    /// fresh generation) and register it for cleanup.
    fn spawn_variant(&mut self, label: &'static str, mode: &str) -> Result<Fixture> {
        let script = self.scripts["s2_variant"].clone();
        let variant_root = self.root.clone();
        let url = format!("ws://127.0.0.1:{}/wss", self.port);
        let sdk_src = self.root.join("sdk/typescript/src").display().to_string();
        let token = self.token_ts.clone();
        let variant = spawn_fixture(
            label,
            "bun",
            |command| {
                command.arg("run").arg(script).current_dir(variant_root);
            },
            &[
                ("CONEX_URL".to_owned(), url),
                ("CONEX_TOKEN".to_owned(), token),
                ("CONEX_SPACE".to_owned(), SPACE.to_owned()),
                ("CONEX_ENDPOINT".to_owned(), "s2-ts".to_owned()),
                ("S2_TS_SDK".to_owned(), sdk_src),
                ("S2_MODE".to_owned(), mode.to_owned()),
            ],
        )?;
        Ok(variant)
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

/// Spawn a fixture child: requests may go in on stdin, `{id, ok, value|error}`
/// responses and `{event, ...}` notifications come out on stdout. `configure`
/// adds args and the working directory.
fn spawn_fixture(
    label: &'static str,
    program: &str,
    configure: impl FnOnce(&mut Command),
    envs: &[(String, String)],
) -> Result<Fixture> {
    let mut command = Command::new(program);
    configure(&mut command);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    for (key, value) in envs {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .with_context(|| format!("spawn fixture {label}"))?;
    let stdin = child.stdin.take().expect("fixture stdin piped");
    let stdout = child.stdout.take().expect("fixture stdout piped");
    let events = Arc::new(Mutex::new(Vec::new()));
    let pending = Arc::new(Mutex::new(HashMap::new()));
    let reader = spawn_reader(stdout, label, events.clone(), pending.clone());
    Ok(Fixture {
        label,
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        events,
        pending,
        reader: Some(reader),
    })
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

    /// First buffered event matching the predicate, waiting only by deadline.
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

    fn kill(&self) -> Result<()> {
        let mut child = self.child.lock().unwrap();
        let _ = child.kill();
        let _ = child.wait();
        Ok(())
    }

    fn terminate(&self) -> Result<()> {
        let pid = self.child.lock().unwrap().id();
        let status = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .context("send SIGTERM")?;
        if !status.success() {
            bail!("{}: SIGTERM dispatch failed", self.label);
        }
        Ok(())
    }

    /// Reap the child and return its exit code (-1 on signal death).
    fn wait_exit(&self) -> Result<i64> {
        let status = self
            .child
            .lock()
            .unwrap()
            .wait()
            .with_context(|| format!("{}: wait exit", self.label))?;
        Ok(status.code().map_or(-1, i64::from))
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

/// Static approval config modeled on examples/s2/host.toml: two service
/// principals, the preinstalled conex.counter contract, four publish approvals
/// (each instance's counter + derived-counter) and eight consume approvals.
fn host_toml(port: u16, dir: &Path, token_rust: &str, token_ts: &str) -> String {
    let digest = |token: &str| hex::encode(Sha256::digest(token.as_bytes()));
    let token = |principal: &str, hash: String| {
        format!(
            "[[tokens]]\ntoken_hash = \"{hash}\"\nprincipal_id = \"{principal}\"\ntenant_id = \"dev\"\naudience = \"conex.dev\"\nrole = \"service\"\n"
        )
    };
    let publish = |node: &str, endpoint: &str| {
        format!(
            "[[capabilities.publish]]\nnode_id = \"{node}\"\nspace_id = \"dev\"\nintegration_id = \"integration\"\nprovider_endpoint_id = \"{endpoint}\"\ncontract_id = \"{}\"\n",
            CONTRACT
        )
    };
    let consume = |principal: &str, endpoint: &str| {
        format!(
            "[[capabilities.consume]]\nprincipal_id = \"{principal}\"\nspace_id = \"dev\"\nprovider_endpoint_id = \"{endpoint}\"\ncontract_id = \"{}\"\n",
            CONTRACT
        )
    };
    let endpoints = [
        ("s2-rust", "s2-rust-counter"),
        ("s2-rust", "s2-rust-derived-counter"),
        ("s2-ts", "s2-ts-counter"),
        ("s2-ts", "s2-ts-derived-counter"),
    ];
    let publishes: String = endpoints
        .iter()
        .map(|(node, endpoint)| publish(node, endpoint))
        .collect();
    let consumes: String = endpoints
        .iter()
        .flat_map(|(_, endpoint)| {
            ["s2-rust", "s2-ts"].map(|principal| consume(principal, endpoint))
        })
        .collect();
    format!(
        concat!(
            "listen = \"127.0.0.1:{port}\"\n",
            "allow_loopback_http = true\n",
            "audience = \"conex.dev\"\n",
            "content_root = \"{content}\"\n",
            "session_root = \"{session}\"\n",
            "operation_root = \"{operation}\"\n",
            "\n",
            "{token_rust}",
            "{token_ts}",
            "\n",
            "[capabilities]\n",
            "\n",
            "[[capabilities.contracts]]\n",
            "contract_id = \"conex.counter\"\n",
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
            "{publishes}",
            "\n",
            "{consumes}",
        ),
        port = port,
        content = dir.join("content").display(),
        session = dir.join("session").display(),
        operation = dir.join("operation").display(),
        token_rust = token("s2-rust", digest(token_rust)),
        token_ts = token("s2-ts", digest(token_ts)),
        publishes = publishes,
        consumes = consumes,
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

fn phase_of(value: &Value) -> Option<&str> {
    value.get("phase").and_then(Value::as_str)
}

fn endpoint_id(module: &Value) -> String {
    jstr(module, "providerEndpointId")
}

/// The probe projects module/list as {"modules":[...]}.
fn modules_of(list: &Value) -> Vec<Value> {
    list.get("modules")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn registration_of(list: &Value, endpoint: &str) -> Result<String> {
    modules_of(list)
        .iter()
        .find(|module| endpoint_id(module) == endpoint)
        .map(|module| jstr(module, "registrationId"))
        .ok_or_else(|| anyhow::anyhow!("module/list misses {endpoint}: {list}"))
}

/// Sample stdout event with an exact field match.
fn sample_event(fixture: &Fixture, name: &str, key: &str, expected: &str) -> Result<Value> {
    fixture.wait_event(|value| jstr(value, "event") == name && jstr(value, key) == expected)
}

fn sample_phase(fixture: &Fixture, phase: &str) -> Result<Value> {
    fixture.wait_event(|value| phase_of(value) == Some(phase))
}

/// Probe read/add as (count, revision) with an exact-value assertion helper.
fn probe_read(env: &Env, endpoint: &str) -> Result<(i64, String)> {
    let probe = env.node("probe");
    let value = probe.cmd_ok(json!({"id": "ctl", "op": "read", "endpoint": endpoint}))?;
    Ok((jnum(&value, "count"), jstr(&value, "revision")))
}

fn probe_add(env: &Env, endpoint: &str, delta: i64) -> Result<(i64, String)> {
    let probe = env.node("probe");
    let value =
        probe.cmd_ok(json!({"id": "ctl", "op": "add", "endpoint": endpoint, "delta": delta}))?;
    Ok((jnum(&value, "count"), jstr(&value, "revision")))
}

fn assert_state(actual: (i64, String), count: i64, revision: &str, what: &str) -> Result<()> {
    if actual.0 != count || actual.1 != revision {
        bail!(
            "{what}: expected {count}/{revision}, got {}/{}",
            actual.0,
            actual.1
        );
    }
    Ok(())
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

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

/// S2-A04: missing local dependency is rejected at composition, before any
/// network I/O (typed `missing_dependency`, zero sockets, zero publications).
fn a04(env: &mut Env) -> Result<Outcome> {
    let before = env
        .node("probe")
        .cmd_ok(json!({"id": "ctl", "op": "list"}))?;
    let variant = env.spawn_variant("reject-missing", "reject-missing")?;
    let rejected = variant.wait_event(|value| jstr(value, "event") == "rejected")?;
    let exit = variant.wait_exit()?;
    env.adopt(variant);
    let after = env
        .node("probe")
        .cmd_ok(json!({"id": "ctl", "op": "list"}))?;

    if exit != 0 {
        bail!("reject-missing variant exited {exit}");
    }
    if jstr(&rejected, "kind") != "missing_dependency"
        || jstr(&rejected, "module") != "a"
        || jstr(&rejected, "dependency") != "ghost"
    {
        bail!("unexpected rejection detail: {rejected}");
    }
    if jnum(&rejected, "sockets") != 0 {
        bail!("composition rejection opened sockets: {rejected}");
    }
    if before != after {
        bail!("host registry changed across rejection: {before} -> {after}");
    }
    Ok(Outcome::new(
        "missing_dependency{module:a,dependency:ghost}, zero sockets, zero publications",
        format!(
            "kind=missing_dependency module=a dependency=ghost sockets=0 registry unchanged, exit {exit}"
        ),
    ))
}

/// S2-A05: circular local graph rejected at composition with the cycle named.
fn a05(env: &mut Env) -> Result<Outcome> {
    let variant = env.spawn_variant("reject-cycle", "reject-cycle")?;
    let rejected = variant.wait_event(|value| jstr(value, "event") == "rejected")?;
    let exit = variant.wait_exit()?;
    env.adopt(variant);

    if exit != 0 {
        bail!("reject-cycle variant exited {exit}");
    }
    if jstr(&rejected, "kind") != "circular_dependency" {
        bail!("expected circular_dependency, got {rejected}");
    }
    let cycle = rejected
        .get("cycle")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let names: Vec<String> = cycle
        .iter()
        .map(|name| name.as_str().unwrap_or_default().to_owned())
        .collect();
    if !(names.contains(&"a".to_owned()) && names.contains(&"b".to_owned())) {
        bail!("cycle does not name both modules: {rejected}");
    }
    if jnum(&rejected, "sockets") != 0 {
        bail!("cycle rejection opened sockets: {rejected}");
    }
    Ok(Outcome::new(
        "circular_dependency naming a and b, zero sockets",
        format!("kind=circular_dependency cycle={names:?} sockets=0, exit {exit}"),
    ))
}

/// S2-A02: the Rust instance's derived module consumes its own counter through
/// the in-process handle: ×2 view, own revision sequence.
fn a02(env: &mut Env) -> Result<Outcome> {
    env.start_samples()?;
    let rust = env.node("rust-sample");

    let snapshot = sample_event(rust, "snapshot", "target", "s2-rust-derived-counter")?;
    if jstr(&snapshot, "revision") != "1" || jnum(&snapshot, "count") != 0 {
        bail!("derived snapshot must start at 0/rev1: {snapshot}");
    }
    let added = rust.wait_event(|value| {
        jstr(value, "event") == "add"
            && jstr(value, "target") == "s2-rust-counter"
            && jnum(value, "count") == 5
    })?;
    if jnum(&added, "delta") != 5 || jstr(&added, "revision") != "1" {
        bail!("counter add(+5) must yield 5/rev1: {added}");
    }
    let update = sample_event(rust, "update", "target", "s2-rust-derived-counter")?;
    if jnum(&update, "count") != 10 || jstr(&update, "revision") != "2" {
        bail!("derived must follow ×2 = 10/rev2: {update}");
    }
    let view = sample_event(rust, "read", "target", "s2-rust-derived-counter")?;
    if jnum(&view, "count") != 10 || jstr(&view, "revision") != "2" {
        bail!("derived read must serve 10/rev2: {view}");
    }
    sample_phase(rust, "A02-local-derivation")?;

    Ok(Outcome::new(
        "derived view = ×2 of own counter via in-process handle; own revision sequence",
        "snapshot 0/rev1; add(+5) -> 5/rev1; derived update 10/rev2; read 10/rev2",
    ))
}

/// S2-A03: the TS instance's derived module is explicitly bound to the Rust
/// counter's ModuleRef; add via the derived forwards across instances.
fn a03(env: &mut Env) -> Result<Outcome> {
    let ts = env.node("ts-sample");
    let rust = env.node("rust-sample");

    let bind = sample_event(ts, "bind-snapshot", "target", "s2-rust-counter")?;
    if jnum(&bind, "count") != 5 || jstr(&bind, "revision") != "1" {
        bail!("TS bind snapshot must capture the committed 5/rev1: {bind}");
    }
    let forwarded = sample_event(ts, "add", "target", "s2-ts-derived-counter")?;
    if jnum(&forwarded, "delta") != 3
        || jnum(&forwarded, "forwardedCount") != 8
        || jstr(&forwarded, "forwardedRevision") != "2"
    {
        bail!("derived add(+3) must forward to the Rust counter (8/rev2): {forwarded}");
    }
    let upstream = sample_event(ts, "read", "target", "s2-rust-counter")?;
    if jnum(&upstream, "count") != 8 || jstr(&upstream, "revision") != "2" {
        bail!("upstream counter must read 8/rev2 after forwarding: {upstream}");
    }
    let view = sample_event(ts, "read", "target", "s2-ts-derived-counter")?;
    if jnum(&view, "count") != 16 || jstr(&view, "revision") != "2" {
        bail!("TS derived view must be ×2 = 16/rev2: {view}");
    }
    sample_phase(ts, "A03-remote-binding")?;
    // The Rust instance independently observed the forwarded commit.
    let gate = rust.wait_event(|value| {
        jstr(value, "event") == "read"
            && jstr(value, "target") == "s2-rust-counter"
            && jnum(value, "count") == 8
            && jstr(value, "gate") == "ts add(+3) landed"
    })?;
    if jstr(&gate, "revision") != "2" {
        bail!("rust gate read expected 8/rev2: {gate}");
    }

    Ok(Outcome::new(
        "add via TS derived forwards across instances to the bound Rust counter",
        "bind snapshot 5/rev1; add(+3) forwarded 8/rev2; upstream read 8/rev2; derived view 16/rev2; rust observed 8/rev2",
    ))
}

/// S2-A07: each instance's derived module is consumed read+subscribe by the
/// other language; counts agree, revision sequences stay independent.
fn a07(env: &mut Env) -> Result<Outcome> {
    let ts = env.node("ts-sample");
    let rust = env.node("rust-sample");

    // Rust consumes the TS derived module (subscribe + forwarded add + update).
    let snapshot = sample_event(rust, "snapshot", "target", "s2-ts-derived-counter")?;
    if jnum(&snapshot, "count") != 16 || jstr(&snapshot, "revision") != "2" {
        bail!("rust snapshot of TS derived must be 16/rev2: {snapshot}");
    }
    let forwarded = sample_event(rust, "add", "target", "s2-ts-derived-counter")?;
    if jnum(&forwarded, "delta") != 4
        || jnum(&forwarded, "forwardedCount") != 12
        || jstr(&forwarded, "forwardedRevision") != "3"
    {
        bail!("rust add(+4) via TS derived must forward (12/rev3): {forwarded}");
    }
    let update = rust.wait_event(|value| {
        jstr(value, "event") == "update"
            && jstr(value, "target") == "s2-ts-derived-counter"
            && jnum(value, "count") >= 24
    })?;
    if jstr(&update, "revision") != "3" {
        bail!("rust update of TS derived expected 24/rev3: {update}");
    }
    sample_phase(rust, "A07-rust-consumes-ts-derived")?;

    // TS consumes the Rust derived module (subscribe + read + direct add).
    let ts_snapshot = sample_event(ts, "snapshot", "target", "s2-rust-derived-counter")?;
    if jnum(&ts_snapshot, "count") != 24 || jstr(&ts_snapshot, "revision") != "4" {
        bail!("ts snapshot of Rust derived must be 24/rev4: {ts_snapshot}");
    }
    let direct = sample_event(ts, "add", "target", "s2-rust-counter")?;
    if jnum(&direct, "delta") != 2
        || jnum(&direct, "count") != 14
        || jstr(&direct, "revision") != "4"
    {
        bail!("ts add(+2) on rust counter must yield 14/rev4: {direct}");
    }
    let seen = sample_event(ts, "update-seen", "target", "s2-rust-derived-counter")?;
    if jnum(&seen, "count") != 28 || jstr(&seen, "revision") != "5" {
        bail!("ts update of Rust derived must reach 28/rev5: {seen}");
    }
    sample_phase(ts, "A07-ts-consumes-rust-derived")?;
    sample_event(env.node("rust-sample"), "idle", "event", "idle")?;
    sample_event(ts, "idle", "event", "idle")?;

    // Independent cross-language check from the probe's own connection.
    let (rust_count, rust_revision) = probe_read(env, "s2-rust-derived-counter")?;
    assert_state(
        (rust_count, rust_revision.clone()),
        28,
        "5",
        "probe read rust derived",
    )?;
    let (ts_count, ts_revision) = probe_read(env, "s2-ts-derived-counter")?;
    assert_state(
        (ts_count, ts_revision.clone()),
        28,
        "4",
        "probe read ts derived",
    )?;
    if rust_count != ts_count {
        bail!("both ×2 views must agree on the upstream count: {rust_count} vs {ts_count}");
    }
    if rust_revision == ts_revision {
        bail!("equal counts must carry independent revision sequences");
    }

    Ok(Outcome::new(
        "bidirectional derived consumption read+subscribe; equal view values, independent revisions",
        "rust: snapshot 16/rev2, forwarded 12/rev3, update 24/rev3; ts: snapshot 24/rev4, update 28/rev5; probe: rust-derived 28/rev5 vs ts-derived 28/rev4",
    ))
}

/// S2-A01: one plugin class, two live instances; independent counts/revisions,
/// overflow rejection leaves the rejected instance unchanged.
fn a01(env: &mut Env) -> Result<Outcome> {
    let rust = env.node("rust-sample");

    // Sample-side isolation evidence (Rust instance observes the TS instance).
    let peer = sample_event(rust, "read", "target", "s2-ts-counter")?;
    if jnum(&peer, "count") != 0 || jstr(&peer, "revision") != "0" {
        bail!("untouched TS counter must read 0/rev0: {peer}");
    }
    let overflow = sample_event(rust, "overflow-rejected", "target", "s2-rust-counter")?;
    if jnum(&overflow, "code") != -32602 {
        bail!("overflow must reject with -32602: {overflow}");
    }
    // The rejected add must not commit: the sample exits 1 if it succeeds, and
    // the counter only advances through legitimate concurrent commits.
    sample_phase(rust, "A01-instance-independence")?;

    // Probe-side isolation evidence against the live instances.
    let ts_before = probe_read(env, "s2-ts-counter")?;
    assert_state(ts_before, 0, "0", "probe read ts counter")?;
    let ts_after = probe_add(env, "s2-ts-counter", 1)?;
    assert_state(ts_after, 1, "1", "probe add(+1) ts counter")?;
    let rust_now = probe_read(env, "s2-rust-counter")?;
    assert_state(rust_now, 14, "4", "probe read rust counter")?;

    Ok(Outcome::new(
        "two instances side by side; independent counts and revision sequences",
        "rust observed ts 0/rev0 and its own overflow add rejected (-32602); probe: ts 0/rev0 -> add(+1) -> 1/rev1 while rust stays 14/rev4",
    ))
}

/// S2-A06: stopping an instance unpublishes its modules, ends active
/// subscriptions, rejects further calls; the other instance keeps serving.
fn a06(env: &mut Env) -> Result<Outcome> {
    // The doomed instance's registration, captured from its publish event.
    let published = sample_event(
        env.node("ts-sample"),
        "publish",
        "providerEndpointId",
        "s2-ts-counter",
    )?;
    let ts_registration = jstr(&published, "registrationId");

    // Live subscription on the doomed instance's module.
    let subscription = env.node("probe").cmd_ok(json!(
        {"id": "sub", "op": "subscribe", "endpoint": "s2-ts-counter"}
    ))?;
    let subscription_id = jstr(&subscription, "subscriptionId");
    let snapshot = env.node("probe").wait_event(|value| {
        jstr(value, "event") == "sample"
            && jstr(value, "subscriptionId") == subscription_id
            && value.get("snapshot").and_then(Value::as_bool) == Some(true)
    })?;
    if jnum(&snapshot, "count") != 1 || jstr(&snapshot, "revision") != "1" {
        bail!("subscription snapshot expected 1/rev1: {snapshot}");
    }

    let ts = env.node("ts-sample");
    ts.terminate()?;
    let ts_exit = ts.wait_exit()?;
    sample_event(ts, "stopping", "event", "stopping")?;
    sample_event(ts, "stopped", "state", "stopped")?;
    if ts_exit != 0 {
        bail!("TS instance must exit 0 after graceful stop, got {ts_exit}");
    }

    // The subscription ends; further calls hit the unregistered path.
    let probe = env.node("probe");
    let end = probe.wait_event(|value| {
        jstr(value, "event") == "sub-end" && jstr(value, "subscriptionId") == subscription_id
    })?;
    let end_code = jnum(&end, "code");
    if end_code != -32005 {
        bail!("provider-stop subscription end must be -32005 unavailable: {end}");
    }
    let invoke_error = probe.cmd_err(json!({
        "id": "sub-invoke",
        "op": "invoke-ref",
        "endpoint": "s2-ts-counter",
        "registrationId": ts_registration,
        "method": "counter/read",
    }))?;
    let invoke_code = jnum(&invoke_error, "code");
    if invoke_code != -32002 {
        bail!("invoke on stopped instance must be forbidden -32002: {invoke_error}");
    }
    env.wait_modules(&[], &["s2-ts-counter", "s2-ts-derived-counter"])?;

    // The survivor keeps serving and then exits cleanly on its own.
    let rust = env.node("rust-sample");
    sample_event(rust, "peer-gone", "event", "peer-gone")?;
    let survivor = rust.wait_event(|value| {
        jstr(value, "event") == "read"
            && jstr(value, "target") == "s2-rust-counter"
            && jnum(value, "count") == 14
    })?;
    if jstr(&survivor, "revision") != "4" {
        bail!("survivor read after peer stop expected 14/rev4: {survivor}");
    }
    let bumped = rust.wait_event(|value| {
        jstr(value, "event") == "add"
            && jstr(value, "target") == "s2-rust-counter"
            && jnum(value, "count") == 15
    })?;
    if jstr(&bumped, "revision") != "5" {
        bail!("survivor add(+1) after peer stop expected 15/rev5: {bumped}");
    }
    sample_event(rust, "stopping", "event", "stopping")?;
    sample_event(rust, "done", "event", "done")?;
    let rust_exit = rust.wait_exit()?;
    if rust_exit != 0 {
        bail!("Rust sample must exit 0 after peer-gone shutdown, got {rust_exit}");
    }

    Ok(Outcome::new(
        "stop unpublishes, ends subscriptions with unavailable, rejects calls; survivor unaffected",
        format!(
            "ts exit {ts_exit} after stopped; subscription end -32005; invoke -32002 not registered; list lacks s2-ts-*; rust survivor 14/rev4 then add 15/rev5, exit {rust_exit}"
        ),
    ))
}

/// S2-A08: stopped instances never revive; a new instance is a new generation
/// (fresh registration id and state) and old ModuleRefs fail explicitly.
fn a08(env: &mut Env) -> Result<Outcome> {
    let old = sample_event(
        env.node("ts-sample"),
        "publish",
        "providerEndpointId",
        "s2-ts-counter",
    )?;
    let old_registration = jstr(&old, "registrationId");
    if old_registration.is_empty() {
        bail!("TS sample publish event lacks registrationId: {old}");
    }

    // Negative observation window: nothing revives the stopped generations.
    for _ in 0..ABSENCE_WINDOW_POLLS {
        let list = env
            .node("probe")
            .cmd_ok(json!({"id": "ctl", "op": "list"}))?;
        if !modules_of(&list).is_empty() {
            bail!("stopped instances left modules registered: {list}");
        }
        std::thread::sleep(ABSENCE_WINDOW_STEP);
    }

    // Fresh instance of the same plugin class on the same endpoints.
    let variant = env.spawn_variant("fresh-variant", "fresh")?;
    let publish = variant.wait_event(|value| {
        jstr(value, "event") == "publish" && jstr(value, "providerEndpointId") == "s2-ts-counter"
    })?;
    let new_registration = jstr(&publish, "registrationId");
    variant.wait_event(|value| jstr(value, "event") == "ready-for-shutdown")?;
    env.adopt(variant);

    if new_registration.is_empty() || new_registration == old_registration {
        bail!(
            "new generation must mint a fresh registrationId: old={old_registration} new={new_registration}"
        );
    }
    let listed = env.wait_modules(&["s2-ts-counter"], &[])?;
    let listed_registration = registration_of(&listed, "s2-ts-counter")?;
    if listed_registration != new_registration {
        bail!("registry shows {listed_registration}, variant published {new_registration}");
    }

    // The old explicit ModuleRef fails with the S1 stale path.
    let stale = env.node("probe").cmd_err(json!({
        "id": "a08-stale",
        "op": "invoke-ref",
        "endpoint": "s2-ts-counter",
        "registrationId": old_registration,
        "method": "counter/read",
    }))?;
    if jnum(&stale, "code") != -32013 {
        bail!("old ModuleRef must fail -32013 stale_revision: {stale}");
    }
    let fresh = probe_read(env, "s2-ts-counter")?;
    assert_state(fresh, 7, "1", "fresh generation read")?;

    let variant = env.node("fresh-variant");
    variant.send(json!({"id": "ctl", "op": "shutdown"}))?;
    let variant_exit = variant.wait_exit()?;
    sample_event(variant, "stopped", "state", "stopped")?;
    if variant_exit != 0 {
        bail!("variant must exit 0 after graceful stop, got {variant_exit}");
    }
    env.wait_modules(&[], &["s2-ts-counter"])?;

    Ok(Outcome::new(
        "no auto-revival; new instance = new generation; old ModuleRef explicitly stale",
        format!(
            "absent across {ABSENCE_WINDOW_POLLS} polls; new registration != old; old ref -32013; fresh read 7/rev1; stopped cleanly, registry empty"
        ),
    ))
}

// ---------------------------------------------------------------------------
// Generated TS fixtures (written to the suite tempdir, run with bun)
// ---------------------------------------------------------------------------

/// SDK consumer probe: independent reads/adds/subscriptions against the
/// instances' published modules plus explicit stale-ref invokes.
const PROBE_TS: &str = r#"#!/usr/bin/env bun
// S2 e2e consumer probe (generated by xtask/src/s2_e2e.rs). JSONL in/out:
// {"id","op",...} -> {"id","ok",true,"value"|{"error"}}; samples and
// subscription ends arrive as {"event",...} lines. Uses the real SDK client.
import { createInterface } from "node:readline";

const SDK = process.env.S2_TS_SDK;
const { ConexWsClient } = await import(`${SDK}/ws-client`);

const CONEX_URL = process.env.CONEX_URL;
const CONEX_TOKEN = process.env.CONEX_TOKEN;
const SPACE = process.env.CONEX_SPACE;
const I32_MIN = -2147483648;
const I32_MAX = 2147483647;

function int32(value, what) {
  const n = Number(value);
  if (!Number.isInteger(n) || n < I32_MIN || n > I32_MAX) throw new Error(`${what} out of int32 range`);
  return n;
}
function result(value) {
  return { count: int32(value.count, "count"), revision: String(value.revision) };
}
const contract = {
  contractId: "conex.counter",
  methods: {
    "counter/read": { execution: "read_only", input: () => ({}), output: result },
    "counter/add": { execution: "non_replayable", input: a => ({ delta: int32(a.delta, "delta") }), output: result },
  },
  states: { "counter/value": { input: () => ({}), value: v => ({ count: int32(v.count, "count") }) } },
};

const ServiceSocket = globalThis.WebSocket;
const link = {
  origin: new URL(CONEX_URL).origin,
  tokenProvider: async () => CONEX_TOKEN,
  createWebSocket: (target, headers) => new ServiceSocket(target, { headers }),
};
const client = new ConexWsClient({ ...link, capabilities: { contracts: [contract] } });

function emit(value) { console.log(JSON.stringify(value)); }
function fail(error) {
  return {
    code: error && typeof error === "object" ? error.code ?? null : null,
    message: error instanceof Error ? error.message : String(error),
  };
}

let cache = [];
async function list() {
  cache = await client.listModules(SPACE);
  return {
    modules: cache.map(m => ({
      providerEndpointId: m.providerEndpointId,
      registrationId: m.registrationId,
      contractId: m.contractId,
    })),
  };
}
function ref(endpoint) {
  const found = cache.find(m => m.providerEndpointId === endpoint);
  if (!found) throw new Error(`endpoint ${endpoint} not in cached list`);
  return found;
}
async function invokeAt(endpoint, registrationId, method, argument) {
  const base = ref(endpoint);
  const target = registrationId === undefined ? base : { ...base, registrationId };
  return await client.invoke(target, method, argument).result;
}

async function run(command) {
  switch (command.op) {
    case "connect": {
      await client.connect();
      return list();
    }
    case "list": return list();
    case "read": return invokeAt(command.endpoint, undefined, "counter/read", {});
    case "add": return invokeAt(command.endpoint, undefined, "counter/add", { delta: int32(command.delta, "delta") });
    case "invoke-ref": return invokeAt(command.endpoint, command.registrationId, command.method, command.argument ?? {});
    case "subscribe": {
      const target = ref(command.endpoint);
      const sub = client.subscribe(target, "counter/value", {}, sample => {
        emit({ event: "sample", subscriptionId: sub.subscriptionId, snapshot: false, count: sample.value.count, revision: String(sample.revision) });
      });
      void sub.snapshot.then(s => emit({ event: "sample", subscriptionId: sub.subscriptionId, snapshot: true, count: s.value.count, revision: String(s.revision) }));
      void sub.ended.then(e => emit({ event: "sub-end", subscriptionId: sub.subscriptionId, code: e.code ?? null, message: e.message }));
      return { subscriptionId: sub.subscriptionId };
    }
    case "shutdown": return { closed: true };
    default: throw new Error(`unknown op ${command.op}`);
  }
}

async function main() {
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  for await (const line of lines) {
    if (!line.trim()) continue;
    let command;
    try { command = JSON.parse(line); } catch (error) { emit({ id: null, ok: false, error: fail(error) }); continue; }
    await run(command).then(
      value => {
        emit({ id: command.id, ok: true, value });
        if (command.op === "shutdown") { client.close(); process.exit(0); }
      },
      error => emit({ id: command.id, ok: false, error: fail(error) }),
    );
  }
  process.exit(0);
}
main().catch(error => { console.error(error instanceof Error ? error.message : String(error)); process.exit(1); });
"#;

/// Plugin variants on the same TS SDK: composition rejections (must fail
/// before any network I/O) and a fresh instance generation for A08.
const VARIANT_TS: &str = r#"#!/usr/bin/env bun
// S2 e2e plugin variant (generated by xtask/src/s2_e2e.rs).
// Modes: reject-missing | reject-cycle (composition must reject before any
// network I/O) and fresh (a new instance generation on the sample endpoints).
import { createInterface } from "node:readline";

const SDK = process.env.S2_TS_SDK;
const { definePlugin, Instance, PluginCompositeError } = await import(`${SDK}/plugin`);
const { AtomicState } = await import(`${SDK}/state`);
const { ConexWsClient } = await import(`${SDK}/ws-client`);

const MODE = process.env.S2_MODE;
const CONEX_URL = process.env.CONEX_URL ?? "";
const CONEX_TOKEN = process.env.CONEX_TOKEN ?? "";
const SPACE = process.env.CONEX_SPACE ?? "dev";
const PREFIX = process.env.CONEX_ENDPOINT ?? "s2-ts";
const I32_MIN = -2147483648;
const I32_MAX = 2147483647;

function emit(value) { console.log(JSON.stringify(value)); }
function int32(value, what) {
  const n = Number(value);
  if (!Number.isInteger(n) || n < I32_MIN || n > I32_MAX) throw new Error(`${what} out of int32 range`);
  return n;
}
function result(value) {
  return { count: int32(value.count, "count"), revision: String(value.revision) };
}
const contract = {
  contractId: "conex.counter",
  methods: {
    "counter/read": { execution: "read_only", input: () => ({}), output: result },
    "counter/add": { execution: "non_replayable", input: a => ({ delta: int32(a.delta, "delta") }), output: result },
  },
  states: { "counter/value": { input: () => ({}), value: v => ({ count: int32(v.count, "count") }) } },
};

let sockets = 0;
function makeLink(real) {
  return {
    origin: new URL(CONEX_URL).origin,
    tokenProvider: async () => CONEX_TOKEN,
    createWebSocket: (target, headers) => {
      sockets += 1;
      if (!real) throw new Error("no network I/O expected before composition succeeds");
      return new globalThis.WebSocket(target, { headers });
    },
  };
}

function counterModule(name) {
  const state = new AtomicState({ value: { count: 0 }, revision: "0", sampledAtMs: String(Date.now()), quality: "current" });
  const commit = count => {
    const revision = (BigInt(state.read().revision) + 1n).toString();
    state.publish({ value: { count }, revision, sampledAtMs: String(Date.now()), quality: "current" });
    return { count, revision };
  };
  return {
    name,
    contract,
    registration: { spaceId: SPACE, providerEndpointId: `${PREFIX}-counter`, integrationId: "integration" },
    handlers: {
      "counter/read": () => {
        const sample = state.read();
        return { count: sample.value.count, revision: sample.revision.toString() };
      },
      "counter/add": raw => {
        const delta = int32(raw.delta, "delta");
        const next = state.read().value.count + delta;
        if (next < I32_MIN || next > I32_MAX) throw new Error("int32 overflow");
        return commit(next);
      },
    },
    states: { "counter/value": (_argument, observer, _context, signal) => state.open({}, observer, signal) },
    start: () => ({ read: () => state.read(), add: commit }),
  };
}

async function reject(modules, expect) {
  const plugin = definePlugin({ id: "reject-variant", graph: () => ({ modules }) });
  const instance = new Instance(plugin, makeLink(false));
  try {
    await instance.start({});
    emit({ event: "error", message: "composition unexpectedly succeeded" });
    process.exit(1);
  } catch (error) {
    if (!(error instanceof PluginCompositeError)) {
      emit({ event: "error", message: `expected PluginCompositeError, got ${error}` });
      process.exit(1);
    }
    const failure = error.failure;
    if (failure.kind !== expect) {
      emit({ event: "error", message: `expected ${expect}, got ${failure.kind}` });
      process.exit(1);
    }
    emit({
      event: "rejected",
      kind: failure.kind,
      module: failure.module,
      dependency: failure.dependency,
      cycle: failure.cycle,
      sockets,
    });
    process.exit(0);
  }
}

async function fresh() {
  const plugin = definePlugin({ id: "counter-plugin", modules: [counterModule("counter")] });
  const instance = new Instance(plugin, makeLink(true));
  await instance.start({});
  const reference = instance.module("counter");
  emit({ event: "publish", providerEndpointId: reference.providerEndpointId, registrationId: reference.registrationId });

  const consumer = new ConexWsClient({ ...makeLink(true), capabilities: { contracts: [contract] } });
  await consumer.connect();
  const modules = await consumer.listModules(SPACE);
  const live = modules.find(m => m.providerEndpointId === `${PREFIX}-counter`);
  if (!live) { emit({ event: "error", message: "fresh registration missing from module/list" }); process.exit(1); }
  const initial = await consumer.invoke(live, "counter/read", {}).result;
  emit({ event: "initial", count: initial.count, revision: String(initial.revision) });
  const added = await consumer.invoke(live, "counter/add", { delta: 7 }).result;
  emit({ event: "add", count: added.count, revision: String(added.revision) });
  emit({ event: "ready-for-shutdown" });

  const shutdown = Promise.withResolvers();
  const lines = createInterface({ input: process.stdin, crlfDelay: Infinity });
  lines.on("line", line => {
    try { if (JSON.parse(line).op === "shutdown") shutdown.resolve(); } catch { /* ignore */ }
  });
  await shutdown.promise;

  emit({ event: "stopping" });
  await instance.stop();
  consumer.close();
  emit({ event: "stopped", state: instance.state });
  process.exit(0);
}

if (MODE === "reject-missing") {
  const a = { ...counterModule("a"), depsLocal: ["ghost"] };
  await reject([a], "missing_dependency");
} else if (MODE === "reject-cycle") {
  const a = { ...counterModule("a"), depsLocal: ["b"] };
  const b = { ...counterModule("b"), depsLocal: ["a"] };
  await reject([a, b], "circular_dependency");
} else if (MODE === "fresh") {
  await fresh();
} else {
  emit({ event: "error", message: `unknown S2_MODE ${MODE}` });
  process.exit(1);
}
"#;

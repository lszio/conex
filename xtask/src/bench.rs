//! `cargo xtask bench --suite hello`: measures the hello path against a real
//! host and writes a JSON report the performance page renders.
//!
//! The numbers must come from a real process, so this starts an actual
//! `conex-host`, runs the Bun bench against it, and tears it down.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

const READY_TIMEOUT: Duration = Duration::from_secs(20);

pub fn run(suite: &str, rounds: Option<u32>) -> Result<()> {
    if suite != "hello" {
        bail!("unknown bench suite: {suite} (supported: hello)");
    }
    let root = repo_root()?;
    let status = Command::new("cargo")
        .args(["build", "-p", "conex-host", "--quiet"])
        .current_dir(&root)
        .status()
        .context("build conex-host")?;
    if !status.success() {
        bail!("cargo build -p conex-host failed");
    }

    let port = free_port()?;
    let dir = tempfile::tempdir().context("create bench temp dir")?;
    for name in ["content", "session", "operation"] {
        std::fs::create_dir_all(dir.path().join(name))?;
    }
    // The bench only exercises the client path, so no data endpoints and no
    // credentials: the smallest config that can serve the page.
    let web_root = root.join("web/dist");
    if !web_root.join("index.html").is_file() {
        let built = Command::new("bun")
            .args(["run", "build:web"])
            .current_dir(&root)
            .status()
            .context("build landing page")?;
        if !built.success() {
            bail!("bun run build:web failed");
        }
    }
    let config = dir.path().join("host.toml");
    let origin = format!("http://127.0.0.1:{port}");
    std::fs::write(
        &config,
        format!(
            "listen = \"127.0.0.1:{port}\"\n\
             allow_loopback_http = true\n\
             audience = \"bench.local\"\n\
             host_origin = \"conex://bench\"\n\
             web_origin = \"{origin}\"\n\
             web_root = \"{}\"\n\
             content_root = \"{}\"\n\
             session_root = \"{}\"\n\
             operation_root = \"{}\"\n\
             \n\
             [web_guest]\n\
             principal_id = \"guest\"\n\
             tenant_id = \"bench\"\n\
             max_sessions = 512\n\
             max_issue_per_minute = 100000\n",
            web_root.display(),
            dir.path().join("content").display(),
            dir.path().join("session").display(),
            dir.path().join("operation").display(),
        ),
    )?;

    let mut child = Command::new(root.join("target/debug/conex-host"))
        .arg(&config)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn conex-host")?;
    let ready = wait_ready(port);
    if ready {
        let out = dir.path().join("bench.json");
        let bench = Command::new("bun")
            .arg("run")
            .arg(root.join("sdk/typescript/tests/bench-hello.ts"))
            .current_dir(&root)
            .env("CONEX_BENCH_ORIGIN", &origin)
            .env("CONEX_BENCH_OUT", &out)
            .env("CONEX_BENCH_ROUNDS", rounds.unwrap_or(60).to_string())
            .status()
            .context("run hello bench")?;
        stop(&mut child);
        if !bench.success() {
            bail!("hello bench failed");
        }
        // The report is committed so the performance page has data to render
        // without every build depending on a live measurement.
        let dest = root.join("web/src/perf-data.json");
        std::fs::copy(&out, &dest).context("copy bench report into web/src")?;
        println!("wrote {}", dest.display());
        println!("{}", std::fs::read_to_string(&dest)?);
        return Ok(());
    }
    stop(&mut child);
    bail!("conex-host did not become ready on port {port}")
}

fn repo_root() -> Result<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .canonicalize()?)
}

fn wait_ready(port: u16) -> bool {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

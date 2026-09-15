//! Cross-language end-to-end driver: real Rust host, real TypeScript SDK.
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};

pub fn run(suite: &str) -> Result<()> {
    match suite {
        "p0-ts" => run_p0_ts(),
        other => bail!("unknown e2e suite: {other}"),
    }
}

fn run_p0_ts() -> Result<()> {
    let root = repo_root()?;
    let build = Command::new("cargo")
        .args(["build", "-p", "conex-host", "--quiet"])
        .current_dir(&root)
        .status()
        .context("build conex-host")?;
    if !build.success() {
        bail!("cargo build -p conex-host failed");
    }

    let port = free_port()?;
    let token = "e2e-token";
    let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
    let fixtures = root.join("fixtures/p0/notes");
    let config = format!(
        "listen = \"127.0.0.1:{port}\"\nallow_loopback_http = true\naudience = \"host.local\"\n\
[[tokens]]\ntoken_hash = \"{token_hash}\"\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\n\
[[policy]]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"notes-local\"\nactions = [\"list\", \"read\", \"search\"]\nroot = \"\"\nsubtree = true\n\
[[endpoints]]\nid = \"notes-local\"\ntenant_id = \"tenant-a\"\nprovider_id = \"source\"\nkind = \"source-fs\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"{}\"\n",
        fixtures.display()
    );
    let dir = tempfile::tempdir().context("create e2e temp dir")?;
    let config_path = dir.path().join("host.toml");
    std::fs::write(&config_path, config).context("write host config")?;

    let binary = root.join("target/debug/conex-host");
    let mut child = Command::new(&binary)
        .arg(&config_path)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn {}", binary.display()))?;

    if !wait_ready(port, Duration::from_secs(10)) {
        let _ = child.kill();
        let _ = child.wait();
        bail!("conex-host did not become ready on port {port}");
    }

    let bun = Command::new("bun")
        .args(["test", "sdk/typescript/tests/e2e.test.ts"])
        .current_dir(&root)
        .env("CONEX_E2E_URL", format!("http://127.0.0.1:{port}/rpc"))
        .env("CONEX_E2E_TOKEN", token)
        .status()
        .context("run bun e2e")?;

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&config_path);

    if !bun.success() {
        bail!("p0-ts e2e failed");
    }
    Ok(())
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
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

#[allow(dead_code)]
fn repo_relative(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).map(PathBuf::from).unwrap_or_else(|_| path.to_path_buf())
}

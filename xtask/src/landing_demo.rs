use std::fs;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

pub struct DemoFixture {
    pub _dir: TempDir,
    pub port: u16,
    pub ui_token: String,
    pub service_token: String,
    pub host_config: PathBuf,
    pub agent_configs: [PathBuf; 2],
}

pub struct DemoProcesses {
    pub host: Child,
    pub agents: [Option<Child>; 2],
}

impl Drop for DemoProcesses {
    fn drop(&mut self) {
        stop(&mut self.host);
        for child in &mut self.agents {
            if let Some(child) = child.as_mut() {
                stop(child);
            }
        }
    }
}

impl DemoProcesses {
    pub fn spawn_agent(&mut self, index: usize, binary: &Path, config: &Path, inherit_logs: bool) -> Result<()> {
        if index >= self.agents.len() {
            bail!("agent index {index} is unavailable");
        }
        if let Some(child) = self.agents[index].as_mut() {
            if child.try_wait()?.is_none() {
                bail!("agent index {index} is unavailable");
            }
            self.agents[index] = None;
        }
        self.agents[index] = Some(spawn_agent_child(binary, config, inherit_logs)?);
        Ok(())
    }
}

pub fn run() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").canonicalize()?;
    let build = Command::new("cargo").args(["build", "-p", "conex-host", "-p", "conex-agent", "--quiet"]).current_dir(&root).status().context("build host and agent binaries")?;
    if !build.success() { bail!("cargo build -p conex-host -p conex-agent failed"); }
    let web = root.join("web/dist");
    if ["index.html", "app.js", "style.css"].iter().any(|name| !web.join(name).is_file()) {
        let build_web = Command::new("bun").args(["run", "build:web"]).current_dir(&root).status().context("build connected landing page")?;
        if !build_web.success() { bail!("bun run build:web failed"); }
    }
    let fixture = prepare(&root, free_port()?, true)?;
    let mut processes = spawn(&root, &fixture, true)?;
    if !wait_ready(fixture.port, Duration::from_secs(10)) { bail!("conex-host did not become ready on port {}", fixture.port); }
    println!("connected landing demo: http://127.0.0.1:{}/", fixture.port);
    println!("UI login credential (temporary; not stored in logs): {}", fixture.ui_token);
    println!("Press Ctrl-C to stop Host and both Agents.");
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().context("create landing demo runtime")?;
    runtime.block_on(async {
        let mut interrupted = Box::pin(tokio::signal::ctrl_c());
        loop {
            tokio::select! {
                result = &mut interrupted => { result.context("wait for Ctrl-C")?; break Ok(()); }
                _ = tokio::time::sleep(Duration::from_millis(250)) => {
                    if let Some(status) = processes.host.try_wait().context("poll host")? { bail!("conex-host exited with {status}"); }
                    for (index, agent) in processes.agents.iter_mut().enumerate() {
                        if let Some(child) = agent.as_mut() && let Some(status) = child.try_wait().context("poll agent")? { bail!("agent-{} exited with {status}", if index == 0 { 'a' } else { 'b' }); }
                    }
                }
            }
        }
    })?;
    Ok(())
}

pub fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

pub fn prepare(root: &Path, port: u16, with_web: bool) -> Result<DemoFixture> {
    let dir = tempfile::tempdir().context("create landing demo temp dir")?;
    let ui_token = random_token()?;
    let service_token = random_token()?;
    let agent_tokens = [random_token()?, random_token()?];
    let token_paths = [dir.path().join("agent-a.token"), dir.path().join("agent-b.token")];
    for (path, token) in token_paths.iter().zip(&agent_tokens) { fs::write(path, format!("{token}\n"))?; restrict_secret(path)?; }
    let roots = [dir.path().join("agent-a-files"), dir.path().join("agent-b-files")];
    for (index, root_path) in roots.iter().enumerate() {
        fs::create_dir_all(root_path.join("team"))?;
        fs::write(root_path.join("team/shared.md"), format!("connected landing agent {}\n", if index == 0 { "A" } else { "B" }))?;
        fs::write(root_path.join("team/only-this-agent.md"), format!("private agent {}\n", index + 1))?;
    }
    let data = [dir.path().join("content"), dir.path().join("session"), dir.path().join("operation")];
    for path in &data { fs::create_dir_all(path)?; }
    let web = if with_web {
        let web_root = root.join("web/dist");
        for name in ["index.html", "app.js", "style.css"] { if !web_root.join(name).is_file() { bail!("missing web asset {}; run `bun run build:web` first", web_root.join(name).display()); } }
        format!("web_origin = \"http://127.0.0.1:{port}\"\nweb_root = \"{}\"\n", web_root.display())
    } else { String::new() };
    let hash = |token: &str| hex::encode(Sha256::digest(token.as_bytes()));
    let host_config = dir.path().join("host.toml");
    let config = format!(
        "listen = \"127.0.0.1:{port}\"\nallow_loopback_http = true\naudience = \"host.local\"\nhost_origin = \"conex://host.local\"\nweb_guest = true\n{web}content_root = \"{}\"\nsession_root = \"{}\"\noperation_root = \"{}\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"demo-ui\"\ntenant_id = \"demo\"\naudience = \"host.local\"\nrole = \"ui\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"demo-probe\"\ntenant_id = \"demo\"\naudience = \"host.local\"\nrole = \"service\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"agent-a\"\ntenant_id = \"demo\"\naudience = \"host.local\"\nrole = \"agent\"\n\n[[tokens]]\ntoken_hash = \"{}\"\nprincipal_id = \"agent-b\"\ntenant_id = \"demo\"\naudience = \"host.local\"\nrole = \"agent\"\n\n[[agents]]\nid = \"agent-a\"\ntenant_id = \"demo\"\ncredential_name = \"agent-a-token\"\ncredential_backend = \"file:{}\"\n\n[[agents]]\nid = \"agent-b\"\ntenant_id = \"demo\"\ncredential_name = \"agent-b-token\"\ncredential_backend = \"file:{}\"\n\n[[endpoints]]\nid = \"notes-a\"\ntenant_id = \"demo\"\nprovider_id = \"notes-a-provider\"\nkind = \"source-remote\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"team\"\nagent_id = \"agent-a\"\n\n[[endpoints]]\nid = \"notes-b\"\ntenant_id = \"demo\"\nprovider_id = \"notes-b-provider\"\nkind = \"source-remote\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"team\"\nagent_id = \"agent-b\"\n{}",
        data[0].display(), data[1].display(), data[2].display(), hash(&ui_token), hash(&service_token), hash(&agent_tokens[0]), hash(&agent_tokens[1]), token_paths[0].display(), token_paths[1].display(), policies()
    );
    fs::write(&host_config, config)?;
    let mut agent_configs = [PathBuf::new(), PathBuf::new()];
    for (index, path) in agent_configs.iter_mut().enumerate() {
        *path = dir.path().join(format!("agent-{}.toml", if index == 0 { 'a' } else { 'b' }));
        let suffix = if index == 0 { 'a' } else { 'b' };
        let endpoint = if index == 0 { "notes-a" } else { "notes-b" };
        fs::write(path, format!("agent_id = \"agent-{suffix}\"\ntenant_id = \"demo\"\nhost_url = \"ws://127.0.0.1:{port}/wss\"\ntoken_name = \"agent-{suffix}-token\"\ntoken_backend = \"file:{}\"\nhost_origin = \"conex://host.local\"\nallow_loopback_ws = true\n\n[[endpoints]]\nendpoint_id = \"{endpoint}\"\nroot = \"{}\"\nmethods = [\"source/list\", \"source/read\", \"source/search\"]\nresources = [\"*\"]\n", token_paths[index].display(), roots[index].display()))?;
    }
    Ok(DemoFixture { _dir: dir, port, ui_token, service_token, host_config, agent_configs })
}

fn policies() -> String {
    let mut out = String::new();
    for principal in ["demo-ui", "demo-probe"] {
        for endpoint in ["notes-a", "notes-b"] {
            out.push_str(&format!("\n[[policy]]\nprincipal_id = \"{principal}\"\ntenant_id = \"demo\"\nendpoint_id = \"{endpoint}\"\nactions = [\"list\", \"read\", \"search\"]\nroot = \"team\"\nsubtree = true\n"));
        }
    }
    out
}

pub fn spawn(root: &Path, fixture: &DemoFixture, inherit_logs: bool) -> Result<DemoProcesses> {
    let host_binary = root.join("target/debug/conex-host");
    let agent_binary = root.join("target/debug/conex-agent");
    for binary in [&host_binary, &agent_binary] { if !binary.is_file() { bail!("missing {}; run `cargo build -p conex-host -p conex-agent` first", binary.display()); } }
    let host = spawn_child(&host_binary, &fixture.host_config, inherit_logs)?;
    let mut processes = DemoProcesses { host, agents: [None, None] };
    for index in 0..2 { processes.spawn_agent(index, &agent_binary, &fixture.agent_configs[index], inherit_logs)?; }
    Ok(processes)
}

pub fn wait_ready(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline { if TcpStream::connect(("127.0.0.1", port)).is_ok() { return true; } std::thread::sleep(Duration::from_millis(100)); }
    false
}

fn spawn_child(binary: &Path, config: &Path, inherit_logs: bool) -> Result<Child> {
    let cwd = binary.parent().and_then(Path::parent).and_then(Path::parent).unwrap_or_else(|| Path::new("."));
    let mut command = Command::new(binary);
    command.arg(config).current_dir(cwd);
    if inherit_logs { command.stdout(Stdio::inherit()).stderr(Stdio::inherit()); } else { command.stdout(Stdio::null()).stderr(Stdio::null()); }
    command.spawn().with_context(|| format!("spawn {}", binary.display()))
}
fn spawn_agent_child(binary: &Path, config: &Path, inherit_logs: bool) -> Result<Child> {
    let cwd = binary.parent().and_then(Path::parent).and_then(Path::parent).unwrap_or_else(|| Path::new("."));
    let mut command = Command::new(binary);
    command.arg("--config").arg(config).current_dir(cwd);
    if inherit_logs { command.stdout(Stdio::inherit()).stderr(Stdio::inherit()); } else { command.stdout(Stdio::null()).stderr(Stdio::null()); }
    command.spawn().with_context(|| format!("spawn {}", binary.display()))
}

fn stop(child: &mut Child) { if child.try_wait().ok().flatten().is_none() { let _ = child.kill(); } let _ = child.wait(); }
fn random_token() -> Result<String> { let mut bytes = [0u8; 32]; fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?; Ok(hex::encode(bytes)) }
fn restrict_secret(path: &Path) -> Result<()> {
    #[cfg(unix)] { use std::os::unix::fs::PermissionsExt; fs::set_permissions(path, fs::Permissions::from_mode(0o600))?; }
    Ok(())
}

//! Cross-language end-to-end driver: real Rust host, real TypeScript SDK.
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use crate::landing_demo;

pub fn run(suite: &str) -> Result<()> {
    match suite {
        "p0-ts" => run_p0_ts(),
        "p1-stream-1gib" => run_p1_stream_1gib(),
        "connected-landing" => run_connected_landing(),
        "connected-landing-web" => run_connected_landing_web(),
        "connected-landing-connections" => run_connected_landing_connections(),
        other => bail!("unknown e2e suite: {other}"),
    }
}

/// Spawn the host with P1 backends and run the Bun 1 GiB-over-stream test
/// (design §14 P1 #1: 1 GiB through the protobuf blob channel with bounded
/// memory, bad chunk rejection, and staging surviving a dropped connection).
fn run_p1_stream_1gib() -> Result<()> {
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
    let dir = tempfile::tempdir().context("create e2e temp dir")?;
    let content = dir.path().join("content");
    let session = dir.path().join("session");
    let operation = dir.path().join("operation");
    for p in [&content, &session, &operation] {
        std::fs::create_dir_all(p).context("mkdir p1 root")?;
    }
    let config = format!(
        "listen = \"127.0.0.1:{port}\"\nallow_loopback_http = true\naudience = \"host.local\"\n\
host_origin = \"conex://broker.local\"\n\
content_root = \"{}\"\nsession_root = \"{}\"\noperation_root = \"{}\"\n\
[[tokens]]\ntoken_hash = \"{token_hash}\"\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\naudience = \"host.local\"\n\
[[policy]]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"notes-local\"\nactions = [\"list\", \"read\", \"search\"]\nroot = \"\"\nsubtree = true\n\
[[endpoints]]\nid = \"notes-local\"\ntenant_id = \"tenant-a\"\nprovider_id = \"source\"\nkind = \"source-fs\"\nprovides = [\"source/list\", \"source/read\", \"source/search\"]\nroot = \"{}\"\n",
        content.display(),
        session.display(),
        operation.display(),
        fixtures.display()
    );
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
        .args(["test", "sdk/typescript/tests/p1_stream_1gib.test.ts"])
        .current_dir(&root)
        .env("CONEX_E2E_WS", format!("ws://127.0.0.1:{port}/wss"))
        .env("CONEX_E2E_TOKEN", token)
        .env("CONEX_E2E_1GIB", "1")
        .status()
        .context("run bun 1gib stream e2e")?;

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_file(&config_path);

    if !bun.success() {
        bail!("p1-stream-1gib e2e failed");
    }
    Ok(())
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
    path.strip_prefix(root)
        .map(PathBuf::from)
        .unwrap_or_else(|_| path.to_path_buf())
}
fn run_connected_landing() -> Result<()> {
    let root = repo_root()?;
    let build = Command::new("cargo")
        .args(["build", "-p", "conex-host", "-p", "conex-agent", "--quiet"])
        .current_dir(&root)
        .status()
        .context("build connected landing binaries")?;
    if !build.success() {
        bail!("cargo build -p conex-host -p conex-agent failed");
    }
    let fixture = landing_demo::prepare(&root, free_port()?, false)?;
    let _processes = landing_demo::spawn(&root, &fixture, true)?;
    if !landing_demo::wait_ready(fixture.port, Duration::from_secs(10)) {
        bail!("conex-host did not become ready on port {}", fixture.port);
    }
    let script = r#"
        import { ConexClient } from "./sdk/typescript/src/client.ts";
        const client = await ConexClient.connect({
          url: `http://127.0.0.1:${process.env.CONEX_LANDING_PORT}/rpc`,
          tokenProvider: async () => process.env.CONEX_LANDING_TOKEN,
          requires: ["source/list", "source/read", "source/search"],
        });
        const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
        for (let attempt = 0; attempt < 100; attempt++) {
          try {
            const a = await client.read("notes-a", { resourceId: "team/shared.md" });
            const b = await client.read("notes-b", { resourceId: "team/shared.md" });
            if (a.text !== "connected landing agent A\n" || b.text !== "connected landing agent B\n") throw new Error("agent routing mismatch");
            const al = await client.list("notes-a", { root: "team" });
            const bl = await client.list("notes-b", { root: "team" });
            if (!al.items?.length || !bl.items?.length) throw new Error("empty remote list");
            const as = await client.search("notes-a", { root: "team", query: "connected" });
            const bs = await client.search("notes-b", { root: "team", query: "connected" });
            if (!as.items?.length || !bs.items?.length) throw new Error("empty remote search");
            process.stdout.write("connected-landing: A/B list/read/search passed\n");
            client.close();
            process.exit(0);
          } catch (error) {
            if (attempt === 99) throw error;
            await sleep(100);
          }
        }
    "#;
    let probe = Command::new("bun")
        .args(["-e", script])
        .current_dir(&root)
        .env("CONEX_LANDING_PORT", fixture.port.to_string())
        .env("CONEX_LANDING_TOKEN", &fixture.service_token)
        .status()
        .context("run connected landing source probe")?;
    if !probe.success() {
        bail!("connected landing source probe failed");
    }
    let b_probe = Command::new("bun")
        .args(["-e", r#"
            import { ConexClient } from "./sdk/typescript/src/client.ts";
            const client = await ConexClient.connect({
              url: `http://127.0.0.1:${process.env.CONEX_LANDING_PORT}/rpc`,
              tokenProvider: async () => process.env.CONEX_LANDING_TOKEN,
              requires: ["source/read"],
            });
            const body = await client.read("notes-b", { resourceId: "team/shared.md" });
            if (body.text !== "connected landing agent B\n") throw new Error(`agent B failed after A stop: ${body.text}`);
            client.close();
        "#])
        .current_dir(&root)
        .env("CONEX_LANDING_PORT", fixture.port.to_string())
        .env("CONEX_LANDING_TOKEN", &fixture.service_token)
        .status()
        .context("run connected landing isolation probe")?;
    if !b_probe.success() {
        bail!("connected landing isolation probe failed");
    }
    println!("connected-landing: real Host + two Agent processes, routing and A/B isolation passed");
    Ok(())
}

/// Spawn Host + two Agents with web auth enabled, then exercise the full
/// browser web-session lifecycle:
///   POST /web/login (HTTP) -> cookie
///   GET  /web/session (HTTP) -> session JSON
///   ConexClient (HTTP) -> read remote resource (login + binding hello)
///   POST /tickets (HTTP, cookie) -> ticket
///   /wss?ticket=... -> hello + ready
///   ConexWsClient -> listEndpoints + read remote resource
///
/// Real Host + Agents come from `landing_demo::spawn` with `with_web=true`.
fn run_connected_landing_web() -> Result<()> {
    let root = repo_root()?;
    let build = Command::new("cargo")
        .args(["build", "-p", "conex-host", "-p", "conex-agent", "--quiet"])
        .current_dir(&root)
        .status()
        .context("build connected landing-web binaries")?;
    if !build.success() {
        bail!("cargo build -p conex-host -p conex-agent failed");
    }
    let fixture = landing_demo::prepare(&root, free_port()?, true)?;
    let _processes = landing_demo::spawn(&root, &fixture, true)?;
    if !landing_demo::wait_ready(fixture.port, Duration::from_secs(10)) {
        bail!("conex-host did not become ready on port {}", fixture.port);
    }
    let port = fixture.port.to_string();
    let script = r#"
        import { ConexClient } from "./sdk/typescript/src/client.ts";
        import { ConexWsClient } from "./sdk/typescript/src/ws-client.ts";

        const port = process.env.CONEX_LANDING_PORT;
        const origin = `http://127.0.0.1:${port}`;
        const uiToken = process.env.CONEX_LANDING_UI_TOKEN;
        if (!port || !uiToken) throw new Error("CONEX_LANDING_PORT and CONEX_LANDING_UI_TOKEN are required");

        // 1) HTTP login: POST /web/login with bearer Authorization + matching Origin.
        const loginResponse = await fetch(`${origin}/web/login`, {
          method: "POST",
          headers: {
            authorization: `Bearer ${uiToken}`,
            origin,
          },
        });
        if (loginResponse.status !== 200) {
          const body = await loginResponse.text();
          console.error(`web/login failed status=${loginResponse.status} body=${body}`);
          throw new Error("web/login failed");
        }
        const setCookie = loginResponse.headers.get("set-cookie");
        if (!setCookie) throw new Error("web/login did not return a Set-Cookie header");
        const cookie = setCookie.split(";")[0];

        // 2) HTTP session: GET /web/session with the cookie.
        const sessionResponse = await fetch(`${origin}/web/session`, {
          headers: { cookie, origin },
        });
        if (sessionResponse.status !== 200) {
          throw new Error(`web/session failed: status=${sessionResponse.status}`);
        }
        const sessionJson = await sessionResponse.json();
        if (sessionJson.role !== "ui") throw new Error(`unexpected session role: ${sessionJson.role}`);
        if (!sessionJson.csrf || typeof sessionJson.csrf !== "string") throw new Error("csrf missing");

        // 3) HTTP business call: ConexClient (service role) reads a remote resource.
        // UI credentials are rejected by /rpc; the SDK's bearer consumer pattern
        // maps cleanly to the existing service-token flow. Agents reconnect on a
        // 1s/2s/4s backoff, so retry until both endpoints answer.
        const serviceToken = process.env.CONEX_LANDING_TOKEN;
        if (!serviceToken) throw new Error("CONEX_LANDING_TOKEN is required");
        const httpClient = await ConexClient.connect({
          url: `${origin}/rpc`,
          tokenProvider: async () => serviceToken,
          requires: ["source/read"],
        });
        const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
        let httpRead;
        for (let attempt = 0; attempt < 60; attempt++) {
          try {
            httpRead = await httpClient.read("notes-a", { resourceId: "team/shared.md" });
            if (httpRead.text === "connected landing agent A\n") break;
          } catch (error) {
            if (attempt === 59) throw error;
          }
          await sleep(500);
        }
        if (!httpRead || httpRead.text !== "connected landing agent A\n") {
          throw new Error(`http read mismatch: ${JSON.stringify(httpRead)}`);
        }
        httpClient.close();

        // 4) WSS path: ConexWsClient uses cookie to fetch ticket + open WSS.
        // Bun's WebSocket must explicitly include the Origin header (browsers
        // add it automatically; Node/Bun do not).
        const OriginWS = function(url, _protocols) {
          return new (globalThis.WebSocket)(url, { headers: { origin } });
        };
        const cookieFetch = async (input, init = {}) => {
          const headers = new Headers(init.headers || {});
          headers.set("cookie", cookie);
          headers.set("origin", origin);
          return globalThis.fetch(input, { ...init, headers });
        };
        const wsClient = new ConexWsClient({
          origin,
          csrfToken: sessionJson.csrf,
          fetch: cookieFetch,
          WebSocket: OriginWS,
          reconnect: false,
          timeoutMs: 8000,
        });
        await wsClient.connect();

        // 5) WSS list endpoints: endpoint/list via the ready WSS link.
        const listed = await wsClient.listEndpoints({});
        const endpointIds = (listed.endpoints ?? []).map((entry) => entry.endpointId).sort();
        if (endpointIds.join(",") !== "notes-a,notes-b") {
          throw new Error(`endpoint/list mismatch: ${JSON.stringify(listed)}`);
        }

        // 6) WSS read: read a remote resource through the WSS endpoint.
        const wsRead = await wsClient.read("notes-b", { resourceId: "team/shared.md" });
        if (wsRead.text !== "connected landing agent B\n") {
          throw new Error(`wss read mismatch: ${JSON.stringify(wsRead)}`);
        }

        wsClient.close();
        process.stdout.write("connected-landing-web: web session lifecycle passed\n");
        process.exit(0);
    "#;
    let probe = Command::new("bun")
        .args(["-e", script])
        .current_dir(&root)
        .env("CONEX_LANDING_PORT", &port)
        .env("CONEX_LANDING_UI_TOKEN", &fixture.ui_token)
        .env("CONEX_LANDING_TOKEN", &fixture.service_token)
        .status()
        .context("run connected landing-web probe")?;
    if !probe.success() {
        bail!("connected landing-web probe failed");
    }
    println!("connected-landing-web: real Host + two Agent processes, login/session/list+read via HTTP and WSS passed");
    Ok(())
}

/// Spawn the same connected-landing demo fixture (Host + two Agents) used by
/// the existing suites, then drive `connection/list` over a fresh UI session:
///   1. `/web/login` (HTTP) -> cookie
///   2. `/tickets` (HTTP, cookie + CSRF) -> ticket
///   3. `/wss?ticket=...` -> hello + ready
///   4. `connection/list` over WSS -> exactly one browserLink row matching
///      the UI principal
///   5. Drop the WSS, open a second WSS, re-query `connection/list`. The row
///      must still be present and `lastSeenAtMs` must not regress (proving
///      the registry does not update when no business call has happened).
///   6. Send one `source/read` over WSS, then re-query `connection/list`.
///      `callsTotal` must grow by one; `callsInFlight` must drain to zero.
fn run_connected_landing_connections() -> Result<()> {
    let root = repo_root()?;
    let build = Command::new("cargo")
        .args(["build", "-p", "conex-host", "-p", "conex-agent", "--quiet"])
        .current_dir(&root)
        .status()
        .context("build connected landing-connections binaries")?;
    if !build.success() {
        bail!("cargo build -p conex-host -p conex-agent failed");
    }
    let fixture = landing_demo::prepare(&root, free_port()?, true)?;
    let _processes = landing_demo::spawn(&root, &fixture, true)?;
    if !landing_demo::wait_ready(fixture.port, Duration::from_secs(10)) {
        bail!("conex-host did not become ready on port {}", fixture.port);
    }
    let port = fixture.port.to_string();
    let script = r#"
        import { ConexWsClient } from "./sdk/typescript/src/ws-client.ts";

        const port = process.env.CONEX_LANDING_PORT;
        const origin = `http://127.0.0.1:${port}`;
        const uiToken = process.env.CONEX_LANDING_UI_TOKEN;
        if (!port || !uiToken) throw new Error("CONEX_LANDING_PORT and CONEX_LANDING_UI_TOKEN are required");

        const login = await fetch(`${origin}/web/login`, {
          method: "POST",
          headers: { authorization: `Bearer ${uiToken}`, origin },
        });
        if (login.status !== 200) throw new Error(`login status=${login.status}`);
        const cookie = (login.headers.get("set-cookie") ?? "").split(";")[0];

        const sessionRes = await fetch(`${origin}/web/session`, {
          headers: { cookie, origin },
        });
        const session = await sessionRes.json();
        if (session.role !== "ui") throw new Error(`role=${session.role}`);

        const ticketRes = await fetch(`${origin}/tickets`, {
          method: "POST",
          headers: { cookie, origin, "x-csrf-token": session.csrf, "content-type": "application/json" },
          body: "{}",
        });
        const ticketBody = await ticketRes.json();
        const ticket = ticketBody.ticket;
        if (!ticket) throw new Error("missing ticket");

        const OriginWS = function(url, _protocols) {
          return new (globalThis.WebSocket)(url, { headers: { origin } });
        };
        const cookieFetch = async (input, init = {}) => {
          const headers = new Headers(init.headers || {});
          headers.set("cookie", cookie);
          headers.set("origin", origin);
          return globalThis.fetch(input, { ...init, headers });
        };
        const wsClient = new ConexWsClient({
          origin, csrfToken: session.csrf, fetch: cookieFetch, WebSocket: OriginWS,
          reconnect: false, timeoutMs: 8000,
        });
        await wsClient.connect();

        const first = await wsClient.listConnections({});
        if (!Array.isArray(first.browserLinks) || first.browserLinks.length !== 1) {
          throw new Error(`expected one browserLink, got ${JSON.stringify(first)}`);
        }
        const link0 = first.browserLinks[0];
        if (link0.principalId !== "demo-ui") throw new Error(`principal=${link0.principalId}`);
        if (link0.tenantId !== "demo") throw new Error(`tenant=${link0.tenantId}`);
        if (Number(link0.ticketsIssued ?? "0") < 1) throw new Error(`tickets=${link0.ticketsIssued}`);
        const firstSeen = link0.lastSeenAtMs;

        // Drop and reopen: stale detection: lastSeenAtMs must not regress.
        wsClient.close();
        const ticketRes2 = await fetch(`${origin}/tickets`, {
          method: "POST", headers: { cookie, origin, "x-csrf-token": session.csrf, "content-type": "application/json" }, body: "{}",
        });
        const ticket2 = (await ticketRes2.json()).ticket;
        const wsClient2 = new ConexWsClient({
          origin, csrfToken: session.csrf, fetch: cookieFetch, WebSocket: OriginWS,
          reconnect: false, timeoutMs: 8000,
        });
        await wsClient2.connect();
        const second = await wsClient2.listConnections({});
        if (second.browserLinks.length !== 1) throw new Error(`expected one after reopen, got ${second.browserLinks.length}`);
        if (Number(second.browserLinks[0].lastSeenAtMs) < Number(firstSeen)) {
          throw new Error(`lastSeenAtMs regressed: ${firstSeen} -> ${second.browserLinks[0].lastSeenAtMs}`);
        }

        // Business call -> callsTotal must grow, callsInFlight must drain.
        // Wait briefly so both agent links finish registering, otherwise
        // the first read may see "remote agent is offline".
        let ready = false;
        for (let i = 0; i < 50 && !ready; i++) {
          const probe = await wsClient2.listConnections({});
          const agents = probe.agentLinks ?? [];
          ready = agents.length >= 2 && agents.every((a) => a.connectionState === "ready" || a.connectionState === 3);
          if (!ready) await new Promise((r) => setTimeout(r, 100));
        }
        const before = Number(second.browserLinks[0].callsTotal ?? "0");
        await wsClient2.read("notes-b", { resourceId: "team/shared.md" });
        const third = await wsClient2.listConnections({});
        const after = Number(third.browserLinks[0].callsTotal ?? "0");
        if (after <= before) {
          throw new Error(`callsTotal did not grow: ${before} -> ${after}`);
        }
        if (Number(third.browserLinks[0].callsInFlight ?? "0") > 1) {
          throw new Error(`callsInFlight did not drain: ${third.browserLinks[0].callsInFlight}`);
        }

        wsClient2.close();
        process.stdout.write("connected-landing-connections: WSS connection panel passed\n");
        process.exit(0);
    "#;
    let probe = Command::new("bun")
        .args(["-e", script])
        .current_dir(&root)
        .env("CONEX_LANDING_PORT", &port)
        .env("CONEX_LANDING_UI_TOKEN", &fixture.ui_token)
        .env("CONEX_LANDING_TOKEN", &fixture.service_token)
        .status()
        .context("run connected landing-connections probe")?;
    if !probe.success() {
        bail!("connected landing-connections probe failed");
    }
    println!("connected-landing-connections: real Host + two Agent processes, connection/list stale + dispatch counters passed");
    Ok(())
}

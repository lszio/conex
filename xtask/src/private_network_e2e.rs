//! Private-network acceptance (MVP-P5.5): real host, real node, real grants
//! over loopback. Every scenario below is an actual process/network round
//! trip — no fixtures stand in for TLS-less loopback facts, and the canary
//! sweep at the end reads the hub's real store directory and captured logs.
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use futures::{SinkExt, StreamExt};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const WEB_ORIGIN: &str = "http://conex-e2e.test";
const PROFILE: &str = "conex-jsonrpc2-wss";

pub fn run() -> Result<()> {
    let root = repo_root()?;
    build_binaries(&root)?;

    #[allow(deprecated)]
    let work = tempfile::tempdir()?.into_path();
    let hub_port = free_port()?;
    let direct_port = free_port()?;
    let hub_url = format!("http://127.0.0.1:{hub_port}");

    // Restricted test keys: a fresh Ed25519 pair per run. The hub holds the
    // pkcs8; the node installs only the raw public key hex.
    let key_pair =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .map_err(|_| anyhow::anyhow!("generate ed25519 key"))?;
    let signing_key = work.join("hub-signing.pkcs8");
    write_0600(&signing_key, key_pair.as_ref())?;
    let public_hex = hex::encode(key_pair_public(&key_pair));

    let node_token = format!("node-token-{}", hex::encode(random_bytes(16)));
    let client_token = format!("client-token-{}", hex::encode(random_bytes(16)));
    let node_token_file = write_0600(&work.join("node.token"), node_token.as_bytes())?;
    let hub_config = hub_config_path(&work, hub_port, &signing_key, &node_token, &client_token)?;
    let node_config =
        node_config_path(&work, direct_port, hub_port, &node_token_file, &public_hex)?;

    let mut hub = spawn_hub(&root, &hub_config, &work)?;
    let hub_logs = hub.log_path.clone();
    if wait_for_port(hub_port, "hub").is_err() {
        eprintln!(
            "hub log ({}): {}",
            hub_logs.display(),
            std::fs::read_to_string(&hub_logs).unwrap_or_default()
        );
        bail!("hub never listened");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        // N-code: the bootstrap code is single use, five minutes, printed once.
        let code = bootstrap_code(&root, &hub_config)?;
        let web = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .context("build http client")?;

        // Private login: exact Origin, one-time code in the body only.
        let session = web
            .post(format!("{hub_url}/private/session"))
            .header("Origin", WEB_ORIGIN)
            .json(&serde_json::json!({"code": code}))
            .send()
            .await
            .context("private session")?;
        assert_eq!(session.status(), 200, "private session must succeed");
        let session: serde_json::Value = session.json().await?;
        let csrf = session["csrf"].as_str().context("csrf missing")?.to_string();
        assert_eq!(session["role"], "ui");

        // Single use + wrong code + missing Origin.
        let replay = web
            .post(format!("{hub_url}/private/session"))
            .header("Origin", WEB_ORIGIN)
            .json(&serde_json::json!({"code": code}))
            .send()
            .await?;
        assert_eq!(replay.status(), 401, "bootstrap code is single use (N04)");
        let wrong = web
            .post(format!("{hub_url}/private/session"))
            .header("Origin", WEB_ORIGIN)
            .json(&serde_json::json!({"code": "not-the-code"}))
            .send()
            .await?;
        assert_eq!(wrong.status(), 401, "wrong code rejected (N02)");
        let no_origin = reqwest::Client::builder()
            .cookie_store(true)
            .build()?
            .post(format!("{hub_url}/private/session"))
            .json(&serde_json::json!({"code": code}))
            .send()
            .await?;
        assert_eq!(no_origin.status(), 401, "exact Origin required (N05)");

        // Node identity registration needs the node running; start it now.
        let mut node = spawn_node(&root, &node_config, &work)?;
        let snapshot = node_snapshot(&work).await?;

        // Grants: cookie path (browser) and bearer path (native), then the
        // anonymous negative.
        let scope_direct = |generation: &str| {
            serde_json::json!({
                "nodeId": snapshot.node_id,
                "scope": {
                    "spaceId": "private",
                    "providerEndpointId": format!("node-a-{}-counter", snapshot.app_id),
                    "contractId": "conex.counter",
                    "moduleGeneration": generation,
                    "methods": ["counter/read", "counter/add"],
                    "states": [],
                    "workspaceIds": [],
                    "conversationIds": [],
                    "purpose": "direct",
                }
            })
        };
        let granted = {
            let hub_url = hub_url.clone();
            move |web: &reqwest::Client, csrf: &str, body: serde_json::Value| {
            let web = web.clone();
            let hub_url = hub_url.clone();
            let csrf = csrf.to_string();
            async move {
                web.post(format!("{hub_url}/private/grant"))
                    .header("Origin", WEB_ORIGIN)
                    .header("x-csrf-token", csrf)
                    .json(&body)
                    .send()
                    .await
            }
            }
        };
        // Wait until the hub has accepted the node identity (publication link up).
        let direct_grant;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let response = granted(&web, &csrf, scope_direct(&snapshot.generation)).await?;
            let status = response.status();
            if status.is_success() {
                direct_grant = response.json::<serde_json::Value>().await?;
                break;
            }
            let failure = format!("{status}: {}", response.text().await.unwrap_or_default());
            if Instant::now() > deadline {
                eprintln!(
                    "node log: {}",
                    std::fs::read_to_string(work.join("node.log")).unwrap_or_default()
                );
                bail!("hub never accepted the node identity for grants: {failure}");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let direct_token = direct_grant["token"].as_str().context("grant token")?.to_string();
        assert_eq!(direct_grant["nodeEpoch"], snapshot.epoch, "grant carries the registered epoch");

        // Native bearer path issues its own grant without any cookie.
        let native = reqwest::Client::builder().build()?;
        let native_grant = native
            .post(format!("{hub_url}/private/grant"))
            .header("Origin", WEB_ORIGIN)
            .header("Authorization", format!("Bearer {client_token}"))
            .json(&scope_direct(&snapshot.generation))
            .send()
            .await?;
        assert_eq!(native_grant.status(), 200, "native bearer grant path (N16)");

        // Anonymous grant requests are refused (N01).
        let anonymous = reqwest::Client::builder()
            .build()?
            .post(format!("{hub_url}/private/grant"))
            .json(&scope_direct(&snapshot.generation))
            .send()
            .await?;
        assert_eq!(anonymous.status(), 401, "anonymous grant refused");

        // A non-administrator cannot revoke.
        let foreign_revoke = native
            .post(format!("{hub_url}/private/revoke"))
            .header("Authorization", format!("Bearer {client_token}"))
            .json(&serde_json::json!({"kind": "jti", "target": direct_grant["jti"]}))
            .send()
            .await?;
        assert_eq!(foreign_revoke.status(), 403, "revoke needs an administrator");

        // Relay-purpose grant (issued fine at the hub) must not open the
        // node's direct entry (N08).
        let mut relay_scope = scope_direct(&snapshot.generation);
        relay_scope["scope"]["purpose"] = "relay".into();
        let relay_grant = granted(&web, &csrf, relay_scope).await?;
        assert_eq!(relay_grant.status(), 200, "relay grant issues at the hub");
        let relay_grant: serde_json::Value = relay_grant.json().await?;
        let relay_token = relay_grant["token"].as_str().context("relay token")?.to_string();

        // Node direct entry: tickets need an exact approved Origin.
        let node_url = format!("http://127.0.0.1:{direct_port}");
        let ticket = web
            .post(format!("{node_url}/private/tickets"))
            .header("Origin", WEB_ORIGIN)
            .header("Authorization", format!("Bearer {direct_token}"))
            .send()
            .await
            .context("node ticket request")?;
        assert_eq!(ticket.status(), 200, "approved origin + direct grant issues a ticket");
        let ticket: serde_json::Value = ticket.json().await?;
        let ticket_value = ticket["ticket"].as_str().context("ticket")?.to_string();
        assert_eq!(ticket["profile"], PROFILE, "the real profile is returned");

        let wrong_origin = web
            .post(format!("{node_url}/private/tickets"))
            .header("Origin", "http://evil.test")
            .header("Authorization", format!("Bearer {direct_token}"))
            .send()
            .await?;
        assert_eq!(wrong_origin.status(), 403, "wrong Origin refused (N05)");
        let relay_on_direct = web
            .post(format!("{node_url}/private/tickets"))
            .header("Origin", WEB_ORIGIN)
            .header("Authorization", format!("Bearer {relay_token}"))
            .send()
            .await?;
        assert_eq!(relay_on_direct.status(), 403, "relay grant cannot ride direct (N08)");
        let anonymous_ticket = reqwest::Client::builder()
            .build()?
            .post(format!("{node_url}/private/tickets"))
            .header("Origin", WEB_ORIGIN)
            .send()
            .await?;
        assert_eq!(anonymous_ticket.status(), 401, "anonymous ticket refused (N01)");

        // Browser direct WSS: ticket subprotocol + real profile. The server
        // must select the profile and never echo the ticket.
        let reply = direct_invoke(
            direct_port,
            &snapshot,
            &[PROFILE.to_string(), format!("conex-ticket.{ticket_value}")],
            None,
        )
        .await?;
        assert!(
            reply["result"].is_object(),
            "first direct read works (N04), got: {reply}"
        );
        assert!(
            !reply.to_string().contains(&ticket_value),
            "the ticket value must never be echoed"
        );

        // Same ticket a second time: single use (N04).
        let replay = direct_invoke(
            direct_port,
            &snapshot,
            &[PROFILE.to_string(), format!("conex-ticket.{ticket_value}")],
            None,
        )
        .await;
        assert!(replay.is_err(), "ticket reuse fails");

        // Native path: bearer on the upgrade itself.
        let native_reply = direct_invoke(
            direct_port,
            &snapshot,
            &[PROFILE.to_string()],
            Some(&direct_token),
        )
        .await?;
        assert!(
            native_reply["result"].is_object(),
            "native bearer direct works, got: {native_reply}"
        );

        // Cross-scope method: counter/add is not in a read-only scope.
        let mut read_only_scope = scope_direct(&snapshot.generation);
        read_only_scope["scope"]["methods"] = serde_json::json!(["counter/read"]);
        let read_only = granted(&web, &csrf, read_only_scope).await?;
        assert_eq!(read_only.status(), 200);
        let read_only: serde_json::Value = read_only.json().await?;
        let read_only_token = read_only["token"].as_str().context("token")?.to_string();
        let read_only_reply = direct_invoke_method(
            direct_port,
            &snapshot,
            &[PROFILE.to_string()],
            Some(&read_only_token),
            "counter/add",
        )
        .await?;
        assert_eq!(
            read_only_reply["error"]["code"], -32002,
            "partial scope match is forbidden, not partially allowed (N07)"
        );

        // Revocation: the hub commits first, the node syncs by revision.
        let revoke = web
            .post(format!("{hub_url}/private/revoke"))
            .header("Origin", WEB_ORIGIN)
            .header("x-csrf-token", &csrf)
            .json(&serde_json::json!({"kind": "jti", "target": direct_grant["jti"]}))
            .send()
            .await?;
        assert_eq!(revoke.status(), 200, "administrator revokes (N11)");
        let revoke_body: serde_json::Value = revoke.json().await?;
        let revision: String = revoke_body["revision"]
            .as_str()
            .context("revision")?
            .to_string();
        assert_ne!(revision, "0", "the revision is durable before the receipt");

        let mut revoked_refused = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            // Refusal may surface at the upgrade (bind-time check, HTTP 403)
            // or at the call itself (per-operation check, JSON-RPC forbidden).
            match direct_invoke(
                direct_port,
                &snapshot,
                &[PROFILE.to_string()],
                Some(&direct_token),
            )
            .await
            {
                Ok(reply) if reply["error"]["code"] == -32002 => {
                    revoked_refused = true;
                    break;
                }
                Err(_) => {
                    revoked_refused = true;
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
        if !revoked_refused {
            eprintln!(
                "node log: {}",
                std::fs::read_to_string(work.join("node.log")).unwrap_or_default()
            );
        }
        assert!(revoked_refused, "revoked grant stops authorizing new calls (N11)");

        // Hub restart: the node re-registers with the same epoch; old grants
        // for that epoch stay valid until exp (N14). Hub-side sessions die
        // with the process (contract §3.1), so the browser re-authenticates
        // with a fresh one-time bootstrap code.
        hub.kill();
        hub = spawn_hub(&root, &hub_config, &work)?;
        wait_for_port(hub_port, "hub")?;
        let code2 = bootstrap_code(&root, &hub_config)?;
        let session2 = web
            .post(format!("{hub_url}/private/session"))
            .header("Origin", WEB_ORIGIN)
            .json(&serde_json::json!({"code": code2}))
            .send()
            .await
            .context("private session after restart")?;
        assert_eq!(session2.status(), 200, "re-login after hub restart");
        let session2: serde_json::Value = session2.json().await?;
        let csrf = session2["csrf"].as_str().context("csrf")?.to_string();
        let mut regrant = None;
        let mut regrant_failure = String::new();
        // The node's reconnect backoff reaches 30s; allow a full backoff
        // window plus restart time before giving up.
        let deadline = Instant::now() + Duration::from_secs(75);
        while Instant::now() < deadline {
            let response = granted(&web, &csrf, scope_direct(&snapshot.generation)).await?;
            let status = response.status();
            if status.is_success() {
                regrant = Some(response.json::<serde_json::Value>().await?);
                break;
            }
            regrant_failure = format!("{status}: {}", response.text().await.unwrap_or_default());
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let regrant = regrant.with_context(|| {
            format!(
                "node re-registered after hub restart (last grant failure: {regrant_failure}); node log: {}",
                std::fs::read_to_string(work.join("node.log")).unwrap_or_default()
            )
        })?;
        assert_eq!(regrant["nodeEpoch"], snapshot.epoch, "node restart did not happen: epoch is stable (N14)");
        let regrant_token = regrant["token"].as_str().context("token")?.to_string();
        let after_restart = direct_invoke(
            direct_port,
            &snapshot,
            &[PROFILE.to_string()],
            Some(&regrant_token),
        )
        .await?;
        assert!(
            after_restart["result"].is_object(),
            "fresh grant works after hub restart, got: {after_restart}"
        );

        // Node restart: old-epoch grants die at the node (N15).
        node.kill();
        node = spawn_node(&root, &node_config, &work)?;
        let snapshot2 = node_snapshot(&work).await?;
        assert_ne!(snapshot2.epoch, snapshot.epoch, "node restart mints a new epoch");
        let mut stale_refused = false;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            match direct_invoke(
                direct_port,
                &snapshot2,
                &[PROFILE.to_string()],
                Some(&regrant_token),
            )
            .await
            {
                Ok(reply) if reply["error"]["code"] == -32002 => {
                    stale_refused = true;
                    break;
                }
                Err(_) => {
                    stale_refused = true;
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
        assert!(stale_refused, "old-epoch grant refused after node restart (N15)");
        let fresh_grant = granted(&web, &csrf, scope_direct(&snapshot2.generation)).await?;
        assert_eq!(fresh_grant.status(), 200, "new epoch registers and grants");
        let fresh_grant: serde_json::Value = fresh_grant.json().await?;
        let fresh_token = fresh_grant["token"].as_str().context("token")?.to_string();
        let fresh_reply = direct_invoke(
            direct_port,
            &snapshot2,
            &[PROFILE.to_string()],
            Some(&fresh_token),
        )
        .await?;
        assert!(
            fresh_reply["result"].is_object(),
            "new-epoch grant invokes, got: {fresh_reply}"
        );

        // Local revocation persists across the node restart we just did:
        // revoke the fresh principal, restart again, expect refusal (N11/N15).
        let revoke2 = web
            .post(format!("{hub_url}/private/revoke"))
            .header("Origin", WEB_ORIGIN)
            .header("x-csrf-token", &csrf)
            .json(&serde_json::json!({"kind": "principal", "target": "owner"}))
            .send()
            .await?;
        assert_eq!(revoke2.status(), 200, "principal revocation commits");
        let mut principal_refused = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match direct_invoke(
                direct_port,
                &snapshot2,
                &[PROFILE.to_string()],
                Some(&fresh_token),
            )
            .await
            {
                Ok(reply) if reply["error"]["code"] == -32002 => {
                    principal_refused = true;
                    break;
                }
                Err(_) => {
                    principal_refused = true;
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(500)).await,
            }
        }
        assert!(principal_refused, "principal revocation propagates by revision (N12)");

        // Canary sweep: no bootstrap code, grant token or client token may sit
        // in the hub's store, logs or the node's captured output (contract §6).
        let secrets = [code.as_str(), direct_token.as_str(), client_token.as_str()];
        let mut scanned = 0;
        for entry in walkdir::WalkDir::new(&work).into_iter().filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            // The signing key file legitimately holds key material; grant
            // tokens live only in HTTP bodies, never in any file.
            if name == "hub-signing.pkcs8" {
                continue;
            }
            if let Ok(bytes) = std::fs::read(path)
                && let Ok(text) = String::from_utf8(bytes) {
                    for secret in secrets {
                        assert!(
                            !text.contains(secret),
                            "secret canary hit in {}",
                            path.display()
                        );
                    }
                    scanned += 1;
                }
        }
        assert!(scanned > 0, "canary sweep actually read files");

        node.kill();
        hub.kill();
        Ok(())
    });
    if let Err(error) = result {
        hub.kill();
        eprintln!("hub log ({}):", hub_logs.display());
        eprintln!("{}", std::fs::read_to_string(&hub_logs).unwrap_or_default());
        eprintln!(
            "node log: {}",
            std::fs::read_to_string(work.join("node.log")).unwrap_or_default()
        );
        return Err(error);
    }
    println!("private-network acceptance: all scenarios passed");
    Ok(())
}

struct Snapshot {
    node_id: String,
    epoch: String,
    app_id: String,
    instance_id: String,
    generation: String,
}

/// Read the node's runtime identity through the restricted LocalApi (same
/// UID over the 0600 unix socket): the caller-facing source of truth for
/// instance/module/generation the direct client needs.
async fn node_snapshot(work: &Path) -> Result<Snapshot> {
    let socket_path = work.join("node-data/operator.sock");
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut stream = loop {
        match UnixStream::connect(&socket_path).await {
            Ok(stream) => break stream,
            Err(_) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(error) => bail!("local api never came up: {error}"),
        }
    };
    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
        "method": "node/snapshot",
        "params": {
            "context": {"plane": "broker"},
            "input": {},
            "timeoutBudgetMs": 5000,
        }
    });
    let mut line = serde_json::to_vec(&request)?;
    line.push(b'\n');
    stream.write_all(&line).await?;
    let mut reader = tokio::io::BufReader::new(stream);
    let mut reply = String::new();
    reader.read_line(&mut reply).await?;
    let reply: serde_json::Value = serde_json::from_str(reply.trim())?;
    if let Some(error) = reply.get("error") {
        bail!("node snapshot failed: {error}");
    }
    let result = &reply["result"];
    let instance = &result["instances"][0];
    Ok(Snapshot {
        node_id: result["nodeId"].as_str().context("nodeId")?.to_string(),
        epoch: result["nodeRuntimeEpoch"]
            .as_str()
            .context("epoch")?
            .to_string(),
        app_id: instance["appId"].as_str().context("appId")?.to_string(),
        instance_id: instance["id"].as_str().context("instance")?.to_string(),
        generation: instance["modules"][0]["generation"]
            .as_str()
            .context("generation")?
            .to_string(),
    })
}

/// One direct-entry business call: upgrade WSS (ticket subprotocol or bearer),
/// send `node/invoke`, return the JSON-RPC reply. An upgrade failure returns
/// Err so callers can assert negatives.
async fn direct_invoke(
    port: u16,
    snapshot: &Snapshot,
    subprotocols: &[String],
    bearer: Option<&str>,
) -> Result<serde_json::Value> {
    direct_invoke_method(port, snapshot, subprotocols, bearer, "counter/read").await
}

async fn direct_invoke_method(
    port: u16,
    snapshot: &Snapshot,
    subprotocols: &[String],
    bearer: Option<&str>,
    method: &str,
) -> Result<serde_json::Value> {
    let url = format!("ws://127.0.0.1:{port}/wss");
    let mut request = url
        .into_client_request()
        .context("build direct ws request")?;
    request
        .headers_mut()
        .insert("Origin", WEB_ORIGIN.parse().context("parse origin header")?);
    if let Some(token) = bearer {
        request.headers_mut().insert(
            "Authorization",
            format!("Bearer {token}").parse().context("parse bearer")?,
        );
    }
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        subprotocols
            .join(", ")
            .parse()
            .context("parse subprotocols")?,
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request)
        .await
        .context("direct ws upgrade")?;
    let selected = response
        .headers()
        .get("sec-websocket-protocol")
        .and_then(|value| value.to_str().ok())
        .context("server must select a subprotocol")?
        .to_string();
    assert_eq!(
        selected, PROFILE,
        "the server selects the real profile only"
    );
    let invoke = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "01ARZ3NDEKTSV4RRFFQ69G5FAX",
        "method": "node/invoke",
        "params": {
            "context": {"plane": "broker"},
            "input": {
                "instanceId": snapshot.instance_id,
                "module": "counter",
                "generation": snapshot.generation,
                "method": method,
                "argument": {},
            },
            "timeoutBudgetMs": 5000,
        }
    });
    socket.send(Message::text(invoke.to_string())).await?;
    let reply = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .context("direct reply timeout")?
        .context("direct stream closed")?;
    let text = match reply? {
        Message::Text(text) => text.to_string(),
        other => bail!("unexpected direct frame: {other:?}"),
    };
    socket.close(None).await.ok();
    Ok(serde_json::from_str(&text)?)
}

fn build_binaries(root: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .args(["build", "-p", "conex-host", "-p", "conex-agent", "--locked"])
        .current_dir(root)
        .status()
        .context("build private-network binaries")?;
    if !status.success() {
        bail!("private-network build failed");
    }
    Ok(())
}

fn key_pair_public(pair: &ring::pkcs8::Document) -> Vec<u8> {
    let pair =
        ring::signature::Ed25519KeyPair::from_pkcs8(pair.as_ref()).expect("generated key parses");
    use ring::signature::KeyPair;
    pair.public_key().as_ref().to_vec()
}

fn hub_config_path(
    work: &Path,
    port: u16,
    signing_key: &Path,
    node_token: &str,
    client_token: &str,
) -> Result<PathBuf> {
    let config = work.join("hub.toml");
    let node_hash = sha256_hex(node_token);
    let client_hash = sha256_hex(client_token);
    let text = format!(
        r#"
listen = "127.0.0.1:{port}"
allow_loopback_http = true
audience = "conex-e2e"
web_origin = "{WEB_ORIGIN}"
content_root = "{content}"
session_root = "{session}"
operation_root = "{operation}"

[[tokens]]
token_hash = "{node_hash}"
principal_id = "node-a"
tenant_id = "private"
audience = "conex-e2e"
role = "service"

[[tokens]]
token_hash = "{client_hash}"
principal_id = "operator"
tenant_id = "private"
audience = "conex-e2e"
role = "service"

[[capabilities.contracts]]
contract_id = "conex.counter"
[[capabilities.contracts.methods]]
name = "counter/read"
input_schema = "conex.CounterReadInput"
output_schema = "conex.CounterResult"
action = "counter.read"
execution = "read_only"
[[capabilities.contracts.methods]]
name = "counter/add"
input_schema = "conex.CounterAddInput"
output_schema = "conex.CounterResult"
action = "counter.add"
execution = "non_replayable"

[private]
issuer = "conex://hub.e2e"
kid = "e2e-1"
signing_key_backend = "file:{signing_key}"
tenant = "e2e-tenant"
store_path = "{store}"
principal = "owner"
admin_principals = ["owner"]
node_principals = ["node-a"]
"#,
        content = work.join("hub/content").display(),
        session = work.join("hub/session").display(),
        operation = work.join("hub/operation").display(),
        store = work.join("hub/private.sqlite").display(),
        signing_key = signing_key.display(),
    );
    std::fs::write(&config, text)?;
    Ok(config)
}

fn node_config_path(
    work: &Path,
    direct_port: u16,
    hub_port: u16,
    node_token_file: &Path,
    public_hex: &str,
) -> Result<PathBuf> {
    // The application package is generated, not reused: `owner_node_ref`
    // must match the installing node.
    let application = work.join("application.toml");
    std::fs::write(
        &application,
        r#"
schema = "conex.application/1"
app_id = "local-counter"
version = "0.1.0"
owner_node_ref = "e2e-node"

[[modules]]
id = "counter"
implementation = "controlled-counter"
config_ref = "counter-default"
permissions = ["counter.read", "counter.add"]
exports = [{ contract_id = "conex.counter", strategy = "direct-only" }]
"#,
    )?;
    let config = work.join("node.toml");
    let text = format!(
        r#"
node_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV"
node_ref = "e2e-node"
operator_principal = "01ARZ3NDEKTSV4RRFFQ69G5FAW"
data_dir = "{data}"
local_socket = "{data}/operator.sock"
applications = ["{application}"]
permissions = ["counter.read", "counter.add"]

[configs.counter-default]

[publication]
url = "ws://127.0.0.1:{hub_port}/wss"
token_backend = "file:{token}"
space_id = "private"
integration_id = "local-apps"
endpoint_prefix = "node-a"
allow_loopback_ws = true
exposure_principals = []
exposure_tenants = []

[private]
hub_issuer = "conex://hub.e2e"
tenant = "e2e-tenant"
direct_listen = "127.0.0.1:{direct_port}"
approved_web_origins = ["{WEB_ORIGIN}"]

[private.approved_public_keys]
"e2e-1" = "{public_hex}"
"#,
        data = work.join("node-data").display(),
        application = application.display(),
        token = node_token_file.display(),
        hub_port = hub_port,
        direct_port = direct_port,
        public_hex = public_hex,
    );
    std::fs::write(&config, text)?;
    Ok(config)
}

struct Process {
    child: Child,
    log_path: PathBuf,
}

impl Process {
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.kill();
    }
}

fn spawn_process(root: &Path, log_name: &str, args: &[&str]) -> Result<Process> {
    let log_path = root.join(log_name);
    let log = std::fs::File::create(&log_path)?;
    let child = Command::new(args[0])
        .args(&args[1..])
        .current_dir(root)
        .env("RUST_LOG", "info")
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .with_context(|| format!("spawn {}", args[0]))?;
    Ok(Process { child, log_path })
}

fn spawn_hub(root: &Path, config: &Path, work: &Path) -> Result<Process> {
    spawn_process(
        work,
        "hub.log",
        &[
            root.join("target/debug/conex-host").to_str().unwrap(),
            config.to_str().unwrap(),
        ],
    )
}

fn spawn_node(root: &Path, config: &Path, work: &Path) -> Result<Process> {
    spawn_process(
        work,
        "node.log",
        &[
            root.join("target/debug/conex-agent").to_str().unwrap(),
            "--node-config",
            config.to_str().unwrap(),
        ],
    )
}

/// Runs the real `conex-host private-bootstrap` subcommand and takes the code
/// from its stdout — the only place the plaintext ever appears.
fn bootstrap_code(root: &Path, config: &Path) -> Result<String> {
    let output = Command::new(root.join("target/debug/conex-host"))
        .arg("private-bootstrap")
        .arg(format!("--config={}", config.display()))
        .output()
        .context("run private-bootstrap")?;
    if !output.status.success() {
        bail!(
            "private-bootstrap failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let code = String::from_utf8(output.stdout)?.trim().to_string();
    if code.len() != 43 {
        bail!("bootstrap code must be 256 bits of base64url");
    }
    Ok(code)
}

fn write_0600(path: &Path, bytes: &[u8]) -> Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let mut file = std::fs::File::create(path)?;
    file.write_all(bytes)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(path.to_path_buf())
}

fn random_bytes(n: usize) -> Vec<u8> {
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = vec![0u8; n];
    SystemRandom::new().fill(&mut bytes).expect("system random");
    bytes
}

fn free_port() -> Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn wait_for_port(port: u16, name: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    bail!("{name} never listened on {port}")
}

#[allow(non_snake_case)]
fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256 as Core};
    hex::encode(Core::digest(input.as_bytes()))
}

fn repo_root() -> Result<PathBuf> {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")
        .map(Path::to_path_buf)
}

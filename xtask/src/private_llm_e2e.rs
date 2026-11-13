//! MVP-P10 acceptance (`cargo xtask e2e --suite private-llm`): real hub +
//! real node + real relay path (S1 publish/consume approvals → CallRouter
//! hop → node llm module) + a fixture OpenAI-compatible upstream in this
//! process. The fixture stands in for omniroute's wire shape only — every
//! conex-side hop is the real binary. Gateway auth, model allowlist, stream
//! and offline semantics are exercised over real HTTP.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const WEB_ORIGIN: &str = "http://conex-e2e.test";

pub fn run() -> Result<()> {
    let root = repo_root()?;
    build_binaries(&root)?;

    #[allow(deprecated)]
    let work = match std::env::var("CONEX_E2E_KEEP") {
        Ok(dir) if !dir.is_empty() => {
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            PathBuf::from(dir)
        }
        _ => tempfile::tempdir()?.into_path(),
    };
    let hub_port = free_port()?;
    let upstream_port = free_port()?;
    let hub_url = format!("http://127.0.0.1:{hub_port}");

    let gateway_key = format!("gw-{}", hex::encode(random_bytes(16)));
    let gateway_key_restricted = format!("gw-{}", hex::encode(random_bytes(16)));
    let node_token = format!("node-token-{}", hex::encode(random_bytes(16)));

    let node_token_file = write_0600(&work.join("node.token"), node_token.as_bytes())?;
    let hub_config = hub_config_path(
        &work,
        hub_port,
        upstream_port,
        &node_token,
        &gateway_key,
        &gateway_key_restricted,
    )?;
    let node_config = node_config_path(&work, hub_port, &node_token_file, upstream_port)?;

    let mut hub = spawn_process(
        &work,
        "hub.log",
        &[
            root.join("target/debug/conex-host").to_str().unwrap(),
            hub_config.to_str().unwrap(),
        ],
    )?;
    if wait_for_port(hub_port, "hub").is_err() {
        eprintln!(
            "hub log: {}",
            std::fs::read_to_string(work.join("hub.log")).unwrap_or_default()
        );
        bail!("hub never listened");
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(async {
        // Fixture OpenAI 上游：wire 形状替身（omniroute/ollama 的公共面），
        // conex 侧每一跳都是真实二进制。
        let upstream = tokio::spawn(fixture_upstream(upstream_port));

        let mut node = spawn_process(
            &work,
            "node.log",
            &[
                root.join("target/debug/conex-agent").to_str().unwrap(),
                "--node-config",
                node_config.to_str().unwrap(),
            ],
        )?;

        let web = reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .context("build http client")?;
        let bearer = |key: &str| format!("Bearer {key}");

        // L04 中继可达：注册激活 + 发布/消费批准 + 转发路径全链。轮询等
        // 节点发布完成（注册表出现现行记录），失败打印两侧日志。
        let models;
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut last_failure;
        loop {
            let response = web
                .get(format!("{hub_url}/v1/models"))
                .header("Authorization", bearer(&gateway_key))
                .send()
                .await?;
            let status = response.status();
            if status == 200 {
                models = response.json::<Value>().await?;
                break;
            }
            last_failure = format!("{status}: {}", response.text().await.unwrap_or_default());
            if Instant::now() > deadline {
                eprintln!(
                    "hub log: {}\nnode log: {}",
                    std::fs::read_to_string(work.join("hub.log")).unwrap_or_default(),
                    std::fs::read_to_string(work.join("node.log")).unwrap_or_default()
                );
                bail!("gateway never reached the node registration (last {last_failure})");
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        let ids: Vec<&str> = models["data"]
            .as_array()
            .context("data array")?
            .iter()
            .filter_map(|entry| entry["id"].as_str())
            .collect();
        assert!(
            ids.contains(&"fixture-mini"),
            "published model must be listed, got {ids:?}"
        );
        println!("llm L04 relay /v1/models ok models={ids:?}");

        // L02 key 负路径：缺失/未知凭据 401；受限 key 的模型越权 403。
        for (label, request) in [
            ("missing key", web.get(format!("{hub_url}/v1/models"))),
            (
                "unknown key",
                web.get(format!("{hub_url}/v1/models"))
                    .header("Authorization", "Bearer not-a-key"),
            ),
        ] {
            let response = request.send().await?;
            assert_eq!(response.status(), 401, "{label} must be refused");
        }
        let restricted_models = web
            .get(format!("{hub_url}/v1/models"))
            .header("Authorization", bearer(&gateway_key_restricted))
            .send()
            .await?;
        assert_eq!(restricted_models.status(), 200);
        let restricted_models: Value = restricted_models.json().await?;
        assert!(
            restricted_models["data"]
                .as_array()
                .map(Vec::is_empty)
                .unwrap_or(false),
            "restricted key sees no allowed models: {restricted_models}"
        );

        // L01 非 SSE 补全：完整 OpenAI 形状回传，usage 透传。
        let chat = web
            .post(format!("{hub_url}/v1/chat/completions"))
            .header("Authorization", bearer(&gateway_key))
            .json(&json!({
                "model": "fixture-mini",
                "messages": [{"role": "user", "content": "ping"}],
            }))
            .send()
            .await?;
        let chat_status = chat.status();
        let chat_text = chat.text().await?;
        assert_eq!(chat_status, 200, "chat must relay: {chat_text}");
        let chat: Value = serde_json::from_str(&chat_text)?;
        assert_eq!(chat["object"], json!("chat.completion"));
        assert_eq!(
            chat["choices"][0]["message"]["content"],
            json!("hello from fixture"),
            "content must come from the upstream"
        );
        assert_eq!(chat["choices"][0]["finish_reason"], json!("stop"));
        assert_eq!(chat["usage"]["total_tokens"], json!(8), "usage relayed");
        println!(
            "llm L01 stream=false ok content={:?}",
            chat["choices"][0]["message"]["content"]
        );

        // L01 SSE：整段单帧 + 终帧 + [DONE]（首版诚实形态，不伪装增量）。
        let sse = web
            .post(format!("{hub_url}/v1/chat/completions"))
            .header("Authorization", bearer(&gateway_key))
            .json(&json!({
                "model": "fixture-mini",
                "messages": [{"role": "user", "content": "ping"}],
                "stream": true,
            }))
            .send()
            .await?;
        assert_eq!(sse.status(), 200);
        assert!(
            sse.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream"),
            "SSE content type"
        );
        let sse_text = sse.text().await?;
        assert!(
            sse_text.contains("chat.completion.chunk"),
            "chunk frames: {sse_text}"
        );
        assert!(sse_text.contains("hello from fixture"), "content in SSE");
        assert!(sse_text.contains("[DONE]"), "SSE terminator");
        let data_events = sse_text
            .lines()
            .filter(|line| line.starts_with("data: "))
            .count();
        assert_eq!(data_events, 3, "single-chunk streaming is 3 data events");

        // 未知模型：节点白名单拒绝（400，明确错误，不静默成功）。
        let unknown = web
            .post(format!("{hub_url}/v1/chat/completions"))
            .header("Authorization", bearer(&gateway_key))
            .json(&json!({
                "model": "not-published",
                "messages": [{"role": "user", "content": "ping"}],
            }))
            .send()
            .await?;
        assert_eq!(unknown.status(), 400, "unknown model refused");
        let unknown: Value = unknown.json().await?;
        assert!(
            unknown["error"]["message"]
                .as_str()
                .unwrap_or_default()
                .contains("not published"),
            "node allowlist message surfaces: {unknown}"
        );

        // 受限 key 对未授权模型的请求 → 403（网关侧先拒，不触达节点）。
        let forbidden = web
            .post(format!("{hub_url}/v1/chat/completions"))
            .header("Authorization", bearer(&gateway_key_restricted))
            .json(&json!({
                "model": "fixture-mini",
                "messages": [{"role": "user", "content": "ping"}],
            }))
            .send()
            .await?;
        assert_eq!(forbidden.status(), 403, "key model scope enforced");

        // L03 节点离线：注册消失后网关明确 503，不静默空响应。
        let _ = node.child.kill();
        node.child.wait()?;
        let mut offline = false;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            let response = web
                .get(format!("{hub_url}/v1/models"))
                .header("Authorization", bearer(&gateway_key))
                .send()
                .await?;
            if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
                offline = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
        assert!(offline, "offline node must surface 503, not empty success");

        // canary：网关 key 明文不出现在任何 hub/node 输出（配置只存哈希）。
        for log in ["hub.log", "node.log"] {
            let text = std::fs::read_to_string(work.join(log)).unwrap_or_default();
            assert!(!text.contains(&gateway_key), "gateway key canary in {log}");
            assert!(
                !text.contains(&gateway_key_restricted),
                "restricted key canary in {log}"
            );
        }

        upstream.abort();
        node.kill();
        Ok(())
    });
    if let Err(error) = result {
        hub.kill();
        eprintln!(
            "hub log: {}\nnode log: {}",
            std::fs::read_to_string(work.join("hub.log")).unwrap_or_default(),
            std::fs::read_to_string(work.join("node.log")).unwrap_or_default()
        );
        return Err(error);
    }
    hub.kill();
    println!("private-llm acceptance: all scenarios passed");
    Ok(())
}

/// 最小 OpenAI 兼容上游：GET /v1/models 与 POST /v1/chat/completions，
/// Connection: close 逐请求应答。只服务本套件，不进任何生产路径。
async fn fixture_upstream(port: u16) {
    let listener = match TcpListener::bind(("127.0.0.1", port)).await {
        Ok(listener) => listener,
        Err(_) => return,
    };
    loop {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        tokio::spawn(async move {
            let mut head = String::new();
            let mut reader = tokio::io::BufReader::new(&mut socket);
            // 请求头：读到空行；解析 content-length 后读请求体。
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => return,
                    Ok(_) => {
                        if line == "\r\n" || line == "\n" {
                            break;
                        }
                        head.push_str(&line);
                    }
                }
            }
            let content_length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            let mut body = vec![0u8; content_length];
            if content_length > 0 {
                use tokio::io::AsyncReadExt;
                if reader.read_exact(&mut body).await.is_err() {
                    return;
                }
            }
            let is_get_models = head.starts_with("GET /v1/models");
            let is_chat = head.starts_with("POST /v1/chat/completions");
            let response: (u16, Value) = if is_get_models {
                (
                    200,
                    json!({
                        "object": "list",
                        "data": [
                            {"id": "fixture-mini", "owned_by": "e2e"},
                            {"id": "fixture-large", "owned_by": "e2e"}
                        ]
                    }),
                )
            } else if is_chat {
                let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let model = request["model"].as_str().unwrap_or("fixture-mini");
                if model == "not-published" {
                    // 上游侧的明确拒绝（节点也会拦，这里保持真实 4xx 形状）。
                    (
                        400,
                        json!({"error": {"message": "model not found", "type": "invalid_request_error"}}),
                    )
                } else {
                    (
                        200,
                        json!({
                            "id": "fixture-1",
                            "object": "chat.completion",
                            "model": model,
                            "choices": [{
                                "index": 0,
                                "message": {"role": "assistant", "content": "hello from fixture"},
                                "finish_reason": "stop"
                            }],
                            "usage": {"prompt_tokens": 3, "completion_tokens": 5, "total_tokens": 8}
                        }),
                    )
                }
            } else {
                (404, json!({"error": {"message": "not found"}}))
            };
            let (status, payload) = response;
            let reason = if status == 200 { "OK" } else { "Bad Request" };
            let bytes = payload.to_string();
            let _ = socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{bytes}",
                        bytes.len()
                    )
                    .as_bytes(),
                )
                .await;
        });
    }
}

fn build_binaries(root: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .args(["build", "-p", "conex-host", "-p", "conex-agent", "--locked"])
        .current_dir(root)
        .status()
        .context("build private-llm binaries")?;
    if !status.success() {
        bail!("private-llm build failed");
    }
    Ok(())
}

fn hub_config_path(
    work: &Path,
    port: u16,
    upstream_port: u16,
    node_token: &str,
    gateway_key: &str,
    gateway_key_restricted: &str,
) -> Result<PathBuf> {
    let config = work.join("hub.toml");
    let text = format!(
        r#"
listen = "127.0.0.1:{port}"
allow_loopback_http = true
audience = "conex-e2e"
web_origin = "{WEB_ORIGIN}"
content_root = "{content}"
session_root = "{session}"
operation_root = "{operation}"
wss_link_timeout_ms = 180000

[[tokens]]
token_hash = "{node_hash}"
principal_id = "node-a"
tenant_id = "private"
audience = "conex-e2e"
role = "service"

[[capabilities.contracts]]
contract_id = "conex.llm"
[[capabilities.contracts.methods]]
name = "llm/providers/list"
input_schema = "conex.LlmProvidersListInput"
output_schema = "conex.LlmProvidersListResult"
action = "llm.providers.list"
execution = "read_only"
[[capabilities.contracts.methods]]
name = "llm/chat"
input_schema = "conex.LlmChatInput"
output_schema = "conex.LlmChatResult"
action = "llm.chat"
execution = "non_replayable"

[[capabilities.publish]]
# node_id 是发布链路认证 principal（registry 按链路主体记账），不是节点 ULID。
node_id = "node-a"
space_id = "private"
integration_id = "local-apps"
provider_endpoint_id = "node-a-llmsvc-llm"
contract_id = "conex.llm"

[[capabilities.consume]]
principal_id = "owner"
space_id = "private"
provider_endpoint_id = "node-a-llmsvc-llm"
contract_id = "conex.llm"
methods = ["llm/providers/list", "llm/chat"]


[llm_gateway]
space_id = "private"
provider_endpoint_id = "node-a-llmsvc-llm"
consumer_principal = "owner"
consumer_tenant = "private"
timeout_ms = 30000
keys = [
    {{ name = "primary", token_hash = "{primary_hash}" }},
    {{ name = "restricted", token_hash = "{restricted_hash}", models = ["other-model"] }},
]

# fixture upstream base_url reference: http://127.0.0.1:{upstream_port}/v1
"#,
        content = work.join("hub/content").display(),
        session = work.join("hub/session").display(),
        operation = work.join("hub/operation").display(),
        node_hash = sha256_hex(node_token),
        primary_hash = sha256_hex(gateway_key),
        restricted_hash = sha256_hex(gateway_key_restricted),
        upstream_port = upstream_port,
    );
    std::fs::write(&config, text)?;
    Ok(config)
}

fn node_config_path(
    work: &Path,
    hub_port: u16,
    node_token_file: &Path,
    upstream_port: u16,
) -> Result<PathBuf> {
    let application = work.join("application.toml");
    std::fs::write(
        &application,
        r#"
schema = "conex.application/1"
app_id = "llmsvc"
version = "0.1.0"
owner_node_ref = "e2e-node"

[[modules]]
id = "llm"
implementation = "node-llm"
permissions = ["llm.providers.list", "llm.chat"]
exports = [{ contract_id = "conex.llm", strategy = "relay-allowed" }]
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
permissions = ["llm.providers.list", "llm.chat"]

[llm]
[[llm.providers]]
id = "fixture"
base_url = "http://127.0.0.1:{upstream_port}/v1"
models_allowlist = ["fixture-mini"]

[publication]
url = "ws://127.0.0.1:{hub_port}/wss"
token_backend = "file:{token}"
space_id = "private"
integration_id = "local-apps"
endpoint_prefix = "node-a"
allow_loopback_ws = true
exposure_principals = ["owner"]
exposure_tenants = ["private"]
link_timeout_ms = 180000
"#,
        data = work.join("node-data").display(),
        application = application.display(),
        token = node_token_file.display(),
        hub_port = hub_port,
        upstream_port = upstream_port,
    );
    std::fs::write(&config, text)?;
    Ok(config)
}

struct Process {
    child: Child,
    #[allow(dead_code)] // 与其他套件同形；日志路径留给失败时人工排查
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

fn spawn_process(work: &Path, log_name: &str, args: &[&str]) -> Result<Process> {
    let log_path = work.join(log_name);
    let log = std::fs::File::create(&log_path)?;
    let child = Command::new(args[0])
        .args(&args[1..])
        .current_dir(work)
        .env("RUST_LOG", "info")
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .with_context(|| format!("spawn {}", args[0]))?;
    Ok(Process { child, log_path })
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

//! MVP-P3 live acceptance (`cargo xtask e2e --suite agent-omp-live`).
//!
//! Real OMP binary with the machine's existing model authorization
//! (CONEX_OMP_MODEL, default bs/DeepSeek-V4-flash per the P0 probe page).
//! Isolated temp workspace/session dirs; nothing outside them is touched.
//! Not part of `cargo xtask check`: it needs a live model, which no fixture
//! may replace (P3 plan §5).
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static REQUEST_SEQ: AtomicU64 = AtomicU64::new(1);

const NODE_ULID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAX";
const OPERATOR_ULID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAY";
/// 真实模型的耐心预算；live 场景不缩短到 fixture 节奏。
const MODEL_TIMEOUT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(500);

pub fn run() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?
        .to_path_buf();
    let status = Command::new("cargo")
        .args(["build", "-p", "conex-agent", "--locked"])
        .current_dir(&root)
        .status()
        .context("build conex-agent")?;
    if !status.success() {
        bail!("conex-agent build failed");
    }
    let node_bin = root.join("target/debug/conex-agent");
    let work = tempfile::Builder::new()
        .prefix("conex-omp-live-")
        .tempdir_in(std::env::temp_dir())?;
    let root = work.path().to_path_buf();
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace)?;
    std::fs::write(workspace.join("notes.txt"), "conex omp live acceptance\n")?;
    let data_dir = root.join("data");
    std::fs::create_dir_all(&data_dir)?;
    // 节点要求运行目录为 0700 私有目录（P2 不变量）。
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))?;
    let socket = data_dir.join("node.sock");

    let app = root.join("omp-agent.toml");
    std::fs::write(
        &app,
        r#"schema = "conex.application/1"
app_id = "omp-agent"
version = "0.1.0"
owner_node_ref = "e2e-omp-node"

[[modules]]
id = "agent"
implementation = "omp"
permissions = ["agent.read", "agent.create", "agent.execute", "agent.cancel", "agent.interact"]
"#,
    )?;
    let model =
        std::env::var("CONEX_OMP_MODEL").unwrap_or_else(|_| "bs/DeepSeek-V4-flash".to_string());
    let config = format!(
        r#"node_id = "{NODE_ULID}"
node_ref = "e2e-omp-node"
operator_principal = "{OPERATOR_ULID}"
data_dir = "{data_dir}"
local_socket = "{socket}"
applications = ["{app}"]
permissions = ["agent.read", "agent.create", "agent.execute", "agent.cancel", "agent.interact"]

[publication]
url = "ws://127.0.0.1:1/wss"
allow_loopback_ws = true
token_backend = "env:CONEX_NODE_TOKEN"
space_id = "private"
integration_id = "local"
endpoint_prefix = "e2e-omp"
exposure_principals = []
exposure_tenants = []

[omp]
backend_binary = "omp"
model = "{model}"
tools = "ask"
approval_mode = "always-ask"
max_concurrent_sessions = 4

[[omp.workspaces]]
id = "main"
display_name = "Live acceptance"
root = "{workspace}""#,
        data_dir = data_dir.display(),
        socket = socket.display(),
        app = app.display(),
        model = model,
        workspace = workspace.display(),
    );
    let config_path = root.join("node.toml");
    std::fs::write(&config_path, config)?;

    let node_log = root.join("node.log");
    let outcome = run_scenarios(&node_bin, &config_path, &socket, &node_log, &model);
    if let Err(error) = &outcome {
        eprintln!("omp-live failed: {error}");
        if let Ok(log) = std::fs::read_to_string(&node_log) {
            let tail: Vec<&str> = log.lines().rev().take(40).collect();
            eprintln!(
                "omp-live node log tail:\n{}",
                tail.iter()
                    .rev()
                    .fold(String::new(), |acc, line| format!("{acc}{line}\n"))
            );
        }
        // 保留现场供人工检查（domain.sqlite、node.log、omp session 目录）。
        let kept = work.keep();
        eprintln!("omp-live artifacts kept at {}", kept.display());
    }
    // 清理自己创建的 OMP 会话进程（按唯一 session 目录匹配，不误伤其他）。
    let _ = Command::new("pkill")
        .args(["-f", &format!("omp-sessions/{}", root.display())])
        .status();
    outcome
}

fn spawn_node(node_bin: &Path, config_path: &Path, node_log: &Path) -> Result<Child> {
    let log = std::fs::File::create(node_log).context("create node log")?;
    Command::new(node_bin)
        .arg("--node-config")
        .arg(config_path)
        .env(
            "CONEX_NODE_TOKEN",
            "e2e-publication-token-not-a-real-secret",
        )
        .stdout(Stdio::from(log.try_clone().context("clone log")?))
        .stderr(Stdio::from(log))
        .spawn()
        .context("spawn conex-agent")
}

fn run_scenarios(
    node_bin: &Path,
    config_path: &Path,
    socket: &Path,
    node_log: &Path,
    model: &str,
) -> Result<()> {
    let mut child = spawn_node(node_bin, config_path, node_log)?;
    let scope = ScopeGuard { child: &mut child };

    let snapshot = wait_active(socket)?;
    let instance = snapshot["instances"][0]["id"]
        .as_str()
        .context("instance id")?
        .to_string();
    let generation = snapshot["instances"][0]["modules"][0]["generation"]
        .as_str()
        .context("module generation")?
        .to_string();

    let endpoints = agent(
        socket,
        &instance,
        &generation,
        "agent/endpoints/list",
        json!({}),
    )?;
    let endpoint_id = endpoints["items"][0]["id"]
        .as_str()
        .context("endpoint id")?
        .to_string();
    let workspaces = agent(
        socket,
        &instance,
        &generation,
        "agent/workspaces/list",
        json!({ "endpointId": endpoint_id }),
    )?;
    let workspace_id = workspaces["items"][0]["id"]
        .as_str()
        .context("workspace id")?
        .to_string();
    println!("omp-live: endpoint ready model={model}");

    // S1 create + 幂等：同键返回同一会话。
    let create_args = |key: &str| {
        json!({
            "endpointId": endpoint_id,
            "workspaceId": workspace_id,
            "idempotencyKey": key,
            "title": "live acceptance"
        })
    };
    let created = agent(
        socket,
        &instance,
        &generation,
        "agent/conversations/create",
        create_args("live-create-1"),
    )?;
    let conversation = created["conversation"]["id"]
        .as_str()
        .context("conversation id")?
        .to_string();
    let replay = agent(
        socket,
        &instance,
        &generation,
        "agent/conversations/create",
        create_args("live-create-1"),
    )?;
    assert_eq!(
        replay["conversation"]["id"].as_str().context("replay id")?,
        conversation,
        "idempotent create returns the same conversation"
    );
    println!("omp-live S1 idempotent-create ok conversation={conversation}");

    // S2 真模型流式完成：submit → 终态 succeeded → 助手消息非空。
    let execution = submit(
        socket,
        &instance,
        &generation,
        &conversation,
        "live-submit-1",
        "Reply with exactly the word READY and nothing else.",
    )?;
    let state = wait_terminal(socket, &instance, &generation, &execution, MODEL_TIMEOUT)?;
    assert_eq!(
        state, "succeeded",
        "real model prompt should settle succeeded"
    );
    let assistant_text = assistant_history(socket, &instance, &generation, &conversation)?;
    assert!(
        assistant_text.to_uppercase().contains("READY"),
        "assistant reply should carry the model answer, got: {assistant_text:?}"
    );
    println!(
        "omp-live S2 real-model streaming ok state={state} reply_len={}",
        assistant_text.len()
    );

    // S3 取消：running 中 cancel → 终态 cancelled；abort 回执不冒充终态。
    let cancel_execution = submit(
        socket,
        &instance,
        &generation,
        &conversation,
        "live-submit-2",
        "Count from 1 to 300 slowly, writing every number on its own line.",
    )?;
    wait_state(
        socket,
        &instance,
        &generation,
        &cancel_execution,
        &["running"],
        Duration::from_secs(90),
    )?;
    let cancelled = agent(
        socket,
        &instance,
        &generation,
        "agent/executions/cancel",
        json!({ "executionId": cancel_execution, "expectedGeneration": generation }),
    )?;
    assert_eq!(cancelled["requested"], json!(true), "cancel requested");
    let cancel_state = wait_terminal(
        socket,
        &instance,
        &generation,
        &cancel_execution,
        MODEL_TIMEOUT,
    )?;
    assert_eq!(
        cancel_state, "cancelled",
        "aborted prompt with settled session must end cancelled, got {cancel_state}"
    );
    println!("omp-live S3 cancel terminal ok state={cancel_state}");

    // S4 人工交互：ask 工具触发 select；拒绝（Stop）后执行自然完成。
    let interact_execution = submit(
        socket,
        &instance,
        &generation,
        &conversation,
        "live-submit-3",
        "Use the ask tool to ask: continue or stop? Offer exactly two options: Continue and Stop. After the user answers Stop, reply with the single word STOPPED and nothing else.",
    )?;
    let interaction = wait_interaction(
        socket,
        &instance,
        &generation,
        &conversation,
        &interact_execution,
    )?;
    let interaction_id = interaction["id"]
        .as_str()
        .context("interaction id")?
        .to_string();
    let revision = match &interaction["revision"] {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let choice_ids = interaction["choices"]
        .as_array()
        .map(|choices| {
            choices
                .iter()
                .filter_map(|choice| choice["id"].as_str())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .context("interaction choices")?;
    let deny = choice_ids
        .iter()
        .find(|choice| choice.to_lowercase().contains("stop"))
        .context("backend must offer a Stop-like option")?
        .clone();
    let answered = agent(
        socket,
        &instance,
        &generation,
        "agent/interactions/respond",
        json!({
            "interactionId": interaction_id,
            "expectedRevision": revision,
            "decision": { "kind": "selection", "choiceIds": [deny] }
        }),
    )?;
    assert_eq!(
        answered["interaction"]["state"].as_str().unwrap_or(""),
        "resolved",
        "deny resolves the interaction"
    );
    let interact_state = wait_terminal(
        socket,
        &instance,
        &generation,
        &interact_execution,
        MODEL_TIMEOUT,
    )?;
    assert_eq!(
        interact_state, "succeeded",
        "denied ask still lets the turn complete"
    );
    println!("omp-live S4 interaction deny ok choice={deny}");

    // S5 并发隔离：第二个会话独立进程，不抢占第一会话。
    let second_conversation = agent(
        socket,
        &instance,
        &generation,
        "agent/conversations/create",
        json!({
            "endpointId": endpoint_id,
            "workspaceId": workspace_id,
            "idempotencyKey": "live-create-2"
        }),
    )?["conversation"]["id"]
        .as_str()
        .context("second conversation")?
        .to_string();
    let second_execution = submit(
        socket,
        &instance,
        &generation,
        &second_conversation,
        "live-submit-4",
        "Reply with exactly the word SECOND and nothing else.",
    )?;
    let second_state = wait_terminal(
        socket,
        &instance,
        &generation,
        &second_execution,
        MODEL_TIMEOUT,
    )?;
    assert_eq!(
        second_state, "succeeded",
        "second conversation completes independently"
    );
    println!("omp-live S5 concurrent conversation ok");

    // S7 子代理（真实 API 关联）：模型可能不派生；两种结果都如实输出，
    // 不把"模型没派生"粉饰成映射已验证。
    let sub_execution = submit(
        socket,
        &instance,
        &generation,
        &second_conversation,
        "live-submit-6",
        "Spawn the scout subagent to list the files in the current directory, then tell me what it found.",
    )?;
    wait_terminal(
        socket,
        &instance,
        &generation,
        &sub_execution,
        MODEL_TIMEOUT,
    )?;
    let subagents = agent(
        socket,
        &instance,
        &generation,
        "agent/subagents/list",
        json!({ "executionId": sub_execution, "limit": 10 }),
    )?;
    let found = subagents["items"].as_array().map(Vec::len).unwrap_or(0);
    println!(
        "omp-live S7 subagent outcome items={found} quality={} (items>0 才证明映射链路 live 走通)",
        subagents["quality"].as_str().unwrap_or("")
    );

    // S6 重启恢复：node 真实重启后同会话继续，历史保留，backend mapping 复用。
    drop(scope);
    let _ = child.kill();
    let _ = child.wait();
    let mut child = spawn_node(node_bin, config_path, node_log)?;
    let snapshot = wait_active(socket)?;
    let instance = snapshot["instances"][0]["id"]
        .as_str()
        .context("instance id after restart")?
        .to_string();
    let generation = snapshot["instances"][0]["modules"][0]["generation"]
        .as_str()
        .context("module generation after restart")?
        .to_string();
    let resumed_execution = submit(
        socket,
        &instance,
        &generation,
        &conversation,
        "live-submit-5",
        "Reply with exactly the word RESUMED and nothing else.",
    )?;
    let resumed_state = wait_terminal(
        socket,
        &instance,
        &generation,
        &resumed_execution,
        MODEL_TIMEOUT,
    )?;
    assert_eq!(
        resumed_state, "succeeded",
        "resume after node restart works"
    );
    let message_count = history_count(socket, &instance, &generation, &conversation)?;
    assert!(
        message_count >= 6,
        "history survives restart, got {message_count} messages"
    );
    println!("omp-live S6 restart resume ok messages={message_count}");
    let _ = child.kill();
    let _ = child.wait();
    println!("omp-live: all live scenarios finished (model={model})");
    Ok(())
}

struct ScopeGuard<'a> {
    child: &'a mut Child,
}

impl Drop for ScopeGuard<'_> {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_active(socket: &Path) -> Result<Value> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if let Ok(snapshot) = call(socket, "node/snapshot", json!({})) {
            let active = snapshot["instances"]
                .as_array()
                .map(|items| {
                    items.iter().any(|instance| {
                        instance["appId"] == "omp-agent" && instance["state"] == "active"
                    })
                })
                .unwrap_or(false);
            if active {
                return Ok(snapshot);
            }
        }
        if Instant::now() > deadline {
            if let Ok(snapshot) = call(socket, "node/snapshot", json!({})) {
                eprintln!("omp-live: last snapshot: {snapshot}");
            }
            bail!("node never reached active state with the omp module");
        }
        std::thread::sleep(POLL);
    }
}

fn submit(
    socket: &Path,
    instance: &str,
    generation: &str,
    conversation: &str,
    key: &str,
    content: &str,
) -> Result<String> {
    let submitted = agent(
        socket,
        instance,
        generation,
        "agent/executions/submit",
        json!({
            "conversationId": conversation,
            "idempotencyKey": key,
            "content": content
        }),
    )?;
    Ok(submitted["execution"]["id"]
        .as_str()
        .context("execution id")?
        .to_string())
}

fn assistant_history(
    socket: &Path,
    instance: &str,
    generation: &str,
    conversation: &str,
) -> Result<String> {
    let history = agent(
        socket,
        instance,
        generation,
        "agent/conversations/history",
        json!({ "conversationId": conversation, "limit": 10 }),
    )?;
    Ok(history["messages"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .filter(|message| message["role"] == "assistant")
                .map(|message| message["parts"][0]["text"]["text"].as_str().unwrap_or(""))
                .collect::<String>()
        })
        .unwrap_or_default())
}

fn history_count(
    socket: &Path,
    instance: &str,
    generation: &str,
    conversation: &str,
) -> Result<usize> {
    let history = agent(
        socket,
        instance,
        generation,
        "agent/conversations/history",
        json!({ "conversationId": conversation, "limit": 100 }),
    )?;
    Ok(history["messages"].as_array().map(Vec::len).unwrap_or(0))
}

fn agent(
    socket: &Path,
    instance: &str,
    generation: &str,
    method: &str,
    argument: Value,
) -> Result<Value> {
    call(
        socket,
        "node/invoke",
        json!({
            "instanceId": instance,
            "module": "agent",
            "generation": generation,
            "method": method,
            "argument": argument
        }),
    )
}

fn wait_terminal(
    socket: &Path,
    instance: &str,
    generation: &str,
    execution_id: &str,
    timeout: Duration,
) -> Result<String> {
    wait_state(
        socket,
        instance,
        generation,
        execution_id,
        &["succeeded", "failed", "cancelled"],
        timeout,
    )
}

fn wait_state(
    socket: &Path,
    instance: &str,
    generation: &str,
    execution_id: &str,
    states: &[&str],
    timeout: Duration,
) -> Result<String> {
    let deadline = Instant::now() + timeout;
    loop {
        let execution = agent(
            socket,
            instance,
            generation,
            "agent/executions/get",
            json!({ "executionId": execution_id }),
        )?;
        let state = execution["execution"]["state"]
            .as_str()
            .context("execution state")?
            .to_string();
        if states.contains(&state.as_str()) {
            return Ok(state);
        }
        if Instant::now() > deadline {
            bail!("execution {execution_id} never reached {states:?}; last={state}");
        }
        std::thread::sleep(POLL);
    }
}

fn wait_interaction(
    socket: &Path,
    instance: &str,
    generation: &str,
    conversation: &str,
    execution_id: &str,
) -> Result<Value> {
    let deadline = Instant::now() + MODEL_TIMEOUT;
    loop {
        let interactions = agent(
            socket,
            instance,
            generation,
            "agent/interactions/list",
            json!({ "conversationId": conversation, "executionId": execution_id, "limit": 10 }),
        )?;
        if let Some(pending) = interactions["items"]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["state"] == "pending"))
        {
            return Ok(pending.clone());
        }
        if Instant::now() > deadline {
            bail!("no pending interaction appeared for {execution_id}");
        }
        std::thread::sleep(POLL);
    }
}

fn next_request_id() -> String {
    // 26 字符、Crockford 子集、首字符 <= '7' 的合成 ULID 形状。
    format!("01{:024X}", REQUEST_SEQ.fetch_add(1, Ordering::SeqCst))
}

/// 本地 0600 socket 的 node/invoke 客户端：一问一答 newline JSON。
fn call(socket: &Path, method: &str, input: Value) -> Result<Value> {
    let mut stream =
        UnixStream::connect(socket).with_context(|| format!("connect {}", socket.display()))?;
    stream.set_read_timeout(Some(Duration::from_secs(190)))?;
    let envelope = json!({
        "jsonrpc": "2.0",
        "id": next_request_id(),
        "method": method,
        "params": { "context": { "plane": "broker" }, "input": input }
    });
    stream.write_all(envelope.to_string().as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let reply: Value = serde_json::from_str(line.trim()).context("local reply decode")?;
    if let Some(error) = reply.get("error") {
        bail!(
            "local call {method} failed: {} {}",
            error["code"].as_i64().unwrap_or(0),
            error["message"].as_str().unwrap_or("")
        );
    }
    Ok(reply.get("result").cloned().unwrap_or(Value::Null))
}

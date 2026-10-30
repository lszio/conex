//! 真实节点存储 smoke（计划 §7）：临时数据库 → 提交/分派/流式 → 关闭 →
//! 重开 → 恢复核对 → 快照与事件页 → NDJSON 供独立 TS 消费者演练 reducer。
//!
//! `--self-check` 额外启动 `bun sdk/typescript/examples/agent-recovery-smoke.ts`
//! 读取该 NDJSON，并要求其退出码为 0；临时文件由创建者回收。
//! 不启动模型，不伪装 Adapter。

use std::path::{Path, PathBuf};
use std::process::Command;

use conex_agent_domain::{
    Admission, AuthorizedScope, BackendObservation, Store, StoreLimits, TerminalState, new_ulid,
};
use conex_proto::{
    AgentAdapterEndpoint, AgentCapabilities, AgentEventCursor, AgentEventReadResult, AgentLimits,
    AgentSubmitInput, Workspace,
};

const NODE_ID: &str = "smoke-node";

async fn seed(store: &Store) -> (String, String) {
    let endpoint_id = new_ulid();
    let workspace_id = new_ulid();
    store
        .put_endpoint(AgentAdapterEndpoint {
            id: endpoint_id.clone(),
            node_id: NODE_ID.into(),
            module_instance_id: new_ulid(),
            generation: 4,
            adapter: "omp".into(),
            backend_version: "18.13.2".into(),
            protocol_version: "1".into(),
            experimental: false,
            capabilities: Some(AgentCapabilities::default()),
            availability: "ready".into(),
            catalog_scope: "node-managed".into(),
            limits: Some(AgentLimits::default()),
            display_name: Some("omp".into()),
        })
        .await
        .expect("endpoint registered");
    store
        .put_workspace(Workspace {
            id: workspace_id.clone(),
            node_id: NODE_ID.into(),
            display_name: "smoke".into(),
            root_ref: "config://workspaces/default".into(),
        })
        .await
        .expect("workspace registered");
    (endpoint_id, workspace_id)
}

fn submit(conversation_id: &str, key: &str, content: &str) -> AgentSubmitInput {
    AgentSubmitInput {
        conversation_id: conversation_id.to_string(),
        idempotency_key: key.to_string(),
        content: content.to_string(),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("repo root")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let self_check = std::env::args().any(|arg| arg == "--self-check");
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("smoke-principal")?;
    let store = Store::open(&path, NODE_ID, StoreLimits::default()).await?;

    let (endpoint, workspace) = seed(&store).await;

    // 会话 D：正常完成 → 终态清槽。
    let done = store
        .create_conversation(&scope, &endpoint, &workspace, "create-d", None, 1_000)
        .await?;
    let Admission::Created {
        execution: done_execution,
        dispatch_token,
    } = store
        .admit(&scope, &submit(&done.id, "key-d", "done"), 1_100)
        .await?
    else {
        return Err("expected a created admission".into());
    };
    store
        .mark_dispatching(
            &done_execution.id,
            done_execution.generation,
            dispatch_token,
            1_200,
        )
        .await?;
    store
        .apply_backend(
            &done_execution.id,
            done_execution.generation,
            BackendObservation::Quiescent {
                state: TerminalState::Succeeded,
            },
            1_300,
        )
        .await?;

    // 会话 B：只到 accepted —— 崩溃后可证明未发送。
    let pending = store
        .create_conversation(&scope, &endpoint, &workspace, "create-b", None, 1_400)
        .await?;
    store
        .admit(&scope, &submit(&pending.id, "key-b", "pending"), 1_500)
        .await?;

    // 会话 U：分派后流式产出正文，随后进程结束。
    let streaming = store
        .create_conversation(&scope, &endpoint, &workspace, "create-u", None, 1_600)
        .await?;
    let Admission::Created {
        execution: stream_execution,
        dispatch_token,
    } = store
        .admit(&scope, &submit(&streaming.id, "key-u", "hello"), 1_700)
        .await?
    else {
        return Err("expected a created admission".into());
    };
    store
        .mark_dispatching(
            &stream_execution.id,
            stream_execution.generation,
            dispatch_token,
            1_800,
        )
        .await?;
    store
        .apply_backend(
            &stream_execution.id,
            stream_execution.generation,
            BackendObservation::Started {
                backend_ref: Some("backend-u".into()),
            },
            1_900,
        )
        .await?;
    let started = store
        .apply_backend(
            &stream_execution.id,
            stream_execution.generation,
            BackendObservation::MessageStarted {
                role: "assistant".into(),
            },
            2_000,
        )
        .await?;
    let message_id = started.message_id.ok_or("no message id")?;
    store
        .apply_backend(
            &stream_execution.id,
            stream_execution.generation,
            BackendObservation::TextAppended {
                message_id: message_id.clone(),
                text: "Hello from ".into(),
            },
            2_100,
        )
        .await?;
    store
        .apply_backend(
            &stream_execution.id,
            stream_execution.generation,
            BackendObservation::TextAppended {
                message_id: message_id.clone(),
                text: "the node".into(),
            },
            2_200,
        )
        .await?;

    // 崩溃：丢弃句柄后重新打开同一节点库。
    drop(store);
    let store = Store::open(&path, NODE_ID, StoreLimits::default()).await?;
    let targets = store.recover_unfinished(3_000).await?;
    let unknown_targets: Vec<&_> = targets.iter().filter(|t| t.state == "unknown").collect();
    assert_eq!(
        unknown_targets.len(),
        1,
        "the dispatching execution must be reconciled as unknown"
    );
    assert_eq!(unknown_targets[0].execution_id, stream_execution.id);

    let done_snapshot = store.snapshot(&scope, &done.id).await?;
    let done_execution = done_snapshot
        .executions
        .iter()
        .find(|e| e.id == done_execution.id)
        .ok_or("missing execution")?;
    assert_eq!(done_execution.state, "succeeded");
    assert!(
        done_snapshot
            .conversation
            .as_ref()
            .and_then(|c| c.active_execution_id.clone())
            .is_none(),
        "a terminal execution must release the slot"
    );
    let pending_snapshot = store.snapshot(&scope, &pending.id).await?;
    let pending_execution = pending_snapshot
        .executions
        .iter()
        .find(|e| {
            e.id == pending_snapshot
                .conversation
                .as_ref()
                .and_then(|c| c.active_execution_id.clone())
                .unwrap_or_default()
        })
        .or_else(|| pending_snapshot.executions.first())
        .ok_or("missing pending execution")?;
    assert_eq!(pending_execution.state, "failed");
    assert_eq!(
        pending_execution.terminal_reason.as_deref(),
        Some("not_dispatched")
    );

    // 快照后后端继续投递一个片段：delta 必须接在已知 totalBytes 之后。
    let snapshot = store.snapshot(&scope, &streaming.id).await?;
    let stream_cursor = snapshot.cursor.clone().ok_or("no cursor")?;
    let snapshot_message = snapshot
        .messages
        .iter()
        .find(|m| m.id == message_id)
        .ok_or("missing assistant message")?;
    match &snapshot_message.parts[0].part {
        Some(conex_proto::agent_message_part::Part::Text(part)) => {
            assert_eq!(part.text, "Hello from the node");
            assert_eq!(part.total_bytes, 19);
        }
        other => return Err(format!("expected a text part, got {other:?}").into()),
    }
    assert_eq!(
        snapshot
            .conversation
            .as_ref()
            .and_then(|c| c.active_execution_id.clone()),
        Some(stream_execution.id.clone()),
        "an unknown execution keeps the slot"
    );
    store
        .apply_backend(
            &stream_execution.id,
            stream_execution.generation,
            BackendObservation::TextAppended {
                message_id: message_id.clone(),
                text: "!".into(),
            },
            4_000,
        )
        .await?;

    let mut pages: Vec<AgentEventReadResult> = Vec::new();
    let mut cursor = AgentEventCursor {
        stream_id: stream_cursor.stream_id.clone(),
        epoch: stream_cursor.epoch.clone(),
        after: stream_cursor.after,
    };
    loop {
        let page = store.read_events(&scope, &cursor, Some(32)).await?;
        let next = page.next_cursor.clone().ok_or("no next cursor")?;
        let done = !page.has_more;
        pages.push(page);
        cursor = next;
        if done {
            break;
        }
    }
    let head = cursor.after;
    let mut ndjson = String::new();
    ndjson.push_str(&format!(
        "{{\"kind\":\"meta\",\"nodeId\":\"{NODE_ID}\",\"head\":\"{head}\"}}\n"
    ));
    ndjson.push_str(&format!(
        "{{\"kind\":\"snapshot\",\"snapshot\":{}}}\n",
        serde_json::to_string(&snapshot)?
    ));
    for page in &pages {
        ndjson.push_str(&format!(
            "{{\"kind\":\"page\",\"page\":{}}}\n",
            serde_json::to_string(page)?
        ));
    }

    if self_check {
        let file = dir.path().join("recovery.ndjson");
        std::fs::write(&file, &ndjson)?;
        let root = repo_root();
        let script = root.join("sdk/typescript/examples/agent-recovery-smoke.ts");
        let output = Command::new("bun")
            .arg(&script)
            .arg(&file)
            .current_dir(&root)
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            return Err(format!(
                "typescript recovery consumer failed ({}):\n{stdout}\n{stderr}",
                output.status
            )
            .into());
        }
        println!("typescript consumer: {}", stdout.trim());
        std::fs::remove_file(&file)?;
    } else {
        print!("{ndjson}");
    }
    Ok(())
}

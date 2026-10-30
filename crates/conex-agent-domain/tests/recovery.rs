//! 恢复、保留与备份恢复回归（契约 §5、恢复契约 §5-§6；计划 §8 表格第 2/5 行）。

use conex_agent_domain::{
    Admission, AuthorizedScope, BackendObservation, Store, StoreError, StoreLimits, TerminalState,
    new_ulid,
};
use conex_proto::{
    AgentAdapterEndpoint, AgentCapabilities, AgentEventCursor, AgentLimits, AgentSubmitInput,
    Workspace,
};

async fn seed(store: &Store, node: &str) -> (String, String) {
    let endpoint_id = new_ulid();
    let workspace_id = new_ulid();
    store
        .put_endpoint(AgentAdapterEndpoint {
            id: endpoint_id.clone(),
            node_id: node.into(),
            module_instance_id: new_ulid(),
            generation: 2,
            adapter: "omp".into(),
            backend_version: "18.13.2".into(),
            protocol_version: "1".into(),
            experimental: false,
            capabilities: Some(AgentCapabilities::default()),
            availability: "ready".into(),
            catalog_scope: "node-managed".into(),
            limits: Some(AgentLimits::default()),
            display_name: None,
        })
        .await
        .expect("endpoint");
    store
        .put_workspace(Workspace {
            id: workspace_id.clone(),
            node_id: node.into(),
            display_name: "work".into(),
            root_ref: "config://workspaces/default".into(),
        })
        .await
        .expect("workspace");
    (endpoint_id, workspace_id)
}

fn submit(conversation_id: &str, key: &str, content: &str) -> AgentSubmitInput {
    AgentSubmitInput {
        conversation_id: conversation_id.to_string(),
        idempotency_key: key.to_string(),
        content: content.to_string(),
    }
}

#[tokio::test]
async fn accepted_execution_is_failed_as_not_dispatched_and_frees_the_slot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store, "node-a").await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    let admission = store
        .admit(&scope, &submit(&conversation.id, "key-a", "hello"), 2_000)
        .await
        .expect("admission");
    let Admission::Created { execution, .. } = admission else {
        panic!("created expected");
    };
    // 模拟崩溃：进程在 dispatching 持久化之前结束。
    drop(store);
    let reopened = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("reopen");
    let targets = reopened.recover_unfinished(3_000).await.expect("recover");
    assert!(
        targets
            .iter()
            .all(|target| target.execution_id != execution.id),
        "an accepted execution is provably unsent and must not become a reconcile target"
    );
    let snapshot = reopened
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    let recovered = snapshot
        .executions
        .iter()
        .find(|e| e.id == execution.id)
        .expect("execution in snapshot");
    assert_eq!(recovered.state, "failed");
    assert_eq!(recovered.terminal_reason.as_deref(), Some("not_dispatched"));
    assert_eq!(
        snapshot
            .conversation
            .as_ref()
            .and_then(|c| c.active_execution_id.clone()),
        None,
        "the slot must be released"
    );
    // 槽释放后可以再次准入。
    let again = reopened
        .admit(&scope, &submit(&conversation.id, "key-b", "retry"), 4_000)
        .await
        .expect("admission after recovery");
    assert!(matches!(again, Admission::Created { .. }));
}

#[tokio::test]
async fn dispatching_execution_becomes_unknown_and_keeps_the_slot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store, "node-a").await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    let admission = store
        .admit(&scope, &submit(&conversation.id, "key-a", "hello"), 2_000)
        .await
        .expect("admission");
    let Admission::Created {
        execution,
        dispatch_token,
    } = admission
    else {
        panic!("created expected");
    };
    store
        .mark_dispatching(&execution.id, execution.generation, dispatch_token, 2_100)
        .await
        .expect("dispatching");
    drop(store);
    let reopened = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("reopen");
    let targets = reopened.recover_unfinished(3_000).await.expect("recover");
    assert_eq!(
        targets.len(),
        1,
        "a dispatching execution needs reconciliation"
    );
    assert_eq!(targets[0].execution_id, execution.id);
    assert_eq!(targets[0].state, "unknown");
    // 槽仍被占用：新提交必须 busy。
    let busy = reopened
        .admit(&scope, &submit(&conversation.id, "key-b", "other"), 4_000)
        .await
        .expect_err("unknown execution keeps the slot");
    assert!(matches!(busy, StoreError::ConversationBusy { .. }));
    // 后端确认退出后由 Quiescent 清槽。
    reopened
        .apply_backend(
            &execution.id,
            execution.generation,
            BackendObservation::Quiescent {
                state: TerminalState::Failed("backend_terminated".into()),
            },
            5_000,
        )
        .await
        .expect("quiescent");
    let snapshot = reopened
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    assert_eq!(
        snapshot
            .conversation
            .as_ref()
            .and_then(|c| c.active_execution_id.clone()),
        None,
        "confirmed termination frees the slot"
    );
}

#[tokio::test]
async fn restore_rotates_epoch_invalidates_old_cursor_and_keeps_dedup() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let backup = dir.path().join("backup.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store, "node-a").await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    store
        .admit(&scope, &submit(&conversation.id, "key-a", "hello"), 2_000)
        .await
        .expect("admission");
    let before = store
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    let old_cursor = before.cursor.clone().expect("cursor");
    store.backup_to(&backup).await.expect("backup");

    let restored_dir = dir.path().join("restored");
    let restored = Store::restore(
        &backup,
        restored_dir.join("node.db"),
        "node-a",
        StoreLimits::default(),
    )
    .await
    .expect("restore");
    let after = restored
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    let new_cursor = after.cursor.clone().expect("cursor");
    assert_ne!(
        new_cursor.epoch, old_cursor.epoch,
        "a restored stream must rotate its epoch"
    );
    let stale = restored
        .read_events(&scope, &old_cursor, None)
        .await
        .expect_err("old epoch must require a resync");
    assert!(matches!(stale, StoreError::ResumeUnavailable { .. }));
    assert!(!after.gaps.is_empty(), "restore records a recovery gap");
    // 既有幂等键仍属于原节点：同键查回原对象，不产生第二次执行。
    let existing = restored
        .admit(&scope, &submit(&conversation.id, "key-a", "hello"), 6_000)
        .await
        .expect("same key after restore");
    assert!(matches!(existing, Admission::Existing { .. }));
}

#[tokio::test]
async fn restore_refuses_a_backup_from_another_node() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let backup = dir.path().join("backup.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store, "node-a").await;
    store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    store.backup_to(&backup).await.expect("backup");
    let error = Store::restore(
        &backup,
        dir.path().join("other").join("node.db"),
        "node-b",
        StoreLimits::default(),
    )
    .await
    .expect_err("a backup belongs to exactly one node");
    assert!(matches!(error, StoreError::NodeMismatch { .. }));
}

#[tokio::test]
async fn reopening_with_another_node_id_is_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store, "node-a").await;
    store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    drop(store);
    let error = Store::open(&path, "node-b", StoreLimits::default())
        .await
        .expect_err("a node library must not be shared");
    assert!(matches!(error, StoreError::NodeMismatch { .. }));
}

#[tokio::test]
async fn cursor_boundaries_distinguish_resync_ahead_and_epoch_change() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store, "node-a").await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    store
        .admit(&scope, &submit(&conversation.id, "key-a", "hello"), 2_000)
        .await
        .expect("admission");
    let snapshot = store
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    let cursor = snapshot.cursor.clone().expect("cursor");
    let head = cursor.after;

    // head+1 是非法游标。
    let ahead = store
        .read_events(
            &scope,
            &AgentEventCursor {
                stream_id: cursor.stream_id.clone(),
                epoch: cursor.epoch.clone(),
                after: head + 1,
            },
            None,
        )
        .await
        .expect_err("cursor ahead must be rejected");
    match ahead {
        StoreError::InvalidArgument(reason) => assert_eq!(reason, "cursor_ahead"),
        other => panic!("expected cursor_ahead, got {other:?}"),
    }

    // 另一个 epoch 必须要求重同步。
    let other_epoch = store
        .read_events(
            &scope,
            &AgentEventCursor {
                stream_id: cursor.stream_id.clone(),
                epoch: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                after: 0,
            },
            None,
        )
        .await
        .expect_err("foreign epoch must require a resync");
    assert!(matches!(other_epoch, StoreError::ResumeUnavailable { .. }));

    // 清理前缀后：firstAvailable-1 可读，更早一位必须重同步。
    store
        .prune(1_000 + 24 * 60 * 60 * 1000)
        .await
        .expect("prune");
    let after_prune = store
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    let first_available = after_prune.first_available;
    assert!(first_available > 1, "the prefix must have been pruned");
    let epoch = after_prune.cursor.clone().expect("cursor").epoch;
    let at_boundary = store
        .read_events(
            &scope,
            &AgentEventCursor {
                stream_id: cursor.stream_id.clone(),
                epoch: epoch.clone(),
                after: first_available - 1,
            },
            None,
        )
        .await
        .expect("firstAvailable-1 is a valid resume point");
    assert_eq!(
        at_boundary.events.first().map(|e| e.position),
        Some(first_available)
    );
    let too_old = store
        .read_events(
            &scope,
            &AgentEventCursor {
                stream_id: cursor.stream_id.clone(),
                epoch,
                after: first_available - 2,
            },
            None,
        )
        .await
        .expect_err("an older cursor must require a resync");
    assert!(matches!(too_old, StoreError::ResumeUnavailable { .. }));
}

#[tokio::test]
async fn snapshot_stays_within_budget_with_many_terminal_executions_and_a_long_reply() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let limits = StoreLimits::default();
    let store = Store::open(&path, "node-a", limits.clone())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store, "node-a").await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    // 超过快照预算内的终态数量：终态按预算裁剪，活动集合必须完整。
    for index in 0..25u64 {
        let admission = store
            .admit(
                &scope,
                &submit(&conversation.id, &format!("key-{index}"), "work"),
                2_000 + index,
            )
            .await
            .expect("admission");
        let Admission::Created {
            execution,
            dispatch_token,
        } = admission
        else {
            panic!("created expected");
        };
        store
            .mark_dispatching(
                &execution.id,
                execution.generation,
                dispatch_token,
                2_100 + index,
            )
            .await
            .expect("dispatching");
        store
            .apply_backend(
                &execution.id,
                execution.generation,
                BackendObservation::Quiescent {
                    state: TerminalState::Succeeded,
                },
                2_200 + index,
            )
            .await
            .expect("quiescent");
    }
    // 一条远超单帧上限的长回复：正文必须可经分页补读，快照不得超预算。
    let admission = store
        .admit(&scope, &submit(&conversation.id, "key-long", "long"), 3_000)
        .await
        .expect("admission");
    let Admission::Created {
        execution,
        dispatch_token,
    } = admission
    else {
        panic!("created expected");
    };
    store
        .mark_dispatching(&execution.id, execution.generation, dispatch_token, 3_100)
        .await
        .expect("dispatching");
    let started = store
        .apply_backend(
            &execution.id,
            execution.generation,
            BackendObservation::MessageStarted {
                role: "assistant".into(),
            },
            3_200,
        )
        .await
        .expect("message started");
    let message_id = started.message_id.expect("message id");
    let long_text = "x".repeat(20_000);
    store
        .apply_backend(
            &execution.id,
            execution.generation,
            BackendObservation::TextAppended {
                message_id: message_id.clone(),
                text: long_text.clone(),
            },
            3_300,
        )
        .await
        .expect("text appended");
    let snapshot = store
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    let encoded = serde_json::to_vec(&snapshot).expect("snapshot encodes");
    assert!(
        encoded.len() <= limits.max_event_page_bytes - 16_384,
        "snapshot must stay inside the result budget: {} bytes",
        encoded.len()
    );
    assert!(
        snapshot.executions.len() <= limits.snapshot_terminal_executions + 1,
        "terminal history is trimmed to the budget"
    );
    let chunk = store
        .read_message(&scope, &conversation.id, &message_id, 0, 0, 65536)
        .await
        .expect("paged read");
    assert_eq!(chunk.total_bytes, 20_000);
    assert_eq!(chunk.text.len(), 20_000);
}

#[tokio::test]
async fn pruning_advances_first_available_and_forces_resync() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store, "node-a").await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    store
        .admit(&scope, &submit(&conversation.id, "key-a", "hello"), 2_000)
        .await
        .expect("admission");
    let snapshot = store
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    let cursor = snapshot.cursor.clone().expect("cursor");
    let head = cursor.after;
    assert!(head > 0);
    // 保留窗口 24h：把 now 推后一天后清理旧前缀。
    let pruned = store
        .prune(1_000 + 24 * 60 * 60 * 1000)
        .await
        .expect("prune");
    assert!(pruned.events_deleted > 0, "old events must be pruned");
    let stale = store
        .read_events(
            &scope,
            &AgentEventCursor {
                stream_id: cursor.stream_id.clone(),
                epoch: cursor.epoch.clone(),
                after: 0,
            },
            None,
        )
        .await
        .expect_err("pruned prefix must require a resync");
    assert!(matches!(stale, StoreError::ResumeUnavailable { .. }));
}

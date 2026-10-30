//! 准入与幂等回归（契约 §5、恢复契约 §4；计划 §8 表格第 1/4 行）。
//! 并发用两个独立 Store（同一 SQLite 文件）与屏障，不用 sleep 排序。

use conex_agent_domain::{
    Admission, AuthorizedScope, BackendObservation, Store, StoreError, StoreLimits, TerminalState,
    new_ulid,
};
use conex_proto::{
    AgentAdapterEndpoint, AgentCapabilities, AgentLimits, AgentSubmitInput, Workspace,
};

async fn seed(store: &Store) -> (String, String) {
    let endpoint_id = new_ulid();
    let workspace_id = new_ulid();
    store
        .put_endpoint(AgentAdapterEndpoint {
            id: endpoint_id.clone(),
            node_id: "node-a".into(),
            module_instance_id: new_ulid(),
            generation: 3,
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
            node_id: "node-a".into(),
            display_name: "work".into(),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_idempotency_key_yields_one_execution_and_no_second_permit() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store).await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");

    // 两个独立 Store 连接同一数据库文件，屏障同时进入 admit。
    let other = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("second open");
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let first = {
        let store = store.clone();
        let barrier = barrier.clone();
        let scope = scope.clone();
        let input = submit(&conversation.id, "same-key", "hello");
        tokio::spawn(async move {
            barrier.wait().await;
            store.admit(&scope, &input, 2_000).await
        })
    };
    let second = {
        let store = other.clone();
        let barrier = barrier.clone();
        let scope = scope.clone();
        let input = submit(&conversation.id, "same-key", "hello");
        tokio::spawn(async move {
            barrier.wait().await;
            store.admit(&scope, &input, 2_000).await
        })
    };
    let first = first.await.expect("first task").expect("first admission");
    let second = second
        .await
        .expect("second task")
        .expect("second admission");
    let admissions = [first, second];
    let created: Vec<&Admission> = admissions
        .iter()
        .filter(|a| matches!(a, Admission::Created { .. }))
        .collect();
    let existing: Vec<&Admission> = admissions
        .iter()
        .filter(|a| matches!(a, Admission::Existing { .. }))
        .collect();
    assert_eq!(
        created.len(),
        1,
        "exactly one admission may create an execution"
    );
    assert_eq!(
        existing.len(),
        1,
        "the other admission must observe the existing execution"
    );
    assert_eq!(
        created[0].execution().id,
        existing[0].execution().id,
        "same key and digest must resolve to the same execution"
    );
}

#[tokio::test]
async fn different_key_on_busy_conversation_is_rejected_without_consuming_key() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store).await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    let admission = store
        .admit(&scope, &submit(&conversation.id, "key-a", "hello"), 2_000)
        .await
        .expect("first admission");
    let Admission::Created { execution, .. } = admission else {
        panic!("first admission must create");
    };
    let busy = store
        .admit(&scope, &submit(&conversation.id, "key-b", "other"), 2_100)
        .await
        .expect_err("second key must be rejected while busy");
    match busy {
        StoreError::ConversationBusy {
            active_execution_id,
        } => {
            assert_eq!(active_execution_id, execution.id);
        }
        other => panic!("expected conversation busy, got {other:?}"),
    }
    // busy 不消耗幂等键：前一个执行结束后，同一 key 仍可正常准入。
    store
        .apply_backend(
            &execution.id,
            execution.generation,
            BackendObservation::Quiescent {
                state: TerminalState::Succeeded,
            },
            2_150,
        )
        .await
        .expect("quiescent");
    let retry = store
        .admit(&scope, &submit(&conversation.id, "key-b", "other"), 2_200)
        .await
        .expect("key was not consumed by the busy rejection");
    assert!(matches!(retry, Admission::Created { .. }));
}

#[tokio::test]
async fn same_key_with_different_content_is_a_conflict() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store).await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    store
        .admit(&scope, &submit(&conversation.id, "key-a", "hello"), 2_000)
        .await
        .expect("first admission");
    let conflict = store
        .admit(
            &scope,
            &submit(&conversation.id, "key-a", "different"),
            2_100,
        )
        .await
        .expect_err("different digest must conflict");
    assert!(matches!(conflict, StoreError::IdempotencyMismatch { .. }));
}

#[tokio::test]
async fn user_message_is_chunked_into_bounded_delta_events() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store).await;
    let conversation = store
        .create_conversation(&scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    let content = "é".repeat(5_000); // 10_000 UTF-8 bytes
    let admission = store
        .admit(&scope, &submit(&conversation.id, "key-a", &content), 2_000)
        .await
        .expect("admission");
    let Admission::Created { execution, .. } = admission else {
        panic!("created expected");
    };
    let snapshot = store
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    let cursor = snapshot.cursor.clone().expect("cursor");
    // 快照游标位于 head；正文事件要从流起点回放。
    let page = store
        .read_events(
            &scope,
            &conex_proto::AgentEventCursor {
                stream_id: cursor.stream_id.clone(),
                epoch: cursor.epoch.clone(),
                after: 0,
            },
            None,
        )
        .await
        .expect("events");
    let deltas: Vec<&conex_proto::AgentMessageDelta> = page
        .events
        .iter()
        .filter_map(|event| match &event.data {
            Some(conex_proto::agent_event::Data::MessageDelta(delta)) => Some(delta),
            _ => None,
        })
        .collect();
    assert!(!deltas.is_empty(), "user text must arrive as delta events");
    for delta in &deltas {
        assert!(
            delta.text.len() <= 4096,
            "delta chunks stay within 4096 bytes"
        );
    }
    let created = page
        .events
        .iter()
        .find(|event| event.r#type == "message.created")
        .expect("message.created event");
    // created 事件不内嵌正文；正文由 delta 追加。
    match &created.data {
        Some(conex_proto::agent_event::Data::MessageCreated(message)) => {
            match &message.parts[0].part {
                Some(conex_proto::agent_message_part::Part::Text(part)) => {
                    assert!(part.text.is_empty(), "created carries an empty text window");
                    assert_eq!(part.total_bytes, content.len() as u64);
                }
                other => panic!("expected text part, got {other:?}"),
            }
        }
        other => panic!("expected message.created payload, got {other:?}"),
    }
    let message_id = match &created.data {
        Some(conex_proto::agent_event::Data::MessageCreated(message)) => message.id.clone(),
        _ => unreachable!(),
    };
    let chunk = store
        .read_message(&scope, &conversation.id, &message_id, 0, 0, 65536)
        .await
        .expect("message read");
    assert_eq!(
        chunk.text, content,
        "paged read must return the whole committed text"
    );
    assert_eq!(chunk.total_bytes, content.len() as u64);
    assert_eq!(chunk.next_offset_bytes, content.len() as u64);
    assert_eq!(execution.state, "accepted");
}

#[tokio::test]
async fn concurrent_create_with_the_same_key_returns_one_conversation() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store).await;
    let other = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("second open");
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let first = {
        let store = store.clone();
        let barrier = barrier.clone();
        let scope = scope.clone();
        let endpoint = endpoint.clone();
        let workspace = workspace.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            store
                .create_conversation(&scope, &endpoint, &workspace, "create-key", None, 1_000)
                .await
        })
    };
    let second = {
        let store = other.clone();
        let barrier = barrier.clone();
        let scope = scope.clone();
        let endpoint = endpoint.clone();
        let workspace = workspace.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            store
                .create_conversation(&scope, &endpoint, &workspace, "create-key", None, 1_000)
                .await
        })
    };
    let first = first.await.expect("first task").expect("first create");
    let second = second.await.expect("second task").expect("second create");
    assert_eq!(
        first.id, second.id,
        "same key must resolve to one conversation"
    );
}

#[tokio::test]
async fn message_quota_isolates_the_endpoint_instead_of_growing_unbounded() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let limits = StoreLimits {
        max_active_messages: 2,
        ..StoreLimits::default()
    };
    let store = Store::open(&path, "node-a", limits).await.expect("open");
    let (endpoint, workspace) = seed(&store).await;
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
    for index in 0..2 {
        store
            .apply_backend(
                &execution.id,
                execution.generation,
                BackendObservation::MessageStarted {
                    role: "assistant".into(),
                },
                2_200 + index,
            )
            .await
            .expect("message within quota");
    }
    // 第三个活跃消息超过配额：有界失败，执行转 unknown，不追加原始对象。
    let overflow = store
        .apply_backend(
            &execution.id,
            execution.generation,
            BackendObservation::MessageStarted {
                role: "assistant".into(),
            },
            2_300,
        )
        .await
        .expect("bounded failure");
    assert!(
        !overflow.applied,
        "an over-quota observation must not be applied"
    );
    assert_eq!(overflow.execution_state.as_deref(), Some("unknown"));
    let snapshot = store
        .snapshot(&scope, &conversation.id)
        .await
        .expect("snapshot");
    assert!(
        snapshot
            .messages
            .iter()
            .filter(|m| m.state == "streaming")
            .count()
            <= 2,
        "the message quota must hold"
    );
    assert!(
        !snapshot.gaps.is_empty(),
        "the bounded failure records a recovery gap"
    );
    assert!(
        snapshot
            .conversation
            .as_ref()
            .and_then(|c| c.active_execution_id.clone())
            .is_some(),
        "an unknown execution keeps the slot"
    );
}

#[tokio::test]
async fn natural_completion_wins_over_late_cancel_and_frees_the_slot() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (endpoint, workspace) = seed(&store).await;
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
    store
        .apply_backend(
            &execution.id,
            execution.generation,
            BackendObservation::Started {
                backend_ref: Some("backend-1".into()),
            },
            2_200,
        )
        .await
        .expect("started");
    store
        .apply_backend(
            &execution.id,
            execution.generation,
            BackendObservation::Quiescent {
                state: TerminalState::Succeeded,
            },
            2_300,
        )
        .await
        .expect("quiescent");
    let cancel = store
        .request_cancel(&scope, &execution.id, execution.generation, 2_400)
        .await
        .expect("cancel request");
    assert!(
        !cancel.requested,
        "a terminal execution cannot be cancelled"
    );
    assert_eq!(cancel.state, "succeeded");
    // 槽已释放：新 key 可以再次准入。
    let next = store
        .admit(&scope, &submit(&conversation.id, "key-b", "again"), 2_500)
        .await
        .expect("slot must be free after a terminal state");
    assert!(matches!(next, Admission::Created { .. }));
}

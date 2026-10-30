//! 人工交互 CAS 回归（契约 §7；计划 §8 表格第 3 行）。
//! 两端竞答用两个独立 Store 与屏障；只允许一次后端发送。

use conex_agent_domain::{
    Admission, AuthorizedScope, BackendObservation, BackendResponseOutcome, Store, StoreError,
    StoreLimits, new_ulid,
};
use conex_proto::{
    AgentAdapterEndpoint, AgentCapabilities, AgentInteractionDecision,
    AgentInteractionRespondInput, AgentLimits, AgentSubmitInput, Workspace,
};

async fn seed(store: &Store) -> (String, String) {
    let endpoint_id = new_ulid();
    let workspace_id = new_ulid();
    store
        .put_endpoint(AgentAdapterEndpoint {
            id: endpoint_id.clone(),
            node_id: "node-a".into(),
            module_instance_id: new_ulid(),
            generation: 1,
            adapter: "codex".into(),
            backend_version: "0.163.0".into(),
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
            node_id: "node-a".into(),
            display_name: "work".into(),
            root_ref: "config://workspaces/default".into(),
        })
        .await
        .expect("workspace");
    (endpoint_id, workspace_id)
}

/// 准备一个待审批交互，返回 (conversation_id, execution_id, interaction_id, revision)。
async fn pending_interaction(
    store: &Store,
    scope: &AuthorizedScope,
    expires_at_ms: Option<u64>,
) -> (String, String, String, u64) {
    let (endpoint, workspace) = seed(store).await;
    let conversation = store
        .create_conversation(scope, &endpoint, &workspace, "create-1", None, 1_000)
        .await
        .expect("conversation");
    let admission = store
        .admit(
            scope,
            &AgentSubmitInput {
                conversation_id: conversation.id.clone(),
                idempotency_key: "key-a".into(),
                content: "run".into(),
            },
            2_000,
        )
        .await
        .expect("admission");
    let Admission::Created { execution, .. } = admission else {
        panic!("created expected");
    };
    let opened = store
        .apply_backend(
            &execution.id,
            execution.generation,
            BackendObservation::InteractionOpened {
                kind: "approval".into(),
                prompt: Some("allow?".into()),
                choices: vec![(
                    "allow_once".into(),
                    "Allow once".into(),
                    "allow_once".into(),
                )],
                expires_at_ms,
            },
            3_000,
        )
        .await
        .expect("interaction opened");
    let interaction_id = opened.interaction_id.expect("interaction id");
    (conversation.id, execution.id, interaction_id, 1)
}

fn respond(interaction_id: &str, revision: u64, choice: &str) -> AgentInteractionRespondInput {
    AgentInteractionRespondInput {
        interaction_id: interaction_id.to_string(),
        expected_revision: revision,
        decision: Some(AgentInteractionDecision {
            kind: "approval".into(),
            choice_id: Some(choice.to_string()),
            choice_ids: Vec::new(),
            text: None,
        }),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_answers_produce_exactly_one_winner() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (_conversation, _execution, interaction, revision) =
        pending_interaction(&store, &scope, None).await;

    let other = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("second open");
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let first = {
        let store = store.clone();
        let barrier = barrier.clone();
        let scope = scope.clone();
        let input = respond(&interaction, revision, "allow_once");
        tokio::spawn(async move {
            barrier.wait().await;
            store.begin_response(&scope, &input, 4_000).await
        })
    };
    let second = {
        let store = other.clone();
        let barrier = barrier.clone();
        let scope = scope.clone();
        let input = respond(&interaction, revision, "allow_once");
        tokio::spawn(async move {
            barrier.wait().await;
            store.begin_response(&scope, &input, 4_000).await
        })
    };
    let first = first.await.expect("first task");
    let second = second.await.expect("second task");
    let mut losers = Vec::new();
    let mut winners = Vec::new();
    let results = vec![first, second];
    for result in results {
        match result {
            Ok(permit) => winners.push(permit),
            Err(error) => losers.push(error),
        }
    }
    assert_eq!(winners.len(), 1, "exactly one answer may win the CAS");
    assert_eq!(losers.len(), 1, "the loser must be rejected");
    assert!(
        matches!(
            losers[0],
            StoreError::Conflict(_) | StoreError::StaleRevision { .. }
        ),
        "unexpected loser error: {:?}",
        losers[0]
    );
}

#[tokio::test]
async fn identical_retry_is_recovered_and_a_different_decision_conflicts() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (_conversation, _execution, interaction, revision) =
        pending_interaction(&store, &scope, None).await;
    let permit = store
        .begin_response(
            &scope,
            &respond(&interaction, revision, "allow_once"),
            4_000,
        )
        .await
        .expect("first answer");
    let resolved = store
        .finish_response(permit, BackendResponseOutcome::Confirmed, 4_100)
        .await
        .expect("finish");
    assert_eq!(resolved.state, "resolved");

    // 同一决定、同一主体的重试查回已解决结果，不产生第二次发送。
    let retry = store
        .begin_response(
            &scope,
            &respond(&interaction, revision, "allow_once"),
            4_200,
        )
        .await
        .expect("same decision retry");
    let retried = store
        .finish_response(retry, BackendResponseOutcome::Confirmed, 4_300)
        .await
        .expect("retry finish");
    assert_eq!(retried.state, "resolved");
    assert_eq!(
        retried.revision, resolved.revision,
        "retry must not bump the revision"
    );

    // 不同决定必须冲突。
    let conflict = store
        .begin_response(&scope, &respond(&interaction, revision, "deny"), 4_400)
        .await
        .expect_err("different decision must conflict");
    assert!(matches!(conflict, StoreError::Conflict(_)));
}

#[tokio::test]
async fn expired_interaction_refuses_new_answers() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (_conversation, _execution, interaction, revision) =
        pending_interaction(&store, &scope, Some(2_500)).await;
    let expired = store
        .begin_response(
            &scope,
            &respond(&interaction, revision, "allow_once"),
            4_000,
        )
        .await
        .expect_err("expired interaction must refuse answers");
    assert!(matches!(
        expired,
        StoreError::Conflict("interaction_expired")
    ));
}

#[tokio::test]
async fn unknown_answer_blocks_a_second_attempt() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (_conversation, _execution, interaction, revision) =
        pending_interaction(&store, &scope, None).await;
    let permit = store
        .begin_response(
            &scope,
            &respond(&interaction, revision, "allow_once"),
            4_000,
        )
        .await
        .expect("answer");
    let unknown = store
        .finish_response(
            permit,
            BackendResponseOutcome::Unknown {
                reason: "transport lost".into(),
            },
            4_100,
        )
        .await
        .expect("finish");
    assert_eq!(unknown.state, "unknown");
    let second = store
        .begin_response(
            &scope,
            &respond(&interaction, revision, "allow_once"),
            4_200,
        )
        .await
        .expect_err("unknown must not accept a second answer");
    assert!(matches!(
        second,
        StoreError::Conflict("interaction_consumed")
    ));
}

#[tokio::test]
async fn unconfirmed_interaction_is_invalidated_without_touching_the_execution() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let scope = AuthorizedScope::new("principal-1").expect("scope");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let (_conversation, execution_id, interaction, _revision) =
        pending_interaction(&store, &scope, None).await;
    let invalidated = store
        .invalidate_unconfirmed_interaction(&interaction, 1, "cannot confirm after restart", 5_000)
        .await
        .expect("invalidate");
    assert_eq!(invalidated.state, "invalidated");
    assert_eq!(invalidated.revision, 2, "invalidation bumps the revision");
    let snapshot = store
        .snapshot(&scope, &_conversation)
        .await
        .expect("snapshot");
    let execution = snapshot
        .executions
        .iter()
        .find(|e| e.id == execution_id)
        .expect("execution in snapshot");
    assert!(
        !matches!(
            execution.state.as_str(),
            "succeeded" | "failed" | "cancelled"
        ),
        "invalidation must not fabricate a terminal state: {}",
        execution.state
    );
}

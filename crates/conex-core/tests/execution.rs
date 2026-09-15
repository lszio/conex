//! Unified execution path: denial has zero side effects; ordering is auditable.
mod support;

use std::time::Duration;

use conex_proto::v1;
use serde_json::json;

use support::TestRig;

#[tokio::test]
async fn denied_call_has_no_business_side_effects() {
    let rig = TestRig::new(false);
    let result = rig
        .host
        .invoke(
            &rig.caller(),
            "test",
            "test/read",
            json!({"resourceId": "secret.md"}),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Forbidden)
    );
    let counts = rig.counts();
    assert_eq!((counts.connect, counts.resolve, counts.execute), (0, 0, 0));
    let events = rig.events();
    assert_eq!(events.last().unwrap().outcome, "deny");
}

#[tokio::test]
async fn allowed_call_records_full_event_sequence() {
    let rig = TestRig::new(true);
    let value = rig
        .host
        .invoke(
            &rig.caller(),
            "test",
            "test/read",
            json!({"resourceId": "ok.md"}),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    assert_eq!(value["text"], "ok");
    let phases: Vec<String> = rig
        .events()
        .iter()
        .map(|event| event.phase.clone())
        .collect();
    assert_eq!(
        phases,
        vec![
            "authorize",
            "connect",
            "peer_verified",
            "authorize",
            "resolve",
            "execute",
            "audit_finish"
        ]
    );
    let counts = rig.counts();
    assert_eq!((counts.connect, counts.resolve, counts.execute), (1, 1, 1));
}

#[tokio::test]
async fn connect_failure_has_no_credential_or_execute() {
    let rig = TestRig::failing_connect(true);
    let result = rig
        .host
        .invoke(
            &rig.caller(),
            "test",
            "test/read",
            json!({"resourceId": "ok.md"}),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::PeerUntrusted)
    );
    let counts = rig.counts();
    assert_eq!((counts.connect, counts.resolve, counts.execute), (1, 0, 0));
}

#[tokio::test]
async fn policy_revoked_during_handshake_blocks_credentials_and_execute() {
    let rig = TestRig::revoked_during_connect();
    let result = rig
        .host
        .invoke(
            &rig.caller(),
            "test",
            "test/read",
            json!({"resourceId": "ok.md"}),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Forbidden)
    );
    let counts = rig.counts();
    assert_eq!((counts.connect, counts.resolve, counts.execute), (1, 0, 0));
}

#[tokio::test]
async fn local_route_does_not_touch_remote_ports() {
    let rig = TestRig::local_only(true);
    rig.host
        .invoke(
            &rig.caller(),
            "test",
            "test/read",
            json!({"resourceId": "ok.md"}),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    let counts = rig.counts();
    assert_eq!((counts.connect, counts.resolve, counts.execute), (0, 0, 1));
}

#[tokio::test]
async fn audit_reserve_failure_rejects_before_execution() {
    let rig = TestRig::with_failing_audit();
    let result = rig
        .host
        .invoke(
            &rig.caller(),
            "test",
            "test/read",
            json!({"resourceId": "ok.md"}),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Unavailable)
    );
    assert_eq!(rig.counts().execute, 0);
}

#[tokio::test]
async fn prepare_rejects_unknown_business_field() {
    let rig = TestRig::new(true);
    let result = rig
        .host
        .invoke(
            &rig.caller(),
            "test",
            "test/read",
            json!({"resourceId": "ok.md", "principalId": "spoofed"}),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::BadRequest)
    );
    assert_eq!(rig.counts().execute, 0);
}

#[tokio::test(start_paused = true)]
async fn slow_handler_is_bounded_by_deadline() {
    let rig = TestRig::slow(true, Duration::from_secs(30));
    let result = rig
        .host
        .invoke(
            &rig.caller(),
            "test",
            "test/read",
            json!({"resourceId": "ok.md"}),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Timeout)
    );
}

#[tokio::test]
async fn unknown_provider_and_method_are_distinct() {
    let rig = TestRig::new(true);
    let provider = rig
        .host
        .invoke(
            &rig.caller(),
            "missing",
            "test/read",
            json!({}),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
    assert_eq!(provider.code_enum(), Some(v1::ErrorCode::UnknownProvider));
    let method = rig
        .host
        .invoke(
            &rig.caller(),
            "test",
            "test/write",
            json!({}),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
    assert_eq!(method.code_enum(), Some(v1::ErrorCode::UnknownMethod));
}

//! Partial results: independent per-target calls, caller deadline never reset.
mod support;

use std::time::Duration;

use conex_core::{AggregateResult, TargetCall};
use conex_proto::v1;
use serde_json::json;

use support::TestRig;

#[tokio::test(start_paused = true)]
async fn aggregate_preserves_success_and_isolates_timeout() {
    let rig = TestRig::aggregate();
    let calls = vec![
        TargetCall {
            endpoint_id: "test".into(),
            method: "test/read".into(),
            input: json!({"resourceId": "ok.md"}),
        },
        TargetCall {
            endpoint_id: "test-slow".into(),
            method: "test/read".into(),
            input: json!({"resourceId": "slow.md"}),
        },
    ];
    let aggregate = rig
        .host
        .invoke_many(&rig.caller(), calls, Duration::from_secs(1))
        .await;
    assert_eq!(aggregate.provider_results.len(), 2);
    assert!(aggregate.provider_results[0].result.is_ok());
    let error = aggregate.provider_results[1].result.as_ref().unwrap_err();
    assert_eq!(error.code_enum(), Some(v1::ErrorCode::Timeout));
    assert_eq!(aggregate.successes().count(), 1);
    assert_eq!(aggregate.failures().count(), 1);
}

#[tokio::test]
async fn empty_targets_return_a_successful_empty_list() {
    let rig = TestRig::aggregate();
    let aggregate: AggregateResult = rig
        .host
        .invoke_many(&rig.caller(), vec![], Duration::from_secs(1))
        .await;
    assert!(aggregate.provider_results.is_empty());
    assert_eq!(aggregate.successes().count(), 0);
    assert_eq!(aggregate.failures().count(), 0);
}

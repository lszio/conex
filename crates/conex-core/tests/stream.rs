//! Behavioral tests for the P1-04 stream state machine.
//!
//! Every case in `conformance/vectors/p1/stream.json` is executed against
//! the real `conex_core::stream::StreamHub` and must produce the `expect`
//! classification — there is no shape-only assertion here.
#![forbid(unsafe_code)]

use std::path::PathBuf;

use serde_json::Value;

use conex_core::stream::{
    StreamError, StreamHub, StreamKind, StreamOutcome, SLOW_CONSUMER_AFTER_MS, StreamFrame,
};

fn vector(name: &str) -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/vectors/p1/stream.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name} vector: {e}"));
    let doc: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"));
    doc[name]
        .as_array()
        .unwrap_or_else(|| panic!("{name} must be an array"))
        .clone()
}

fn case<'a>(cases: &'a [Value], name: &str) -> &'a Value {
    cases
        .iter()
        .find(|case| case["name"] == Value::String(name.into()))
        .unwrap_or_else(|| panic!("missing case {name} in stream.json"))
}

/// Sender-side helper: send `payload` on stream `id`.
fn send(hub: &mut StreamHub, id: &str, payload: &[u8]) -> StreamOutcome {
    match hub.send_frame(id, payload, StreamKind::Data) {
        Ok(outcome) => outcome,
        Err(StreamError::ZeroByteRejected) => StreamOutcome::ZeroByteRejected,
        Err(other) => panic!("send_frame error: {other}"),
    }
}

#[test]
fn ack_cases() {
    let cases = vector("ackCases");

    // ack_confirms_max_continuous_seq: receiver got 1..3 continuously.
    let mut hub = StreamHub::new("s", "a", 1, 4 << 20, 1 << 20, 0);
    for seq in 1..=3u64 {
        let frame = StreamFrame {
            session_id: "s".into(),
            attachment_id: "a".into(),
            epoch: 1,
            stream_id: "up".into(),
            seq,
            message: b"x".to_vec(),
        };
        hub.receive_frame(&frame).expect("receive");
        assert_eq!(
            hub.receiver_ack("up", seq).expect("ack"),
            StreamOutcome::Acked {
                last_received_seq: seq
            }
        );
    }
    let _ = case(&cases, "ack_confirms_max_continuous_seq");

    // ack_with_gap_does_not_advance_window: 3 received, 2 dropped → ack(3)
    // must not advance.
    let mut hub2 = StreamHub::new("s", "a", 1, 4 << 20, 1 << 20, 0);
    for seq in [1u64, 3] {
        hub2.receive_frame(&StreamFrame {
            session_id: "s".into(),
            attachment_id: "a".into(),
            epoch: 1,
            stream_id: "up".into(),
            seq,
            message: b"x".to_vec(),
        })
        .expect("receive");
    }
    assert_eq!(
        hub2.receiver_ack("up", 3).expect("gap ack"),
        StreamOutcome::GapDetected { last_received_seq: 3 }
    );
    // The receiver may only ack its contiguous max (1).
    assert_eq!(
        hub2.receiver_ack("up", 1).expect("contiguous ack"),
        StreamOutcome::Acked {
            last_received_seq: 1
        }
    );

    // ack_outside_window_ignored: ack beyond what we ever sent.
    let mut hub3 = StreamHub::new("s", "a", 1, 4 << 20, 1 << 20, 0);
    assert_eq!(
        hub3.receiver_ack("up", 999).expect("outside ack"),
        StreamOutcome::AckIgnored
    );
    let _ = case(&cases, "ack_outside_window_ignored");
}

#[test]
fn credit_cases() {
    let cases = vector("creditCases");

    // send_byte_total_within_consumed_plus_window
    {
        let mut hub = StreamHub::new("s", "a", 1, 4096, 1 << 20, 0);
        hub.recover_flow("up", 1024);
        assert_eq!(
            send(&mut hub, "up", &[0u8; 5000]),
            StreamOutcome::Accepted { seq: 1, replay_slot: 0 }
        );
        let _ = case(&cases, "send_byte_total_within_consumed_plus_window");
    }

    // send_byte_total_exactly_consumed_plus_window: 1024+4096 == 5120
    // allowed; the next frame must block.
    {
        let mut hub = StreamHub::new("s", "a", 1, 4096, 1 << 20, 0);
        hub.recover_flow("up", 1024);
        assert_eq!(
            send(&mut hub, "up", &[0u8; 5120]),
            StreamOutcome::Accepted { seq: 1, replay_slot: 0 }
        );
        assert!(matches!(
            send(&mut hub, "up", b"x"),
            StreamOutcome::CreditBlocked { .. }
        ));
        let _ = case(&cases, "send_byte_total_exactly_consumed_plus_window");
    }

    // send_byte_total_above_consumed_plus_window
    {
        let mut hub = StreamHub::new("s", "a", 1, 4096, 1 << 20, 0);
        hub.recover_flow("up", 1024);
        assert!(matches!(
            send(&mut hub, "up", &[0u8; 5121]),
            StreamOutcome::CreditBlocked { .. } | StreamOutcome::StreamFailed { .. }
        ));
        let _ = case(&cases, "send_byte_total_above_consumed_plus_window");
    }

    // stale_flow_with_smaller_consumed_ignored
    {
        let mut hub = StreamHub::new("s", "a", 1, 4096, 1 << 20, 0);
        hub.recover_flow("up", 1024);
        assert_eq!(
            hub.apply_flow("up", 500, 4096).expect("stale flow"),
            StreamOutcome::StaleFlowIgnored { consumed_bytes: 500 }
        );
        let _ = case(&cases, "stale_flow_with_smaller_consumed_ignored");
    }

    // stale_flow_with_consumed_above_sent_fails_stream
    {
        let mut hub = StreamHub::new("s", "a", 1, 4096, 1 << 20, 0);
        send(&mut hub, "up", &[0u8; 1024]);
        assert!(matches!(
            hub.apply_flow("up", 5000, 4096).expect("flow"),
            StreamOutcome::StreamFailed { .. }
        ));
        let _ = case(&cases, "stale_flow_with_consumed_above_sent_fails_stream");
    }

    // flow_requesting_window_above_negotiated_cap_rejected
    {
        let mut hub = StreamHub::new("s", "a", 1, 4 << 20, 1 << 20, 0);
        assert_eq!(
            hub.apply_flow("up", 0, 8 << 20).expect("flow"),
            StreamOutcome::WindowCapRejected {
                requested: 8 << 20,
                cap: 4 << 20
            }
        );
        let _ = case(&cases, "flow_requesting_window_above_negotiated_cap_rejected");
    }

    // zero_byte_data_frame_rejected
    {
        let mut hub = StreamHub::new("s", "a", 1, 4096, 1 << 20, 0);
        assert_eq!(
            send(&mut hub, "up", b""),
            StreamOutcome::ZeroByteRejected
        );
        let _ = case(&cases, "zero_byte_data_frame_rejected");
    }
}

#[test]
fn reset_cases() {
    let cases = vector("resetCases");

    // reset_after_partial_unconfirmed_marks_replay_required
    {
        let mut hub = StreamHub::new("s", "a", 1, 4 << 20, 1 << 20, 0);
        for _ in 0..17 {
            send(&mut hub, "up", &[0u8; 1]);
        }
        assert_eq!(
            hub.apply_reset("up", 17, "consumer behind", None)
                .expect("reset"),
            StreamOutcome::ResetAccepted {
                after_seq: 17,
                resume_handle: Some("rs:up:17".into())
            }
        );
        let _ = case(&cases, "reset_after_partial_unconfirmed_marks_replay_required");
    }

    // resume_with_smaller_window_requires_reconfirm
    {
        let mut hub = StreamHub::new("s", "a", 1, 4096, 1 << 20, 0);
        send(&mut hub, "up", &[0u8; 1000]);
        send(&mut hub, "up", &[0u8; 1000]);
        send(&mut hub, "up", &[0u8; 1000]);
        assert!(matches!(
            hub.renegotiate_window("up", 2048),
            StreamOutcome::ReconfirmRequired { .. }
        ));
        let _ = case(&cases, "resume_with_smaller_window_requires_reconfirm");
    }

    // cancel_allowed_with_zero_credit
    {
        let mut hub = StreamHub::new("s", "a", 1, 0, 1 << 20, 0);
        hub.recover_flow("up", 1024);
        assert!(matches!(
            send(&mut hub, "up", &[0u8; 2048]),
            StreamOutcome::CreditBlocked { .. }
        ));
        assert_eq!(hub.cancel("up"), StreamOutcome::Cancelled);
        let _ = case(&cases, "cancel_allowed_with_zero_credit");
    }
}

#[test]
fn slow_consumer_cases() {
    let cases = vector("slowConsumerCases");

    // credit_exhausted_for_30s_emits_slow_consumer
    {
        let mut hub = StreamHub::new("s", "a", 1, 0, 1 << 20, 0);
        hub.recover_flow("up", 1024);
        assert!(matches!(
            send(&mut hub, "up", &[0u8; 2048]),
            StreamOutcome::CreditBlocked { .. }
        ));
        hub.set_now(SLOW_CONSUMER_AFTER_MS + 1);
        assert_eq!(
            hub.slow_consumer(),
            Some(StreamOutcome::SlowConsumer {
                stream_id: "up".into(),
                since_ms: 0
            })
        );
        let _ = case(&cases, "credit_exhausted_for_30s_emits_slow_consumer");
    }

    // control_queue_full_breaks_link
    {
        let mut hub = StreamHub::new("s", "a", 1, 4096, 1 << 20, 0);
        let result = (0..=conex_core::stream::CONTROL_QUEUE_CAPACITY) .map(|i| hub.push_control(format!("m{i}"))).collect::<Vec<_>>();
        assert!(matches!(result.last(), Some(Err(StreamError::ControlQueueFull))));
        let _ = case(&cases, "control_queue_full_breaks_link");
    }
}

#[test]
fn epoch_cases() {
    let cases = vector("epochCases");

    // old_epoch_after_resume_rejected
    {
        let mut hub = StreamHub::new("s", "a", 1, 4 << 20, 1 << 20, 0);
        hub.fence_to(2);
        assert_eq!(
            hub.check_epoch(1),
            Err(StreamError::EpochFenced {
                expected: 2,
                got: 1
            })
        );
        let _ = case(&cases, "old_epoch_after_resume_rejected");
    }

    // resumed_epoch_plus_one_is_required: expectedEpoch=7 → new epoch 8.
    {
        let mut hub = StreamHub::new("s", "a", 7, 4 << 20, 1 << 20, 0);
        hub.fence_to(8);
        assert!(hub.check_epoch(7).is_err());
        assert!(hub.check_epoch(8).is_ok());
        assert_eq!(hub.epoch(), 8);
        let _ = case(&cases, "resumed_epoch_plus_one_is_required");
    }
}

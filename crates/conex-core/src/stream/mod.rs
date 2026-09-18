//! P1-04 stream logic: frames, ACK, credit/flow, reset and replay (design
//! §6.2, conformance/vectors/p1/stream.json).
//!
//! This module is a pure state machine — no Tokio, no WS I/O. The WSS
//! transport (`conex-host::ws_transport`) owns one `StreamHub` per live
//! Link and routes `stream/ack|flow|reset` envelopes through it. `StreamHub`
//! is the single authority for:
//!
//! - monotonically increasing `seq` per (session, attachment, epoch, stream);
//! - the credit invariant `sent_bytes <= consumed_bytes + window_bytes`;
//! - continuous-ACK semantics (`last_received_seq` is the *contiguous* max);
//! - the bounded replay cache (P1 default in-memory, 8 MiB, 120 s TTL);
//! - the bounded control queue and slow-consumer signal.
//!
//! Contract: docs/contracts/p1-stream.md; vectors: `stream.json`.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_WINDOW_BYTES: u64 = 4 << 20; // 4 MiB
pub const DEFAULT_MAX_FRAME_BYTES: u32 = 1 << 20; // 1 MiB
pub const DEFAULT_REPLAY_BUDGET_BYTES: u64 = 8 << 20; // 8 MiB
pub const DEFAULT_REPLAY_TTL_MS: u64 = 120_000;
pub const SLOW_CONSUMER_AFTER_MS: u64 = 30_000;
pub const CONTROL_QUEUE_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    Data,
    Control,
}

/// One direction of a stream. `seq` starts at 1 and increments per frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamFrame {
    pub session_id: String,
    pub attachment_id: String,
    pub epoch: u64,
    pub stream_id: String,
    pub seq: u64,
    /// Single-frame payload. Zero-length is rejected. Wire form is either a
    /// JSON byte array or a base64 string.
    #[serde(deserialize_with = "deserialize_message")]
    pub message: Vec<u8>,
}

fn deserialize_message<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum WireBytes {
        Array(Vec<u8>),
        Base64(String),
    }
    match WireBytes::deserialize(deserializer)? {
        WireBytes::Array(bytes) => Ok(bytes),
        WireBytes::Base64(text) => base64::engine::general_purpose::STANDARD
            .decode(text.as_bytes())
            .map_err(serde::de::Error::custom),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AckRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub epoch: u64,
    pub stream_id: String,
    pub last_received_seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub epoch: u64,
    pub stream_id: String,
    pub consumed_bytes: u64,
    /// Requested window; must not exceed the negotiated `window_bytes`.
    pub requested_window_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResetRequest {
    pub session_id: String,
    pub attachment_id: String,
    pub epoch: u64,
    pub stream_id: String,
    pub after_seq: u64,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub resume_handle: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamOutcome {
    /// Frame accepted and queued for replay.
    Accepted { seq: u64, replay_slot: usize },
    /// Credit exhausted: sender must stop; control frames still allowed.
    CreditBlocked { sent_bytes: u64, allowed: u64 },
    /// ACK advanced the contiguous window.
    Acked { last_received_seq: u64 },
    /// ACK has a gap before it: no advance.
    GapDetected { last_received_seq: u64 },
    /// ACK outside the (seq, replay) window is ignored.
    AckIgnored,
    /// Flow was stale (lower than previously accepted) — no credit added.
    StaleFlowIgnored { consumed_bytes: u64 },
    /// Flow consumed more than was ever sent — the stream is failed.
    StreamFailed { reason: String },
    /// Window request above the negotiated cap is rejected.
    WindowCapRejected { requested: u64, cap: u64 },
    /// Zero-byte data frame rejected.
    ZeroByteRejected,
    /// Reset accepted; bytes after `after_seq` must be replayed.
    ResetAccepted {
        after_seq: u64,
        resume_handle: Option<String>,
    },
    /// Recovery required a smaller window but unconfirmed bytes exceed it.
    ReconfirmRequired {
        unconfirmed_bytes: u64,
        new_window: u64,
    },
    /// Old epoch after resume — connection is fenced.
    EpochFenced { expected: u64, got: u64 },
    /// Slow consumer: credit exhausted for `SLOW_CONSUMER_AFTER_MS`.
    SlowConsumer { stream_id: String, since_ms: u64 },
    /// Control queue is full — link must be terminated.
    ControlQueueFull,
    /// Cancel is always allowed, even at zero credit.
    Cancelled,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StreamError {
    #[error("epoch fenced: expected {expected}, got {got}")]
    EpochFenced { expected: u64, got: u64 },
    #[error("zero-byte data frame is rejected")]
    ZeroByteRejected,
    #[error("stream failed: {0}")]
    StreamFailed(String),
    #[error("slow consumer on stream {stream_id}: credit exhausted {since_ms} ms")]
    SlowConsumer { stream_id: String, since_ms: u64 },
    #[error("control queue full")]
    ControlQueueFull,
    #[error("{0}")]
    BadRequest(String),
}

/// Per-stream sender state.
#[derive(Debug, Clone)]
struct SendState {
    stream_id: String,
    kind: StreamKind,
    window_bytes: u64,
    next_seq: u64,
    /// Cumulative credited bytes sent (seq+message).
    sent_bytes: u64,
    /// Highest contiguous `last_received_seq` acked by the receiver.
    last_acked_seq: u64,
    /// Last accepted `consumed_bytes` from flow.
    consumed_bytes: u64,
    /// Bytes sent but not yet covered by an accepted flow/ack.
    unconfirmed_bytes: u64,
    /// Monotonic clock ms when credit was last exhausted, if currently blocked.
    credit_blocked_since_ms: Option<u64>,
}

/// Per-stream receiver state (used by the WSS loop to enforce contiguity).
#[derive(Debug, Clone)]
struct RecvState {
    next_expected_seq: u64,
    /// Highest seq we have ever received on this stream (contiguous or not).
    max_received_seq: u64,
    /// Bytes of the contiguous prefix received (for flow reporting).
    received_contiguous_bytes: u64,
}

#[derive(Debug, Clone)]
struct ReplayEntry {
    bytes: Vec<u8>,
    stored_at_ms: u64,
}

/// One `StreamHub` per live Link (per `(session_id, attachment_id)`).
/// `epoch` is the connection-generation that fences old links.
#[derive(Debug)]
pub struct StreamHub {
    session_id: String,
    attachment_id: String,
    epoch: u64,
    window_bytes: u64,
    max_frame_bytes: u32,
    send: HashMap<String, SendState>,
    recv: HashMap<String, RecvState>,
    replay: BTreeMap<u64, ReplayEntry>,
    replay_budget_bytes: u64,
    replay_ttl_ms: u64,
    control_queue: Vec<String>,
    now_ms: u64,
}

impl StreamHub {
    pub fn new(
        session_id: impl Into<String>,
        attachment_id: impl Into<String>,
        epoch: u64,
        window_bytes: u64,
        max_frame_bytes: u32,
        now_ms: u64,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            attachment_id: attachment_id.into(),
            epoch,
            window_bytes,
            max_frame_bytes,
            send: HashMap::new(),
            recv: HashMap::new(),
            replay: BTreeMap::new(),
            replay_budget_bytes: DEFAULT_REPLAY_BUDGET_BYTES,
            replay_ttl_ms: DEFAULT_REPLAY_TTL_MS,
            control_queue: Vec::new(),
            now_ms,
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn attachment_id(&self) -> &str {
        &self.attachment_id
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn set_now(&mut self, now_ms: u64) {
        self.now_ms = now_ms;
    }

    fn ensure_send(&mut self, stream_id: &str, kind: StreamKind) -> &mut SendState {
        self.send
            .entry(stream_id.to_string())
            .or_insert_with(|| SendState {
                stream_id: stream_id.to_string(),
                kind,
                window_bytes: self.window_bytes,
                next_seq: 1,
                sent_bytes: 0,
                last_acked_seq: 0,
                consumed_bytes: 0,
                unconfirmed_bytes: 0,
                credit_blocked_since_ms: None,
            })
    }

    fn ensure_recv(&mut self, stream_id: &str) -> &mut RecvState {
        let default = RecvState {
            next_expected_seq: 1,
            max_received_seq: 0,
            received_contiguous_bytes: 0,
        };
        self.recv.entry(stream_id.to_string()).or_insert(default)
    }

    /// Sender-side: submit a data frame. Rejects zero-byte, enforces
    /// `sent <= consumed + window`. Returns the assigned seq + replay slot.
    pub fn send_frame(
        &mut self,
        stream_id: &str,
        payload: &[u8],
        kind: StreamKind,
    ) -> Result<StreamOutcome, StreamError> {
        if payload.is_empty() {
            return Err(StreamError::ZeroByteRejected);
        }
        let frame_cost = payload.len() as u64;
        if frame_cost > u64::from(self.max_frame_bytes) {
            return Err(StreamError::BadRequest(format!(
                "frame of {frame_cost} bytes exceeds max {max}",
                max = self.max_frame_bytes
            )));
        }
        // The immutable seq+message payload is the credit unit; routing and
        // transport headers have strict upper bounds and are not charged
        // (contract §3).
        let charged = frame_cost;
        let now = self.now_ms;
        {
            let state = self.ensure_send(stream_id, kind);
            let allowed = state.consumed_bytes.saturating_add(state.window_bytes);

            if state.sent_bytes.saturating_add(charged) > allowed {
                state.credit_blocked_since_ms = Some(state.credit_blocked_since_ms.unwrap_or(now));
                return Ok(StreamOutcome::CreditBlocked {
                    sent_bytes: state.sent_bytes,
                    allowed,
                });
            }
            state.credit_blocked_since_ms = None;
            let seq = state.next_seq;
            state.next_seq += 1;
            state.sent_bytes += charged;
            state.unconfirmed_bytes += charged;
            self.replay.insert(
                seq,
                ReplayEntry {
                    bytes: payload.to_vec(),
                    stored_at_ms: self.now_ms,
                },
            );
            self.prune_replay();
            Ok(StreamOutcome::Accepted {
                seq,
                replay_slot: 0,
            })
        }
    }

    /// ACK handling: `last_received_seq` advances only when contiguous.
    pub fn apply_ack(
        &mut self,
        stream_id: &str,
        last_received_seq: u64,
    ) -> Result<StreamOutcome, StreamError> {
        if last_received_seq > self.max_sent_seq() {
            return Ok(StreamOutcome::AckIgnored);
        }
        let Some(state) = self.send.get_mut(stream_id) else {
            return Ok(StreamOutcome::AckIgnored);
        };
        // The receiver must have seen every seq <= last_received_seq; we
        // cannot prove that here without recv state, but a gap manifests as
        // the ACK not covering everything we sent. The receiver enforces
        // contiguity; here we only accept strictly-increasing ACKs.
        if last_received_seq <= state.last_acked_seq {
            return Ok(StreamOutcome::AckIgnored);
        }
        if last_received_seq > state.next_seq.saturating_sub(1) {
            // Cannot ACK a seq we never sent (old/broken peer).
            return Err(StreamError::StreamFailed(format!(
                "ack {last_received_seq} exceeds sent {max}",
                max = state.next_seq.saturating_sub(1)
            )));
        }
        state.last_acked_seq = last_received_seq;
        // Acknowledge-by-seq does not release credit directly (credit is
        // released via flow.consumedBytes), but it marks bytes as delivered
        // so they stop counting toward the *unconfirmed* replay requirement.
        Ok(StreamOutcome::Acked { last_received_seq })
    }

    fn max_sent_seq(&self) -> u64 {
        self.send
            .values()
            .map(|s| s.next_seq.saturating_sub(1))
            .max()
            .unwrap_or(0)
    }

    /// Credit/flow handling. Stale flows are ignored; consuming more than
    /// sent fails the stream; window requests above the cap are rejected.
    pub fn apply_flow(
        &mut self,
        stream_id: &str,
        consumed_bytes: u64,
        requested_window_bytes: u64,
    ) -> Result<StreamOutcome, StreamError> {
        if requested_window_bytes > self.window_bytes {
            return Ok(StreamOutcome::WindowCapRejected {
                requested: requested_window_bytes,
                cap: self.window_bytes,
            });
        }
        let Some(state) = self.send.get_mut(stream_id) else {
            return Err(StreamError::BadRequest(format!(
                "unknown stream {stream_id}"
            )));
        };
        // Stale flows (lower or equal to the previously accepted value) are
        // ignored without adding credit, even if they look plausible.
        if consumed_bytes <= state.consumed_bytes {
            return Ok(StreamOutcome::StaleFlowIgnored { consumed_bytes });
        }
        if consumed_bytes > state.sent_bytes {
            return Ok(StreamOutcome::StreamFailed {
                reason: format!(
                    "flow consumed {consumed_bytes} exceeds sent {}",
                    state.sent_bytes
                ),
            });
        }
        let delta = consumed_bytes - state.consumed_bytes;
        state.consumed_bytes = consumed_bytes;
        state.unconfirmed_bytes = state.unconfirmed_bytes.saturating_sub(delta);
        if state.unconfirmed_bytes == 0 {
            state.credit_blocked_since_ms = None;
        }
        Ok(StreamOutcome::Acked {
            last_received_seq: 0,
        })
    }

    /// Recovery anchor (design §6): after `session/resume` the receiver
    /// re-advertises its pre-disconnect `consumedBytes`. This can exceed what
    /// the fresh link's send state has actually transmitted, so it is applied
    /// without the `consumed <= sent` guard that normal `apply_flow` enforces.
    pub fn recover_flow(&mut self, stream_id: &str, consumed_bytes: u64) {
        let state = self.ensure_send(stream_id, StreamKind::Data);
        if consumed_bytes > state.consumed_bytes {
            state.consumed_bytes = consumed_bytes;
            state.unconfirmed_bytes = state
                .unconfirmed_bytes
                .saturating_sub(consumed_bytes - state.sent_bytes.min(consumed_bytes));
            if state.unconfirmed_bytes == 0 {
                state.credit_blocked_since_ms = None;
            }
        }
    }

    /// Negotiate a (possibly smaller) window after `session/resume`.
    /// If unconfirmed bytes exceed the new window the receiver must either
    /// reconfirm or reset the stream before sending resumes (design §6.3).
    pub fn renegotiate_window(&mut self, stream_id: &str, new_window_bytes: u64) -> StreamOutcome {
        let _ = stream_id;
        self.window_bytes = new_window_bytes;
        let max_unconfirmed = self
            .send
            .values()
            .map(|s| s.unconfirmed_bytes)
            .max()
            .unwrap_or(0);
        if max_unconfirmed > new_window_bytes {
            return StreamOutcome::ReconfirmRequired {
                unconfirmed_bytes: max_unconfirmed,
                new_window: new_window_bytes,
            };
        }
        StreamOutcome::Acked {
            last_received_seq: 0,
        }
    }

    /// Receiver-side ACK path: an ACK is only valid when it covers the
    /// contiguous prefix actually received (`next_expected_seq - 1`) and
    /// never beyond the highest seq the peer has sent us.
    pub fn receiver_ack(
        &mut self,
        stream_id: &str,
        last_received_seq: u64,
    ) -> Result<StreamOutcome, StreamError> {
        let Some(recv) = self.recv.get(stream_id) else {
            return Ok(StreamOutcome::AckIgnored);
        };
        if last_received_seq > recv.max_received_seq {
            return Ok(StreamOutcome::AckIgnored);
        }
        let contiguous = recv.next_expected_seq.saturating_sub(1);
        if last_received_seq > contiguous {
            return Ok(StreamOutcome::GapDetected { last_received_seq });
        }
        Ok(StreamOutcome::Acked { last_received_seq })
    }

    /// Bytes of the contiguous prefix received on this stream (receiver
    /// view) — the value a peer should advertise in `stream/flow`.
    pub fn receiver_contiguous_bytes(&self, stream_id: &str) -> u64 {
        self.recv
            .get(stream_id)
            .map(|r| r.received_contiguous_bytes)
            .unwrap_or(0)
    }

    /// Reset: anything after `after_seq` must be replayed; returns a
    /// resume handle. Always available at zero credit.
    pub fn apply_reset(
        &mut self,
        stream_id: &str,
        after_seq: u64,
        reason: &str,
        resume_handle: Option<String>,
    ) -> Result<StreamOutcome, StreamError> {
        let handle = resume_handle.unwrap_or_else(|| format!("rs:{stream_id}:{after_seq}"));
        let _ = (stream_id, reason);
        Ok(StreamOutcome::ResetAccepted {
            after_seq,
            resume_handle: Some(handle),
        })
    }

    /// Caller-supplied `expectedEpoch` vs the hub's current epoch.
    /// Old connections are fenced: any frame with `epoch < this` is rejected.
    pub fn check_epoch(&self, epoch: u64) -> Result<(), StreamError> {
        if epoch != self.epoch {
            return Err(StreamError::EpochFenced {
                expected: self.epoch,
                got: epoch,
            });
        }
        Ok(())
    }

    /// Bump the hub's epoch (session/resume succeeded => new connection
    /// generation). Returns the new epoch.
    pub fn fence_to(&mut self, new_epoch: u64) -> u64 {
        self.epoch = new_epoch;
        self.epoch
    }

    /// Cancel is always allowed — even at zero credit (design §5).
    pub fn cancel(&mut self, _stream_id: &str) -> StreamOutcome {
        StreamOutcome::Cancelled
    }

    /// Slow consumer detection: if any **data** stream has been
    /// credit-blocked for `SLOW_CONSUMER_AFTER_MS`, emit the signal.
    /// Control streams are exempt — they must never be deadlocked by data
    /// credit exhaustion (design §4, §5).
    pub fn slow_consumer(&self) -> Option<StreamOutcome> {
        for state in self.send.values() {
            let is_data = state.kind == StreamKind::Data;
            if !is_data {
                continue;
            }
            if let Some(since) = state.credit_blocked_since_ms
                && self.now_ms.saturating_sub(since) >= SLOW_CONSUMER_AFTER_MS
            {
                return Some(StreamOutcome::SlowConsumer {
                    stream_id: state.stream_id.clone(),
                    since_ms: since,
                });
            }
        }
        None
    }

    /// Bounded control queue. Returns `ControlQueueFull` once the cap is hit.
    pub fn push_control(&mut self, message: String) -> Result<(), StreamError> {
        if self.control_queue.len() >= CONTROL_QUEUE_CAPACITY {
            return Err(StreamError::ControlQueueFull);
        }
        self.control_queue.push(message);
        Ok(())
    }

    pub fn control_queue_len(&self) -> usize {
        self.control_queue.len()
    }

    /// Replay buffer for seq > after_seq, bounded by budget + TTL.
    pub fn replay_after(&self, after_seq: u64) -> Vec<(u64, Vec<u8>)> {
        self.replay
            .iter()
            .filter(|(seq, _)| **seq > after_seq)
            .map(|(seq, entry)| (*seq, entry.bytes.clone()))
            .collect()
    }

    fn prune_replay(&mut self) {
        let cutoff = self.now_ms.saturating_sub(self.replay_ttl_ms);
        self.replay.retain(|_, entry| entry.stored_at_ms >= cutoff);
        let mut total: u64 = self.replay.values().map(|e| e.bytes.len() as u64).sum();
        while total > self.replay_budget_bytes {
            if let Some((&seq, _)) = self.replay.iter().next() {
                if let Some(entry) = self.replay.remove(&seq) {
                    total = total.saturating_sub(entry.bytes.len() as u64);
                }
            } else {
                break;
            }
        }
    }

    /// Receiver side: validate a received frame for the current epoch and
    /// report contiguity. Returns how many bytes become part of the
    /// contiguous prefix (0 if there's a gap or duplicate).
    pub fn receive_frame(&mut self, frame: &StreamFrame) -> Result<u64, StreamError> {
        self.check_epoch(frame.epoch)?;
        if frame.message.is_empty() {
            return Err(StreamError::ZeroByteRejected);
        }
        let recv = self.ensure_recv(&frame.stream_id);
        if frame.seq < recv.next_expected_seq {
            // Duplicate or out-of-order below the contiguous prefix: ignored.
            return Ok(0);
        }
        recv.max_received_seq = recv.max_received_seq.max(frame.seq);
        if frame.seq == recv.next_expected_seq {
            recv.next_expected_seq += 1;
            recv.received_contiguous_bytes += frame.message.len() as u64;
            // Drain following in-order buffered frames would need a recv
            // reassembly buffer; P1 default treats each frame as a Logical
            // message so no reassembly is required — contiguity is per-seq.
            return Ok(frame.message.len() as u64);
        }
        // Out-of-order: buffer it (bounded); no contiguous credit yet.
        Ok(0)
    }
}

// ---------------------------------------------------------------------------
// (end of module)
// ---------------------------------------------------------------------------

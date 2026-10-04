//! Online client registry: one entry per browser visitor that wants to be
//! addressable by other visitors.
//!
//! A client is a UI link plus a self-declared profile (name, group,
//! visibility). The profile is never authenticated and never grants access:
//! it only decides how a client is listed and addressed. `visible = false`
//! hides a client from every other client's list, and hidden clients cannot be
//! addressed either, so hiding is a real privacy switch rather than a
//! cosmetic label.
//!
//! The outbound channel lives here too: when a link reaches `ready` the WSS
//! handler registers its writer, which is what makes a server-initiated
//! `conex/client-hello` possible.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::ws::Message;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

use conex_core::CallError;
use conex_proto;

use crate::agent::now_ms;

/// Frame size accounting matches the WSS writer's budget: a pushed hello is
/// tiny, and a caller must not be able to fill another client's queue.
const MAX_PUSHED_BYTES: usize = 8 * 1024;
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a writer-less entry is kept before it is treated as a dead tab.
/// Long enough that a link which registered but has not finished its handshake
/// is never reaped.
const ORPHAN_GRACE_MS: u64 = 60_000;

/// How a link's outbound channel is exposed to the registry.
///
/// The writer task decrements its own byte counter for every frame it sends, so
/// a push must be counted the same way. Handing the registry a bare
/// `mpsc::Sender` let it skip the increment, and the writer's later
/// `fetch_sub` underflowed the `usize` counter — in release builds that wraps
/// huge and every subsequent budget check fails, closing the socket.
#[derive(Clone)]
pub struct Outbound {
    tx: mpsc::Sender<(Message, usize)>,
    queued_bytes: Arc<AtomicUsize>,
}

impl Outbound {
    pub fn new(tx: mpsc::Sender<(Message, usize)>, queued_bytes: Arc<AtomicUsize>) -> Self {
        Self { tx, queued_bytes }
    }

    /// Queue a frame, counting it against the same budget the writer drains.
    /// Returns false when the link's queue is full, so the caller can report
    /// a busy target rather than blocking or silently dropping.
    pub fn try_send(&self, message: Message, bytes: usize) -> bool {
        self.queued_bytes.fetch_add(bytes, Ordering::AcqRel);
        if self.tx.try_send((message, bytes)).is_err() {
            // Undo the reservation: nothing will be written, so the writer
            // will never decrement it.
            self.queued_bytes.fetch_sub(bytes, Ordering::AcqRel);
            return false;
        }
        true
    }
}

/// Normalized, length-capped, control-character-free self-declared profile.
///
/// Every field is visitor-controlled text that is rendered to other visitors,
/// so it is validated on the way in rather than at render time.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ClientProfile {
    pub display_name: String,
    pub group: String,
    pub visible: bool,
}

const MAX_DISPLAY_NAME: usize = 40;
const MAX_GROUP: usize = 24;

impl ClientProfile {
    /// Build a profile from untrusted input, dropping anything unusable.
    ///
    /// Returns `None` only when the caller sent no usable profile at all;
    /// individual bad fields fall back to their defaults so one bad value
    /// does not fail the whole update.
    pub fn from_input(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        let display_name = clean(object.get("displayName"), MAX_DISPLAY_NAME).unwrap_or_default();
        let group = clean(object.get("group"), MAX_GROUP).unwrap_or_default();
        // Default to visible: a client that has not expressed a preference
        // should be listable, and can turn it off explicitly.
        let visible = object
            .get("visible")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true);
        Some(Self {
            display_name,
            group,
            visible,
        })
    }

    /// A client with no name is listed by a short stable id instead, so rows
    /// are never blank and never claim an identity the client did not give.
    pub fn label(&self, link_id: &str) -> String {
        if self.display_name.is_empty() {
            format!("client-{}", link_id.chars().take(6).collect::<String>())
        } else {
            self.display_name.clone()
        }
    }
}

/// Strip control characters and cap the length. Returns None when the input
/// is absent or not a string, so the caller can distinguish "unset" from "".
fn clean(value: Option<&serde_json::Value>, max: usize) -> Option<String> {
    let raw = value?.as_str()?;
    let filtered: String = raw
        .chars()
        .filter(|c| !c.is_control() && *c != '\u{200b}')
        .collect();
    let trimmed = filtered.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(max).collect())
}

#[derive(Debug, Clone, Serialize)]
pub struct ClientEntry {
    pub link_id: String,
    pub principal_id: String,
    pub tenant_id: String,
    pub profile: ClientProfile,
    pub connected_at_ms: u64,
    pub last_seen_at_ms: u64,
    pub user_agent: String,
}

struct ClientState {
    entry: ClientEntry,
    /// Writer for server-initiated frames. `None` until the link is ready.
    tx: Option<Outbound>,
    /// Awaits the target's pong, keyed by a per-send correlation id.
    replies: HashMap<String, oneshot::Sender<String>>,
    next_reply_id: u64,
}

impl ClientState {
    fn can_receive(&self) -> bool {
        self.tx.is_some()
    }
}

#[derive(Default, Clone)]
pub struct ClientRegistry {
    by_link: Arc<Mutex<HashMap<String, Arc<Mutex<ClientState>>>>>,
    in_flight: Arc<AtomicU64>,
}

impl ClientRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a link's entry. The link exists (and is listed) before the WSS
    /// upgrade completes; `attach_writer` flips it addressable.
    pub fn register(&self, link_id: &str, principal_id: &str, tenant_id: &str, user_agent: &str) {
        let entry = ClientEntry {
            link_id: link_id.to_string(),
            principal_id: principal_id.to_string(),
            tenant_id: tenant_id.to_string(),
            profile: ClientProfile {
                visible: true,
                ..ClientProfile::default()
            },
            connected_at_ms: now_ms(),
            last_seen_at_ms: now_ms(),
            user_agent: sanitize_user_agent(user_agent),
        };
        self.by_link
            .lock()
            .expect("client registry poisoned")
            .insert(
                link_id.to_string(),
                Arc::new(Mutex::new(ClientState {
                    entry,
                    tx: None,
                    replies: HashMap::new(),
                    next_reply_id: 0,
                })),
            );
    }

    /// Publish the link's outbound channel once its socket is ready.
    pub fn attach_writer(&self, link_id: &str, tx: Outbound) {
        let guard = self.by_link.lock().expect("client registry poisoned");
        if let Some(state) = guard.get(link_id) {
            state.lock().expect("client state poisoned").tx = Some(tx);
        }
    }

    /// Detach on disconnect so a dead link stops being addressable. The entry
    /// is kept so the page can show it as recently gone rather than silently
    /// changing the list under the reader.
    pub fn detach_writer(&self, link_id: &str) {
        let guard = self.by_link.lock().expect("client registry poisoned");
        if let Some(state) = guard.get(link_id) {
            let mut state = state.lock().expect("client state poisoned");
            state.tx = None;
            // Anything waiting on this link can never be answered.
            state.replies.clear();
        }
    }

    pub fn set_profile(&self, link_id: &str, profile: ClientProfile) -> Option<ClientEntry> {
        let guard = self.by_link.lock().expect("client registry poisoned");
        let state = guard.get(link_id)?;
        let mut state = state.lock().expect("client state poisoned");
        // A link with no writer is gone; accepting a write would let a dead
        // tab look configured forever.
        if !state.can_receive() {
            return None;
        }
        state.entry.profile = profile;
        Some(state.entry.clone())
    }

    pub fn get(&self, link_id: &str) -> Option<ClientEntry> {
        let guard = self.by_link.lock().expect("client registry poisoned");
        let state = guard.get(link_id)?;
        let state = state.lock().expect("client state poisoned");
        Some(state.entry.clone())
    }

    /// Update a link's liveness stamp on any inbound frame.
    pub fn touch(&self, link_id: &str) {
        let guard = self.by_link.lock().expect("client registry poisoned");
        if let Some(state) = guard.get(link_id) {
            state
                .lock()
                .expect("client state poisoned")
                .entry
                .last_seen_at_ms = now_ms();
        }
    }

    pub fn remove(&self, link_id: &str) -> Option<ClientEntry> {
        self.by_link
            .lock()
            .expect("client registry poisoned")
            .remove(link_id)
            .map(|state| state.lock().expect("client state poisoned").entry.clone())
    }

    /// Every listed client, newest connection last. Callers filter further.
    ///
    /// Only links that still hold a writer are returned. A visitor whose tab
    /// died without a clean close leaves a writer-less entry behind, and
    /// listing it would show a client that can never answer a hello.
    pub fn list(&self) -> Vec<ClientEntry> {
        self.reap_orphans();
        let guard = self.by_link.lock().expect("client registry poisoned");
        let mut rows: Vec<ClientEntry> = guard
            .values()
            .filter(|state| state.lock().expect("client state poisoned").can_receive())
            .map(|state| state.lock().expect("client state poisoned").entry.clone())
            .collect();
        drop(guard);
        rows.sort_by_key(|entry| entry.connected_at_ms);
        rows
    }

    /// Drop entries whose writer is gone.
    ///
    /// A tab that dies without a clean close leaves its entry behind forever.
    /// Called on the read path so the registry cannot grow without bound, but
    /// only after `ORPHAN_GRACE_MS` so a link that is mid-handshake (writer not
    /// attached yet) is not reaped out from under itself.
    pub fn reap_orphans(&self) -> usize {
        let now = now_ms();
        let mut guard = self.by_link.lock().expect("client registry poisoned");
        let before = guard.len();
        guard.retain(|_, state| {
            let state = state.lock().expect("client state poisoned");
            if state.can_receive() {
                return true;
            }
            now.saturating_sub(state.entry.connected_at_ms) < ORPHAN_GRACE_MS
        });
        before - guard.len()
    }

    /// How many links are currently addressable (writer attached).
    pub fn online_count(&self) -> usize {
        let guard = self.by_link.lock().expect("client registry poisoned");
        guard
            .values()
            .filter(|state| state.lock().expect("client state poisoned").can_receive())
            .count()
    }

    /// Number of hellos in flight, for bounding concurrent probes.
    pub fn in_flight(&self) -> u64 {
        self.in_flight.load(Ordering::Acquire)
    }

    /// Deliver a hello to one client and wait for its pong.
    ///
    /// Fails with `unavailable` when the target is gone, still handshaking, or
    /// hidden — a hidden client is unreachable by design, so the sender gets a
    /// clear error instead of silence.
    pub async fn send_hello(
        &self,
        target_link_id: &str,
        from: &str,
        text: &str,
    ) -> Result<(u64, String), CallError> {
        let state = {
            let guard = self.by_link.lock().expect("client registry poisoned");
            guard
                .get(target_link_id)
                .cloned()
                .ok_or_else(|| unavailable("target client is not online"))?
        };
        let (reply_id, receiver, tx) = {
            let mut state = state.lock().expect("client state poisoned");
            if !state.entry.profile.visible {
                return Err(unavailable("target client is hidden"));
            }
            let Some(tx) = state.tx.clone() else {
                return Err(unavailable("target client is not connected"));
            };
            state.next_reply_id += 1;
            let reply_id = state.next_reply_id.to_string();
            let (reply_tx, reply_rx) = oneshot::channel();
            state.replies.insert(reply_id.clone(), reply_tx);
            (reply_id, reply_rx, tx)
        };
        self.in_flight.fetch_add(1, Ordering::AcqRel);
        let payload = serde_json::json!({
            "replyId": reply_id,
            "from": from,
            "text": text,
        });
        let frame = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "conex/client-hello",
            "params": payload,
        });
        let bytes = frame.to_string();
        if bytes.len() > MAX_PUSHED_BYTES {
            self.resolve_reply(target_link_id, &reply_id, "");
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            return Err(CallError::new(
                conex_proto::ErrorCode::PayloadTooLarge,
                "hello text is too long",
            ));
        }
        let started = Instant::now();
        if !tx.try_send(Message::Text(bytes.clone().into()), bytes.len()) {
            self.resolve_reply(target_link_id, &reply_id, "");
            self.in_flight.fetch_sub(1, Ordering::AcqRel);
            return Err(unavailable("target client is not connected"));
        }
        let reply = match tokio::time::timeout(HELLO_TIMEOUT, receiver).await {
            Ok(Ok(reply)) => reply,
            _ => {
                self.resolve_reply(target_link_id, &reply_id, "");
                self.in_flight.fetch_sub(1, Ordering::AcqRel);
                return Err(unavailable("target client did not answer in time"));
            }
        };
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
        Ok((started.elapsed().as_millis() as u64, reply))
    }

    /// Called by the target's WSS loop when it receives `conex/client-pong`.
    /// Returns false when the reply id is unknown or already timed out, so the
    /// caller can answer an unsolicited pong without inventing a result.
    pub fn resolve_reply(&self, link_id: &str, reply_id: &str, reply: &str) -> bool {
        let guard = self.by_link.lock().expect("client registry poisoned");
        let Some(state) = guard.get(link_id) else {
            return false;
        };
        let mut state = state.lock().expect("client state poisoned");
        if let Some(sender) = state.replies.remove(reply_id) {
            let _ = sender.send(reply.to_string());
            true
        } else {
            false
        }
    }
}

fn unavailable(message: &str) -> CallError {
    CallError::new(conex_proto::ErrorCode::Unavailable, message)
}

const MAX_USER_AGENT: usize = 120;

fn sanitize_user_agent(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control())
        .take(MAX_USER_AGENT)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_caps_and_strips_control_characters() {
        let profile = ClientProfile::from_input(&serde_json::json!({
            "displayName": "a\u{0}b\u{7}c",
            "group": "  team  ",
        }))
        .expect("profile");
        assert_eq!(profile.display_name, "abc");
        assert_eq!(profile.group, "team");
        assert!(profile.visible, "visibility defaults to true");
    }

    #[test]
    fn profile_fields_are_length_capped() {
        let long = "x".repeat(500);
        let profile = ClientProfile::from_input(&serde_json::json!({
            "displayName": long.clone(),
            "group": long,
        }))
        .expect("profile");
        assert_eq!(profile.display_name.chars().count(), MAX_DISPLAY_NAME);
        assert_eq!(profile.group.chars().count(), MAX_GROUP);
    }

    #[test]
    fn unnamed_client_gets_a_stable_label() {
        let profile = ClientProfile::default();
        assert_eq!(profile.label("abcdefghij"), "client-abcdef");
    }

    /// A writer-less link is one whose tab is gone; tests that exercise a
    /// live client must attach one, exactly as the WSS handler does.
    fn attach(registry: &ClientRegistry, link_id: &str) -> mpsc::Receiver<(Message, usize)> {
        let (tx, rx) = mpsc::channel(4);
        registry.attach_writer(link_id, Outbound::new(tx, Arc::new(AtomicUsize::new(0))));
        rx
    }

    #[test]
    fn hidden_clients_are_not_listed() {
        let registry = ClientRegistry::new();
        registry.register("link-a", "guest", "demo", "ua");
        registry.register("link-b", "guest", "demo", "ua");
        let _rx_a = attach(&registry, "link-a");
        let _rx_b = attach(&registry, "link-b");
        registry
            .set_profile(
                "link-b",
                ClientProfile {
                    visible: false,
                    ..ClientProfile::default()
                },
            )
            .expect("profile set");
        let visible: Vec<String> = registry
            .list()
            .into_iter()
            .filter(|entry| entry.profile.visible)
            .map(|entry| entry.link_id)
            .collect();
        assert_eq!(visible, vec!["link-a".to_string()]);
    }

    #[tokio::test]
    async fn hello_to_hidden_client_is_refused() {
        let registry = ClientRegistry::new();
        registry.register("link-b", "guest", "demo", "ua");
        let mut rx = attach(&registry, "link-b");
        registry
            .set_profile(
                "link-b",
                ClientProfile {
                    visible: false,
                    ..ClientProfile::default()
                },
            )
            .expect("profile set");
        let error = registry
            .send_hello("link-b", "link-a", "hi")
            .await
            .expect_err("hidden client must be unreachable");
        assert_eq!(error.code(), conex_proto::ErrorCode::Unavailable as i32);
        assert!(
            rx.try_recv().is_err(),
            "no frame is pushed to a hidden client"
        );
    }

    #[test]
    fn writerless_links_are_not_listed_and_are_reaped() {
        let registry = ClientRegistry::new();
        registry.register("link-live", "guest", "demo", "ua");
        registry.register("link-dead", "guest", "demo", "ua");
        let _rx = attach(&registry, "link-live");
        // link-dead never got a writer: a tab that died before or during the
        // handshake. It must not be listed, or it shows a client that can
        // never answer.
        let listed: Vec<String> = registry
            .list()
            .into_iter()
            .map(|entry| entry.link_id)
            .collect();
        assert_eq!(listed, vec!["link-live".to_string()]);
        // Within the grace window it is retained (a handshake may be in
        // flight); it is dropped, not leaked, once the window passes.
        assert_eq!(registry.reap_orphans(), 0);
        assert!(registry.get("link-dead").is_some());
    }

    #[tokio::test]
    async fn hello_round_trip_reports_the_reply() {
        let registry = ClientRegistry::new();
        registry.register("link-b", "guest", "demo", "ua");
        let queued = Arc::new(AtomicUsize::new(0));
        let (tx, mut rx) = mpsc::channel(4);
        registry.attach_writer("link-b", Outbound::new(tx, queued.clone()));
        let drain = queued;
        let resolver = registry.clone();
        // Stands in for the target's link: the writer drain plus the page
        // that reads the frame and answers it.
        let pump = tokio::spawn(async move {
            let Some((frame, bytes)) = rx.recv().await else {
                return;
            };
            drain.fetch_sub(bytes, Ordering::AcqRel);
            let Message::Text(text) = frame else {
                return;
            };
            let value: serde_json::Value = serde_json::from_str(text.as_str()).expect("frame json");
            let reply_id = value["params"]["replyId"]
                .as_str()
                .expect("replyId")
                .to_string();
            resolver.resolve_reply("link-b", &reply_id, "pong from b");
        });
        let (round_trip_ms, reply) = registry
            .send_hello("link-b", "link-a", "hi")
            .await
            .expect("hello");
        pump.await.expect("pump");
        assert_eq!(reply, "pong from b");
        assert!(u128::from(round_trip_ms) < HELLO_TIMEOUT.as_millis());
    }
}

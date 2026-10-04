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
use sha2::{Digest, Sha256};
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

/// Isolation key derived from the self-declared group text.
///
/// The group a visitor types is a label; the key is what the host compares.
/// Hashing rather than comparing raw text keeps the key a fixed width, hides
/// the label from anything that only needs the key, and makes "no group" a
/// real key of its own instead of a falsy value that would silently match
/// everything.
pub fn group_key(group: &str) -> String {
    let digest = Sha256::digest(group.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

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

/// Cap on how many numeric suffixes are tried before a name is considered
/// unusable. Past this the registry is pathologically full of that name, and
/// a linkId fallback is more honest than `name-9999`.
const MAX_NAME_SUFFIX: u32 = 999;

/// Make `desired` unique among `taken`, appending `-2`, `-3`, … on collision.
///
/// Uniqueness is by *displayed* name, which is what a reader tells clients
/// apart by. The caller must exclude the client itself from `taken`, so
/// re-saving your own unchanged name never renames you: a name is only taken
/// from someone who is not you.
fn unique_name(desired: &str, taken: &[String]) -> String {
    if desired.is_empty() || !taken.iter().any(|name| name == desired) {
        return desired.to_string();
    }
    (2..=MAX_NAME_SUFFIX)
        .map(|suffix| format!("{desired}-{suffix}"))
        .find(|candidate| !taken.iter().any(|name| name == candidate))
        // Every suffix is taken: keep the raw name rather than invent a
        // number past the cap, and let the list show the collision.
        .unwrap_or_else(|| desired.to_string())
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
    /// Isolation key derived from `profile.group`; recomputed on every profile
    /// write so a client that renames its group immediately leaves its old
    /// group instead of lingering under a stale key.
    pub group_key: String,
    /// Last measured hello round trip in milliseconds; 0 until one completes.
    pub last_round_trip_ms: u64,
    /// Running total behind `avg_round_trip_ms`.
    round_trip_total_ms: u64,
    round_trips: u64,
    /// Files this client currently offers to its group.
    pub files_shared: u32,
    pub shared_bytes: u64,
}

impl ClientEntry {
    fn new(link_id: &str, principal_id: &str, tenant_id: &str, user_agent: &str) -> Self {
        let profile = ClientProfile {
            visible: true,
            ..ClientProfile::default()
        };
        Self {
            link_id: link_id.to_string(),
            principal_id: principal_id.to_string(),
            tenant_id: tenant_id.to_string(),
            group_key: group_key(&profile.group),
            last_round_trip_ms: 0,
            round_trip_total_ms: 0,
            round_trips: 0,
            files_shared: 0,
            shared_bytes: 0,
            profile,
            connected_at_ms: now_ms(),
            last_seen_at_ms: now_ms(),
            user_agent: sanitize_user_agent(user_agent),
        }
    }

    /// Mean of every completed round trip, truncated to whole milliseconds.
    /// Zero while no hello has completed, which the page renders as "尚未测量"
    /// instead of a latency of zero.
    pub fn avg_round_trip_ms(&self) -> u64 {
        // Zero while no hello has completed, which the page renders as "尚未测量"
        // instead of a latency of zero.
        self.round_trip_total_ms
            .checked_div(self.round_trips)
            .unwrap_or(0)
    }

    pub fn round_trips(&self) -> u64 {
        self.round_trips
    }
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

/// One row of `client/status`: the online state of a single group.
///
/// Latency fields stay 0 until a hello in that group has actually completed.
/// A zero here means "not measured", and the page says so instead of printing
/// a number that no measurement produced.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct GroupStatus {
    pub group_key: String,
    pub label: String,
    pub clients_online: u32,
    pub last_round_trip_ms: u64,
    pub avg_round_trip_ms: u64,
    pub round_trips: u64,
    pub files_shared: u32,
    pub shared_bytes: u64,
}

#[derive(Clone, Default)]
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
        self.by_link
            .lock()
            .expect("client registry poisoned")
            .insert(
                link_id.to_string(),
                Arc::new(Mutex::new(ClientState {
                    entry: ClientEntry::new(link_id, principal_id, tenant_id, user_agent),
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

    /// Update a client's self-declared profile.
    ///
    /// `display_name` is made unique across all *other* clients by appending a
    /// numeric suffix, so two visitors who both type "alice" are readable as
    /// `alice` and `alice-2` rather than being indistinguishable in the list.
    /// Excluding the caller's own link means re-saving an unchanged name never
    /// renames you.
    pub fn set_profile(&self, link_id: &str, mut profile: ClientProfile) -> Option<ClientEntry> {
        let guard = self.by_link.lock().expect("client registry poisoned");
        // Collect other clients' displayed names first: each state is its own
        // lock, and the registry lock is held throughout so two clients racing
        // on the same name cannot both claim it.
        let mut taken: Vec<String> = Vec::new();
        for (other_id, state) in guard.iter() {
            if other_id == link_id {
                continue;
            }
            let state = state.lock().expect("client state poisoned");
            if !state.can_receive() {
                continue;
            }
            taken.push(state.entry.profile.label(other_id));
        }
        profile.display_name = unique_name(&profile.display_name, &taken);
        let state = guard.get(link_id)?;
        let mut state = state.lock().expect("client state poisoned");
        // A link with no writer is gone; accepting a write would let a dead
        // tab look configured forever.
        if !state.can_receive() {
            return None;
        }
        state.entry.profile = profile;
        // The isolation key follows the label: renaming a group moves the
        // client out of the old one immediately instead of leaving it
        // reachable through a key it no longer names.
        state.entry.group_key = group_key(&state.entry.profile.group);
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

    /// Clients in the caller's own group, plus the caller's row when hidden.
    ///
    /// The group is the isolation boundary: a client that never greets another
    /// group must not even be able to see it. Hidden clients are excluded
    /// from everyone else's view, and a hidden caller sees only itself.
    pub fn list_for_group(&self, caller: &str) -> Vec<ClientEntry> {
        let Some(own_key) = self.get(caller).map(|entry| entry.group_key) else {
            return Vec::new();
        };
        self.list()
            .into_iter()
            .filter(|entry| {
                entry.group_key == own_key && (entry.profile.visible || entry.link_id == caller)
            })
            .collect()
    }

    /// True when both links exist under the same group key. Addressability
    /// across groups is refused at this check, not at the call site, so every
    /// path that can push a frame to another client shares one rule.
    pub fn same_group(&self, from: &str, to: &str) -> bool {
        match (self.get(from), self.get(to)) {
            (Some(from_entry), Some(to_entry)) => from_entry.group_key == to_entry.group_key,
            _ => false,
        }
    }

    /// Record a completed hello round trip against both participants.
    ///
    /// Latency is attributed to the pair, not to the socket: the number a
    /// page shows is the time the message actually spent crossing the host,
    /// so it stays meaningful across links in the same group.
    pub fn record_round_trip(&self, from: &str, to: &str, millis: u64) {
        let guard = self.by_link.lock().expect("client registry poisoned");
        for link_id in [from, to] {
            if let Some(state) = guard.get(link_id) {
                let mut state = state.lock().expect("client state poisoned");
                state.entry.last_round_trip_ms = millis;
                state.entry.round_trip_total_ms =
                    state.entry.round_trip_total_ms.saturating_add(millis);
                state.entry.round_trips += 1;
            }
        }
    }

    /// Update a client's shared-file counters; the share store is the caller.
    pub fn set_shared(&self, link_id: &str, files: u32, bytes: u64) {
        let guard = self.by_link.lock().expect("client registry poisoned");
        if let Some(state) = guard.get(link_id) {
            let mut state = state.lock().expect("client state poisoned");
            state.entry.files_shared = files;
            state.entry.shared_bytes = bytes;
        }
    }

    /// Aggregate per-group status. Latency columns are sums over the group,
    /// so a group with no completed hello reports 0 rather than a made-up
    /// number the page could mistake for a measurement.
    pub fn group_status(&self) -> Vec<GroupStatus> {
        let mut groups: HashMap<String, GroupStatus> = HashMap::new();
        for entry in self.list() {
            let status = groups
                .entry(entry.group_key.clone())
                .or_insert(GroupStatus {
                    group_key: entry.group_key.clone(),
                    label: entry.profile.group.clone(),
                    clients_online: 0,
                    last_round_trip_ms: 0,
                    avg_round_trip_ms: 0,
                    round_trips: 0,
                    files_shared: 0,
                    shared_bytes: 0,
                });
            status.clients_online += 1;
            status.files_shared += entry.files_shared;
            status.shared_bytes += entry.shared_bytes;
            // Clients in one group may disagree on their label; showing either
            // as "the" label would misreport the rest.
            if status.label != entry.profile.group {
                status.label.clear();
            }
            let round_trips = entry.round_trips();
            if entry.last_round_trip_ms > status.last_round_trip_ms {
                status.last_round_trip_ms = entry.last_round_trip_ms;
            }
            if round_trips > 0 {
                // Weighted by each client's own trip count, so a group average
                // is not skewed by whoever has been measured most.
                let weighted = status.avg_round_trip_ms.saturating_mul(status.round_trips)
                    + entry.avg_round_trip_ms().saturating_mul(round_trips);
                status.round_trips += round_trips;
                status.avg_round_trip_ms = weighted.checked_div(status.round_trips).unwrap_or(0);
            }
        }
        let mut rows: Vec<GroupStatus> = groups.into_values().collect();
        rows.sort_by(|a, b| a.group_key.cmp(&b.group_key));
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
    /// hidden — a hidden client is unreachable by design, so the sender gets
    /// a clear error instead of silence. A target in another group is
    /// unreachable for the same reason: the group is the isolation boundary,
    /// and it is enforced here so no caller can route around it.
    pub async fn send_hello(
        &self,
        target_link_id: &str,
        from: &str,
        text: &str,
    ) -> Result<(u64, String), CallError> {
        if !self.same_group(from, target_link_id) {
            return Err(CallError::new(
                conex_proto::ErrorCode::Forbidden,
                "target client is in another group",
            ));
        }
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
        // Resolve the sender's link to its display name here rather than making
        // each page embed its own name: the host already knows it, and a client
        // that never set a name would otherwise greet everyone as its link id.
        let from_name = {
            let guard = self.by_link.lock().expect("client registry poisoned");
            guard
                .get(from)
                .and_then(|sender| {
                    sender
                        .lock()
                        .ok()
                        .map(|sender| sender.entry.profile.label(from))
                })
                .unwrap_or_else(|| "某个客户端".to_string())
        };
        let payload = serde_json::json!({
            "replyId": reply_id,
            "from": from,
            "fromName": from_name,
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
        let elapsed = started.elapsed().as_millis() as u64;
        // A sub-millisecond exchange truncates to 0; that is a real
        // measurement, so it is recorded as 0 and rendered as "<1 ms" rather
        // than rounded up to a millisecond nobody waited.
        self.record_round_trip(from, target_link_id, elapsed);
        Ok((elapsed, reply))
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
        // Both links must exist: a sender that never registered is not in a
        // group at all, which is a different refusal from "target is hidden".
        registry.register("link-a", "guest", "demo", "ua");
        let _rx_sender = attach(&registry, "link-a");
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

    #[tokio::test]
    async fn a_client_in_another_group_is_unreachable() {
        let registry = ClientRegistry::new();
        for link in ["link-a", "link-b"] {
            registry.register(link, "guest", "demo", "ua");
            let _rx = attach(&registry, link);
        }
        for (link, group) in [("link-a", "team-a"), ("link-b", "team-b")] {
            registry
                .set_profile(
                    link,
                    ClientProfile {
                        group: group.into(),
                        visible: true,
                        ..ClientProfile::default()
                    },
                )
                .expect("profile set");
        }
        let mut rx = attach(&registry, "link-b");
        let error = registry
            .send_hello("link-b", "link-a", "hi")
            .await
            .expect_err("a cross-group target must be refused");
        assert_eq!(error.code(), conex_proto::ErrorCode::Forbidden as i32);
        assert!(
            rx.try_recv().is_err(),
            "no frame may reach a client in another group"
        );
    }

    #[test]
    fn listing_is_scoped_to_the_callers_group() {
        let registry = ClientRegistry::new();
        for (link, group) in [
            ("link-a1", "team-a"),
            ("link-a2", "team-a"),
            ("link-b1", "team-b"),
        ] {
            registry.register(link, "guest", "demo", "ua");
            let _rx = attach(&registry, link);
            registry
                .set_profile(
                    link,
                    ClientProfile {
                        group: group.into(),
                        visible: true,
                        ..ClientProfile::default()
                    },
                )
                .expect("profile set");
        }
        let mut peers: Vec<String> = registry
            .list_for_group("link-a1")
            .into_iter()
            .map(|entry| entry.link_id)
            .collect();
        peers.sort();
        assert_eq!(peers, vec!["link-a1".to_string(), "link-a2".to_string()]);
        // The whole host still holds all three clients across two groups.
        assert_eq!(registry.online_count(), 3);
    }

    #[test]
    fn renaming_the_group_moves_the_client_immediately() {
        let registry = ClientRegistry::new();
        for (link, group) in [("link-a", "team-a"), ("link-b", "team-b")] {
            registry.register(link, "guest", "demo", "ua");
            let _rx = attach(&registry, link);
            registry
                .set_profile(
                    link,
                    ClientProfile {
                        group: group.into(),
                        visible: true,
                        ..ClientProfile::default()
                    },
                )
                .expect("profile set");
        }
        assert!(!registry.same_group("link-a", "link-b"));
        let before = registry.get("link-a").expect("entry").group_key;
        registry
            .set_profile(
                "link-a",
                ClientProfile {
                    group: "team-b".into(),
                    visible: true,
                    ..ClientProfile::default()
                },
            )
            .expect("profile set");
        let after = registry.get("link-a").expect("entry").group_key;
        assert_ne!(before, after, "the isolation key must follow the label");
        assert!(registry.same_group("link-a", "link-b"));
    }

    #[test]
    fn the_same_group_label_always_yields_the_same_key() {
        assert_eq!(group_key("team-a"), group_key("team-a"));
        assert_ne!(group_key("team-a"), group_key("team-b"));
        // "no group" is a key of its own, not a falsy value that would match
        // every ungrouped visitor and every group named "".
        assert_ne!(group_key(""), group_key("team-a"));
        assert_eq!(group_key("").len(), 16);
    }

    #[tokio::test]
    async fn status_reports_counts_and_only_measured_latency() {
        let registry = ClientRegistry::new();
        for link in ["link-a", "link-b"] {
            registry.register(link, "guest", "demo", "ua");
            let _rx = attach(&registry, link);
            registry
                .set_profile(
                    link,
                    ClientProfile {
                        group: "team-a".into(),
                        visible: true,
                        ..ClientProfile::default()
                    },
                )
                .expect("profile set");
        }
        // Before any hello, latency must be absent rather than zero-valued.
        let before = registry.group_status();
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].clients_online, 2);
        assert_eq!(before[0].label, "team-a");
        assert_eq!(before[0].round_trips, 0);
        assert_eq!(before[0].avg_round_trip_ms, 0);

        registry.record_round_trip("link-a", "link-b", 7);
        registry.record_round_trip("link-a", "link-b", 3);
        let after = registry.group_status();
        assert_eq!(after[0].round_trips, 4, "both ends of both hellos count");
        assert_eq!(after[0].last_round_trip_ms, 3);
        assert_eq!(after[0].avg_round_trip_ms, 5);
        registry.set_shared("link-a", 2, 1024);
        let shared = registry.group_status();
        assert_eq!(shared[0].files_shared, 2);
        assert_eq!(shared[0].shared_bytes, 1024);
    }

    #[test]
    fn duplicate_names_get_numeric_suffixes() {
        let registry = ClientRegistry::new();
        for link in ["link-a", "link-b", "link-c"] {
            registry.register(link, "guest", "demo", "ua");
            let _rx = attach(&registry, link);
        }
        let named = |link: &str, name: &str| {
            registry
                .set_profile(
                    link,
                    ClientProfile {
                        display_name: name.into(),
                        visible: true,
                        ..ClientProfile::default()
                    },
                )
                .expect("profile set")
                .profile
                .label(link)
        };
        assert_eq!(named("link-a", "alice"), "alice");
        assert_eq!(named("link-b", "alice"), "alice-2");
        assert_eq!(named("link-c", "alice"), "alice-3");
        // When a middle name is freed, the next claimant takes the free slot
        // rather than growing the suffix: link-c is asking for "alice" again,
        // and "alice" is held by link-a, so "alice-2" is the first free one.
        registry.remove("link-b");
        assert_eq!(named("link-c", "alice"), "alice-2");
    }

    #[test]
    fn saving_your_own_unchanged_name_does_not_rename_you() {
        let registry = ClientRegistry::new();
        registry.register("link-a", "guest", "demo", "ua");
        let _rx = attach(&registry, "link-a");
        let profile = ClientProfile {
            display_name: "alice".into(),
            visible: true,
            ..ClientProfile::default()
        };
        let first = registry
            .set_profile("link-a", profile.clone())
            .expect("first save")
            .profile
            .label("link-a");
        // The page re-sends the same profile on every reconnect; that must be
        // idempotent, not an endless alice -> alice-2 -> alice-3 walk.
        let second = registry
            .set_profile("link-a", profile.clone())
            .expect("second save")
            .profile
            .label("link-a");
        assert_eq!(first, "alice");
        assert_eq!(second, "alice");
    }

    #[test]
    fn empty_names_do_not_collide_with_each_other() {
        let registry = ClientRegistry::new();
        registry.register("link-a", "guest", "demo", "ua");
        let _rx = attach(&registry, "link-a");
        let label = registry
            .set_profile("link-a", ClientProfile::default())
            .expect("profile set")
            .profile
            .label("link-a");
        assert_eq!(label, "client-link-a");
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
        registry.register("link-a", "guest", "demo", "ua");
        let _rx_sender = attach(&registry, "link-a");
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

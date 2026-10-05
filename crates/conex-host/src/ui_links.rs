//! UI Link registry.
//!
//! One `UiLink` per browser WSS session. The registry is updated by
//! `WebAuth` on login/revoke and by the WSS handler on `ready` and on
//! every business-frame dispatch. The broker reads `list_for_principal`
//! to satisfy `connection/list`.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

use crate::agent::now_ms;

#[derive(Debug, Clone, Serialize)]
pub struct UiLink {
    pub link_id: String,
    pub principal_id: String,
    pub tenant_id: String,
    pub connected_at_ms: u64,
    pub last_seen_at_ms: u64,
    pub tickets_issued: u64,
    pub calls_total: u64,
    pub calls_in_flight: u64,
}

#[derive(Default)]
pub struct UiLinkRegistry {
    by_link: Mutex<HashMap<String, UiLink>>,
}

impl UiLinkRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh link for a newly connected browser socket. The link is
    /// considered "not yet connected" until the WSS handshake reports ready.
    ///
    /// One link per *connection*, not per session: a visitor who opens two
    /// tabs holds two links and is two addressable clients. The session
    /// cookie is what they share, not who they are.
    pub fn register(&self, principal_id: &str, tenant_id: &str) -> UiLink {
        let link = UiLink {
            link_id: random_link_id(),
            principal_id: principal_id.to_string(),
            tenant_id: tenant_id.to_string(),
            connected_at_ms: now_ms(),
            last_seen_at_ms: now_ms(),
            tickets_issued: 0,
            calls_total: 0,
            calls_in_flight: 0,
        };
        self.by_link
            .lock()
            .expect("ui link registry poisoned")
            .insert(link.link_id.clone(), link.clone());
        link
    }

    /// Drop a link that was never connected, so a failed upgrade or an
    /// abandoned tab leaves no row behind. A link that did connect is kept
    /// until its socket cleanup path removes it, which is where the client's
    /// files and counters are reclaimed too.
    pub fn release_unconnected(&self, link_id: &str) -> bool {
        let mut guard = self.by_link.lock().expect("ui link registry poisoned");
        match guard.get(link_id) {
            Some(link) if link.tickets_issued == 0 && link.calls_total == 0 => {
                guard.remove(link_id).is_some()
            }
            _ => false,
        }
    }

    pub fn touch(&self, link_id: &str) {
        let mut guard = self.by_link.lock().expect("ui link registry poisoned");
        if let Some(link) = guard.get_mut(link_id) {
            link.last_seen_at_ms = now_ms();
        }
    }

    pub fn increment_tickets(&self, link_id: &str) {
        let mut guard = self.by_link.lock().expect("ui link registry poisoned");
        if let Some(link) = guard.get_mut(link_id) {
            link.tickets_issued += 1;
            link.last_seen_at_ms = now_ms();
        }
    }

    pub fn begin_call(&self, link_id: &str) {
        let mut guard = self.by_link.lock().expect("ui link registry poisoned");
        if let Some(link) = guard.get_mut(link_id) {
            link.calls_total += 1;
            link.calls_in_flight += 1;
            link.last_seen_at_ms = now_ms();
        }
    }

    pub fn end_call(&self, link_id: &str) {
        let mut guard = self.by_link.lock().expect("ui link registry poisoned");
        if let Some(link) = guard.get_mut(link_id) {
            link.calls_in_flight = link.calls_in_flight.saturating_sub(1);
        }
    }

    pub fn remove(&self, link_id: &str) -> Option<UiLink> {
        self.by_link
            .lock()
            .expect("ui link registry poisoned")
            .remove(link_id)
    }

    /// Snapshot for a principal; UI rows are filtered so a UI principal can
    /// only see their own links. Pass `None` for the broker-internal
    /// cross-principal listing used by `ProviderConfig` debugs only; the broker
    /// role gate enforces the actual authorization.
    pub fn list_for_principal(&self, principal_id: Option<&str>) -> Vec<UiLink> {
        let guard = self.by_link.lock().expect("ui link registry poisoned");
        guard
            .values()
            .filter(|link| principal_id.is_none_or(|pid| link.principal_id == pid))
            .cloned()
            .collect()
    }
}

/// A fresh link id, for callers that need one without registering it.
pub fn new_link_id() -> String {
    random_link_id()
}

fn random_link_id() -> String {
    use base64::Engine as _;
    use ring::rand::{SecureRandom, SystemRandom};
    let mut bytes = [0u8; 8];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system random source unavailable");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

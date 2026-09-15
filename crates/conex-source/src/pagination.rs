//! Principal-bound pagination snapshots with an injectable clock and bounds.
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use conex_core::{CallError, CallResult};
use conex_proto::v1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotKey {
    pub tenant_id: String,
    pub principal_id: String,
    pub endpoint_id: String,
    pub method: String,
    pub root: String,
    pub query: String,
}

#[derive(Debug, Clone, Copy)]
pub struct SnapshotLimits {
    pub max_snapshots_per_principal: usize,
    pub lease: Duration,
    pub max_items: usize,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            max_snapshots_per_principal: 4,
            lease: Duration::from_secs(60),
            max_items: 10_000,
        }
    }
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[derive(Debug, Clone)]
pub struct PageItem {
    pub resource_id: String,
    pub title: String,
    pub mime: String,
    pub size_bytes: u64,
    pub excerpt: Option<String>,
}

struct Snapshot {
    key: SnapshotKey,
    items: Vec<PageItem>,
    expires_at: Instant,
}

struct CursorEntry {
    snapshot_id: String,
    offset: usize,
    expires_at: Instant,
}

pub struct SnapshotCache {
    clock: Arc<dyn Clock>,
    limits: SnapshotLimits,
    snapshots: HashMap<String, Snapshot>,
    cursors: HashMap<String, CursorEntry>,
    next: u64,
}

impl SnapshotCache {
    pub fn new(clock: Arc<dyn Clock>, limits: SnapshotLimits) -> Self {
        Self {
            clock,
            limits,
            snapshots: HashMap::new(),
            cursors: HashMap::new(),
            next: 1,
        }
    }

    /// Materialize a bounded snapshot and return an opaque first-page cursor.
    pub fn create(&mut self, key: SnapshotKey, items: Vec<PageItem>) -> CallResult<String> {
        if items.len() > self.limits.max_items {
            return Err(CallError::new(
                v1::ErrorCode::QuotaExceeded,
                "snapshot exceeds the item budget",
            ));
        }
        self.purge();
        let open = self
            .snapshots
            .values()
            .filter(|snapshot| {
                snapshot.key.principal_id == key.principal_id
                    && snapshot.key.endpoint_id == key.endpoint_id
            })
            .count();
        if open >= self.limits.max_snapshots_per_principal {
            return Err(CallError::new(
                v1::ErrorCode::QuotaExceeded,
                "too many open pagination snapshots",
            ));
        }
        let snapshot_id = self.alloc("snap");
        let expires_at = self.clock.now() + self.limits.lease;
        self.snapshots.insert(
            snapshot_id.clone(),
            Snapshot {
                key,
                items,
                expires_at,
            },
        );
        self.issue_cursor(&snapshot_id, 0)
    }

    /// Consume a cursor and return one page. The cursor is single-use; the key
    /// must match the original principal, endpoint, root and query.
    pub fn page(
        &mut self,
        cursor: &str,
        expected: &SnapshotKey,
        limit: usize,
    ) -> CallResult<(Vec<PageItem>, Option<String>)> {
        self.purge();
        let entry = self
            .cursors
            .remove(cursor)
            .ok_or_else(|| bad_request("unknown or expired cursor"))?;
        let snapshot = self
            .snapshots
            .get(&entry.snapshot_id)
            .ok_or_else(|| bad_request("snapshot expired"))?;
        if &snapshot.key != expected {
            return Err(bad_request(
                "cursor does not match this principal, endpoint or query",
            ));
        }
        let start = entry.offset.min(snapshot.items.len());
        let end = (start + limit.max(1)).min(snapshot.items.len());
        let items = snapshot.items[start..end].to_vec();
        let next = if end < snapshot.items.len() {
            Some(self.issue_cursor(&entry.snapshot_id, end)?)
        } else {
            None
        };
        Ok((items, next))
    }

    fn issue_cursor(&mut self, snapshot_id: &str, offset: usize) -> CallResult<String> {
        let id = self.alloc("cur");
        let expires_at = self.clock.now() + self.limits.lease;
        self.cursors.insert(
            id.clone(),
            CursorEntry {
                snapshot_id: snapshot_id.to_string(),
                offset,
                expires_at,
            },
        );
        Ok(id)
    }

    fn purge(&mut self) {
        let now = self.clock.now();
        self.snapshots
            .retain(|_, snapshot| snapshot.expires_at > now);
        self.cursors.retain(|_, cursor| cursor.expires_at > now);
    }

    fn alloc(&mut self, prefix: &str) -> String {
        self.next += 1;
        format!("{prefix}-{:016x}", self.next)
    }
}

fn bad_request(message: &str) -> CallError {
    CallError::new(v1::ErrorCode::BadRequest, message)
}

//! Snapshot pagination: stable pages, key binding, lease expiry, quotas.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use conex_source::pagination::{Clock, PageItem, SnapshotCache, SnapshotKey, SnapshotLimits};

struct ManualClock {
    base: Instant,
    offset_ms: AtomicU64,
}

impl ManualClock {
    fn new() -> Self {
        Self {
            base: Instant::now(),
            offset_ms: AtomicU64::new(0),
        }
    }

    fn advance(&self, ms: u64) {
        self.offset_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.base + Duration::from_millis(self.offset_ms.load(Ordering::SeqCst))
    }
}

fn key(principal: &str, query: &str) -> SnapshotKey {
    SnapshotKey {
        tenant_id: "t".into(),
        principal_id: principal.into(),
        endpoint_id: "fs".into(),
        method: "source/list".into(),
        root: String::new(),
        query: query.into(),
    }
}

fn item(id: &str) -> PageItem {
    PageItem {
        resource_id: id.into(),
        title: id.into(),
        mime: "text/markdown".into(),
        size_bytes: 1,
        excerpt: None,
    }
}

#[test]
fn pages_are_stable_and_drain() {
    let clock = Arc::new(ManualClock::new());
    let mut cache = SnapshotCache::new(clock, SnapshotLimits::default());
    let k = key("alice", "");
    let items: Vec<PageItem> = (0..5).map(|i| item(&format!("f{i}.md"))).collect();
    let cursor = cache.create(k.clone(), items).unwrap();
    let (first, next) = cache.page(&cursor, &k, 2).unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(first[0].resource_id, "f0.md");
    let (second, next2) = cache.page(next.as_ref().unwrap(), &k, 2).unwrap();
    assert_eq!(second.len(), 2);
    let (third, next3) = cache.page(next2.as_ref().unwrap(), &k, 2).unwrap();
    assert_eq!(third.len(), 1);
    assert!(next3.is_none());
}

#[test]
fn cursor_is_bound_to_principal_and_query() {
    let clock = Arc::new(ManualClock::new());
    let mut cache = SnapshotCache::new(clock, SnapshotLimits::default());
    let cursor = cache.create(key("alice", ""), vec![item("a.md")]).unwrap();
    assert!(cache.page(&cursor, &key("bob", ""), 10).is_err());
    let cursor = cache.create(key("alice", ""), vec![item("a.md")]).unwrap();
    assert!(cache.page(&cursor, &key("alice", "other"), 10).is_err());
}

#[test]
fn expired_cursor_is_rejected() {
    let clock = Arc::new(ManualClock::new());
    let mut cache = SnapshotCache::new(clock.clone(), SnapshotLimits::default());
    let k = key("alice", "");
    let cursor = cache.create(k.clone(), vec![item("a.md")]).unwrap();
    clock.advance(61_000);
    assert!(cache.page(&cursor, &k, 10).is_err());
}

#[test]
fn per_principal_snapshot_limit_is_enforced() {
    let clock = Arc::new(ManualClock::new());
    let mut cache = SnapshotCache::new(clock, SnapshotLimits::default());
    let k = key("alice", "");
    for _ in 0..4 {
        cache.create(k.clone(), vec![]).unwrap();
    }
    let error = cache.create(k.clone(), vec![]).unwrap_err();
    assert_eq!(
        error.code_enum(),
        Some(conex_proto::v1::ErrorCode::QuotaExceeded)
    );
    assert!(cache.create(key("bob", ""), vec![]).is_ok());
}

#[test]
fn item_budget_is_enforced() {
    let clock = Arc::new(ManualClock::new());
    let limits = SnapshotLimits {
        max_snapshots_per_principal: 4,
        lease: Duration::from_secs(60),
        max_items: 2,
    };
    let mut cache = SnapshotCache::new(clock, limits);
    let items: Vec<PageItem> = (0..3).map(|i| item(&format!("f{i}.md"))).collect();
    assert!(cache.create(key("alice", ""), items).is_err());
}

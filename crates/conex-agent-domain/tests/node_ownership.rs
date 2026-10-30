//! P2.2 持久身份、模块代际与自有资源所有权回归
//! （docs/plans/2026-10-29-conex-mvp-p2.md §3；private-runtime 契约 §1/§6）。
//! 真实 SQLite：boot epoch/generation 跨重启持久、nodeId 匹配、unknown 保留、
//! v1→v2 原子迁移与迁移前一致快照、未来版本拒绝。

use std::os::unix::fs::PermissionsExt;

use conex_agent_domain::{OwnedResource, ResourceState, Store, StoreError, StoreLimits, new_ulid};
use rusqlite::Connection;

/// P1 发布的 schema（user_version=1），用于构造真实旧库；必须与迁移前的
/// schema.sql 保持一致。
const V1_SCHEMA: &str = "\
PRAGMA foreign_keys=ON;
CREATE TABLE store_identity(node_id TEXT PRIMARY KEY);
CREATE TABLE streams(
 id TEXT PRIMARY KEY, epoch TEXT NOT NULL,
 head INTEGER NOT NULL DEFAULT 0 CHECK(head>=0),
 first_available INTEGER NOT NULL DEFAULT 1 CHECK(first_available>=1),
 event_bytes INTEGER NOT NULL DEFAULT 0 CHECK(event_bytes>=0)
);
CREATE TABLE conversations(
 id TEXT PRIMARY KEY REFERENCES streams(id), endpoint_id TEXT NOT NULL,
 workspace_id TEXT NOT NULL, active_execution_id TEXT,
 revision INTEGER NOT NULL CHECK(revision>=1), projection BLOB NOT NULL
);
CREATE TABLE executions(
 id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 parent_execution_id TEXT REFERENCES executions(id), generation INTEGER NOT NULL,
 state TEXT NOT NULL CHECK(state IN
 ('accepted','dispatching','running','unknown','succeeded','failed','cancelled')),
 revision INTEGER NOT NULL CHECK(revision>=1), projection BLOB NOT NULL
);
CREATE UNIQUE INDEX one_top_level_active ON executions(conversation_id)
 WHERE parent_execution_id IS NULL
 AND state IN ('accepted','dispatching','running','unknown');
CREATE TABLE interactions(
 id TEXT PRIMARY KEY, execution_id TEXT NOT NULL REFERENCES executions(id),
 state TEXT NOT NULL CHECK(state IN
 ('pending','responding','resolved','expired','invalidated','unknown')),
 revision INTEGER NOT NULL CHECK(revision>=1),
 expires_at_ms INTEGER NOT NULL, decision_digest TEXT, responder_id TEXT,
 projection BLOB NOT NULL
);
CREATE TABLE messages(
 id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 execution_id TEXT NOT NULL REFERENCES executions(id), ordinal INTEGER NOT NULL,
 backend_key TEXT, revision INTEGER NOT NULL CHECK(revision>=1),
 state TEXT NOT NULL CHECK(state IN ('streaming','complete','incomplete')), projection BLOB NOT NULL,
 UNIQUE(conversation_id,ordinal), UNIQUE(conversation_id,backend_key)
);
CREATE TABLE backend_mappings(
 adapter TEXT NOT NULL, backend_instance_key TEXT NOT NULL,
 backend_conversation_id TEXT NOT NULL, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 PRIMARY KEY(adapter,backend_instance_key,backend_conversation_id)
);
CREATE TABLE entities(
 id TEXT PRIMARY KEY, kind TEXT NOT NULL, revision INTEGER NOT NULL, projection BLOB NOT NULL
);
CREATE TABLE dedup(
 principal_id TEXT NOT NULL, endpoint_id TEXT NOT NULL, operation TEXT NOT NULL,
 key TEXT NOT NULL, digest TEXT NOT NULL, object_id TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL, expires_at_ms INTEGER NOT NULL,
 PRIMARY KEY(principal_id,endpoint_id,operation,key)
);
CREATE TABLE events(
 stream_id TEXT NOT NULL REFERENCES streams(id), position INTEGER NOT NULL CHECK(position>0),
 recorded_at_ms INTEGER NOT NULL, encoded BLOB NOT NULL,
 encoded_bytes INTEGER NOT NULL CHECK(encoded_bytes>0),
 PRIMARY KEY(stream_id,position)
);
CREATE INDEX events_age ON events(recorded_at_ms,stream_id,position);
PRAGMA user_version=1;
";

fn seed_v1(path: &std::path::Path, node_id: &str) -> rusqlite::Result<()> {
    let db = Connection::open(path)?;
    db.execute_batch(V1_SCHEMA)?;
    let id = new_ulid();
    db.execute(
        "INSERT INTO store_identity(node_id) VALUES (?1)",
        rusqlite::params![node_id],
    )?;
    db.execute(
        "INSERT INTO streams(id, epoch, head, first_available, event_bytes)
         VALUES (?1, ?2, 1, 1, 0)",
        rusqlite::params![id, "e1"],
    )?;
    db.execute(
        "INSERT INTO conversations(id, endpoint_id, workspace_id, active_execution_id,
                                   revision, projection)
         VALUES (?1, ?2, ?3, NULL, 1, x'00')",
        rusqlite::params![id, "ep1", "ws1"],
    )?;
    db.execute(
        "INSERT INTO events(stream_id, position, recorded_at_ms, encoded, encoded_bytes)
         VALUES (?1, 1, 1000, x'ab', 2)",
        rusqlite::params![id],
    )?;
    Ok(())
}

fn migration_backup_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .expect("read dir")
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("node.db.backup-v1.")
        })
        .map(|entry| entry.path())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn boot_epoch_persists_rotates_and_binds_node() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let first = store.begin_node_boot("node-a").await.expect("boot");
    assert!(conex_proto::wire::is_ulid(&first));
    let second = store.begin_node_boot("node-a").await.expect("boot again");
    assert!(conex_proto::wire::is_ulid(&second));
    assert_ne!(first, second, "epoch rotates per boot");

    // 节点重启：换 Store 连接同一库，epoch 继续轮换（持久登记，不是进程内值）
    drop(store);
    let reopened = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("reopen");
    let third = reopened
        .begin_node_boot("node-a")
        .await
        .expect("boot after restart");
    assert_ne!(second, third);

    // nodeId 必须匹配库绑定身份；错误 nodeId 打开同库也拒绝
    let err = reopened
        .begin_node_boot("node-b")
        .await
        .expect_err("wrong node boot refused");
    assert!(matches!(err, StoreError::NodeMismatch { .. }));
    let err = Store::open(&path, "node-b", StoreLimits::default())
        .await
        .expect_err("wrong node open refused");
    assert!(matches!(err, StoreError::NodeMismatch { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn module_generation_monotonic_across_restarts_and_stores() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    assert_eq!(
        store.next_module_generation("agent").await.expect("gen1"),
        1
    );
    assert_eq!(
        store.next_module_generation("agent").await.expect("gen2"),
        2
    );
    assert_eq!(
        store
            .next_module_generation("metrics")
            .await
            .expect("metric1"),
        1
    );
    drop(store);

    // 跨重启持久单调：不是进程内计数"重启+1"
    let reopened = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("reopen");
    assert_eq!(
        reopened
            .next_module_generation("agent")
            .await
            .expect("gen3"),
        3
    );
    assert_eq!(
        reopened
            .next_module_generation("metrics")
            .await
            .expect("metric2"),
        2
    );

    // 两个 Store 并发递增同一 module_key：事务串行、单调、无重复无漏号
    let other = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("second handle");
    let mut seen = std::collections::BTreeSet::new();
    let (mut a_prev, mut b_prev) = (0u64, 0u64);
    for _ in 0..32 {
        let a = reopened
            .next_module_generation("race")
            .await
            .expect("race a");
        assert!(a > a_prev, "per-handle monotonic");
        a_prev = a;
        seen.insert(a);
        let b = other.next_module_generation("race").await.expect("race b");
        assert!(b > b_prev, "per-handle monotonic");
        b_prev = b;
        seen.insert(b);
    }
    assert_eq!(b_prev, 64);
    assert_eq!(seen.len(), 64, "64 calls yield exactly 1..=64");
    assert_eq!(seen.first().copied(), Some(1));
    assert_eq!(seen.last().copied(), Some(64));

    // 空/超长 module_key 拒绝
    let err = reopened
        .next_module_generation("")
        .await
        .expect_err("empty key rejected");
    assert!(matches!(err, StoreError::InvalidArgument(_)));
    let err = reopened
        .next_module_generation(&"k".repeat(129))
        .await
        .expect_err("long key rejected");
    assert!(matches!(err, StoreError::InvalidArgument(_)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn owned_resources_reconcile_lifecycle_and_unknown_retention() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("open");
    let epoch = store.begin_node_boot("node-a").await.expect("boot");
    let generation = store
        .next_module_generation("agent")
        .await
        .expect("generation");

    let recorded = OwnedResource {
        id: new_ulid(),
        module_key: "agent".into(),
        generation,
        owner_epoch: epoch.clone(),
        control_kind: "pidfd".into(),
        control_identity: vec![1, 2, 3, 4],
        state: ResourceState::Created,
    };
    store
        .record_owned_resource(recorded.clone())
        .await
        .expect("record");
    let err = store
        .record_owned_resource(recorded.clone())
        .await
        .expect_err("duplicate id conflicts");
    assert!(matches!(err, StoreError::Conflict(_)));

    // 崩溃遗留的 unknown 资源：仍占约束，保持未知直到明确证据
    let ghost = OwnedResource {
        id: new_ulid(),
        module_key: "metrics".into(),
        generation: 1,
        owner_epoch: epoch,
        control_kind: "pidfd".into(),
        control_identity: vec![9, 8],
        state: ResourceState::Unknown,
    };
    store
        .record_owned_resource(ghost.clone())
        .await
        .expect("record ghost");

    let un = store.list_unreconciled_resources().await.expect("list");
    assert_eq!(un.len(), 2);
    assert!(un.contains(&recorded));
    assert!(un.contains(&ghost));
    // 不透明控制身份按字节原样往返
    let found = un.iter().find(|r| r.id == ghost.id).expect("ghost present");
    assert_eq!(found.control_identity, vec![9, 8]);
    assert_eq!(found.owner_epoch, ghost.owner_epoch);
    assert_eq!(found.state, ResourceState::Unknown);
    assert!(matches!(
        store
            .record_resource_ready(&recorded.id, &new_ulid(), vec![5])
            .await,
        Err(StoreError::Conflict("resource_reservation_changed"))
    ));
    store
        .record_resource_ready(&recorded.id, &recorded.owner_epoch, vec![5, 6])
        .await
        .expect("verified readiness");
    assert!(matches!(
        store
            .record_resource_ready(&recorded.id, &recorded.owner_epoch, vec![7])
            .await,
        Err(StoreError::Conflict("resource_reservation_changed"))
    ));
    let ready = store
        .list_unreconciled_resources()
        .await
        .expect("ready list");
    let ready = ready
        .iter()
        .find(|resource| resource.id == recorded.id)
        .expect("ready resource");
    assert_eq!(ready.state, ResourceState::Active);
    assert_eq!(ready.control_identity, vec![5, 6]);

    // released 从清单消失；active 核对后仍在清单并更新状态
    store
        .record_resource_reconciled(&recorded.id, ResourceState::Released)
        .await
        .expect("release recorded");
    let mut expected_active = ghost.clone();
    expected_active.state = ResourceState::Active;
    store
        .record_resource_reconciled(&ghost.id, ResourceState::Active)
        .await
        .expect("confirm active");
    let un = store
        .list_unreconciled_resources()
        .await
        .expect("list after reconcile");
    assert_eq!(un.len(), 1);
    assert!(un.contains(&expected_active));
    store
        .record_resource_reconciled(&ghost.id, ResourceState::Unknown)
        .await
        .expect("lost control");
    let unknown = store
        .list_unreconciled_resources()
        .await
        .expect("unknown survives");
    assert_eq!(unknown[0].state, ResourceState::Unknown);

    // 全部核对到 released 后清单为空
    store
        .record_resource_reconciled(&ghost.id, ResourceState::Released)
        .await
        .expect("release ghost");
    let un = store
        .list_unreconciled_resources()
        .await
        .expect("empty list");
    assert!(un.is_empty());
    assert!(matches!(
        store
            .record_resource_reconciled(&ghost.id, ResourceState::Active)
            .await,
        Err(StoreError::Conflict("resource_already_released"))
    ));

    // 非法核对状态与缺失资源
    for bad in [ResourceState::Created] {
        let err = store
            .record_resource_reconciled(&recorded.id, bad)
            .await
            .expect_err("not a reconcile outcome");
        assert!(matches!(err, StoreError::InvalidArgument(_)));
    }
    let err = store
        .record_resource_reconciled(&new_ulid(), ResourceState::Released)
        .await
        .expect_err("missing resource");
    assert!(matches!(err, StoreError::NotFound { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn v1_database_migrates_with_snapshot_and_preserves_records() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    seed_v1(&path, "node-a").expect("seed v1");
    // 预置 8 个历史迁移备份，验证保留最近 7 份
    for i in 1..=8 {
        seed_v1(&dir.path().join(format!("node.db.backup-v1.{i}")), "node-a")
            .expect("seed historical snapshot");
    }

    let store = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect("migrate open");
    // 迁移后 v2 表可经公开 API 正常使用
    let epoch = store
        .begin_node_boot("node-a")
        .await
        .expect("boot after migration");
    assert!(conex_proto::wire::is_ulid(&epoch));
    assert_eq!(
        store
            .next_module_generation("agent")
            .await
            .expect("gen after migration"),
        1
    );
    store
        .record_owned_resource(OwnedResource {
            id: new_ulid(),
            module_key: "agent".into(),
            generation: 1,
            owner_epoch: epoch,
            control_kind: "pidfd".into(),
            control_identity: vec![7],
            state: ResourceState::Active,
        })
        .await
        .expect("record after migration");
    drop(store);

    // P1 记录保留 + 版本推进 + 新表存在（直接查迁移后的库）
    let db = Connection::open(&path).expect("open migrated db");
    let version: i64 = db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
    let identity: String = db
        .query_row("SELECT node_id FROM store_identity", [], |r| r.get(0))
        .unwrap();
    assert_eq!(identity, "node-a");
    for (table, expected) in [
        ("conversations", 1i64),
        ("streams", 1),
        ("events", 1),
        ("dedup", 0),
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap_or_else(|e| panic!("count {table}: {e}"));
        assert_eq!(count, expected, "P1 rows in {table} preserved");
    }
    let news: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
             ('node_metadata','module_generations','owned_resources')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(news, 3, "v2 tables created");

    // 迁移前一致快照存在、是 v1、0600
    let snapshots: Vec<std::path::PathBuf> = migration_backup_files(dir.path())
        .into_iter()
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            match name.strip_prefix("node.db.backup-v1.") {
                Some(stamp) => stamp.parse::<u128>().is_ok_and(|stamp| stamp > 8),
                None => false,
            }
        })
        .collect();
    assert_eq!(snapshots.len(), 1, "exactly one real migration snapshot");
    let meta = std::fs::metadata(&snapshots[0]).expect("snapshot metadata");
    assert_eq!(meta.permissions().mode() & 0o777, 0o600, "snapshot 0600");
    let snap_db = Connection::open(&snapshots[0]).expect("open snapshot");
    let snap_version: i64 = snap_db
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(snap_version, 1, "snapshot is pre-migration v1");
    let snap_identity: String = snap_db
        .query_row("SELECT node_id FROM store_identity", [], |r| r.get(0))
        .unwrap();
    assert_eq!(snap_identity, "node-a");

    // 保留最近 7 份迁移备份
    assert_eq!(migration_backup_files(dir.path()).len(), 7, "retain last 7");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn future_schema_version_refused() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("node.db");
    let db = Connection::open(&path).expect("create db");
    db.pragma_update(None, "user_version", 99)
        .expect("bump version");
    drop(db);
    let err = Store::open(&path, "node-a", StoreLimits::default())
        .await
        .expect_err("future version refused");
    assert!(matches!(err, StoreError::InvalidArgument(_)));
}

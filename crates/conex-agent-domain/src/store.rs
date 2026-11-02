//! 真实节点 SQLite 领域存储（docs/plans/2026-10-29-conex-mvp-p1.md §3-§6；
//! P2.2 版本迁移与身份/所有权表见 docs/plans/2026-10-29-conex-mvp-p2.md §3）。
//!
//! 不变量（契约 §5/恢复契约 §4）：
//! - 先授权，再在 BEGIN IMMEDIATE 内完成幂等查重、占槽、事件与提交；
//! - 没有已提交的 Execution 不向后端发送：dispatching 必须先持久化；
//! - 终态单次 CAS；迟到冲突观察保留诊断而不覆盖；
//! - 事件位置连续、编码逐条受限、投影/事件同事务；
//! - 数据库 I/O 全部在专用阻塞线程，async 侧经有界 mpsc + oneshot 回执；
//! - 跨进程并发仍依赖 SQLite 事务（WAL + busy_timeout），不以进程内锁替代；
//! - v1→v2 迁移前用 backup API 复制一致快照并保留最近 7 份，nodeId/世代单调
//!   跨重启持久；不认识的新版本或 integrity_check 失败拒绝启动（契约 §6）。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use conex_proto::{
    AgentEvent, AgentEventCursor, AgentEventReadResult, AgentGap, AgentHistoryInfo,
    AgentInteractionRespondInput, AgentMessage, AgentMessageDelta, AgentMessageReadResult,
    AgentSnapshot, AgentSubmitInput, Conversation, Execution, InteractionChoice,
    InteractionRequest, Workspace, agent_event, agent_message_part,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use tokio::sync::{mpsc, oneshot};

use crate::model::{
    Admission, ApplyResult, AuthorizedScope, BackendObservation, BackendResponseOutcome,
    CancelTarget, DispatchPermit, DispatchToken, PruneResult, ReconcileTarget, ResponsePermit,
    StoreError, StoreLimits, TerminalState,
};
use crate::projection::{decode, digest, encode, encode_bounded, split_utf8};
use crate::ulid::IdGen;
use crate::{OwnedResource, ResourceState};

type Job = Box<dyn FnOnce(&mut Option<Connection>, &IdGen, &StoreLimits) + Send>;

const ACTIVE_EXECUTION_STATES: [&str; 4] = ["accepted", "dispatching", "running", "unknown"];
const TERMINAL_EXECUTION_STATES: [&str; 3] = ["succeeded", "failed", "cancelled"];
const ROLES: [&str; 4] = ["user", "assistant", "tool", "system"];
const INTERACTION_KINDS: [&str; 3] = ["approval", "selection", "input"];
const EFFECTS: [&str; 4] = ["allow_once", "deny", "select", "submit"];

#[derive(Clone)]
pub struct Store {
    tx: mpsc::Sender<Job>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store")
            .field("free_slots", &self.tx.max_capacity())
            .finish()
    }
}

/// 事件载荷：与 events.proto 一一对应。
pub(crate) enum Ev {
    ConversationUpdated(Box<Conversation>),
    ExecutionUpdated(Box<Execution>),
    InteractionCreated(Box<InteractionRequest>),
    InteractionUpdated(Box<InteractionRequest>),
    MessageCreated(Box<AgentMessage>),
    Delta {
        message_id: String,
        part_index: u32,
        offset_bytes: u64,
        text: String,
    },
    MessageCompleted {
        message_id: String,
        revision: u64,
        state: String,
    },
    MessagePart {
        part: conex_proto::AgentMessagePart,
    },
    Gap {
        reason: String,
        backend_range: Option<String>,
    },
}

impl Store {
    /// 打开（或创建）节点库。目录 0700、库文件 0600；fresh 库执行 schema.sql
    /// 并写入 store_identity；既有 v1 库经一致快照后原子迁移到当前版本，其他
    /// nodeId 打开同库拒绝、不认识的新版本或 integrity 失败拒绝启动。
    pub async fn open(
        path: impl AsRef<Path>,
        node_id: &str,
        limits: StoreLimits,
    ) -> Result<Store, StoreError> {
        if node_id.is_empty() {
            return Err(StoreError::InvalidArgument(
                "node_id must not be empty".into(),
            ));
        }
        let path = path.as_ref().to_path_buf();
        let node = node_id.to_string();
        Store::spawn_worker(limits, move |conn, ids, _limits| {
            prepare_file(&path)?;
            let db = open_rw(&path)?;
            init_or_verify(&db, &path, &node)?;
            *conn = Some(db);
            let _ = ids;
            Ok(())
        })
        .await
    }

    /// 从备份恢复到全新目标目录：先校验备份完整性/同 nodeId，再经 backup API
    /// 复制、把备份推进到当前 schema（v1 备份迁移 v2，不重复留快照）、为每个
    /// stream 换新 epoch 并记录 recovery.gap。旧 cursor 因此明确失效；恢复后
    /// 的 dedup 仍属于原 node。
    pub async fn restore(
        backup_path: impl AsRef<Path>,
        destination: impl AsRef<Path>,
        node_id: &str,
        limits: StoreLimits,
    ) -> Result<Store, StoreError> {
        if node_id.is_empty() {
            return Err(StoreError::InvalidArgument(
                "node_id must not be empty".into(),
            ));
        }
        let backup = backup_path.as_ref().to_path_buf();
        let destination = destination.as_ref().to_path_buf();
        let node = node_id.to_string();
        Store::spawn_worker(limits, move |conn, ids, limits| {
            if destination.exists() {
                return Err(StoreError::InvalidArgument(
                    "restore destination must not exist".into(),
                ));
            }
            {
                let source = open_ro(&backup)?;
                verify_backup(&source, &node)?;
                prepare_file(&destination)?;
                let mut target = open_rw(&destination)?;
                init_fresh(&target, &node)?;
                let writer = rusqlite::backup::Backup::new(&source, &mut target)?;
                writer
                    .run_to_completion(256, Duration::from_millis(1), None)
                    .map_err(|error| StoreError::Db(format!("backup copy: {error}")))?;
            }
            let mut db = open_rw(&destination)?;
            // 恢复后的库必须落在当前 schema：v1 备份拷贝后迁移到 v2（源备份
            // 就是迁移前的一致快照，不再重复留快照）；其余版本拒绝。
            match current_version(&db)? {
                1 => migrate_to_v2(&db)?,
                2 => migrate_to_v3(&db)?,
                3 => migrate_to_v4(&db)?,
                4 => migrate_to_v5(&db)?,
                5 => {}
                other => {
                    return Err(StoreError::InvalidArgument(format!(
                        "restored database version {other} is not supported"
                    )));
                }
            }
            verify_identity(&db, &node)?;
            rotate_epochs_after_restore(&mut db, ids, limits)?;
            *conn = Some(db);
            Ok(())
        })
        .await
    }

    /// 一致性备份到新文件（restore 的正向操作；测试与运维使用）。
    pub async fn backup_to(&self, backup_path: impl AsRef<Path>) -> Result<(), StoreError> {
        let backup = backup_path.as_ref().to_path_buf();
        self.run(move |conn, _gen, _limits| {
            let db = conn.as_mut().ok_or(StoreError::Closed)?;
            if backup.exists() {
                return Err(StoreError::InvalidArgument(
                    "backup path already exists".into(),
                ));
            }
            prepare_file(&backup)?;
            let mut target = open_rw(&backup)?;
            init_fresh(&target, &store_node(db)?)?;
            let writer = rusqlite::backup::Backup::new(db, &mut target)?;
            writer
                .run_to_completion(256, Duration::from_millis(1), None)
                .map_err(|error| StoreError::Db(format!("backup: {error}")))?;
            Ok(())
        })
        .await
    }

    async fn spawn_worker<F>(limits: StoreLimits, first: F) -> Result<Store, StoreError>
    where
        F: FnOnce(&mut Option<Connection>, &IdGen, &StoreLimits) -> Result<(), StoreError>
            + Send
            + 'static,
    {
        let (tx, mut rx) = mpsc::channel::<Job>(limits.max_dispatch_queue.max(1));
        let (ready_tx, ready_rx) = oneshot::channel::<Result<(), StoreError>>();
        let ids = Arc::new(IdGen::new());
        let limits = Arc::new(limits);
        let worker_gen = ids.clone();
        let worker_limits = limits.clone();
        std::thread::Builder::new()
            .name("conex-agent-domain".into())
            .spawn(move || {
                let mut conn: Option<Connection> = None;
                let _ = ready_tx.send(first(&mut conn, &worker_gen, &worker_limits));
                while let Some(job) = rx.blocking_recv() {
                    job(&mut conn, &worker_gen, &worker_limits);
                }
            })
            .map_err(|error| StoreError::Db(format!("spawn worker: {error}")))?;
        ready_rx
            .await
            .map_err(|_| StoreError::Closed)?
            .map(|_| Store { tx })
    }

    async fn run<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Option<Connection>, &IdGen, &StoreLimits) -> Result<T, StoreError>
            + Send
            + 'static,
    {
        let (reply, wait) = oneshot::channel::<Result<T, StoreError>>();
        let job: Job = Box::new(move |conn, ids, limits| {
            let _ = reply.send(f(conn, ids, limits));
        });
        self.tx
            .try_send(job)
            .map_err(|_| StoreError::QuotaExceeded("dispatch_queue_full"))?;
        wait.await.map_err(|_| StoreError::Closed)?
    }

    async fn immediate<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Transaction<'_>, &IdGen, &StoreLimits) -> Result<T, StoreError>
            + Send
            + 'static,
    {
        self.run(move |conn, ids, limits| {
            let db = conn.as_mut().ok_or(StoreError::Closed)?;
            let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let value = f(&tx, ids, limits)?;
            tx.commit()?;
            Ok(value)
        })
        .await
    }

    async fn readonly<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Transaction<'_>, &IdGen, &StoreLimits) -> Result<T, StoreError>
            + Send
            + 'static,
    {
        self.run(move |conn, ids, limits| {
            let db = conn.as_ref().ok_or(StoreError::Closed)?;
            let tx = db.unchecked_transaction()?;
            let value = f(&tx, ids, limits)?;
            drop(tx);
            Ok(value)
        })
        .await
    }
}

// ---- 文件与初始化 ----------------------------------------------------------

fn prepare_file(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(parent)
            .map_err(|error| StoreError::Db(format!("create data dir: {error}")))?;
    }
    if !path.exists() {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| StoreError::Db(format!("create database: {error}")))?;
    }
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    for suffix in ["-wal", "-shm"] {
        let mut side = path.as_os_str().to_os_string();
        side.push(suffix);
        let side = Path::new(&side);
        if side.exists() {
            let _ = std::fs::set_permissions(side, std::fs::Permissions::from_mode(0o600));
        }
    }
    Ok(())
}

fn open_rw(path: &Path) -> Result<Connection, StoreError> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    configure(&db)?;
    Ok(db)
}

fn open_ro(path: &Path) -> Result<Connection, StoreError> {
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    configure(&db)?;
    Ok(db)
}

fn configure(db: &Connection) -> Result<(), StoreError> {
    db.busy_timeout(Duration::from_millis(2000))?;
    db.pragma_update(None, "journal_mode", "WAL")?;
    db.pragma_update(None, "synchronous", "FULL")?;
    db.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

const SCHEMA: &str = include_str!("schema.sql");

fn init_fresh(db: &Connection, node_id: &str) -> Result<(), StoreError> {
    let version: i64 = db.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version != 0 {
        return Err(StoreError::InvalidArgument(format!(
            "database already initialized at version {version}"
        )));
    }
    db.execute_batch(SCHEMA)?;
    db.execute(
        "INSERT INTO store_identity(node_id) VALUES (?1)",
        params![node_id],
    )?;
    Ok(())
}

fn current_version(db: &Connection) -> Result<i64, StoreError> {
    db.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(StoreError::from)
}

/// 库绑定的 nodeId 必须与打开者一致（private-runtime §1；P1 不变量）。
fn verify_identity(db: &Connection, node_id: &str) -> Result<(), StoreError> {
    let stored: String = db
        .query_row("SELECT node_id FROM store_identity", [], |row| row.get(0))
        .optional()?
        .ok_or_else(|| StoreError::Db("store_identity missing".into()))?;
    if stored != node_id {
        return Err(StoreError::NodeMismatch {
            expected: stored,
            actual: node_id.into(),
        });
    }
    Ok(())
}

/// 完整性失败必须拒绝启动，不用坏库继续"成功"（契约 §6）。
fn require_integrity(db: &Connection) -> Result<(), StoreError> {
    let check: String = db.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(StoreError::Db(format!("database integrity: {check}")));
    }
    Ok(())
}

/// v1→v2 增量（P2.2）：必须与 schema.sql 的 v2 段一致；只加表不覆盖 P1 记录，
/// user_version 在事务内单调推进，事务失败整体回滚。
const MIGRATE_V1_TO_V2: &str = "\
CREATE TABLE node_metadata(
 node_id TEXT PRIMARY KEY,
 runtime_epoch TEXT NOT NULL
);
CREATE TABLE module_generations(
 module_key TEXT PRIMARY KEY,
 generation INTEGER NOT NULL CHECK(generation>0)
);
CREATE TABLE owned_resources(
 id TEXT PRIMARY KEY, module_key TEXT NOT NULL, generation INTEGER NOT NULL,
 owner_epoch TEXT NOT NULL, control_kind TEXT NOT NULL, control_identity BLOB NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('created','active','unknown','released'))
);
PRAGMA user_version=2;
";

fn migrate_to_v2(db: &Connection) -> Result<(), StoreError> {
    // Startup runs exclusively on this connection's single database worker.
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    tx.execute_batch(MIGRATE_V1_TO_V2)?;
    tx.commit()?;
    Ok(())
}

/// v2→v3 增量（P5.3）：只加访问撤销表；必须与 schema.sql 的 v3 段一致。
const MIGRATE_V2_TO_V3: &str = "\
CREATE TABLE access_revocations(
 source TEXT NOT NULL CHECK(source IN ('hub','local')),
 revision INTEGER NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('jti','principal','kid')),
 target TEXT NOT NULL,
 effective_at_ms INTEGER NOT NULL,
 PRIMARY KEY(source, revision)
);
CREATE TABLE access_sync_watermark(
 id INTEGER PRIMARY KEY CHECK(id=1),
 hub_revision INTEGER NOT NULL CHECK(hub_revision>=0)
);
PRAGMA user_version=3;
";

fn migrate_to_v3(db: &Connection) -> Result<(), StoreError> {
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    tx.execute_batch(MIGRATE_V2_TO_V3)?;
    tx.commit()?;
    Ok(())
}

/// v3→v4 增量（P3.2）：只加子代理关系表；必须与 schema.sql 的 v4 段一致。
const MIGRATE_V3_TO_V4: &str = "\
CREATE TABLE relationships(
 id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id),
 from_ref TEXT NOT NULL, to_ref TEXT NOT NULL, kind TEXT NOT NULL,
 source TEXT NOT NULL CHECK(source IN ('backend','config','user')),
 quality TEXT NOT NULL CHECK(quality IN ('current','stale','historical','unknown')),
 observed_at_ms INTEGER NOT NULL, projection BLOB NOT NULL
);
CREATE INDEX relationships_conversation ON relationships(conversation_id, observed_at_ms);
PRAGMA user_version=4;
";

fn migrate_to_v4(db: &Connection) -> Result<(), StoreError> {
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    tx.execute_batch(MIGRATE_V3_TO_V4)?;
    tx.commit()?;
    Ok(())
}

/// v4→v5 增量（P6.3）：只加实体标注表；必须与 schema.sql 的 v5 段一致。
const MIGRATE_V4_TO_V5: &str = "
CREATE TABLE entity_tags(
 name TEXT NOT NULL, owner TEXT NOT NULL CHECK(owner IN ('node','user')),
 entity_kind TEXT NOT NULL, entity_id TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 PRIMARY KEY(owner, entity_kind, entity_id, name)
);
PRAGMA user_version=5;
";

fn migrate_to_v5(db: &Connection) -> Result<(), StoreError> {
    let tx = rusqlite::Transaction::new_unchecked(db, TransactionBehavior::Immediate)?;
    tx.execute_batch(MIGRATE_V4_TO_V5)?;
    tx.commit()?;
    Ok(())
}

/// 保留最近 7 份本机迁移备份（契约 §6）。只清理本库路径 `*.backup-v<N>.*`
/// 前缀文件，不触碰用户显式 backup_to 的任意路径备份。
const MIGRATION_BACKUP_KEEP: usize = 7;

fn migration_backup_path(db_path: &Path, from_version: i64, stamp_ms: u128) -> PathBuf {
    let mut name = db_path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".backup-v{from_version}.{stamp_ms}"));
    db_path.with_file_name(name)
}

/// 迁移前用 SQLite backup API 复制一致快照：不直接拷贝活跃 WAL 主文件；
/// 快照与其目录同权限（目录 0700、文件 0600）。
fn snapshot_before_migration(
    db: &Connection,
    db_path: &Path,
    from_version: i64,
) -> Result<(), StoreError> {
    let mut stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let snapshot_path = loop {
        let candidate = migration_backup_path(db_path, from_version, stamp);
        if !candidate.exists() {
            break candidate;
        }
        stamp += 1;
    };
    prepare_file(&snapshot_path)?;
    let mut target = open_rw(&snapshot_path)?;
    init_fresh(&target, &store_node(db)?)?;
    let writer = rusqlite::backup::Backup::new(db, &mut target)?;
    writer
        .run_to_completion(256, Duration::from_millis(1), None)
        .map_err(|error| StoreError::Db(format!("migration snapshot: {error}")))?;
    drop(writer);
    // A backup is a single immutable artifact, not a live WAL database.
    target.pragma_update(None, "journal_mode", "DELETE")?;
    Ok(())
}

fn retain_migration_backups(db_path: &Path, from_version: i64) -> Result<(), StoreError> {
    let Some(parent) = db_path.parent() else {
        return Ok(());
    };
    let mut prefix = db_path.file_name().unwrap_or_default().to_os_string();
    prefix.push(format!(".backup-v{from_version}."));
    let prefix = prefix.to_string_lossy().into_owned();
    let entries = std::fs::read_dir(parent)
        .map_err(|error| StoreError::Db(format!("list data dir: {error}")))?;
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.strip_prefix(&prefix).is_some_and(|stamp| {
            !stamp.is_empty() && stamp.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            continue;
        }
        if let Ok(meta) = entry.metadata()
            && let Ok(modified) = meta.modified()
        {
            candidates.push((modified, entry.path()));
        }
    }
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    for (_, path) in candidates.into_iter().skip(MIGRATION_BACKUP_KEEP) {
        let _ = std::fs::remove_file(path);
    }
    Ok(())
}

fn init_or_verify(db: &Connection, path: &Path, node_id: &str) -> Result<(), StoreError> {
    match current_version(db)? {
        0 => {
            let tables: i64 = db.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            if tables > 0 {
                return Err(StoreError::InvalidArgument(
                    "database has tables but user_version=0; refusing to guess a migration".into(),
                ));
            }
            init_fresh(db, node_id)
        }
        1 => {
            verify_identity(db, node_id)?;
            require_integrity(db)?;
            snapshot_before_migration(db, path, 1)?;
            retain_migration_backups(path, 1)?;
            migrate_to_v2(db)
        }
        2 => {
            verify_identity(db, node_id)?;
            require_integrity(db)?;
            snapshot_before_migration(db, path, 2)?;
            retain_migration_backups(path, 2)?;
            migrate_to_v3(db)
        }
        3 => {
            verify_identity(db, node_id)?;
            require_integrity(db)?;
            snapshot_before_migration(db, path, 3)?;
            retain_migration_backups(path, 3)?;
            migrate_to_v4(db)
        }
        4 => {
            verify_identity(db, node_id)?;
            require_integrity(db)?;
            snapshot_before_migration(db, path, 4)?;
            retain_migration_backups(path, 4)?;
            migrate_to_v5(db)
        }
        5 => {
            verify_identity(db, node_id)?;
            require_integrity(db)?;
            Ok(())
        }
        newer => Err(StoreError::InvalidArgument(format!(
            "database version {newer} is newer than this program; refusing to open"
        ))),
    }
}

fn verify_backup(db: &Connection, node_id: &str) -> Result<(), StoreError> {
    match current_version(db)? {
        1..=5 => {}
        other => {
            return Err(StoreError::InvalidArgument(format!(
                "backup version {other} is not supported (expected 1-2)"
            )));
        }
    }
    let check: String = db.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(StoreError::Db(format!("backup integrity: {check}")));
    }
    verify_identity(db, node_id)
}

fn store_node(db: &Connection) -> Result<String, StoreError> {
    db.query_row("SELECT node_id FROM store_identity", [], |row| row.get(0))
        .map_err(StoreError::from)
}

/// 恢复后每个 stream 换新 epoch 并记录 recovery.gap；旧 cursor 明确失效。
fn rotate_epochs_after_restore(
    db: &mut Connection,
    ids: &IdGen,
    limits: &StoreLimits,
) -> Result<(), StoreError> {
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let streams: Vec<String> = {
        let mut stmt = tx.prepare("SELECT id FROM streams ORDER BY id")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for stream in streams {
        let epoch = ids.mint();
        tx.execute(
            "UPDATE streams SET epoch=?1 WHERE id=?2",
            params![epoch, stream],
        )?;
        let head: i64 = tx.query_row(
            "SELECT head FROM streams WHERE id=?1",
            params![stream],
            |row| row.get(0),
        )?;
        append_event(
            &tx,
            ids,
            limits,
            &stream,
            &epoch,
            0,
            "recovery.gap",
            &stream,
            head as u64,
            Ev::Gap {
                reason: "restored_from_backup".into(),
                backend_range: None,
            },
        )?;
    }
    tx.commit()?;
    Ok(())
}

// ---- 事件与投影辅助 --------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_event(
    tx: &rusqlite::Transaction<'_>,
    ids: &IdGen,
    limits: &StoreLimits,
    stream_id: &str,
    epoch: &str,
    recorded_at_ms: u64,
    event_type: &str,
    entity_id: &str,
    entity_revision: u64,
    payload: Ev,
) -> Result<AgentEvent, StoreError> {
    let (head, _bytes): (i64, i64) = tx.query_row(
        "SELECT head, event_bytes FROM streams WHERE id=?1",
        params![stream_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let position = u64::try_from(head)
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(StoreError::QuotaExceeded("position_overflow"))?;
    if position > i64::MAX as u64 {
        return Err(StoreError::QuotaExceeded("position_overflow"));
    }
    let data = match &payload {
        Ev::ConversationUpdated(c) => agent_event::Data::ConversationUpdated((**c).clone()),
        Ev::ExecutionUpdated(e) => agent_event::Data::ExecutionUpdated((**e).clone()),
        Ev::InteractionCreated(i) => agent_event::Data::InteractionCreated((**i).clone()),
        Ev::InteractionUpdated(i) => agent_event::Data::InteractionUpdated((**i).clone()),
        Ev::MessageCreated(m) => agent_event::Data::MessageCreated((**m).clone()),
        Ev::Delta {
            message_id,
            part_index,
            offset_bytes,
            text,
        } => agent_event::Data::MessageDelta(AgentMessageDelta {
            message_id: message_id.clone(),
            part_index: *part_index,
            offset_bytes: *offset_bytes,
            text: text.clone(),
        }),
        Ev::MessageCompleted {
            message_id,
            revision,
            state,
        } => agent_event::Data::MessageCompleted(conex_proto::AgentMessageCompleted {
            message_id: message_id.clone(),
            revision: *revision,
            state: state.clone(),
        }),
        Ev::MessagePart { part, .. } => agent_event::Data::MessagePart(part.clone()),
        Ev::Gap {
            reason,
            backend_range,
        } => agent_event::Data::RecoveryGap(AgentGap {
            reason: reason.clone(),
            backend_range: backend_range.clone(),
        }),
    };
    let event = AgentEvent {
        stream_id: stream_id.to_string(),
        epoch: epoch.to_string(),
        position,
        recorded_at_ms,
        r#type: event_type.to_string(),
        entity_id: entity_id.to_string(),
        entity_revision,
        data: Some(data),
    };
    let encoded = encode_bounded(&event, limits.max_event_bytes)?;
    tx.execute(
        "INSERT INTO events(stream_id,position,recorded_at_ms,encoded,encoded_bytes)
         VALUES (?1,?2,?3,?4,?5)",
        params![
            stream_id,
            position as i64,
            recorded_at_ms as i64,
            encoded,
            encoded.len() as i64
        ],
    )?;
    tx.execute(
        "UPDATE streams SET head=?2, event_bytes=event_bytes+?3 WHERE id=?1",
        params![stream_id, position as i64, encoded.len() as i64],
    )?;
    let _ = ids; // ID minting happens at record level, not per event.
    Ok(event)
}

fn load_conversation(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<Conversation, StoreError> {
    let blob: Option<Vec<u8>> = tx
        .query_row(
            "SELECT projection FROM conversations WHERE id=?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let blob = blob.ok_or_else(|| StoreError::NotFound {
        entity: "conversation",
        id: id.into(),
    })?;
    decode::<Conversation>(&blob)
}

fn load_execution(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<Execution, StoreError> {
    let blob: Option<Vec<u8>> = tx
        .query_row(
            "SELECT projection FROM executions WHERE id=?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let blob = blob.ok_or_else(|| StoreError::NotFound {
        entity: "execution",
        id: id.into(),
    })?;
    decode::<Execution>(&blob)
}

fn save_execution(tx: &rusqlite::Transaction<'_>, execution: &Execution) -> Result<(), StoreError> {
    let projection = encode(execution)?;
    tx.execute(
        "UPDATE executions SET state=?2, revision=?3, projection=?4 WHERE id=?1",
        params![
            execution.id,
            execution.state,
            execution.revision as i64,
            projection
        ],
    )?;
    Ok(())
}

fn save_conversation(
    tx: &rusqlite::Transaction<'_>,
    conversation: &Conversation,
) -> Result<(), StoreError> {
    let projection = encode(conversation)?;
    tx.execute(
        "UPDATE conversations SET active_execution_id=?2, revision=?3, projection=?4 WHERE id=?1",
        params![
            conversation.id,
            conversation.active_execution_id,
            conversation.revision as i64,
            projection
        ],
    )?;
    Ok(())
}

type InteractionColumns = (Vec<u8>, String, Option<String>, Option<String>, i64);

type InteractionRow = (
    InteractionRequest,
    String,
    Option<String>,
    Option<String>,
    u64,
);

fn load_interaction(
    tx: &rusqlite::Transaction<'_>,
    id: &str,
) -> Result<InteractionRow, StoreError> {
    let row: Option<InteractionColumns> = tx
        .query_row(
            "SELECT projection, state, decision_digest, responder_id, revision
             FROM interactions WHERE id=?1",
            params![id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    let (blob, state, decision_digest, responder_id, revision) =
        row.ok_or_else(|| StoreError::NotFound {
            entity: "interaction",
            id: id.into(),
        })?;
    Ok((
        decode::<InteractionRequest>(&blob)?,
        state,
        decision_digest,
        responder_id,
        revision as u64,
    ))
}

fn load_message(tx: &rusqlite::Transaction<'_>, id: &str) -> Result<AgentMessage, StoreError> {
    let blob: Option<Vec<u8>> = tx
        .query_row(
            "SELECT projection FROM messages WHERE id=?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    let blob = blob.ok_or_else(|| StoreError::NotFound {
        entity: "message",
        id: id.into(),
    })?;
    decode::<AgentMessage>(&blob)
}

fn save_message(tx: &rusqlite::Transaction<'_>, message: &AgentMessage) -> Result<(), StoreError> {
    let projection = encode(message)?;
    tx.execute(
        "UPDATE messages SET state=?2, revision=?3, projection=?4 WHERE id=?1",
        params![
            message.id,
            message.state,
            message.revision as i64,
            projection
        ],
    )?;
    Ok(())
}

fn endpoint_generation(
    tx: &rusqlite::Transaction<'_>,
    endpoint_id: &str,
) -> Result<u64, StoreError> {
    let blob: Option<Vec<u8>> = tx
        .query_row(
            "SELECT projection FROM entities WHERE id=?1 AND kind='endpoint'",
            params![endpoint_id],
            |row| row.get(0),
        )
        .optional()?;
    let blob = blob.ok_or_else(|| StoreError::NotFound {
        entity: "endpoint",
        id: endpoint_id.into(),
    })?;
    let endpoint: conex_proto::AgentAdapterEndpoint = decode(&blob)?;
    Ok(endpoint.generation)
}

fn stream_epoch(tx: &rusqlite::Transaction<'_>, stream_id: &str) -> Result<String, StoreError> {
    tx.query_row(
        "SELECT epoch FROM streams WHERE id=?1",
        params![stream_id],
        |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| StoreError::NotFound {
        entity: "stream",
        id: stream_id.into(),
    })
}

/// 活动执行/未确定交互/活跃消息超限时：有界事务写 gap、执行转 unknown、
/// 端点隔离；不追加无法快照的原始对象，不自动批准（契约 §5、恢复契约 §5）。
fn bounded_failure(
    tx: &rusqlite::Transaction<'_>,
    ids: &IdGen,
    limits: &StoreLimits,
    now: u64,
    execution: &Execution,
    reason: &'static str,
) -> Result<ApplyResult, StoreError> {
    let mut row = execution.clone();
    let epoch = stream_epoch(tx, &row.conversation_id)?;
    append_event(
        tx,
        ids,
        limits,
        &row.conversation_id,
        &epoch,
        now,
        "recovery.gap",
        &row.id,
        row.revision,
        Ev::Gap {
            reason: reason.to_string(),
            backend_range: None,
        },
    )?;
    row.state = "unknown".into();
    row.revision += 1;
    row.updated_at_ms = now;
    save_execution(tx, &row)?;
    append_event(
        tx,
        ids,
        limits,
        &row.conversation_id,
        &epoch,
        now,
        "execution.updated",
        &row.id,
        row.revision,
        Ev::ExecutionUpdated(Box::new(row.clone())),
    )?;
    isolate_endpoint(tx, &row.endpoint_id)?;
    Ok(ApplyResult {
        execution_state: Some("unknown".into()),
        applied: false,
        ..ApplyResult::default()
    })
}

fn isolate_endpoint(tx: &rusqlite::Transaction<'_>, endpoint_id: &str) -> Result<(), StoreError> {
    let blob: Option<Vec<u8>> = tx
        .query_row(
            "SELECT projection FROM entities WHERE id=?1 AND kind='endpoint'",
            params![endpoint_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(blob) = blob else { return Ok(()) };
    let mut endpoint: conex_proto::AgentAdapterEndpoint = decode(&blob)?;
    endpoint.availability = "unreachable".into();
    let projection = encode(&endpoint)?;
    tx.execute(
        "UPDATE entities SET revision=revision+1, projection=?2 WHERE id=?1 AND kind='endpoint'",
        params![endpoint_id, projection],
    )?;
    Ok(())
}

/// 幂等记录：先删除过期键，再做容量检查与插入（契约 §5；恢复契约 §5）。
fn dedup_maintenance(
    tx: &rusqlite::Transaction<'_>,
    principal: &str,
    endpoint: &str,
    operation: &str,
    key: &str,
    now: u64,
    limits: &StoreLimits,
) -> Result<(), StoreError> {
    tx.execute(
        "DELETE FROM dedup WHERE principal_id=?1 AND endpoint_id=?2 AND operation=?3
         AND key=?4 AND expires_at_ms<=?5",
        params![principal, endpoint, operation, key, now as i64],
    )?;
    let unexpired: i64 = tx.query_row(
        "SELECT COUNT(*) FROM dedup WHERE expires_at_ms>?1",
        params![now as i64],
        |row| row.get(0),
    )?;
    if unexpired as usize >= limits.max_dedup_records {
        return Err(StoreError::QuotaExceeded("dedup_capacity"));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_dedup(
    tx: &rusqlite::Transaction<'_>,
    principal: &str,
    endpoint: &str,
    operation: &str,
    key: &str,
    object_digest: &str,
    object_id: &str,
    now: u64,
    limits: &StoreLimits,
) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO dedup(principal_id,endpoint_id,operation,key,digest,object_id,
         created_at_ms,expires_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            principal,
            endpoint,
            operation,
            key,
            object_digest,
            object_id,
            now as i64,
            (now + limits.dedup_ttl_ms) as i64
        ],
    )?;
    Ok(())
}

fn find_dedup(
    tx: &rusqlite::Transaction<'_>,
    principal: &str,
    endpoint: &str,
    operation: &str,
    key: &str,
) -> Result<Option<(String, String)>, StoreError> {
    Ok(tx
        .query_row(
            "SELECT digest, object_id FROM dedup
             WHERE principal_id=?1 AND endpoint_id=?2 AND operation=?3 AND key=?4",
            params![principal, endpoint, operation, key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?)
}

// ---- 公开领域操作 ----------------------------------------------------------

impl Store {
    /// P2.2 节点启动登记：持久换新 nodeRuntimeEpoch（ULID）并绑定
    /// node_metadata。epoch 只在节点进程启动改变；中心 link ready 不改变它
    /// （private-runtime §1）。nodeId 必须与库绑定的 store_identity 一致，
    /// 开始时已校验，此处再防调用方传错节点直接登记。
    pub async fn begin_node_boot(&self, node_id: &str) -> Result<String, StoreError> {
        let node = node_id.to_string();
        self.immediate(move |tx, ids, _limits| {
            let stored: String =
                tx.query_row("SELECT node_id FROM store_identity", [], |row| row.get(0))?;
            if stored != node {
                return Err(StoreError::NodeMismatch {
                    expected: stored,
                    actual: node,
                });
            }
            let epoch = ids.mint();
            tx.execute(
                "INSERT INTO node_metadata(node_id, runtime_epoch) VALUES (?1, ?2)
                 ON CONFLICT(node_id) DO UPDATE SET runtime_epoch=excluded.runtime_epoch",
                params![node, &epoch],
            )?;
            Ok(epoch)
        })
        .await
    }

    /// P2.2 模块代际：在立即事务内持久单调递增，跨重启有效——不能靠进程内
    /// 计数"重启+1"（计划 §3）。首次调用返回 1；到达 i64::MAX 上限拒绝再增。
    pub async fn next_module_generation(&self, module_key: &str) -> Result<u64, StoreError> {
        require_module_key(module_key)?;
        let key = module_key.to_string();
        self.immediate(move |tx, _ids, _limits| {
            let current: Option<i64> = tx
                .query_row(
                    "SELECT generation FROM module_generations WHERE module_key=?1",
                    params![key],
                    |row| row.get(0),
                )
                .optional()?;
            let next = match current {
                None => 1,
                Some(generation) => {
                    if generation == i64::MAX {
                        return Err(StoreError::QuotaExceeded("module_generation_exhausted"));
                    }
                    generation + 1
                }
            };
            tx.execute(
                "INSERT INTO module_generations(module_key, generation) VALUES (?1, ?2)
                 ON CONFLICT(module_key) DO UPDATE SET generation=?2",
                params![key, next],
            )?;
            Ok(next as u64)
        })
        .await
    }

    /// P2.2 登记自有资源：真实创建者 + 可验证控制身份（private-runtime §1/§6）。
    /// control_identity 是不透明控制通道凭据，仅节点受限存储，不投影中心。
    /// 重复登记同名资源返回 conflict。
    pub async fn record_owned_resource(&self, resource: OwnedResource) -> Result<(), StoreError> {
        require_ulid(&resource.id, "resource id")?;
        require_module_key(&resource.module_key)?;
        if resource.generation > i64::MAX as u64 {
            return Err(StoreError::InvalidArgument(
                "generation exceeds i64 range".into(),
            ));
        }
        self.immediate(move |tx, _ids, _limits| {
            let inserted = tx.execute(
                "INSERT INTO owned_resources(
                     id, module_key, generation, owner_epoch, control_kind,
                     control_identity, state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    resource.id,
                    resource.module_key,
                    resource.generation as i64,
                    resource.owner_epoch,
                    resource.control_kind,
                    resource.control_identity,
                    resource.state.as_str(),
                ],
            );
            match inserted {
                Ok(1) => Ok(()),
                Ok(_) => Err(StoreError::Db(
                    "owned_resources insert affected an unexpected row count".into(),
                )),
                Err(rusqlite::Error::SqliteFailure(error, _))
                    if error.code == rusqlite::ErrorCode::ConstraintViolation =>
                {
                    Err(StoreError::Conflict("owned_resource_already_recorded"))
                }
                Err(error) => Err(StoreError::from(error)),
            }
        })
        .await
    }

    /// Complete a previously persisted spawn reservation. The creator's epoch
    /// and created state fence late readiness from changing another resource.
    pub async fn record_resource_ready(
        &self,
        resource_id: &str,
        owner_epoch: &str,
        control_identity: Vec<u8>,
    ) -> Result<(), StoreError> {
        let id = resource_id.to_owned();
        let epoch = owner_epoch.to_owned();
        self.immediate(move |tx, _ids, _limits| {
            let updated = tx.execute(
                "UPDATE owned_resources SET control_identity=?3, state='active'
                 WHERE id=?1 AND owner_epoch=?2 AND state='created'",
                params![id, epoch, control_identity],
            )?;
            if updated != 1 {
                return Err(StoreError::Conflict("resource_reservation_changed"));
            }
            Ok(())
        })
        .await
    }

    /// P2.2 未核对清单：state IN ('created','active','unknown')，即上一 boot
    /// 残留、尚不能证明已退出的自有资源。节点崩溃后先核对清单再决定是否重建；
    /// 未知仍占其运行约束，不自动并行启动冲突新实例（private-runtime §1）。
    /// released 不返回；按 id 稳定排序。
    pub async fn list_unreconciled_resources(&self) -> Result<Vec<OwnedResource>, StoreError> {
        self.readonly(move |tx, _ids, _limits| {
            type ResourceRow = (String, String, i64, String, String, Vec<u8>, String);
            let rows: Vec<ResourceRow> = {
                let mut stmt = tx.prepare(
                    "SELECT id, module_key, generation, owner_epoch, control_kind,
                            control_identity, state
                     FROM owned_resources WHERE state <> 'released' ORDER BY id",
                )?;
                let iter = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, String>(6)?,
                    ))
                })?;
                iter.collect::<Result<Vec<_>, _>>()?
            };
            let mut resources = Vec::with_capacity(rows.len());
            for (id, module_key, generation, owner_epoch, control_kind, control_identity, state) in
                rows
            {
                let state = ResourceState::parse_label(&state).ok_or_else(|| {
                    StoreError::Db(format!("owned_resources invalid state {state:?}"))
                })?;
                resources.push(OwnedResource {
                    id,
                    module_key,
                    generation: generation as u64,
                    owner_epoch,
                    control_kind,
                    control_identity,
                    state,
                });
            }
            Ok(resources)
        })
        .await
    }

    /// Record verified control, verified exit, or an explicitly unknown outcome.
    /// Released is terminal: a late callback cannot resurrect a retired resource.
    pub async fn record_resource_reconciled(
        &self,
        resource_id: &str,
        state: ResourceState,
    ) -> Result<(), StoreError> {
        if state == ResourceState::Created {
            return Err(StoreError::InvalidArgument(
                "reconciliation cannot restore a creation reservation".into(),
            ));
        }
        let id = resource_id.to_string();
        self.immediate(move |tx, _ids, _limits| {
            let updated = tx.execute(
                "UPDATE owned_resources SET state=?2
                 WHERE id=?1 AND (state <> 'released' OR ?2='released')",
                params![id, state.as_str()],
            )?;
            if updated != 1 {
                if tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM owned_resources WHERE id=?1)",
                    params![id],
                    |row| row.get::<_, bool>(0),
                )? {
                    return Err(StoreError::Conflict("resource_already_released"));
                }
                return Err(StoreError::NotFound {
                    entity: "owned_resource",
                    id,
                });
            }
            Ok(())
        })
        .await
    }

    /// 登记端点实体（节点批准的适配器配置，不是客户端输入）。
    pub async fn put_endpoint(
        &self,
        endpoint: conex_proto::AgentAdapterEndpoint,
    ) -> Result<(), StoreError> {
        self.immediate(move |tx, _ids, _limits| {
            let projection = encode(&endpoint)?;
            tx.execute(
                "INSERT INTO entities(id,kind,revision,projection) VALUES (?1,'endpoint',1,?2)
                 ON CONFLICT(id) DO UPDATE SET revision=revision+1, projection=?2",
                params![endpoint.id, projection],
            )?;
            Ok(())
        })
        .await
    }

    /// 登记工作区实体（rootRef 是节点批准的配置键）。
    pub async fn put_workspace(&self, workspace: Workspace) -> Result<(), StoreError> {
        self.immediate(move |tx, _ids, _limits| {
            let projection = encode(&workspace)?;
            tx.execute(
                "INSERT INTO entities(id,kind,revision,projection) VALUES (?1,'workspace',1,?2)
                 ON CONFLICT(id) DO UPDATE SET revision=revision+1, projection=?2",
                params![workspace.id, projection],
            )?;
            Ok(())
        })
        .await
    }

    /// 原子创建节点侧空会话与幂等记录；同键同摘要返回同一会话。
    pub async fn create_conversation(
        &self,
        authorized: &AuthorizedScope,
        endpoint_id: &str,
        workspace_id: &str,
        idempotency_key: &str,
        title: Option<&str>,
        now: u64,
    ) -> Result<Conversation, StoreError> {
        require_ulid(endpoint_id, "endpointId")?;
        require_ulid(workspace_id, "workspaceId")?;
        require_key(idempotency_key)?;
        let authorized = authorized.clone();
        let endpoint = endpoint_id.to_string();
        let workspace = workspace_id.to_string();
        let key = idempotency_key.to_string();
        let title = title.map(str::to_string);
        self.immediate(move |tx, ids, limits| {
            let exists: bool = tx.query_row(
                "SELECT COUNT(*) FROM entities WHERE id=?1 AND kind='endpoint'",
                params![endpoint],
                |row| row.get::<_, i64>(0),
            )? > 0;
            if !exists {
                return Err(StoreError::NotFound {
                    entity: "endpoint",
                    id: endpoint.clone(),
                });
            }
            let exists: bool = tx.query_row(
                "SELECT COUNT(*) FROM entities WHERE id=?1 AND kind='workspace'",
                params![workspace],
                |row| row.get::<_, i64>(0),
            )? > 0;
            if !exists {
                return Err(StoreError::NotFound {
                    entity: "workspace",
                    id: workspace.clone(),
                });
            }
            let digest_value = digest(&[
                endpoint.as_bytes(),
                workspace.as_bytes(),
                title.as_deref().unwrap_or("").as_bytes(),
            ]);
            dedup_maintenance(
                tx,
                &authorized.principal_id,
                &endpoint,
                "create",
                &key,
                now,
                limits,
            )?;
            if let Some((stored, object_id)) =
                find_dedup(tx, &authorized.principal_id, &endpoint, "create", &key)?
            {
                if stored != digest_value {
                    return Err(StoreError::IdempotencyMismatch {
                        existing_id: object_id,
                    });
                }
                return load_conversation(tx, &object_id);
            }
            let conversation_id = ids.mint();
            let epoch = ids.mint();
            tx.execute(
                "INSERT INTO streams(id,epoch) VALUES (?1,?2)",
                params![conversation_id, epoch],
            )?;
            let conversation = Conversation {
                id: conversation_id.clone(),
                endpoint_id: endpoint.clone(),
                workspace_id: workspace.clone(),
                created_at_ms: now,
                revision: 1,
                quality: "current".into(),
                title,
                active_execution_id: None,
            };
            let projection = encode_bounded(&conversation, limits.max_conversation_bytes)?;
            tx.execute(
                "INSERT INTO conversations(id,endpoint_id,workspace_id,active_execution_id,
                 revision,projection) VALUES (?1,?2,?3,NULL,1,?4)",
                params![conversation_id, endpoint, workspace, projection],
            )?;
            // 会话也是实体（P6 标注的存在性校验以 entities 表为准）；与端点/
            // 工作区一致：创建即登记，标注永远指向已存在实体。
            tx.execute(
                "INSERT INTO entities(id,kind,revision,projection) VALUES (?1,'conversation',1,?2)
                 ON CONFLICT(id) DO UPDATE SET revision=revision+1, projection=?2",
                params![conversation_id, encode(&conversation)?],
            )?;
            insert_dedup(
                tx,
                &authorized.principal_id,
                &endpoint,
                "create",
                &key,
                &digest_value,
                &conversation_id,
                now,
                limits,
            )?;
            append_event(
                tx,
                ids,
                limits,
                &conversation_id,
                &epoch,
                now,
                "conversation.updated",
                &conversation_id,
                1,
                Ev::ConversationUpdated(Box::new(conversation.clone())),
            )?;
            Ok(conversation)
        })
        .await
    }

    /// 准入（契约 §5 算法）：同键同摘要返回 Existing（无 permit），不同摘要
    /// conflict；busy 不入队也不消耗幂等键。
    pub async fn admit(
        &self,
        authorized: &AuthorizedScope,
        module_instance_id: &str,
        input: &AgentSubmitInput,
        now: u64,
    ) -> Result<Admission, StoreError> {
        require_ulid(module_instance_id, "moduleInstanceId")?;
        require_ulid(&input.conversation_id, "conversationId")?;
        require_key(&input.idempotency_key)?;
        if input.content.is_empty() {
            return Err(StoreError::InvalidArgument(
                "content must not be empty".into(),
            ));
        }
        if input.content.len() > 65536 {
            return Err(StoreError::PayloadTooLarge { limit: 65536 });
        }
        let authorized = authorized.clone();
        let input = input.clone();
        let module_instance = module_instance_id.to_string();
        self.immediate(move |tx, ids, limits| {
            let module_instance_id = module_instance.as_str();
            let conversation = load_conversation(tx, &input.conversation_id)?;
            let endpoint = conversation.endpoint_id.clone();
            let digest_value = digest(&[
                input.conversation_id.as_bytes(),
                input.idempotency_key.as_bytes(),
                input.content.as_bytes(),
            ]);
            dedup_maintenance(
                tx,
                &authorized.principal_id,
                &endpoint,
                "submit",
                &input.idempotency_key,
                now,
                limits,
            )?;
            if let Some((stored, object_id)) = find_dedup(
                tx,
                &authorized.principal_id,
                &endpoint,
                "submit",
                &input.idempotency_key,
            )? {
                if stored != digest_value {
                    return Err(StoreError::IdempotencyMismatch {
                        existing_id: object_id,
                    });
                }
                let execution = load_execution(tx, &object_id)?;
                return Ok(Admission::Existing { execution });
            }
            if let Some(active) = conversation.active_execution_id.clone() {
                return Err(StoreError::ConversationBusy {
                    active_execution_id: active,
                });
            }
            let active_count: i64 = tx.query_row(
                "SELECT COUNT(*) FROM executions WHERE conversation_id=?1
                 AND state IN ('accepted','dispatching','running','unknown')",
                params![conversation.id],
                |row| row.get(0),
            )?;
            if active_count as usize >= limits.max_active_executions {
                return Err(StoreError::QuotaExceeded("active_executions"));
            }
            let generation = endpoint_generation(tx, &endpoint)?;
            let execution_id = ids.mint();
            let execution = Execution {
                id: execution_id.clone(),
                conversation_id: conversation.id.clone(),
                endpoint_id: endpoint.clone(),
                module_instance_id: module_instance_id.to_string(),
                generation,
                state: "accepted".into(),
                revision: 1,
                created_at_ms: now,
                updated_at_ms: now,
                parent_execution_id: None,
                terminal_reason: None,
                cancel_requested_at_ms: None,
                backend_ref: None,
            };
            let projection = encode_bounded(&execution, limits.max_execution_meta_bytes)?;
            tx.execute(
                "INSERT INTO executions(id,conversation_id,parent_execution_id,generation,
                 state,revision,projection) VALUES (?1,?2,NULL,?3,'accepted',1,?4)",
                params![execution_id, conversation.id, generation as i64, projection],
            )?;
            let mut conversation = conversation;
            conversation.active_execution_id = Some(execution_id.clone());
            conversation.revision += 1;
            save_conversation(tx, &conversation)?;
            insert_dedup(
                tx,
                &authorized.principal_id,
                &endpoint,
                "submit",
                &input.idempotency_key,
                &digest_value,
                &execution_id,
                now,
                limits,
            )?;
            let epoch = stream_epoch(tx, &conversation.id)?;
            append_event(
                tx,
                ids,
                limits,
                &conversation.id,
                &epoch,
                now,
                "execution.updated",
                &execution_id,
                1,
                Ev::ExecutionUpdated(Box::new(execution.clone())),
            )?;
            append_event(
                tx,
                ids,
                limits,
                &conversation.id,
                &epoch,
                now,
                "conversation.updated",
                &conversation.id,
                conversation.revision,
                Ev::ConversationUpdated(Box::new(conversation.clone())),
            )?;
            // 用户消息：complete、全文进入投影；事件先 created（空窗口）再
            // <=4096 字节 delta 与 completed（契约 §4）。
            let message_id = ids.mint();
            let ordinal: i64 = tx.query_row(
                "SELECT COALESCE(MAX(ordinal),0) FROM messages WHERE conversation_id=?1",
                params![conversation.id],
                |row| row.get(0),
            )?;
            let message = AgentMessage {
                id: message_id.clone(),
                conversation_id: conversation.id.clone(),
                execution_id: execution_id.clone(),
                role: "user".into(),
                state: "complete".into(),
                revision: 1,
                parts: vec![conex_proto::AgentMessagePart {
                    part: Some(agent_message_part::Part::Text(conex_proto::AgentTextPart {
                        text: input.content.clone(),
                        offset_bytes: 0,
                        total_bytes: input.content.len() as u64,
                    })),
                }],
            };
            let mut window_message = message.clone();
            if let Some(mp) = window_message.parts.first_mut()
                && let Some(agent_message_part::Part::Text(part)) = mp.part.as_mut()
            {
                part.text = String::new();
            }
            let meta = encode(&window_message)?;
            if meta.len() > limits.max_message_meta_bytes {
                return Err(StoreError::QuotaExceeded("message_metadata"));
            }
            tx.execute(
                "INSERT INTO messages(id,conversation_id,execution_id,ordinal,backend_key,
                 revision,state,projection)
                 VALUES (?1,?2,?3,?4,NULL,1,'complete',?5)",
                params![
                    message_id,
                    conversation.id,
                    execution_id,
                    ordinal + 1,
                    encode(&message)?
                ],
            )?;
            append_event(
                tx,
                ids,
                limits,
                &conversation.id,
                &epoch,
                now,
                "message.created",
                &message_id,
                1,
                Ev::MessageCreated(Box::new(window_message)),
            )?;
            let mut offset = 0u64;
            for chunk in split_utf8(&input.content, limits.max_delta_bytes) {
                append_event(
                    tx,
                    ids,
                    limits,
                    &conversation.id,
                    &epoch,
                    now,
                    "message.delta",
                    &message_id,
                    1,
                    Ev::Delta {
                        message_id: message_id.clone(),
                        part_index: 0,
                        offset_bytes: offset,
                        text: chunk.to_string(),
                    },
                )?;
                offset += chunk.len() as u64;
            }
            append_event(
                tx,
                ids,
                limits,
                &conversation.id,
                &epoch,
                now,
                "message.completed",
                &message_id,
                1,
                Ev::MessageCompleted {
                    message_id: message_id.clone(),
                    revision: 1,
                    state: "complete".into(),
                },
            )?;
            Ok(Admission::Created {
                execution,
                dispatch_token: DispatchToken::new(execution_id, generation),
            })
        })
        .await
    }

    /// dispatching 必须先持久化才能发送：accepted→dispatching 单次 CAS。
    pub async fn mark_dispatching(
        &self,
        execution_id: &str,
        generation: u64,
        token: DispatchToken,
        now: u64,
    ) -> Result<DispatchPermit, StoreError> {
        if token.execution_id() != execution_id || token.generation() != generation {
            return Err(StoreError::InvalidArgument(
                "dispatch token does not match execution".into(),
            ));
        }
        let execution = execution_id.to_string();
        self.immediate(move |tx, ids, limits| {
            let loaded = load_execution(tx, &execution)?;
            if loaded.generation != generation {
                return Err(StoreError::StaleRevision {
                    expected: generation,
                    actual: loaded.generation,
                });
            }
            let updated = tx.execute(
                "UPDATE executions SET state='dispatching', revision=revision+1 WHERE id=?1
                 AND state='accepted'",
                params![execution],
            )?;
            if updated != 1 {
                return Err(match loaded.state.as_str() {
                    "accepted" => StoreError::Conflict("execution_state"),
                    _ => StoreError::Conflict("already_dispatched"),
                });
            }
            let mut row = loaded;
            row.state = "dispatching".into();
            row.revision += 1;
            row.updated_at_ms = now;
            save_execution(tx, &row)?;
            let conversation_id = row.conversation_id.clone();
            let epoch = stream_epoch(tx, &conversation_id)?;
            append_event(
                tx,
                ids,
                limits,
                &conversation_id,
                &epoch,
                now,
                "execution.updated",
                &execution,
                row.revision,
                Ev::ExecutionUpdated(Box::new(row)),
            )?;
            Ok(DispatchPermit::new(execution, generation))
        })
        .await
    }

    /// 依据真实协议观察推进执行；适配器不是任意后台 JSON 的搬运工。
    pub async fn apply_backend(
        &self,
        execution_id: &str,
        generation: u64,
        observation: BackendObservation,
        now: u64,
    ) -> Result<ApplyResult, StoreError> {
        require_ulid(execution_id, "executionId")?;
        let execution = execution_id.to_string();
        self.immediate(move |tx, ids, limits| {
            let current = load_execution(tx, &execution)?;
            if current.generation != generation {
                return Err(StoreError::StaleRevision {
                    expected: generation,
                    actual: current.generation,
                });
            }
            if TERMINAL_EXECUTION_STATES.contains(&current.state.as_str()) {
                // 终态单次转换；迟到观察保留诊断而不覆盖。
                return Ok(ApplyResult {
                    applied: false,
                    ..ApplyResult::default()
                });
            }
            let epoch = stream_epoch(tx, &current.conversation_id)?;
            match observation {
                BackendObservation::Started { backend_ref } => {
                    let mut row = current;
                    row.state = "running".into();
                    row.revision += 1;
                    row.updated_at_ms = now;
                    row.backend_ref = backend_ref;
                    save_execution(tx, &row)?;
                    append_event(
                        tx,
                        ids,
                        limits,
                        &row.conversation_id,
                        &epoch,
                        now,
                        "execution.updated",
                        &row.id,
                        row.revision,
                        Ev::ExecutionUpdated(Box::new(row.clone())),
                    )?;
                    Ok(ApplyResult {
                        execution_state: Some(row.state),
                        applied: true,
                        ..ApplyResult::default()
                    })
                }
                BackendObservation::MessageStarted { role, backend_key } => {
                    if !ROLES.contains(&role.as_str()) {
                        return Err(StoreError::InvalidArgument(format!("unknown role {role}")));
                    }
                    if let Some(key) = backend_key.as_deref()
                        && key.is_empty()
                    {
                        return Err(StoreError::InvalidArgument(
                            "backend message key must not be empty".into(),
                        ));
                    }
                    let active_messages: i64 = tx.query_row(
                        "SELECT COUNT(*) FROM messages WHERE conversation_id=?1
                         AND state='streaming'",
                        params![current.conversation_id],
                        |row| row.get(0),
                    )?;
                    if active_messages as usize >= limits.max_active_messages {
                        return bounded_failure(tx, ids, limits, now, &current, "message_quota");
                    }
                    let ordinal: i64 = tx.query_row(
                        "SELECT COALESCE(MAX(ordinal),0) FROM messages WHERE conversation_id=?1",
                        params![current.conversation_id],
                        |row| row.get(0),
                    )?;
                    let message_id = ids.mint();
                    // 零部件起步：全默认的空 text part 会被 pbjson 序列化成
                    // {"text":{}}（默认值省略），无法反序列化回投影；首个部件
                    // 由第一次 TextAppended/ToolPart 创建。
                    let message = AgentMessage {
                        id: message_id.clone(),
                        conversation_id: current.conversation_id.clone(),
                        execution_id: current.id.clone(),
                        role,
                        state: "streaming".into(),
                        revision: 1,
                        parts: vec![],
                    };
                    let meta = encode(&message)?;
                    if meta.len() > limits.max_message_meta_bytes {
                        return bounded_failure(tx, ids, limits, now, &current, "message_metadata");
                    }
                    tx.execute(
                        "INSERT INTO messages(id,conversation_id,execution_id,ordinal,backend_key,
                         revision,state,projection) VALUES (?1,?2,?3,?4,?5,1,'streaming',?6)",
                        params![
                            message_id,
                            message.conversation_id,
                            message.execution_id,
                            ordinal + 1,
                            backend_key,
                            meta
                        ],
                    )?;
                    append_event(
                        tx,
                        ids,
                        limits,
                        &current.conversation_id,
                        &epoch,
                        now,
                        "message.created",
                        &message_id,
                        1,
                        Ev::MessageCreated(Box::new(message)),
                    )?;
                    Ok(ApplyResult {
                        message_id: Some(message_id),
                        applied: true,
                        ..ApplyResult::default()
                    })
                }
                BackendObservation::TextAppended { message_id, text } => {
                    append_text(tx, ids, limits, now, &current, &epoch, &message_id, &text)
                }
                BackendObservation::MessageCompleted {
                    message_id,
                    incomplete,
                } => {
                    let mut message = load_message(tx, &message_id)?;
                    if message.conversation_id != current.conversation_id
                        || message.execution_id != current.id
                    {
                        return Err(StoreError::NotFound {
                            entity: "message",
                            id: message_id,
                        });
                    }
                    message.state = if incomplete {
                        "incomplete".to_string()
                    } else {
                        "complete".to_string()
                    };
                    message.revision += 1;
                    save_message(tx, &message)?;
                    append_event(
                        tx,
                        ids,
                        limits,
                        &current.conversation_id,
                        &epoch,
                        now,
                        "message.completed",
                        &message_id,
                        message.revision,
                        Ev::MessageCompleted {
                            message_id: message_id.clone(),
                            revision: message.revision,
                            state: message.state.clone(),
                        },
                    )?;
                    Ok(ApplyResult {
                        applied: true,
                        ..ApplyResult::default()
                    })
                }
                BackendObservation::ToolPartAppended {
                    message_id,
                    name,
                    phase,
                    summary,
                } => {
                    if !matches!(phase.as_str(), "started" | "completed") {
                        return Err(StoreError::InvalidArgument(format!(
                            "unknown tool phase {phase}"
                        )));
                    }
                    if name.trim().is_empty() {
                        return Err(StoreError::InvalidArgument(
                            "tool name must not be empty".into(),
                        ));
                    }
                    if summary.len() > limits.max_tool_summary_bytes {
                        return Err(StoreError::PayloadTooLarge {
                            limit: limits.max_tool_summary_bytes,
                        });
                    }
                    let mut message = load_message(tx, &message_id)?;
                    if message.conversation_id != current.conversation_id
                        || message.execution_id != current.id
                    {
                        return Err(StoreError::NotFound {
                            entity: "message",
                            id: message_id,
                        });
                    }
                    if message.parts.len() >= limits.max_parts {
                        return bounded_failure(tx, ids, limits, now, &current, "part_quota");
                    }
                    let part = conex_proto::AgentMessagePart {
                        part: Some(agent_message_part::Part::Tool(conex_proto::AgentToolPart {
                            name: name.clone(),
                            phase: phase.clone(),
                            summary: summary.clone(),
                        })),
                    };
                    message.parts.push(part.clone());
                    message.revision += 1;
                    save_message(tx, &message)?;
                    append_event(
                        tx,
                        ids,
                        limits,
                        &current.conversation_id,
                        &epoch,
                        now,
                        "message.part",
                        &message.id,
                        message.revision,
                        Ev::MessagePart { part },
                    )?;
                    Ok(ApplyResult {
                        applied: true,
                        ..ApplyResult::default()
                    })
                }
                BackendObservation::InteractionOpened {
                    kind,
                    prompt,
                    choices,
                    expires_at_ms,
                } => open_interaction(
                    tx,
                    ids,
                    limits,
                    now,
                    &current,
                    &epoch,
                    kind,
                    prompt,
                    choices,
                    expires_at_ms,
                ),
                BackendObservation::Quiescent { state } => {
                    apply_terminal(tx, ids, limits, now, &current, &epoch, state)
                }
                BackendObservation::OutcomeUnknown { .. } => {
                    let mut row = current;
                    row.state = "unknown".into();
                    row.revision += 1;
                    row.updated_at_ms = now;
                    save_execution(tx, &row)?;
                    append_event(
                        tx,
                        ids,
                        limits,
                        &row.conversation_id,
                        &epoch,
                        now,
                        "execution.updated",
                        &row.id,
                        row.revision,
                        Ev::ExecutionUpdated(Box::new(row.clone())),
                    )?;
                    Ok(ApplyResult {
                        execution_state: Some("unknown".into()),
                        applied: true,
                        ..ApplyResult::default()
                    })
                }
            }
        })
        .await
    }

    /// 取消是请求不是承诺；已终态返回 requested=false 与当前 state。
    pub async fn request_cancel(
        &self,
        _authorized: &AuthorizedScope,
        execution_id: &str,
        generation: u64,
        now: u64,
    ) -> Result<CancelTarget, StoreError> {
        require_ulid(execution_id, "executionId")?;
        let execution = execution_id.to_string();
        self.immediate(move |tx, ids, limits| {
            let mut row = load_execution(tx, &execution)?;
            if row.generation != generation {
                return Err(StoreError::StaleRevision {
                    expected: generation,
                    actual: row.generation,
                });
            }
            if TERMINAL_EXECUTION_STATES.contains(&row.state.as_str()) {
                return Ok(CancelTarget {
                    execution_id: execution,
                    requested: false,
                    state: row.state,
                });
            }
            row.cancel_requested_at_ms = Some(now);
            row.revision += 1;
            row.updated_at_ms = now;
            save_execution(tx, &row)?;
            let epoch = stream_epoch(tx, &row.conversation_id)?;
            append_event(
                tx,
                ids,
                limits,
                &row.conversation_id,
                &epoch,
                now,
                "execution.updated",
                &execution,
                row.revision,
                Ev::ExecutionUpdated(Box::new(row.clone())),
            )?;
            Ok(CancelTarget {
                execution_id: execution,
                requested: true,
                state: row.state,
            })
        })
        .await
    }

    /// 恢复核对（契约 §5）：accepted 证明未发送→failed/not_dispatched 清槽；
    /// dispatching/running→unknown 保持占槽；responding 交互→unknown；
    /// pending 保持待核对。返回只读目标，不含任何 permit。
    pub async fn recover_unfinished(&self, now: u64) -> Result<Vec<ReconcileTarget>, StoreError> {
        self.immediate(move |tx, ids, limits| {
            let mut targets = Vec::new();
            let rows: Vec<(String, String, i64)> = {
                let mut stmt = tx.prepare(
                    "SELECT id, state, generation FROM executions
                     WHERE state IN ('accepted','dispatching','running','unknown')",
                )?;
                let mapped =
                    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
                mapped.collect::<Result<Vec<_>, _>>()?
            };
            for (id, state, _generation) in rows {
                let mut execution = load_execution(tx, &id)?;
                let epoch = stream_epoch(tx, &execution.conversation_id)?;
                match state.as_str() {
                    "accepted" => {
                        execution.state = "failed".into();
                        execution.terminal_reason = Some("not_dispatched".into());
                        execution.revision += 1;
                        execution.updated_at_ms = now;
                        save_execution(tx, &execution)?;
                        clear_slot(tx, &execution.conversation_id, &execution.id)?;
                        append_event(
                            tx,
                            ids,
                            limits,
                            &execution.conversation_id,
                            &epoch,
                            now,
                            "execution.updated",
                            &execution.id,
                            execution.revision,
                            Ev::ExecutionUpdated(Box::new(execution.clone())),
                        )?;
                        invalidate_pending_interactions(
                            tx,
                            ids,
                            limits,
                            now,
                            &execution,
                            &epoch,
                            "invalidated",
                        )?;
                    }
                    "dispatching" | "running" => {
                        execution.state = "unknown".into();
                        execution.revision += 1;
                        execution.updated_at_ms = now;
                        save_execution(tx, &execution)?;
                        append_event(
                            tx,
                            ids,
                            limits,
                            &execution.conversation_id,
                            &epoch,
                            now,
                            "execution.updated",
                            &execution.id,
                            execution.revision,
                            Ev::ExecutionUpdated(Box::new(execution.clone())),
                        )?;
                        demote_responding_interactions(tx, ids, limits, now, &execution, &epoch)?;
                        targets.push(target_of(&execution));
                    }
                    _ => targets.push(target_of(&execution)),
                }
            }
            Ok(targets)
        })
        .await
    }

    /// 恢复核对无法确认 pending/responding 审批仍有效：事务失效该交互、
    /// 增加 revision、写事件；执行保持 unknown/占槽，不自动拒绝或重放批准。
    pub async fn invalidate_unconfirmed_interaction(
        &self,
        interaction_id: &str,
        generation: u64,
        _reason: &str,
        now: u64,
    ) -> Result<InteractionRequest, StoreError> {
        require_ulid(interaction_id, "interactionId")?;
        let interaction = interaction_id.to_string();
        self.immediate(move |tx, ids, limits| {
            let (mut request, state, _digest, _responder, _revision) =
                load_interaction(tx, &interaction)?;
            let execution = load_execution(tx, &request.execution_id)?;
            if execution.generation != generation {
                return Err(StoreError::StaleRevision {
                    expected: generation,
                    actual: execution.generation,
                });
            }
            if !matches!(state.as_str(), "pending" | "responding") {
                return Err(StoreError::Conflict("interaction_consumed"));
            }
            request.state = "invalidated".into();
            request.revision += 1;
            save_interaction_cas(tx, &request, &state)?;
            let epoch = stream_epoch(tx, &execution.conversation_id)?;
            append_event(
                tx,
                ids,
                limits,
                &execution.conversation_id,
                &epoch,
                now,
                "interaction.updated",
                &interaction,
                request.revision,
                Ev::InteractionUpdated(Box::new(request.clone())),
            )?;
            Ok(request)
        })
        .await
    }

    /// 审批 CAS（契约 §7）：所有验证在同一事务；行数必须正好 1。
    pub async fn begin_response(
        &self,
        authorized: &AuthorizedScope,
        input: &AgentInteractionRespondInput,
        now: u64,
    ) -> Result<ResponsePermit, StoreError> {
        require_ulid(&input.interaction_id, "interactionId")?;
        let decision = input
            .decision
            .as_ref()
            .ok_or_else(|| StoreError::InvalidArgument("decision is required".into()))?;
        validate_decision_shape(decision)?;
        let authorized = authorized.clone();
        let input = input.clone();
        self.immediate(move |tx, ids, limits| {
            let (request, state, stored_digest, stored_responder, revision) =
                load_interaction(tx, &input.interaction_id)?;
            let decision_json = encode(&input.decision)?;
            let decision_digest = digest(&[&decision_json]);
            let responder = authorized.principal_id.clone();
            if state != "pending" {
                // 同一已成功决定的同主体重试返回 permit；finish_response 查回
                // resolved 投影。unknown 不是成功，禁止以任何形式再次回答。
                if state == "resolved"
                    && stored_digest.as_deref() == Some(decision_digest.as_str())
                    && stored_responder.as_deref() == Some(responder.as_str())
                {
                    return Ok(ResponsePermit::new(request.id, decision_digest, responder));
                }
                return Err(StoreError::Conflict("interaction_consumed"));
            }
            let expires = request.expires_at_ms.unwrap_or(u64::MAX);
            if expires <= now {
                return Err(StoreError::Conflict("interaction_expired"));
            }
            let execution = load_execution(tx, &request.execution_id)?;
            if TERMINAL_EXECUTION_STATES.contains(&execution.state.as_str()) {
                return Err(StoreError::Conflict("interaction_consumed"));
            }
            validate_decision_against_choices(
                &request,
                input.decision.as_ref().expect("decision shape validated"),
            )?;
            if revision != input.expected_revision {
                return Err(StoreError::StaleRevision {
                    expected: input.expected_revision,
                    actual: revision,
                });
            }
            let mut answered = request.clone();
            answered.state = "responding".into();
            answered.revision += 1;
            let projection = encode(&answered)?;
            // 关键 CAS：前置验证都在同一事务内，行数必须正好 1。
            let updated = tx.execute(
                "UPDATE interactions SET state='responding', revision=revision+1,
                 decision_digest=?1, responder_id=?2, projection=?3
                 WHERE id=?4 AND state='pending' AND revision=?5 AND expires_at_ms>?6",
                params![
                    decision_digest,
                    responder,
                    projection,
                    answered.id,
                    input.expected_revision as i64,
                    now as i64
                ],
            )?;
            if updated != 1 {
                let (_r, state, _d, _resp, actual) = load_interaction(tx, &answered.id)?;
                return Err(if state != "pending" {
                    StoreError::Conflict("interaction_consumed")
                } else {
                    StoreError::StaleRevision {
                        expected: input.expected_revision,
                        actual,
                    }
                });
            }
            let epoch = stream_epoch(tx, &execution.conversation_id)?;
            let answered_id = answered.id.clone();
            let answered_revision = answered.revision;
            append_event(
                tx,
                ids,
                limits,
                &execution.conversation_id,
                &epoch,
                now,
                "interaction.updated",
                &answered_id,
                answered_revision,
                Ev::InteractionUpdated(Box::new(answered)),
            )?;
            Ok(ResponsePermit::new(request.id, decision_digest, responder))
        })
        .await
    }

    /// 只接受本 permit 的响应：确认→resolved；不明→unknown；失效→invalidated。
    /// 同一已成功决定的重复 finish 查回当前 resolved 投影，不二次发送。
    pub async fn finish_response(
        &self,
        permit: ResponsePermit,
        outcome: BackendResponseOutcome,
        now: u64,
    ) -> Result<InteractionRequest, StoreError> {
        self.immediate(move |tx, ids, limits| {
            let (request, state, stored_digest, stored_responder, _revision) =
                load_interaction(tx, permit.interaction_id())?;
            if stored_digest.as_deref() != Some(permit.decision_digest())
                || stored_responder.as_deref() != Some(permit.responder_id())
            {
                return Err(StoreError::Conflict("interaction_consumed"));
            }
            if state == "resolved" {
                return Ok(request.clone());
            }
            if state != "responding" {
                return Err(StoreError::Conflict("interaction_consumed"));
            }
            let mut updated = request;
            updated.state = match outcome {
                BackendResponseOutcome::Confirmed => "resolved",
                BackendResponseOutcome::Unknown { .. } => "unknown",
                BackendResponseOutcome::Invalidated { .. } => "invalidated",
            }
            .into();
            updated.revision += 1;
            save_interaction_cas(tx, &updated, "responding")?;
            let execution = load_execution(tx, &updated.execution_id)?;
            let epoch = stream_epoch(tx, &execution.conversation_id)?;
            let updated_id = updated.id.clone();
            let updated_revision = updated.revision;
            append_event(
                tx,
                ids,
                limits,
                &execution.conversation_id,
                &epoch,
                now,
                "interaction.updated",
                &updated_id,
                updated_revision,
                Ev::InteractionUpdated(Box::new(updated.clone())),
            )?;
            Ok(updated)
        })
        .await
    }

    /// 同一事务版本读取快照（恢复契约 §3）；正文窗口按预算填充，可为空。
    pub async fn snapshot(
        &self,
        _authorized: &AuthorizedScope,
        conversation_id: &str,
    ) -> Result<AgentSnapshot, StoreError> {
        require_ulid(conversation_id, "conversationId")?;
        let conversation = conversation_id.to_string();
        self.readonly(move |tx, _gen, limits| {
            let conversation_row = load_conversation(tx, &conversation)?;
            let epoch = stream_epoch(tx, &conversation)?;
            let (head, first_available): (u64, u64) = tx.query_row(
                "SELECT head, first_available FROM streams WHERE id=?1",
                params![conversation],
                |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, i64>(1)? as u64)),
            )?;
            let mut executions = Vec::new();
            {
                let mut stmt = tx.prepare(
                    "SELECT projection FROM executions WHERE conversation_id=?1
                     AND state IN ('accepted','dispatching','running','unknown')",
                )?;
                for row in stmt.query_map(params![conversation], |row| row.get::<_, Vec<u8>>(0))? {
                    executions.push(decode::<Execution>(&row?)?);
                }
            }
            {
                let mut stmt = tx.prepare(
                    "SELECT projection FROM executions WHERE conversation_id=?1
                     AND state IN ('succeeded','failed','cancelled')
                     ORDER BY revision DESC, id DESC LIMIT ?2",
                )?;
                for row in stmt.query_map(
                    params![conversation, limits.snapshot_terminal_executions as i64],
                    |row| row.get::<_, Vec<u8>>(0),
                )? {
                    executions.push(decode::<Execution>(&row?)?);
                }
            }
            let mut interactions = Vec::new();
            {
                let mut stmt = tx.prepare(
                    "SELECT i.projection FROM interactions i
                     JOIN executions e ON e.id=i.execution_id
                     WHERE e.conversation_id=?1
                       AND i.state IN ('pending','responding','unknown')",
                )?;
                for row in stmt.query_map(params![conversation], |row| row.get::<_, Vec<u8>>(0))? {
                    interactions.push(decode::<InteractionRequest>(&row?)?);
                }
            }
            {
                let mut stmt = tx.prepare(
                    "SELECT i.projection FROM interactions i
                     JOIN executions e ON e.id=i.execution_id
                     WHERE e.conversation_id=?1 AND i.state='resolved'
                     ORDER BY i.revision DESC, i.id DESC LIMIT ?2",
                )?;
                for row in stmt.query_map(
                    params![conversation, limits.snapshot_resolved_interactions as i64],
                    |row| row.get::<_, Vec<u8>>(0),
                )? {
                    interactions.push(decode::<InteractionRequest>(&row?)?);
                }
            }
            let mut messages = Vec::new();
            let has_more_history;
            {
                let mut stmt = tx.prepare(
                    "SELECT projection FROM messages WHERE conversation_id=?1
                     ORDER BY ordinal DESC LIMIT ?2",
                )?;
                let mut included = 0usize;
                for row in stmt.query_map(
                    params![conversation, limits.snapshot_messages as i64],
                    |row| row.get::<_, Vec<u8>>(0),
                )? {
                    messages.push(decode::<AgentMessage>(&row?)?);
                    included += 1;
                }
                let total: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM messages WHERE conversation_id=?1",
                    params![conversation],
                    |row| row.get(0),
                )?;
                has_more_history = total as usize > included;
            }
            messages.reverse();
            let history = AgentHistoryInfo {
                has_more: has_more_history,
                before_message_id: messages.first().map(|m| m.id.clone()),
            };
            let full_messages = messages.clone();
            let mut snapshot = AgentSnapshot {
                conversation: Some(conversation_row.clone()),
                executions,
                interactions,
                messages,
                relationships: Vec::new(),
                cursor: Some(AgentEventCursor {
                    stream_id: conversation.clone(),
                    epoch: epoch.clone(),
                    after: head,
                }),
                first_available,
                history: Some(history),
                relationships_has_more: false,
                quality: conversation_row.quality.clone(),
                gaps: Vec::new(),
            };
            strip_windows(&mut snapshot);
            let active_ids: Vec<String> = snapshot
                .executions
                .iter()
                .filter(|e| ACTIVE_EXECUTION_STATES.contains(&e.state.as_str()))
                .map(|e| e.id.clone())
                .collect();
            let mut budget = limits
                .snapshot_budget_bytes
                .saturating_sub(encode(&snapshot)?.len());
            for message in snapshot.messages.iter_mut() {
                let is_active =
                    message.state == "streaming" || active_ids.contains(&message.execution_id);
                if !is_active {
                    continue;
                }
                let Some(original) = full_messages.iter().find(|m| m.id == message.id) else {
                    continue;
                };
                let mut candidate = message.clone();
                candidate.parts = original.parts.clone();
                let cost = encode(&candidate)?
                    .len()
                    .saturating_sub(encode(message)?.len());
                if cost > budget {
                    continue;
                }
                budget -= cost;
                *message = candidate;
            }
            {
                let mut stmt = tx.prepare(
                    "SELECT encoded FROM events WHERE stream_id=?1 AND position>=?2
                     AND position<=?3 ORDER BY position",
                )?;
                let rows = stmt.query_map(
                    params![conversation, first_available as i64, head as i64],
                    |row| row.get::<_, Vec<u8>>(0),
                )?;
                for row in rows {
                    let event: AgentEvent = decode(&row?)?;
                    if event.r#type == "recovery.gap"
                        && let Some(agent_event::Data::RecoveryGap(gap)) = event.data
                    {
                        snapshot.gaps.push(gap);
                    }
                }
            }
            let encoded = encode(&snapshot)?.len();
            if encoded > limits.max_event_page_bytes.saturating_sub(16384) {
                return Err(StoreError::QuotaExceeded("snapshot_budget"));
            }
            Ok(snapshot)
        })
        .await
    }

    /// 严格连续事件页（恢复契约 §3/§6）：epoch、min-1<=after<=head、页字节预算。
    pub async fn read_events(
        &self,
        _authorized: &AuthorizedScope,
        cursor: &AgentEventCursor,
        limit: Option<u32>,
    ) -> Result<AgentEventReadResult, StoreError> {
        require_ulid(&cursor.stream_id, "streamId")?;
        require_ulid(&cursor.epoch, "epoch")?;
        if cursor.after > i64::MAX as u64 {
            return Err(StoreError::InvalidArgument(
                "cursor.after out of range".into(),
            ));
        }
        let limit = limit.unwrap_or(32).clamp(1, 32) as usize;
        let cursor = cursor.clone();
        self.readonly(move |tx, _gen, limits| {
            let (epoch, head, first_available): (String, u64, u64) = tx
                .query_row(
                    "SELECT epoch, head, first_available FROM streams WHERE id=?1",
                    params![cursor.stream_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, i64>(1)? as u64,
                            row.get::<_, i64>(2)? as u64,
                        ))
                    },
                )
                .optional()?
                .ok_or_else(|| StoreError::NotFound {
                    entity: "stream",
                    id: cursor.stream_id.clone(),
                })?;
            if epoch != cursor.epoch || cursor.after < first_available.saturating_sub(1) {
                return Err(StoreError::ResumeUnavailable {
                    stream_id: cursor.stream_id.clone(),
                    epoch,
                    min: first_available,
                    head,
                });
            }
            if cursor.after > head {
                return Err(StoreError::InvalidArgument("cursor_ahead".into()));
            }
            let mut events = Vec::new();
            let mut bytes = 0usize;
            let mut oversized = false;
            {
                let mut stmt = tx.prepare(
                    "SELECT position, encoded FROM events WHERE stream_id=?1 AND position>?2
                     ORDER BY position LIMIT ?3",
                )?;
                let rows = stmt.query_map(
                    params![cursor.stream_id, cursor.after as i64, (limit + 1) as i64],
                    |row| Ok((row.get::<_, i64>(0)? as u64, row.get::<_, Vec<u8>>(1)?)),
                )?;
                for row in rows {
                    let (position, encoded) = row?;
                    if events.len() >= limit {
                        break;
                    }
                    if bytes + encoded.len() > limits.max_event_page_bytes {
                        oversized = true;
                        break;
                    }
                    bytes += encoded.len();
                    events.push(decode::<AgentEvent>(&encoded)?);
                    let _ = position;
                }
            }
            if events.is_empty() && oversized {
                return Err(StoreError::PayloadTooLarge {
                    limit: limits.max_event_page_bytes,
                });
            }
            let next_after = events.last().map(|e| e.position).unwrap_or(cursor.after);
            Ok(AgentEventReadResult {
                events,
                next_cursor: Some(AgentEventCursor {
                    stream_id: cursor.stream_id.clone(),
                    epoch: cursor.epoch.clone(),
                    after: next_after,
                }),
                head,
                first_available,
                has_more: next_after < head,
            })
        })
        .await
    }

    /// UTF-8 边界分页读正文；旧字节不可重写（契约 §4）。
    pub async fn read_message(
        &self,
        _authorized: &AuthorizedScope,
        conversation_id: &str,
        message_id: &str,
        part_index: u32,
        offset_bytes: u64,
        max_bytes: u32,
    ) -> Result<AgentMessageReadResult, StoreError> {
        require_ulid(conversation_id, "conversationId")?;
        require_ulid(message_id, "messageId")?;
        if max_bytes == 0 || max_bytes > 65536 {
            return Err(StoreError::InvalidArgument(
                "maxBytes must be within 1..=65536".into(),
            ));
        }
        let conversation = conversation_id.to_string();
        let message = message_id.to_string();
        self.readonly(move |tx, _gen, _limits| {
            let stored = load_message(tx, &message)?;
            if stored.conversation_id != conversation {
                return Err(StoreError::NotFound {
                    entity: "message",
                    id: message.clone(),
                });
            }
            let Some(Some(agent_message_part::Part::Text(part))) =
                stored.parts.get(part_index as usize).map(|p| &p.part)
            else {
                return Err(StoreError::InvalidArgument("part is not text".into()));
            };
            let text = part.text.as_bytes();
            if offset_bytes as usize > text.len() {
                return Err(StoreError::InvalidArgument(
                    "offset beyond totalBytes".into(),
                ));
            }
            if offset_bytes > 0 && !part.text.is_char_boundary(offset_bytes as usize) {
                return Err(StoreError::InvalidArgument(
                    "offset must be a UTF-8 boundary".into(),
                ));
            }
            let start = offset_bytes as usize;
            let mut end = (start + max_bytes as usize).min(text.len());
            while end > start && !part.text.is_char_boundary(end) {
                end -= 1;
            }
            Ok(AgentMessageReadResult {
                text: part.text[start..end].to_string(),
                offset_bytes: start as u64,
                next_offset_bytes: end as u64,
                total_bytes: part.total_bytes,
                state: stored.state.clone(),
                revision: stored.revision,
            })
        })
        .await
    }

    /// 登记后端会话映射（契约 §2：`(adapter,backendInstanceKey,backendConversationId)`
    /// 唯一）。仅在首次受控创建/恢复成功后调用；重复登记同键同值幂等。
    pub async fn record_backend_mapping(
        &self,
        adapter: &str,
        backend_instance_key: &str,
        backend_conversation_id: &str,
        conversation_id: &str,
    ) -> Result<(), StoreError> {
        if adapter.is_empty()
            || backend_instance_key.is_empty()
            || backend_conversation_id.is_empty()
        {
            return Err(StoreError::InvalidArgument(
                "backend mapping fields must not be empty".into(),
            ));
        }
        require_ulid(conversation_id, "conversationId")?;
        let (adapter, key, backend_id, conversation) = (
            adapter.to_string(),
            backend_instance_key.to_string(),
            backend_conversation_id.to_string(),
            conversation_id.to_string(),
        );
        self.immediate(move |tx, _ids, _limits| {
            let exists: bool = tx.query_row(
                "SELECT COUNT(*) FROM conversations WHERE id=?1",
                params![conversation],
                |row| row.get::<_, i64>(0),
            )? > 0;
            if !exists {
                return Err(StoreError::NotFound {
                    entity: "conversation",
                    id: conversation,
                });
            }
            tx.execute(
                "INSERT INTO backend_mappings(adapter,backend_instance_key,backend_conversation_id,conversation_id)
                 VALUES (?1,?2,?3,?4)
                 ON CONFLICT(adapter,backend_instance_key,backend_conversation_id)
                 DO UPDATE SET conversation_id=excluded.conversation_id",
                params![adapter, key, backend_id, conversation],
            )?;
            Ok(())
        })
        .await
    }

    /// 按后端身份找回节点会话；无映射返回 None，不猜测。
    pub async fn find_conversation_by_backend(
        &self,
        adapter: &str,
        backend_instance_key: &str,
        backend_conversation_id: &str,
    ) -> Result<Option<String>, StoreError> {
        let (adapter, key, backend_id) = (
            adapter.to_string(),
            backend_instance_key.to_string(),
            backend_conversation_id.to_string(),
        );
        self.readonly(move |tx, _gen, _limits| {
            tx.query_row(
                "SELECT conversation_id FROM backend_mappings
                 WHERE adapter=?1 AND backend_instance_key=?2 AND backend_conversation_id=?3",
                params![adapter, key, backend_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(StoreError::from)
        })
        .await
    }

    /// 端点目录读取（entities kind='endpoint'）；投影按 id 稳定排序。
    pub async fn list_endpoints(
        &self,
    ) -> Result<Vec<conex_proto::AgentAdapterEndpoint>, StoreError> {
        self.readonly(move |tx, _gen, _limits| {
            let mut stmt = tx
                .prepare("SELECT id, projection FROM entities WHERE kind='endpoint' ORDER BY id")?;
            let mut out = Vec::new();
            let rows = stmt.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            for row in rows {
                let (_id, projection) = row?;
                out.push(decode(&projection)?);
            }
            Ok(out)
        })
        .await
    }

    /// 会话列表（catalogScope=node-managed：只列本节点登记会话）。
    /// 按 (createdAtMs,id) 稳定排序；分页由调用方按契约 §4 切页。
    pub async fn list_conversations(
        &self,
        endpoint_id: &str,
        workspace_id: Option<&str>,
    ) -> Result<Vec<Conversation>, StoreError> {
        require_ulid(endpoint_id, "endpointId")?;
        if let Some(workspace) = workspace_id {
            require_ulid(workspace, "workspaceId")?;
        }
        let endpoint = endpoint_id.to_string();
        let workspace = workspace_id.map(str::to_string);
        self.readonly(move |tx, _gen, _limits| {
            let mut stmt = tx
                .prepare("SELECT projection FROM conversations WHERE endpoint_id=?1 ORDER BY id")?;
            let mut out = Vec::new();
            let rows = stmt.query_map(params![endpoint], |row| row.get::<_, Vec<u8>>(0))?;
            for row in rows {
                let conversation: Conversation = decode(&row?)?;
                if let Some(workspace) = workspace.as_deref()
                    && conversation.workspace_id != workspace
                {
                    continue;
                }
                out.push(conversation);
            }
            out.sort_by(|a, b| (a.created_at_ms, &a.id).cmp(&(b.created_at_ms, &b.id)));
            Ok(out)
        })
        .await
    }

    /// 单条会话投影；未授权目标在边界已被拒绝，这里只处理存在性。
    pub async fn read_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Conversation, StoreError> {
        require_ulid(conversation_id, "conversationId")?;
        let conversation = conversation_id.to_string();
        self.readonly(move |tx, _gen, _limits| load_conversation(tx, &conversation))
            .await
    }

    /// 单条执行投影（含终态；snapshot 只投影活动执行）。
    pub async fn read_execution(&self, execution_id: &str) -> Result<Execution, StoreError> {
        require_ulid(execution_id, "executionId")?;
        let execution = execution_id.to_string();
        self.readonly(move |tx, _gen, _limits| load_execution(tx, &execution))
            .await
    }

    /// 派生子执行准入（契约 §5：parentExecutionId 不抢顶层槽）。同一会话内
    /// 父执行仍活跃时才准入；state 直接 running（后端生命周期帧即为证据）。
    /// 幂等性由调用方（会话 actor 的 backend_ref 映射）保证，不重复登记。
    pub async fn admit_backend_subexecution(
        &self,
        conversation_id: &str,
        parent_execution_id: &str,
        backend_ref: &str,
        now: u64,
    ) -> Result<Execution, StoreError> {
        require_ulid(conversation_id, "conversationId")?;
        require_ulid(parent_execution_id, "parentExecutionId")?;
        if backend_ref.is_empty() {
            return Err(StoreError::InvalidArgument(
                "backend_ref must not be empty".into(),
            ));
        }
        let conversation = conversation_id.to_string();
        let parent = parent_execution_id.to_string();
        let reference = backend_ref.to_string();
        self.immediate(move |tx, ids, limits| {
            let parent_row = load_execution(tx, &parent)?;
            if parent_row.conversation_id != conversation {
                return Err(StoreError::InvalidArgument(
                    "parent execution belongs to another conversation".into(),
                ));
            }
            if TERMINAL_EXECUTION_STATES.contains(&parent_row.state.as_str()) {
                return Err(StoreError::Conflict("parent_execution_terminal"));
            }
            let execution_id = ids.mint();
            let execution = Execution {
                id: execution_id.clone(),
                conversation_id: conversation.clone(),
                endpoint_id: parent_row.endpoint_id.clone(),
                module_instance_id: parent_row.module_instance_id.clone(),
                generation: parent_row.generation,
                state: "running".into(),
                revision: 1,
                created_at_ms: now,
                updated_at_ms: now,
                parent_execution_id: Some(parent.clone()),
                terminal_reason: None,
                cancel_requested_at_ms: None,
                backend_ref: Some(reference),
            };
            let projection = encode_bounded(&execution, limits.max_execution_meta_bytes)?;
            tx.execute(
                "INSERT INTO executions(id,conversation_id,parent_execution_id,generation,
                 state,revision,projection) VALUES (?1,?2,?3,?4,'running',1,?5)",
                params![
                    execution_id,
                    conversation,
                    parent,
                    execution.generation as i64,
                    projection
                ],
            )?;
            let epoch = stream_epoch(tx, &execution.conversation_id)?;
            append_event(
                tx,
                ids,
                limits,
                &execution.conversation_id,
                &epoch,
                now,
                "execution.updated",
                &execution.id,
                execution.revision,
                Ev::ExecutionUpdated(Box::new(execution.clone())),
            )?;
            Ok(execution)
        })
        .await
    }

    /// 子代理列表（agent/subagents/list 数据源）：父执行派生的执行记录，
    /// 来自后端真实生命周期帧。quality 恒为 current——记录即后端报告；
    /// 空列表表示后端未报告任何子代理，不推断为能力缺失。
    pub async fn list_subagents(
        &self,
        parent_execution_id: &str,
        limit: u32,
    ) -> Result<(Vec<Execution>, String), StoreError> {
        require_ulid(parent_execution_id, "executionId")?;
        if limit == 0 || limit > 100 {
            return Err(StoreError::InvalidArgument("limit out of range".into()));
        }
        let parent = parent_execution_id.to_string();
        self.readonly(move |tx, _gen, _limits| {
            let mut stmt = tx.prepare(
                "SELECT projection FROM executions WHERE parent_execution_id=?1
                 ORDER BY rowid LIMIT ?2",
            )?;
            let mut out = Vec::new();
            let rows = stmt.query_map(params![parent, limit as i64], |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
            for row in rows {
                out.push(decode(&row?)?);
            }
            Ok((out, "current".to_string()))
        })
        .await
    }

    /// 历史读取（契约 §8）：按节点稳定 ordinal 向前翻已结束消息；
    /// beforeMessageId 必须属于同会话。返回按 ordinal 升序、hasMore 指示更早页。
    pub async fn read_history(
        &self,
        conversation_id: &str,
        before_message_id: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<AgentMessage>, bool), StoreError> {
        require_ulid(conversation_id, "conversationId")?;
        if limit == 0 || limit > 100 {
            return Err(StoreError::InvalidArgument("limit out of range".into()));
        }
        let conversation = conversation_id.to_string();
        let before = before_message_id.map(str::to_string);
        self.readonly(move |tx, _gen, _limits| {
            let before_ordinal = match before.as_deref() {
                None => i64::MAX,
                Some(before) => {
                    let stored = load_message(tx, before)?;
                    if stored.conversation_id != conversation {
                        return Err(StoreError::InvalidArgument(
                            "beforeMessageId belongs to another conversation".into(),
                        ));
                    }
                    let ordinal: i64 = tx.query_row(
                        "SELECT ordinal FROM messages WHERE id=?1",
                        params![before],
                        |row| row.get(0),
                    )?;
                    ordinal
                }
            };
            let mut rows: Vec<AgentMessage> = Vec::new();
            let mut stmt = tx.prepare(
                "SELECT projection FROM messages WHERE conversation_id=?1 AND ordinal<?2
                 AND state!='streaming' ORDER BY ordinal DESC LIMIT ?3",
            )?;
            let projections = stmt.query_map(
                params![conversation, before_ordinal, limit as i64 + 1],
                |row| row.get::<_, Vec<u8>>(0),
            )?;
            for row in projections {
                let message: AgentMessage = decode(&row?)?;
                rows.push(message);
            }
            let mut has_more = false;
            if rows.len() > limit as usize {
                has_more = true;
                rows.truncate(limit as usize);
            }
            rows.reverse();
            Ok((rows, has_more))
        })
        .await
    }

    /// 交互列表；executionId 过滤可选，按 (createdAtMs,id) 稳定排序。
    pub async fn list_interactions(
        &self,
        conversation_id: &str,
        execution_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<InteractionRequest>, StoreError> {
        require_ulid(conversation_id, "conversationId")?;
        if let Some(execution) = execution_id {
            require_ulid(execution, "executionId")?;
        }
        if limit == 0 || limit > 100 {
            return Err(StoreError::InvalidArgument("limit out of range".into()));
        }
        let conversation = conversation_id.to_string();
        let execution = execution_id.map(str::to_string);
        self.readonly(move |tx, _gen, _limits| {
            let mut stmt = tx.prepare(
                "SELECT i.projection FROM interactions i
                 JOIN executions e ON e.id=i.execution_id
                 WHERE e.conversation_id=?1 ORDER BY i.rowid DESC LIMIT ?2",
            )?;
            let mut out = Vec::new();
            let rows = stmt.query_map(params![conversation, limit as i64], |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
            for row in rows {
                let request: InteractionRequest = decode(&row?)?;
                if let Some(execution) = execution.as_deref()
                    && request.execution_id != execution
                {
                    continue;
                }
                out.push(request);
            }
            out.sort_by(|a, b| (a.created_at_ms, &a.id).cmp(&(b.created_at_ms, &b.id)));
            Ok(out)
        })
        .await
    }

    /// 登记/更新后端关系（契约 §2 Relationship）。同 id 重写用于生命周期推进
    /// （quality 变化）；from/to 是实体引用，不承担授权语义。
    pub async fn record_relationship(
        &self,
        conversation_id: &str,
        relationship: conex_proto::Relationship,
    ) -> Result<(), StoreError> {
        require_ulid(conversation_id, "conversationId")?;
        if relationship.id.is_empty()
            || relationship.from.is_empty()
            || relationship.to.is_empty()
            || relationship.kind.is_empty()
        {
            return Err(StoreError::InvalidArgument(
                "relationship fields must not be empty".into(),
            ));
        }
        match relationship.source.as_str() {
            "backend" | "config" | "user" => {}
            other => {
                return Err(StoreError::InvalidArgument(format!(
                    "unknown relationship source {other}"
                )));
            }
        }
        match relationship.quality.as_str() {
            "current" | "stale" | "historical" | "unknown" => {}
            other => {
                return Err(StoreError::InvalidArgument(format!(
                    "unknown relationship quality {other}"
                )));
            }
        }
        let conversation = conversation_id.to_string();
        self.immediate(move |tx, _ids, _limits| {
            let exists: bool = tx.query_row(
                "SELECT COUNT(*) FROM conversations WHERE id=?1",
                params![conversation],
                |row| row.get::<_, i64>(0),
            )? > 0;
            if !exists {
                return Err(StoreError::NotFound {
                    entity: "conversation",
                    id: conversation.clone(),
                });
            }
            let projection = encode(&relationship)?;
            tx.execute(
                "INSERT INTO relationships(id,conversation_id,from_ref,to_ref,kind,source,
                 quality,observed_at_ms,projection) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
                 ON CONFLICT(id) DO UPDATE SET quality=excluded.quality,
                 observed_at_ms=excluded.observed_at_ms, projection=excluded.projection",
                params![
                    relationship.id,
                    conversation,
                    relationship.from,
                    relationship.to,
                    relationship.kind,
                    relationship.source,
                    relationship.quality,
                    relationship.observed_at_ms as i64,
                    projection
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// 登记/覆盖实体标注（P6.3：Tags 是标注不是权限）。同 owner+entity 内
    /// name 幂等覆盖；entity 必须已存在（endpoint/conversation/workspace）。
    pub async fn record_tag(
        &self,
        owner: &str,
        entity_kind: &str,
        entity_id: &str,
        name: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        match owner {
            "node" | "user" => {}
            other => {
                return Err(StoreError::InvalidArgument(format!(
                    "unknown tag owner {other}"
                )));
            }
        }
        match entity_kind {
            "endpoint" | "conversation" | "workspace" => {}
            other => {
                return Err(StoreError::InvalidArgument(format!(
                    "unknown tag entity kind {other}"
                )));
            }
        }
        if name.trim().is_empty() || name.len() > 128 {
            return Err(StoreError::InvalidArgument(
                "tag name must be 1..=128 bytes".into(),
            ));
        }
        require_ulid(entity_id, "entityId")?;
        let (owner, entity_kind, entity, name) = (
            owner.to_string(),
            entity_kind.to_string(),
            entity_id.to_string(),
            name.to_string(),
        );
        self.immediate(move |tx, _ids, _limits| {
            let exists: bool = tx.query_row(
                "SELECT COUNT(*) FROM entities WHERE id=?1 AND kind=?2",
                params![entity, entity_kind],
                |row| row.get::<_, i64>(0),
            )? > 0;
            if !exists {
                return Err(StoreError::NotFound {
                    entity: "entity",
                    id: entity,
                });
            }
            tx.execute(
                "INSERT INTO entity_tags(name,owner,entity_kind,entity_id,created_at_ms)
                 VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(owner,entity_kind,entity_id,name)
                 DO UPDATE SET created_at_ms=excluded.created_at_ms",
                params![name, owner, entity_kind, entity, now as i64],
            )?;
            Ok(())
        })
        .await
    }

    /// 列出实体标注；按 (owner,name) 稳定排序，上限受限。
    pub async fn list_tags(
        &self,
        entity_kind: &str,
        entity_id: &str,
    ) -> Result<Vec<conex_proto::EntityTag>, StoreError> {
        require_ulid(entity_id, "entityId")?;
        let entity = entity_id.to_string();
        let kind = entity_kind.to_string();
        self.readonly(move |tx, _gen, _limits| {
            let mut stmt = tx.prepare(
                "SELECT name, owner, entity_kind, entity_id, created_at_ms
                 FROM entity_tags WHERE entity_kind=?1 AND entity_id=?2
                 ORDER BY owner, name",
            )?;
            let mut out = Vec::new();
            let rows = stmt.query_map(params![kind, entity], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            for row in rows {
                let (name, owner, entity_kind, entity_id, created_at_ms) = row?;
                out.push(conex_proto::EntityTag {
                    name,
                    owner,
                    entity_kind,
                    entity_id,
                    created_at_ms: created_at_ms as u64,
                });
            }
            Ok(out)
        })
        .await
    }

    /// 删除标注；只删标注行，不影响任何实体或执行（P6.3）。
    pub async fn delete_tag(
        &self,
        owner: &str,
        entity_kind: &str,
        entity_id: &str,
        name: &str,
    ) -> Result<(), StoreError> {
        match owner {
            "node" | "user" => {}
            other => {
                return Err(StoreError::InvalidArgument(format!(
                    "unknown tag owner {other}"
                )));
            }
        }
        let (owner, entity_kind, entity, name) = (
            owner.to_string(),
            entity_kind.to_string(),
            entity_id.to_string(),
            name.to_string(),
        );
        self.immediate(move |tx, _ids, _limits| {
            tx.execute(
                "DELETE FROM entity_tags WHERE owner=?1 AND entity_kind=?2 AND entity_id=?3 AND name=?4",
                params![owner, entity_kind, entity, name],
            )?;
            Ok(())
        })
        .await
    }

    /// 关系列表（agent/relationships/list 的数据源）；按观察时间倒序、上限受限。
    pub async fn list_relationships(
        &self,
        conversation_id: &str,
        limit: u32,
    ) -> Result<Vec<conex_proto::Relationship>, StoreError> {
        require_ulid(conversation_id, "conversationId")?;
        if limit == 0 || limit > 100 {
            return Err(StoreError::InvalidArgument("limit out of range".into()));
        }
        let conversation = conversation_id.to_string();
        self.readonly(move |tx, _gen, _limits| {
            let mut stmt = tx.prepare(
                "SELECT projection FROM relationships WHERE conversation_id=?1
                 ORDER BY observed_at_ms DESC, id LIMIT ?2",
            )?;
            let mut out = Vec::new();
            let rows = stmt.query_map(params![conversation, limit as i64], |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
            for row in rows {
                out.push(decode(&row?)?);
            }
            Ok(out)
        })
        .await
    }

    /// 保留清理：24h/64MiB 每流、512MiB 节点总量；只删连续前缀并更新
    /// firstAvailable/字节数（恢复契约 §5）。
    pub async fn prune(&self, now: u64) -> Result<PruneResult, StoreError> {
        self.immediate(move |tx, _gen, limits| {
            let mut result = PruneResult::default();
            let cutoff = now.saturating_sub(limits.retention_ms);
            let streams: Vec<String> = {
                let mut stmt = tx.prepare("SELECT id FROM streams WHERE head>0 ORDER BY id")?;
                let rows = stmt.query_map([], |row| row.get(0))?;
                rows.collect::<Result<Vec<_>, _>>()?
            };
            for stream in streams {
                let (events, bytes) = trim_stream_prefix(tx, &stream, cutoff)?;
                result.events_deleted += events;
                result.bytes_deleted += bytes;
                let (_, stream_bytes): (i64, i64) = tx.query_row(
                    "SELECT head, event_bytes FROM streams WHERE id=?1",
                    params![stream],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                if stream_bytes as u64 > limits.max_stream_event_bytes {
                    let excess = stream_bytes as u64 - limits.max_stream_event_bytes;
                    let (events, bytes) = trim_stream_bytes(tx, &stream, excess)?;
                    result.events_deleted += events;
                    result.bytes_deleted += bytes;
                }
            }
            let total: i64 = tx.query_row(
                "SELECT COALESCE(SUM(event_bytes),0) FROM streams",
                [],
                |row| row.get(0),
            )?;
            if total as u64 > limits.max_total_event_bytes {
                let mut excess = total as u64 - limits.max_total_event_bytes;
                let oldest: Vec<String> = {
                    let mut stmt = tx.prepare(
                        "SELECT s.id FROM streams s
                         LEFT JOIN events e ON e.stream_id=s.id
                         GROUP BY s.id
                         ORDER BY MIN(e.recorded_at_ms) NULLS LAST, s.id",
                    )?;
                    let rows = stmt.query_map([], |row| row.get(0))?;
                    rows.collect::<Result<Vec<_>, _>>()?
                };
                for stream in oldest {
                    if excess == 0 {
                        break;
                    }
                    let (_, stream_bytes): (i64, i64) = tx.query_row(
                        "SELECT head, event_bytes FROM streams WHERE id=?1",
                        params![stream],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )?;
                    let (events, bytes) =
                        trim_stream_bytes(tx, &stream, excess.min(stream_bytes as u64))?;
                    result.events_deleted += events;
                    result.bytes_deleted += bytes;
                    excess = excess.saturating_sub(bytes);
                }
            }
            Ok(result)
        })
        .await
    }
}

// ---- 私有辅助 --------------------------------------------------------------

fn target_of(execution: &Execution) -> ReconcileTarget {
    ReconcileTarget {
        execution_id: execution.id.clone(),
        conversation_id: execution.conversation_id.clone(),
        generation: execution.generation,
        state: execution.state.clone(),
        backend_ref: execution.backend_ref.clone(),
    }
}

fn strip_windows(snapshot: &mut AgentSnapshot) {
    for message in &mut snapshot.messages {
        for part in message.parts.iter_mut() {
            if let Some(agent_message_part::Part::Text(text)) = part.part.as_mut() {
                text.text = String::new();
            }
        }
    }
}

fn require_ulid(value: &str, field: &str) -> Result<(), StoreError> {
    if conex_proto::wire::is_ulid(value) {
        Ok(())
    } else {
        Err(StoreError::InvalidArgument(format!(
            "{field} must be a ULID"
        )))
    }
}

fn require_key(key: &str) -> Result<(), StoreError> {
    if key.is_empty() {
        Err(StoreError::InvalidArgument(
            "idempotency key must not be empty".into(),
        ))
    } else if key.len() > 128 {
        Err(StoreError::InvalidArgument(
            "idempotency key exceeds 128 bytes".into(),
        ))
    } else {
        Ok(())
    }
}

fn require_module_key(key: &str) -> Result<(), StoreError> {
    if key.is_empty() {
        Err(StoreError::InvalidArgument(
            "module_key must not be empty".into(),
        ))
    } else if key.len() > 128 {
        Err(StoreError::InvalidArgument(
            "module_key exceeds 128 bytes".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_decision_shape(
    decision: &conex_proto::AgentInteractionDecision,
) -> Result<(), StoreError> {
    match decision.kind.as_str() {
        "approval" => {
            let ok = decision.choice_id.is_some()
                && decision.choice_ids.is_empty()
                && decision.text.is_none();
            ok.then_some(())
                .ok_or(StoreError::Conflict("decision_shape"))
        }
        "selection" => {
            let ok = decision.choice_id.is_none()
                && !decision.choice_ids.is_empty()
                && decision.text.is_none();
            ok.then_some(())
                .ok_or(StoreError::Conflict("decision_shape"))
        }
        "input" => {
            let ok = decision.choice_id.is_none()
                && decision.choice_ids.is_empty()
                && decision.text.as_deref().is_some_and(|t| !t.is_empty());
            ok.then_some(())
                .ok_or(StoreError::Conflict("decision_shape"))
        }
        _ => Err(StoreError::InvalidArgument(
            "decision.kind must be approval/selection/input".into(),
        )),
    }
}

fn validate_decision_against_choices(
    request: &InteractionRequest,
    decision: &conex_proto::AgentInteractionDecision,
) -> Result<(), StoreError> {
    let known: Vec<&str> = request.choices.iter().map(|c| c.id.as_str()).collect();
    let referenced: Vec<&str> = match decision.kind.as_str() {
        "approval" => vec![decision.choice_id.as_deref().unwrap_or("")],
        "selection" => decision.choice_ids.iter().map(String::as_str).collect(),
        _ => vec![],
    };
    for id in referenced {
        if !known.contains(&id) {
            return Err(StoreError::InvalidArgument(format!(
                "decision choice {id} is not part of this request"
            )));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn append_text(
    tx: &rusqlite::Transaction<'_>,
    ids: &IdGen,
    limits: &StoreLimits,
    now: u64,
    execution: &Execution,
    epoch: &str,
    message_id: &str,
    text: &str,
) -> Result<ApplyResult, StoreError> {
    let mut message = load_message(tx, message_id)?;
    if message.conversation_id != execution.conversation_id || message.execution_id != execution.id
    {
        return Err(StoreError::NotFound {
            entity: "message",
            id: message_id.to_string(),
        });
    }
    if message.state != "streaming" {
        return Err(StoreError::Conflict("message_not_active"));
    }
    if message.parts.is_empty() {
        message.parts.push(conex_proto::AgentMessagePart {
            part: Some(agent_message_part::Part::Text(conex_proto::AgentTextPart {
                text: String::new(),
                offset_bytes: 0,
                total_bytes: 0,
            })),
        });
    }
    let (mut offset, chunk_count);
    {
        let Some(Some(agent_message_part::Part::Text(part))) =
            message.parts.first_mut().map(|p| &mut p.part)
        else {
            return Err(StoreError::Conflict("message_part_not_text"));
        };
        offset = part.total_bytes;
        let chunks = split_utf8(text, limits.max_delta_bytes);
        chunk_count = chunks.len();
        for chunk in &chunks {
            append_event(
                tx,
                ids,
                limits,
                &execution.conversation_id,
                epoch,
                now,
                "message.delta",
                message_id,
                message.revision,
                Ev::Delta {
                    message_id: message_id.to_string(),
                    part_index: 0,
                    offset_bytes: offset,
                    text: (*chunk).to_string(),
                },
            )?;
            offset += chunk.len() as u64;
        }
        part.text.push_str(text);
        part.total_bytes = offset;
    }
    message.revision += 1;
    let meta = {
        let mut window = message.clone();
        if let Some(mp) = window.parts.first_mut()
            && let Some(agent_message_part::Part::Text(p)) = mp.part.as_mut()
        {
            p.text = String::new();
        }
        encode(&window)?
    };
    if meta.len() > limits.max_message_meta_bytes || message.parts.len() > limits.max_parts {
        return bounded_failure(tx, ids, limits, now, execution, "message_metadata");
    }
    save_message(tx, &message)?;
    Ok(ApplyResult {
        applied: true,
        events_appended: chunk_count as u64,
        ..ApplyResult::default()
    })
}

#[allow(clippy::too_many_arguments)]
fn open_interaction(
    tx: &rusqlite::Transaction<'_>,
    ids: &IdGen,
    limits: &StoreLimits,
    now: u64,
    execution: &Execution,
    epoch: &str,
    kind: String,
    prompt: Option<String>,
    choices: Vec<(String, String, String)>,
    expires_at_ms: Option<u64>,
) -> Result<ApplyResult, StoreError> {
    if !INTERACTION_KINDS.contains(&kind.as_str()) {
        return Err(StoreError::InvalidArgument(format!("unknown kind {kind}")));
    }
    let undecided: i64 = tx.query_row(
        "SELECT COUNT(*) FROM interactions i JOIN executions e ON e.id=i.execution_id
         WHERE e.conversation_id=?1 AND i.state IN ('pending','responding','unknown')",
        params![execution.conversation_id],
        |row| row.get(0),
    )?;
    if undecided as usize >= limits.max_undecided_interactions {
        return bounded_failure(tx, ids, limits, now, execution, "interaction_quota");
    }
    let mut mapped = Vec::new();
    for (id, label, effect) in &choices {
        if id.is_empty() || !EFFECTS.contains(&effect.as_str()) {
            return Err(StoreError::InvalidArgument(
                "interaction choice id/effect invalid".into(),
            ));
        }
        if label.len() > limits.max_tool_summary_bytes {
            return bounded_failure(tx, ids, limits, now, execution, "interaction_metadata");
        }
        mapped.push(InteractionChoice {
            id: id.clone(),
            label: label.clone(),
            effect: effect.clone(),
        });
    }
    let interaction_id = ids.mint();
    let interaction = InteractionRequest {
        id: interaction_id.clone(),
        execution_id: execution.id.clone(),
        kind,
        state: "pending".into(),
        revision: 1,
        created_at_ms: now,
        expires_at_ms: Some(expires_at_ms.unwrap_or(now + limits.default_interaction_ttl_ms)),
        prompt,
        choices: mapped,
    };
    let projection = encode_bounded(&interaction, limits.max_interaction_bytes)
        .map_err(|_| StoreError::QuotaExceeded("interaction_metadata"))?;
    tx.execute(
        "INSERT INTO interactions(id,execution_id,state,revision,expires_at_ms,
         decision_digest,responder_id,projection)
         VALUES (?1,?2,'pending',1,?3,NULL,NULL,?4)",
        params![
            interaction_id,
            interaction.execution_id,
            interaction.expires_at_ms.unwrap_or(now) as i64,
            projection
        ],
    )?;
    append_event(
        tx,
        ids,
        limits,
        &execution.conversation_id,
        epoch,
        now,
        "interaction.created",
        &interaction_id,
        1,
        Ev::InteractionCreated(Box::new(interaction)),
    )?;
    Ok(ApplyResult {
        interaction_id: Some(interaction_id),
        applied: true,
        ..ApplyResult::default()
    })
}

/// 清槽必须同时更新列与投影：投影是读取来源（load_conversation 解码投影），
/// 只更新列会让会话永远显示 busy。旧回调不得清掉别人的槽。
fn clear_slot(
    tx: &rusqlite::Transaction<'_>,
    conversation_id: &str,
    execution_id: &str,
) -> Result<(), StoreError> {
    let updated = tx.execute(
        "UPDATE conversations SET active_execution_id=NULL, revision=revision+1
         WHERE id=?1 AND active_execution_id=?2",
        params![conversation_id, execution_id],
    )?;
    if updated == 1
        && let Some(mut conversation) = load_conversation(tx, conversation_id).ok()
    {
        conversation.active_execution_id = None;
        conversation.revision += 1;
        save_conversation(tx, &conversation)?;
    }
    Ok(())
}

/// 终态单次 CAS + 清槽 + 失效剩余交互（契约 §5；Quiescent 触发）。
fn apply_terminal(
    tx: &rusqlite::Transaction<'_>,
    ids: &IdGen,
    limits: &StoreLimits,
    now: u64,
    execution: &Execution,
    epoch: &str,
    state: TerminalState,
) -> Result<ApplyResult, StoreError> {
    let mut row = execution.clone();
    let updated = tx.execute(
        "UPDATE executions SET state=?2, revision=revision+1 WHERE id=?1
         AND state IN ('accepted','dispatching','running','unknown')",
        params![row.id, state.as_str()],
    )?;
    if updated != 1 {
        return Ok(ApplyResult {
            applied: false,
            ..ApplyResult::default()
        });
    }
    row.state = state.as_str().into();
    row.terminal_reason = state.reason().map(str::to_string);
    row.revision += 1;
    row.updated_at_ms = now;
    save_execution(tx, &row)?;
    clear_slot(tx, &row.conversation_id, &row.id)?;
    append_event(
        tx,
        ids,
        limits,
        &row.conversation_id,
        epoch,
        now,
        "execution.updated",
        &row.id,
        row.revision,
        Ev::ExecutionUpdated(Box::new(row.clone())),
    )?;
    invalidate_pending_interactions(tx, ids, limits, now, &row, epoch, "invalidated")?;
    Ok(ApplyResult {
        execution_state: Some(row.state),
        applied: true,
        ..ApplyResult::default()
    })
}

fn invalidate_pending_interactions(
    tx: &rusqlite::Transaction<'_>,
    ids: &IdGen,
    limits: &StoreLimits,
    now: u64,
    execution: &Execution,
    epoch: &str,
    reason_state: &str,
) -> Result<(), StoreError> {
    let pending: Vec<String> = {
        let mut stmt =
            tx.prepare("SELECT id FROM interactions WHERE execution_id=?1 AND state='pending'")?;
        let rows = stmt.query_map(params![execution.id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for id in pending {
        let (mut request, state, _digest, _responder, _revision) = load_interaction(tx, &id)?;
        if state != "pending" {
            continue;
        }
        request.state = reason_state.into();
        request.revision += 1;
        save_interaction_cas(tx, &request, "pending")?;
        append_event(
            tx,
            ids,
            limits,
            &execution.conversation_id,
            epoch,
            now,
            "interaction.updated",
            &id,
            request.revision,
            Ev::InteractionUpdated(Box::new(request)),
        )?;
    }
    Ok(())
}

fn demote_responding_interactions(
    tx: &rusqlite::Transaction<'_>,
    ids: &IdGen,
    limits: &StoreLimits,
    now: u64,
    execution: &Execution,
    epoch: &str,
) -> Result<(), StoreError> {
    let responding: Vec<String> = {
        let mut stmt =
            tx.prepare("SELECT id FROM interactions WHERE execution_id=?1 AND state='responding'")?;
        let rows = stmt.query_map(params![execution.id], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for id in responding {
        let (mut request, state, _digest, _responder, _revision) = load_interaction(tx, &id)?;
        if state != "responding" {
            continue;
        }
        request.state = "unknown".into();
        request.revision += 1;
        save_interaction_cas(tx, &request, "responding")?;
        append_event(
            tx,
            ids,
            limits,
            &execution.conversation_id,
            epoch,
            now,
            "interaction.updated",
            &id,
            request.revision,
            Ev::InteractionUpdated(Box::new(request)),
        )?;
    }
    Ok(())
}

/// 带状态条件的投影写回：expected_state 是写回前的当前状态，防止覆盖其他
/// 事务刚完成的 CAS 迁移。
fn save_interaction_cas(
    tx: &rusqlite::Transaction<'_>,
    interaction: &InteractionRequest,
    expected_state: &str,
) -> Result<(), StoreError> {
    let projection = encode(interaction)?;
    let updated = tx.execute(
        "UPDATE interactions SET state=?2, revision=?3, projection=?4
         WHERE id=?1 AND state=?5 AND revision=?3 - 1",
        params![
            interaction.id,
            interaction.state,
            interaction.revision as i64,
            projection,
            expected_state
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::Conflict("interaction_state"));
    }
    Ok(())
}

/// 按时间截止删除连续前缀，返回 (events, bytes)。
fn trim_stream_prefix(
    tx: &rusqlite::Transaction<'_>,
    stream: &str,
    cutoff_ms: u64,
) -> Result<(u64, u64), StoreError> {
    let keep: Option<i64> = tx
        .query_row(
            "SELECT MIN(position) FROM events WHERE stream_id=?1 AND recorded_at_ms>?2",
            params![stream, cutoff_ms as i64],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let boundary = match keep {
        Some(keep) => keep - 1,
        None => {
            let head: i64 = tx.query_row(
                "SELECT head FROM streams WHERE id=?1",
                params![stream],
                |row| row.get(0),
            )?;
            head
        }
    };
    delete_prefix_position(tx, stream, boundary)
}

/// 按字节预算删除最旧前缀。
fn trim_stream_bytes(
    tx: &rusqlite::Transaction<'_>,
    stream: &str,
    excess_bytes: u64,
) -> Result<(u64, u64), StoreError> {
    let positions: Vec<(i64, i64)> = {
        let mut stmt = tx.prepare(
            "SELECT position, encoded_bytes FROM events WHERE stream_id=?1 ORDER BY position",
        )?;
        let rows = stmt.query_map(params![stream], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    let mut removed_bytes = 0u64;
    let mut boundary: Option<i64> = None;
    for (position, bytes) in positions {
        if removed_bytes >= excess_bytes {
            break;
        }
        removed_bytes += bytes as u64;
        boundary = Some(position);
    }
    match boundary {
        Some(boundary) => delete_prefix_position(tx, stream, boundary),
        None => Ok((0, 0)),
    }
}

fn delete_prefix_position(
    tx: &rusqlite::Transaction<'_>,
    stream: &str,
    boundary: i64,
) -> Result<(u64, u64), StoreError> {
    if boundary < 1 {
        return Ok((0, 0));
    }
    let (events, bytes): (i64, i64) = tx.query_row(
        "SELECT COUNT(*), COALESCE(SUM(encoded_bytes),0) FROM events
         WHERE stream_id=?1 AND position<=?2",
        params![stream, boundary],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if events == 0 {
        return Ok((0, 0));
    }
    tx.execute(
        "DELETE FROM events WHERE stream_id=?1 AND position<=?2",
        params![stream, boundary],
    )?;
    tx.execute(
        "UPDATE streams SET first_available=?2, event_bytes=event_bytes-?3 WHERE id=?1",
        params![stream, boundary + 1, bytes],
    )?;
    Ok((events as u64, bytes as u64))
}

impl Store {
    /// 本地授权撤销（P5.3）：先持久提交再回执；revision 为本库单调序列，
    /// 重启不丢失（契约 §3.3）。
    pub async fn record_local_access_revocation(
        &self,
        kind: &str,
        target: &str,
        now_ms: u64,
    ) -> Result<u64, StoreError> {
        let kind = kind.to_string();
        let target = target.to_string();
        self.immediate(move |tx, _ids, _limits| {
            if !matches!(kind.as_str(), "jti" | "principal" | "kid") {
                return Err(StoreError::InvalidArgument(
                    "revocation kind must be jti, principal or kid".into(),
                ));
            }
            if target.is_empty() || target.len() > 256 {
                return Err(StoreError::InvalidArgument(
                    "revocation target must be 1..=256 bytes".into(),
                ));
            }
            let next: i64 = tx.query_row(
                "SELECT COALESCE(MAX(revision),0)+1 FROM access_revocations WHERE source='local'",
                [],
                |row| row.get(0),
            )?;
            tx.execute(
                "INSERT INTO access_revocations(source, revision, kind, target, effective_at_ms)
                 VALUES ('local', ?1, ?2, ?3, ?4)",
                params![next, kind, target, now_ms as i64],
            )?;
            Ok(next as u64)
        })
        .await
    }

    /// 应用 hub 撤权分页（P5.4）：同 revision 幂等；单个事务写全部项并推进
    /// 水位，先提交后回执。返回新水位。
    pub async fn apply_hub_access_revocations(
        &self,
        items: Vec<(u64, String, String, u64)>,
    ) -> Result<u64, StoreError> {
        self.immediate(move |tx, _ids, _limits| {
            let mut watermark: u64 = tx
                .query_row(
                    "SELECT hub_revision FROM access_sync_watermark WHERE id=1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(0) as u64;
            for (revision, kind, target, effective_at_ms) in items {
                if !matches!(kind.as_str(), "jti" | "principal" | "kid") {
                    return Err(StoreError::InvalidArgument(format!(
                        "hub revocation kind {kind} is not accepted"
                    )));
                }
                tx.execute(
                    "INSERT OR IGNORE INTO access_revocations(source, revision, kind, target, effective_at_ms)
                     VALUES ('hub', ?1, ?2, ?3, ?4)",
                    params![revision as i64, kind, target, effective_at_ms as i64],
                )?;
                watermark = watermark.max(revision);
            }
            tx.execute(
                "INSERT INTO access_sync_watermark(id, hub_revision) VALUES (1, ?1)
                 ON CONFLICT(id) DO UPDATE SET hub_revision=excluded.hub_revision",
                params![watermark as i64],
            )?;
            Ok(watermark)
        })
        .await
    }

    /// 节点重启后的同步起点：持久的 hub revision 水位。
    pub async fn access_sync_watermark(&self) -> Result<u64, StoreError> {
        self.readonly(move |_tx, _ids, _limits| {
            let conn = _tx;
            Ok(conn
                .query_row(
                    "SELECT hub_revision FROM access_sync_watermark WHERE id=1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(0) as u64)
        })
        .await
    }

    /// 每次操作的撤销检查：jti/principal/kid 任一命中即拒绝（契约 §3.3）。
    pub async fn access_revocation_blocked(
        &self,
        jti: &str,
        principal: &str,
        kid: &str,
    ) -> Result<bool, StoreError> {
        let jti = jti.to_string();
        let principal = principal.to_string();
        let kid = kid.to_string();
        self.readonly(move |tx, _ids, _limits| {
            let blocked: i64 = tx.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM access_revocations
                    WHERE (kind='jti' AND target=?1)
                       OR (kind='principal' AND target=?2)
                       OR (kind='kid' AND target=?3)
                 )",
                params![jti, principal, kid],
                |row| row.get(0),
            )?;
            Ok(blocked == 1)
        })
        .await
    }
}

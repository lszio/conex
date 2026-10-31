//! Hub-side private store (docs/contracts/private-runtime.md §3.1-§3.3, §6):
//! bootstrap codes, the grant ledger, and the monotonic revocation log in one
//! SQLite database. The ledger never stores grant tokens, private keys or
//! conversation content — only jti/subject/scope metadata the canary sweep may
//! see. Blocking database work runs on a dedicated thread behind a bounded
//! queue; the async side never holds a database lock across an await.
use std::path::{Path, PathBuf};

use conex_core::CallError;
use rusqlite::Connection;

const DB_QUEUE_CAPACITY: usize = 64;
pub const MAX_REVOKE_PAGE: usize = 128;
const MAX_PAGE_BYTES: usize = 196_608;
pub const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct GrantLedgerRow {
    pub jti: String,
    pub principal: String,
    pub tenant: String,
    pub node_id: String,
    pub node_epoch: String,
    pub purpose: String,
    pub scope_json: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RevocationRow {
    pub revision: u64,
    pub kind: String,
    pub target: String,
    pub effective_at_ms: u64,
}

#[derive(Debug)]
pub struct RevocationPage {
    pub items: Vec<RevocationRow>,
    pub next_revision: u64,
    pub has_more: bool,
}

type ResultReply = tokio::sync::oneshot::Sender<Result<(), CallError>>;
type BoolReply = tokio::sync::oneshot::Sender<Result<bool, CallError>>;
type RevisionReply = tokio::sync::oneshot::Sender<Result<u64, CallError>>;
type PageReply = tokio::sync::oneshot::Sender<Result<RevocationPage, CallError>>;

enum Job {
    PutBootstrap {
        code_hash: String,
        expires_at_ms: u64,
        reply: ResultReply,
    },
    ConsumeBootstrap {
        code_hash: String,
        now_ms: u64,
        reply: BoolReply,
    },
    RecordGrant {
        row: GrantLedgerRow,
        reply: ResultReply,
    },
    AppendRevocation {
        kind: String,
        target: String,
        now_ms: u64,
        reply: RevisionReply,
    },
    SyncRevocations {
        after_revision: u64,
        limit: usize,
        reply: PageReply,
    },
    LatestRevision(RevisionReply),
}

struct Database {
    connection: Connection,
}

impl Database {
    fn open(path: &Path) -> Result<Self, CallError> {
        if let Some(parent) = path.parent() {
            super::private_access::ensure_private_dir(parent)?;
        }
        let connection = Connection::open(path)
            .map_err(|error| store_error(format!("private store open: {error}")))?;
        set_mode(&connection, path)?;
        connection
            .execute_batch(&format!(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=FULL;
                 PRAGMA foreign_keys=ON;
                 PRAGMA user_version={SCHEMA_VERSION};"
            ))
            .map_err(|error| store_error(format!("private store pragmas: {error}")))?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| store_error(format!("private store version: {error}")))?;
        if version != SCHEMA_VERSION {
            // Refuse to run on a database this program does not know instead
            // of "upgrading" it blindly (contract §6: fail closed).
            return Err(store_error(format!(
                "private store user_version {version} is not {SCHEMA_VERSION}"
            )));
        }
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS bootstrap_codes(
                    code_hash TEXT PRIMARY KEY,
                    expires_at_ms INTEGER NOT NULL,
                    consumed INTEGER NOT NULL DEFAULT 0 CHECK(consumed IN (0,1))
                );
                CREATE TABLE IF NOT EXISTS grant_ledger(
                    jti TEXT PRIMARY KEY,
                    principal TEXT NOT NULL,
                    tenant TEXT NOT NULL,
                    node_id TEXT NOT NULL,
                    node_epoch TEXT NOT NULL,
                    purpose TEXT NOT NULL,
                    scope_json TEXT NOT NULL,
                    issued_at_ms INTEGER NOT NULL,
                    expires_at_ms INTEGER NOT NULL
                );
                CREATE TABLE IF NOT EXISTS revocations(
                    revision INTEGER PRIMARY KEY AUTOINCREMENT,
                    kind TEXT NOT NULL CHECK(kind IN ('jti','principal','kid')),
                    target TEXT NOT NULL,
                    created_at_ms INTEGER NOT NULL
                );",
            )
            .map_err(|error| store_error(format!("private store schema: {error}")))?;
        Ok(Self { connection })
    }

    fn put_bootstrap(&self, code_hash: &str, expires_at_ms: u64) -> Result<(), CallError> {
        self.connection
            .execute(
                "INSERT INTO bootstrap_codes(code_hash, expires_at_ms, consumed) VALUES(?1, ?2, 0)",
                rusqlite::params![code_hash, expires_at_ms as i64],
            )
            .map_err(|error| store_error(format!("bootstrap insert: {error}")))?;
        Ok(())
    }

    fn consume_bootstrap(&self, code_hash: &str, now_ms: u64) -> Result<bool, CallError> {
        let mut statement = self
            .connection
            .prepare("SELECT expires_at_ms, consumed FROM bootstrap_codes WHERE code_hash = ?1")
            .map_err(|error| store_error(format!("bootstrap select: {error}")))?;
        let row: Option<(i64, i64)> = statement
            .query_row([code_hash], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .optional()
            .map_err(|error| store_error(format!("bootstrap query: {error}")))?;
        let Some((expires_at_ms, consumed)) = row else {
            return Ok(false);
        };
        if consumed != 0 || (expires_at_ms as u64) <= now_ms {
            return Ok(false);
        }
        // Single use: the flag flip and the acceptance are one transaction.
        let updated = self
            .connection
            .execute(
                "UPDATE bootstrap_codes SET consumed = 1
                 WHERE code_hash = ?1 AND consumed = 0 AND expires_at_ms > ?2",
                rusqlite::params![code_hash, now_ms as i64],
            )
            .map_err(|error| store_error(format!("bootstrap consume: {error}")))?;
        Ok(updated == 1)
    }

    fn record_grant(&self, row: &GrantLedgerRow) -> Result<(), CallError> {
        self.connection
            .execute(
                "INSERT INTO grant_ledger(jti, principal, tenant, node_id, node_epoch,
                    purpose, scope_json, issued_at_ms, expires_at_ms)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    row.jti,
                    row.principal,
                    row.tenant,
                    row.node_id,
                    row.node_epoch,
                    row.purpose,
                    row.scope_json,
                    row.issued_at_ms as i64,
                    row.expires_at_ms as i64,
                ],
            )
            .map_err(|error| store_error(format!("grant ledger insert: {error}")))?;
        Ok(())
    }

    fn append_revocation(&self, kind: &str, target: &str, now_ms: u64) -> Result<u64, CallError> {
        self.connection
            .execute(
                "INSERT INTO revocations(kind, target, created_at_ms) VALUES(?1, ?2, ?3)",
                rusqlite::params![kind, target, now_ms as i64],
            )
            .map_err(|error| store_error(format!("revocation insert: {error}")))?;
        let revision: i64 = self
            .connection
            .query_row("SELECT last_insert_rowid()", [], |row| row.get(0))
            .map_err(|error| store_error(format!("revocation revision: {error}")))?;
        Ok(revision as u64)
    }

    fn sync_revocations(
        &self,
        after_revision: u64,
        limit: usize,
    ) -> Result<RevocationPage, CallError> {
        let limit = limit.clamp(1, MAX_REVOKE_PAGE);
        let mut statement = self
            .connection
            .prepare(
                "SELECT revision, kind, target, created_at_ms FROM revocations
                 WHERE revision > ?1 ORDER BY revision LIMIT ?2",
            )
            .map_err(|error| store_error(format!("revocation select: {error}")))?;
        let rows = statement
            .query_map(
                rusqlite::params![after_revision as i64, limit as i64 + 1],
                |row| {
                    Ok(RevocationRow {
                        revision: row.get::<_, i64>(0)? as u64,
                        kind: row.get(1)?,
                        target: row.get(2)?,
                        effective_at_ms: row.get::<_, i64>(3)? as u64,
                    })
                },
            )
            .map_err(|error| store_error(format!("revocation query: {error}")))?;
        let mut items: Vec<RevocationRow> = rows
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| store_error(format!("revocation read: {error}")))?;
        let mut bytes = 0usize;
        let mut has_more = false;
        let mut kept = Vec::new();
        for row in items.drain(..) {
            bytes += row.kind.len() + row.target.len() + 32;
            if kept.len() >= limit || bytes > MAX_PAGE_BYTES {
                has_more = true;
                break;
            }
            kept.push(row);
        }
        let next_revision = kept
            .last()
            .map(|row| row.revision)
            .unwrap_or(after_revision);
        Ok(RevocationPage {
            items: kept,
            next_revision,
            has_more,
        })
    }

    fn latest_revision(&self) -> Result<u64, CallError> {
        let revision: Option<i64> = self
            .connection
            .query_row("SELECT MAX(revision) FROM revocations", [], |row| {
                row.get(0)
            })
            .map_err(|error| store_error(format!("revocation head: {error}")))?;
        Ok(revision.unwrap_or(0) as u64)
    }
}

trait OptionalRow<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}
impl<T> OptionalRow<T> for rusqlite::Result<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

fn set_mode(_connection: &Connection, path: &Path) -> Result<(), CallError> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
        .map_err(|error| store_error(format!("private store dir mode: {error}")))?;
    for suffix in ["", "-wal", "-shm"] {
        let file = PathBuf::from(format!("{}{suffix}", path.display()));
        if file.exists() {
            fs::set_permissions(&file, fs::Permissions::from_mode(0o600))
                .map_err(|error| store_error(format!("private store file mode: {error}")))?;
        }
    }
    Ok(())
}

fn store_error(message: String) -> CallError {
    CallError::new(conex_proto::ErrorCode::Internal, message)
}

/// Handle handed to HTTP handlers; all calls queue onto the database thread.
#[derive(Clone)]
pub struct PrivateStore {
    tx: std::sync::mpsc::SyncSender<Job>,
}

impl PrivateStore {
    pub fn open(path: &Path) -> Result<Self, CallError> {
        let database = Database::open(path)?;
        let (tx, rx) = std::sync::mpsc::sync_channel::<Job>(DB_QUEUE_CAPACITY);
        std::thread::Builder::new()
            .name("conex-private-store".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    run_job(&database, job);
                }
            })
            .map_err(|error| store_error(format!("private store thread: {error}")))?;
        Ok(Self { tx })
    }

    fn send(&self, job: Job) -> Result<(), CallError> {
        self.tx
            .try_send(job)
            .map_err(|_| store_error("private store queue is full".into()))
    }

    pub async fn put_bootstrap(
        &self,
        code_hash: String,
        expires_at_ms: u64,
    ) -> Result<(), CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::PutBootstrap {
            code_hash,
            expires_at_ms,
            reply,
        })?;
        wait.await
            .map_err(|_| store_error("private store stopped".into()))?
    }

    pub async fn consume_bootstrap(
        &self,
        code_hash: String,
        now_ms: u64,
    ) -> Result<bool, CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::ConsumeBootstrap {
            code_hash,
            now_ms,
            reply,
        })?;
        wait.await
            .map_err(|_| store_error("private store stopped".into()))?
    }

    pub async fn record_grant(&self, row: GrantLedgerRow) -> Result<(), CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::RecordGrant { row, reply })?;
        wait.await
            .map_err(|_| store_error("private store stopped".into()))?
    }

    /// Commits the revocation before returning the revision: callers may hand
    /// the revision out only after it is durable (contract §3.3).
    pub async fn append_revocation(
        &self,
        kind: String,
        target: String,
        now_ms: u64,
    ) -> Result<u64, CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::AppendRevocation {
            kind,
            target,
            now_ms,
            reply,
        })?;
        wait.await
            .map_err(|_| store_error("private store stopped".into()))?
    }

    pub async fn sync_revocations(
        &self,
        after_revision: u64,
        limit: usize,
    ) -> Result<RevocationPage, CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::SyncRevocations {
            after_revision,
            limit,
            reply,
        })?;
        wait.await
            .map_err(|_| store_error("private store stopped".into()))?
    }

    pub async fn latest_revision(&self) -> Result<u64, CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::LatestRevision(reply))?;
        wait.await
            .map_err(|_| store_error("private store stopped".into()))?
    }
}

fn run_job(database: &Database, job: Job) {
    match job {
        Job::PutBootstrap {
            code_hash,
            expires_at_ms,
            reply,
        } => {
            let _ = reply.send(database.put_bootstrap(&code_hash, expires_at_ms));
        }
        Job::ConsumeBootstrap {
            code_hash,
            now_ms,
            reply,
        } => {
            let _ = reply.send(database.consume_bootstrap(&code_hash, now_ms));
        }
        Job::RecordGrant { row, reply } => {
            let _ = reply.send(database.record_grant(&row));
        }
        Job::AppendRevocation {
            kind,
            target,
            now_ms,
            reply,
        } => {
            let _ = reply.send(database.append_revocation(&kind, &target, now_ms));
        }
        Job::SyncRevocations {
            after_revision,
            limit,
            reply,
        } => {
            let _ = reply.send(database.sync_revocations(after_revision, limit));
        }
        Job::LatestRevision(reply) => {
            let _ = reply.send(database.latest_revision());
        }
    }
}

/// Blocking helper for the administrator `private-bootstrap` command, which
/// runs before any async runtime needs the store.
pub fn put_bootstrap_blocking(
    path: &Path,
    code_hash: &str,
    expires_at_ms: u64,
) -> Result<(), CallError> {
    Database::open(path)?.put_bootstrap(code_hash, expires_at_ms)
}

pub use MAX_REVOKE_PAGE as MAX_REVOKE_PAGE_ITEMS;

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
pub const SCHEMA_VERSION: i64 = 2;

/// One stored account. `kind` keeps local and future OIDC identities in one
/// table without pretending they authenticate the same way: `local` rows
/// carry an Argon2id PHC string, `oidc` rows carry (issuer, subject) and no
/// password. The authorization principal is unrelated to this table.
#[derive(Debug, Clone)]
pub struct AccountRow {
    pub username: String,
    pub kind: String,
    pub password_hash: Option<String>,
    pub role: String,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

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
type CountReply = tokio::sync::oneshot::Sender<Result<usize, CallError>>;
type AccountReply = tokio::sync::oneshot::Sender<Result<Option<AccountRow>, CallError>>;

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
    CountAccounts(CountReply),
    GetAccount {
        username: String,
        reply: AccountReply,
    },
    PutAccount {
        row: AccountRow,
        reply: ResultReply,
    },
    SetPasswordHash {
        username: String,
        password_hash: String,
        updated_at_ms: u64,
        reply: ResultReply,
    },
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
            .execute_batch(
                "PRAGMA journal_mode=WAL;
                 PRAGMA synchronous=FULL;
                 PRAGMA foreign_keys=ON;",
            )
            .map_err(|error| store_error(format!("private store pragmas: {error}")))?;
        // The version is read before anything writes it: a database from a
        // newer build must fail closed here instead of being silently
        // downgraded (contract §6). The historical code set user_version in
        // the pragma batch above, which made this check unreachable.
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(|error| store_error(format!("private store version: {error}")))?;
        if version > SCHEMA_VERSION {
            return Err(store_error(format!(
                "private store user_version {version} is newer than {SCHEMA_VERSION}"
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
        // v1 -> v2 is purely additive: the accounts table appears next to the
        // untouched v1 tables, so an upgrade preserves bootstrap codes, the
        // grant ledger and revocations verbatim.
        if version < 2 {
            connection
                .execute_batch(SCHEMA_V2_ACCOUNTS)
                .map_err(|error| store_error(format!("private store v2 accounts: {error}")))?;
        }
        connection
            .execute_batch(&format!("PRAGMA user_version={SCHEMA_VERSION};"))
            .map_err(|error| store_error(format!("private store version write: {error}")))?;
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

    fn count_accounts(&self) -> Result<usize, CallError> {
        let count: i64 = self
            .connection
            .query_row("SELECT COUNT(*) FROM accounts", [], |row| row.get(0))
            .map_err(|error| store_error(format!("account count: {error}")))?;
        Ok(count as usize)
    }

    fn get_account(&self, username: &str) -> Result<Option<AccountRow>, CallError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT username, kind, password_hash, role, created_at_ms, updated_at_ms
                 FROM accounts WHERE username = ?1",
            )
            .map_err(|error| store_error(format!("account select: {error}")))?;
        let row = statement
            .query_row([username], |row| {
                Ok(AccountRow {
                    username: row.get(0)?,
                    kind: row.get(1)?,
                    password_hash: row.get(2)?,
                    role: row.get(3)?,
                    created_at_ms: row.get::<_, i64>(4)? as u64,
                    updated_at_ms: row.get::<_, i64>(5)? as u64,
                })
            })
            .optional()
            .map_err(|error| store_error(format!("account query: {error}")))?;
        Ok(row)
    }

    fn put_account(&self, row: &AccountRow) -> Result<(), CallError> {
        let inserted = self
            .connection
            .execute(
                "INSERT INTO accounts(username, kind, password_hash, issuer, subject,
                    role, created_at_ms, updated_at_ms)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    row.username,
                    row.kind,
                    row.password_hash,
                    Option::<String>::None,
                    Option::<String>::None,
                    row.role,
                    row.created_at_ms as i64,
                    row.updated_at_ms as i64,
                ],
            )
            .map_err(|error| {
                let conflict = matches!(
                    &error,
                    rusqlite::Error::SqliteFailure(ffi, _)
                        if ffi.code == rusqlite::ErrorCode::ConstraintViolation
                );
                if conflict {
                    store_error("account already provisioned or username taken".into())
                } else {
                    store_error(format!("account insert: {error}"))
                }
            })?;
        if inserted != 1 {
            return Err(store_error("account insert inserted no row".into()));
        }
        Ok(())
    }

    fn set_password_hash(
        &self,
        username: &str,
        password_hash: &str,
        updated_at_ms: u64,
    ) -> Result<(), CallError> {
        let updated = self
            .connection
            .execute(
                "UPDATE accounts SET password_hash = ?2, updated_at_ms = ?3
                 WHERE username = ?1 AND kind = 'local'",
                rusqlite::params![username, password_hash, updated_at_ms as i64],
            )
            .map_err(|error| store_error(format!("account update: {error}")))?;
        if updated == 0 {
            return Err(store_error("no local account carries this username".into()));
        }
        Ok(())
    }
}

/// The v2 accounts table. Local rows must carry a password hash; OIDC rows
/// (route B, not provisioned yet) must carry issuer+subject and are unique
/// per identity so a future IdP login can look itself up.
const SCHEMA_V2_ACCOUNTS: &str = "CREATE TABLE IF NOT EXISTS accounts(
    username TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('local','oidc')),
    password_hash TEXT,
    issuer TEXT,
    subject TEXT,
    role TEXT NOT NULL DEFAULT 'admin' CHECK(role IN ('admin')),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    CHECK(kind <> 'local' OR password_hash IS NOT NULL),
    CHECK(kind <> 'oidc' OR (issuer IS NOT NULL AND subject IS NOT NULL))
);
CREATE UNIQUE INDEX IF NOT EXISTS accounts_oidc_identity
    ON accounts(issuer, subject)
    WHERE kind = 'oidc' AND issuer IS NOT NULL AND subject IS NOT NULL;";

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

    /// The number of stored accounts. The private login page uses it to pick
    /// first-provisioning vs password login; zero reveals nothing sensitive.
    pub async fn count_accounts(&self) -> Result<usize, CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::CountAccounts(reply))?;
        wait.await
            .map_err(|_| store_error("private store stopped".into()))?
    }

    pub async fn get_account(&self, username: String) -> Result<Option<AccountRow>, CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::GetAccount { username, reply })?;
        wait.await
            .map_err(|_| store_error("private store stopped".into()))?
    }

    pub async fn put_account(&self, row: AccountRow) -> Result<(), CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::PutAccount { row, reply })?;
        wait.await
            .map_err(|_| store_error("private store stopped".into()))?
    }

    pub async fn set_password_hash(
        &self,
        username: String,
        password_hash: String,
        updated_at_ms: u64,
    ) -> Result<(), CallError> {
        let (reply, wait) = tokio::sync::oneshot::channel();
        self.send(Job::SetPasswordHash {
            username,
            password_hash,
            updated_at_ms,
            reply,
        })?;
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
        Job::CountAccounts(reply) => {
            let _ = reply.send(database.count_accounts());
        }
        Job::GetAccount { username, reply } => {
            let _ = reply.send(database.get_account(&username));
        }
        Job::PutAccount { row, reply } => {
            let _ = reply.send(database.put_account(&row));
        }
        Job::SetPasswordHash {
            username,
            password_hash,
            updated_at_ms,
            reply,
        } => {
            let _ =
                reply.send(database.set_password_hash(&username, &password_hash, updated_at_ms));
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

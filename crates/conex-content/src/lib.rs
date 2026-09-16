//! conex-content: local persistent content store for P1-06/P1-07.
//!
//! Implements the state machine from `docs/contracts/p1-blob.md`:
//!   blob/put(open) → uploading → verified → commit → committed
//!                          ↘ cancel/expire → staging 回收
//!
//! Concurrency: each public call acquires the inner `refs` mutex; on-disk
//! writes use cap-std tmp + rename for crash safety. The blob/json recovery
//! scan at open time ensures no "missing block but committed root" state can
//! survive a crash.
#![forbid(unsafe_code)]

pub mod content;
pub mod error;
pub mod receipt;
pub mod store;
pub mod transfer;

pub use content::{ContentStore, Upload};
pub use error::ContentError;
pub use receipt::{CommitRecord, PinRecord, UploadState};
pub use store::{
    ContentRoot, LocalBlockStore, cid_for_bytes, deterministic_id, parse_cid, sha256_digest,
};

/// Default lease: 1 hour (设计 §5.5).
pub const DEFAULT_LEASE_MS: u64 = 60 * 60 * 1000;
/// Default chunk size: 256 KiB.
pub const DEFAULT_CHUNK_SIZE: u32 = 262_144;

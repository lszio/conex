//! Local persistent content store errors. Mapped to conex ErrorCode by callers.
use thiserror::Error;

use conex_proto::cid::cid_for_raw;

#[derive(Debug, Error)]
pub enum ContentError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("serde_json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid cid: {0}")]
    InvalidCid(String),
    #[error("bad chunk: declared_cid {declared} != computed {computed}")]
    BadChunk { declared: String, computed: String },
    #[error("unknown upload: {0}")]
    UnknownUpload(String),
    #[error("upload expired")]
    UploadExpired,
    #[error("chunk out of range: index {index}, max {max}")]
    ChunkOutOfRange { index: u32, max: u32 },
    #[error("duplicate chunk index {0}")]
    DuplicateChunk(u32),
    #[error("chunk index {0} missing from the upload")]
    MissingChunkIndex(u32),
    #[error("block {index} length {actual} != expected {expected}")]
    BlockLengthMismatch {
        index: u32,
        expected: u64,
        actual: u64,
    },
    #[error("declared root kind {0} is not supported by the local backend")]
    UnsupportedRootKind(String),
    #[error("canonical manifest rejected: {0}")]
    Manifest(String),
    #[error("declared root mismatch: declared {declared}, recomputed {recomputed}")]
    DeclaredRootMismatch {
        declared: String,
        recomputed: String,
    },
    #[error("missing chunks: {0:?}")]
    MissingChunks(Vec<String>),
    #[error("persistence level {0} not supported by local backend")]
    UnsupportedPersistence(String),
    #[error("unknown pin: {0}")]
    UnknownPin(String),
    #[error("unknown block: {0}")]
    UnknownBlock(String),
    #[error("capability path traversal blocked: {0}")]
    PathTraversal(String),
}

impl ContentError {
    pub fn bad_request_code(&self) -> i32 {
        use conex_proto::v1::ErrorCode;
        match self {
            ContentError::BadChunk { .. }
            | ContentError::InvalidCid(_)
            | ContentError::PathTraversal(_)
            | ContentError::ChunkOutOfRange { .. }
            | ContentError::DuplicateChunk(_)
            | ContentError::DeclaredRootMismatch { .. }
            | ContentError::MissingChunks(_)
            | ContentError::MissingChunkIndex(_)
            | ContentError::BlockLengthMismatch { .. }
            | ContentError::UnsupportedRootKind(_)
            | ContentError::Manifest(_) => ErrorCode::BadBlob as i32,
            ContentError::UnknownUpload(_)
            | ContentError::UnknownBlock(_)
            | ContentError::UnknownPin(_) => ErrorCode::UnknownProvider as i32,
            ContentError::UploadExpired => ErrorCode::Timeout as i32,
            ContentError::UnsupportedPersistence(_) => ErrorCode::Unavailable as i32,
            ContentError::Io(_) | ContentError::Json(_) => ErrorCode::Internal as i32,
        }
    }

    /// Build the matched CID for an actual byte payload (uses cid_for_raw).
    pub fn expected_cid_for(bytes: &[u8]) -> String {
        cid_for_raw(bytes)
    }
}

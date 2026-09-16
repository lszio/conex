//! FsRoot: capability-relative reads with symlink and type confinement.
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use conex_core::{CallContext, CallError, CallResult, ExecutionIo, Handler};
use conex_proto::v1;
use serde_json::Value;

use crate::list::build_summary;

pub const MAX_DOC_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone)]
pub struct ReadSnapshot {
    pub bytes: Bytes,
    pub cid: String,
}

pub struct FsRoot {
    dir: Dir,
}

impl FsRoot {
    pub fn open(path: &Path) -> CallResult<FsRoot> {
        let dir = Dir::open_ambient_dir(path, ambient_authority()).map_err(|error| {
            CallError::new(v1::ErrorCode::Internal, format!("open fs root: {error}"))
        })?;
        Ok(Self { dir })
    }

    pub(crate) fn dir(&self) -> &Dir {
        &self.dir
    }

    pub(crate) fn open_regular(&self, resource: &str) -> CallResult<(cap_std::fs::File, u64)> {
        self.reject_symlinks(resource)?;
        let file = self.dir.open(resource).map_err(map_io)?;
        let metadata = file.metadata().map_err(map_io)?;
        if !metadata.is_file() {
            return Err(CallError::new(
                v1::ErrorCode::Forbidden,
                "only regular files are readable",
            ));
        }
        Ok((file, metadata.len()))
    }

    fn reject_symlinks(&self, resource: &str) -> CallResult<()> {
        let mut prefix = String::new();
        for segment in resource.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            let metadata = self.dir.symlink_metadata(&prefix).map_err(map_io)?;
            if metadata.file_type().is_symlink() {
                return Err(CallError::new(
                    v1::ErrorCode::Forbidden,
                    "symlink components are not allowed",
                ));
            }
        }
        Ok(())
    }

    /// Read one immutable byte snapshot (bounded) and compute its raw CID.
    pub fn read(&self, resource: &str, limit: usize) -> CallResult<ReadSnapshot> {
        let resource = conex_source::resource::normalize_resource(resource)?;
        if resource.is_empty() {
            return Err(CallError::new(
                v1::ErrorCode::BadRequest,
                "resourceId must not be empty",
            ));
        }
        let (file, length) = self.open_regular(&resource)?;
        if length > limit as u64 {
            return Err(CallError::new(
                v1::ErrorCode::PayloadTooLarge,
                "document exceeds the per-document limit",
            ));
        }
        let mut bytes = Vec::with_capacity(length as usize);
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(map_io)?;
        if bytes.len() > limit {
            return Err(CallError::new(
                v1::ErrorCode::PayloadTooLarge,
                "document exceeds the per-document limit",
            ));
        }
        // Same function as every `blob/*` root (design §5.3): a document that
        // ever exceeds one chunk must address as its manifest root, not raw.
        let cid = conex_proto::cid::content_cid(&bytes, conex_proto::cid::CHUNK_SIZE);
        Ok(ReadSnapshot {
            bytes: Bytes::from(bytes),
            cid,
        })
    }
}

pub fn map_io(error: std::io::Error) -> CallError {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => CallError::new(v1::ErrorCode::BadRequest, "resource not found"),
        ErrorKind::PermissionDenied => {
            CallError::new(v1::ErrorCode::Forbidden, "resource is not readable")
        }
        _ => CallError::new(
            v1::ErrorCode::Unavailable,
            format!("filesystem error: {error}"),
        ),
    }
}

pub struct ReadHandler {
    pub root: Arc<FsRoot>,
}

#[async_trait]
impl Handler for ReadHandler {
    async fn execute(
        &self,
        ctx: &CallContext,
        input: Value,
        _io: ExecutionIo,
    ) -> CallResult<Value> {
        let resource = input
            .get("resourceId")
            .and_then(Value::as_str)
            .ok_or_else(|| CallError::new(v1::ErrorCode::BadRequest, "resourceId is required"))?;
        if ctx.claim.resource_id != resource {
            return Err(CallError::new(
                v1::ErrorCode::Forbidden,
                "resource does not match the authorized claim",
            ));
        }
        let snapshot = self.root.read(resource, MAX_DOC_BYTES)?;
        let text = String::from_utf8(snapshot.bytes.to_vec()).map_err(|_| {
            CallError::new(v1::ErrorCode::BadRequest, "document is not valid UTF-8")
        })?;
        let summary = build_summary(resource, snapshot.bytes.len() as u64)?;
        let response = v1::SourceReadResponse {
            resource: Some(summary),
            text,
            cid: snapshot.cid,
        };
        serde_json::to_value(response).map_err(|error| {
            CallError::new(v1::ErrorCode::Internal, format!("encode response: {error}"))
        })
    }
}

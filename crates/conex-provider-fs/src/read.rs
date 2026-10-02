//! FsRoot: capability-relative reads with symlink and type confinement.
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use bytes::Bytes;
use cap_std::ambient_authority;
use cap_std::fs::Dir;
use conex_core::{CallContext, CallError, CallResult, ExecutionIo, Handler};
use conex_proto;
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
            CallError::new(
                conex_proto::ErrorCode::Internal,
                format!("open fs root: {error}"),
            )
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
                conex_proto::ErrorCode::Forbidden,
                "only regular files are readable",
            ));
        }
        Ok((file, metadata.len()))
    }

    /// Metadata-only stat (len, mtime in nanoseconds) with the same symlink
    /// and regular-file confinement as [`FsRoot::open_regular`]. Never opens
    /// the file, so oversized documents can be listed and referenced without
    /// reading content.
    pub fn stat(&self, resource: &str) -> CallResult<(u64, u64)> {
        self.reject_symlinks(resource)?;
        let metadata = self.dir.metadata(resource).map_err(map_io)?;
        if !metadata.is_file() {
            return Err(CallError::new(
                conex_proto::ErrorCode::Forbidden,
                "only regular files are readable",
            ));
        }
        let mtime_ns = metadata
            .modified()
            .map_err(map_io)?
            .into_std()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        Ok((metadata.len(), mtime_ns))
    }

    /// M3: bounded slice read `[offset, offset + len)` with the same
    /// confinement as [`FsRoot::read`]; returns `(bytes, total_len,
    /// mtime_ns)`. `len` must be ≤ the protocol chunk size; the caller
    /// enforces revision binding before reading.
    pub fn read_range(
        &self,
        resource: &str,
        offset: u64,
        len: usize,
    ) -> CallResult<(Vec<u8>, u64, u64)> {
        let (total, mtime_ns) = self.stat(resource)?;
        if offset >= total {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "offset is beyond the end of the resource",
            ));
        }
        let (mut file, _) = self.open_regular(resource)?;
        use std::io::{Read as _, Seek as _};
        file.seek(std::io::SeekFrom::Start(offset))
            .map_err(map_io)?;
        let mut bytes = vec![0u8; len];
        file.read_exact(&mut bytes).map_err(map_io)?;
        Ok((bytes, total, mtime_ns))
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
                    conex_proto::ErrorCode::Forbidden,
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
                conex_proto::ErrorCode::BadRequest,
                "resourceId must not be empty",
            ));
        }
        let (file, length) = self.open_regular(&resource)?;
        if length > limit as u64 {
            return Err(CallError::new(
                conex_proto::ErrorCode::PayloadTooLarge,
                "document exceeds the per-document limit",
            ));
        }
        let mut bytes = Vec::with_capacity(length as usize);
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(map_io)?;
        if bytes.len() > limit {
            return Err(CallError::new(
                conex_proto::ErrorCode::PayloadTooLarge,
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
        ErrorKind::NotFound => {
            CallError::new(conex_proto::ErrorCode::BadRequest, "resource not found")
        }
        ErrorKind::PermissionDenied => CallError::new(
            conex_proto::ErrorCode::Forbidden,
            "resource is not readable",
        ),
        _ => CallError::new(
            conex_proto::ErrorCode::Unavailable,
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
            .ok_or_else(|| {
                CallError::new(conex_proto::ErrorCode::BadRequest, "resourceId is required")
            })?;
        if ctx.claim.resource_id != resource {
            return Err(CallError::new(
                conex_proto::ErrorCode::Forbidden,
                "resource does not match the authorized claim",
            ));
        }
        let (len, mtime_ns) = self.root.stat(resource)?;
        let revision = Some(mtime_ns.to_string());
        let ext = resource.rsplit_once('.').map(|(_, extension)| extension);
        if matches!(ext, Some("md") | Some("org") | Some("txt")) && len <= MAX_DOC_BYTES as u64 {
            let snapshot = self.root.read(resource, MAX_DOC_BYTES)?;
            let text = String::from_utf8(snapshot.bytes.to_vec()).map_err(|_| {
                CallError::new(
                    conex_proto::ErrorCode::BadRequest,
                    "document is not valid UTF-8",
                )
            })?;
            let summary = build_summary(resource, len, revision)?;
            let response = conex_proto::SourceReadResponse {
                resource: Some(summary),
                text: Some(text),
                cid: Some(snapshot.cid),
                content: None,
            };
            return serde_json::to_value(response).map_err(|error| {
                CallError::new(
                    conex_proto::ErrorCode::Internal,
                    format!("encode response: {error}"),
                )
            });
        }
        // Attachments, oversized and unknown-type files are referenced, not
        // read: bytes stay on disk, cid stays absent (never fake a CID).
        let summary = build_summary(resource, len, revision.clone())?;
        let mime = summary.mime.clone();
        let response = conex_proto::SourceReadResponse {
            resource: Some(summary),
            text: None,
            cid: None,
            content: Some(conex_proto::BlobRef {
                cid: None,
                size_bytes: len.to_string(),
                mime,
                access: Some(conex_proto::BlobAccess {
                    endpoint_id: ctx.endpoint_id.clone(),
                    plane: "broker".into(),
                    space_id: None,
                    resource_id: resource.to_string(),
                }),
                revision,
            }),
        };
        serde_json::to_value(response).map_err(|error| {
            CallError::new(
                conex_proto::ErrorCode::Internal,
                format!("encode response: {error}"),
            )
        })
    }
}

/// M3: the same handler serves bounded raw slices for `/content` on local
/// endpoints; remote endpoints take the binary agent link instead, and both
/// share the same revision binding.
#[async_trait]
impl conex_core::RangeReader for ReadHandler {
    async fn read_range(
        &self,
        ctx: &CallContext,
        offset: u64,
        length: usize,
        expected_revision: Option<&str>,
    ) -> CallResult<(Vec<u8>, String, bool)> {
        let resource = ctx.claim.resource_id.as_str();
        if resource.is_empty() {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "resourceId is required",
            ));
        }
        if length == 0 || length > conex_proto::cid::CHUNK_SIZE {
            return Err(CallError::new(
                conex_proto::ErrorCode::BadRequest,
                "range length must be within one protocol chunk",
            ));
        }
        let (_total, mtime_ns) = self.root.stat(resource)?;
        let revision = mtime_ns.to_string();
        if let Some(expected) = expected_revision {
            if expected != revision {
                return Err(CallError::new(
                    conex_proto::ErrorCode::StaleRevision,
                    format!("resource revision moved: requested {expected}, current {revision}"),
                ));
            }
        }
        let (bytes, total, mtime_ns) = self.root.read_range(resource, offset, length)?;
        let eof = offset + bytes.len() as u64 >= total;
        Ok((bytes, mtime_ns.to_string(), eof))
    }
}

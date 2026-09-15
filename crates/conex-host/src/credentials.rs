//! Scoped env/file credential resolution.
//!
//! Construction validates references and allowed paths only; secrets are read
//! after the Host has verified the peer (design S8.2). The backend path comes
//! only from installation config, never from caller input.
use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use conex_core::{CallError, CallResult, CredentialKey, CredentialStore, Secret, VerifiedPeer};
use conex_proto::v1;

pub const MAX_SECRET_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialBackend {
    Env(String),
    File(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialBinding {
    pub key: CredentialKey,
    pub backend: CredentialBackend,
}

/// Injected access to the actual secret bytes; production uses the system.
#[async_trait]
pub trait SecretReader: Send + Sync {
    async fn read_env(&self, name: &str) -> CallResult<String>;
    async fn read_file(&self, path: &Path) -> CallResult<String>;
}

pub struct SystemSecretReader;

#[async_trait]
impl SecretReader for SystemSecretReader {
    async fn read_env(&self, name: &str) -> CallResult<String> {
        std::env::var(name).map_err(|_| forbidden("environment credential is missing"))
    }

    async fn read_file(&self, path: &Path) -> CallResult<String> {
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|_| forbidden("credential file is unreadable"))?;
        if metadata.file_type().is_symlink() {
            return Err(forbidden("credential file must not be a symlink"));
        }
        if !metadata.is_file() {
            return Err(forbidden("credential path is not a regular file"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(forbidden("credential file must be owner-only"));
            }
        }
        std::fs::read_to_string(path).map_err(|_| forbidden("credential file is unreadable"))
    }
}

pub struct EnvFileStore {
    bindings: HashMap<CredentialKey, CredentialBackend>,
    reader: Arc<dyn SecretReader>,
}

impl EnvFileStore {
    pub fn new(bindings: Vec<CredentialBinding>) -> CallResult<EnvFileStore> {
        Self::with_reader(bindings, Arc::new(SystemSecretReader))
    }

    pub fn with_reader(
        bindings: Vec<CredentialBinding>,
        reader: Arc<dyn SecretReader>,
    ) -> CallResult<EnvFileStore> {
        let mut map: HashMap<CredentialKey, CredentialBackend> = HashMap::new();
        for binding in bindings {
            validate_backend(&binding.backend)?;
            if map.insert(binding.key.clone(), binding.backend).is_some() {
                return Err(CallError::new(
                    v1::ErrorCode::Internal,
                    "duplicate credential binding",
                ));
            }
        }
        Ok(Self {
            bindings: map,
            reader,
        })
    }
}

impl fmt::Debug for EnvFileStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvFileStore")
            .field("bindings", &self.bindings.len())
            .finish()
    }
}

#[async_trait]
impl CredentialStore for EnvFileStore {
    async fn resolve(&self, key: &CredentialKey, peer: &VerifiedPeer) -> CallResult<Secret> {
        let backend = self
            .bindings
            .get(key)
            .ok_or_else(|| forbidden("no credential is bound for this key"))?;
        if key.audience != peer.audience {
            return Err(forbidden(
                "credential audience does not match the verified peer",
            ));
        }
        let value = match backend {
            CredentialBackend::Env(name) => self.reader.read_env(name).await?,
            CredentialBackend::File(path) => self.reader.read_file(path).await?,
        };
        validate_secret(&value)?;
        Ok(Secret::new(value))
    }
}

fn forbidden(message: &str) -> CallError {
    CallError::new(v1::ErrorCode::Forbidden, message)
}

fn validate_secret(value: &str) -> CallResult<()> {
    if value.is_empty() {
        return Err(forbidden("credential value is empty"));
    }
    if value.len() > MAX_SECRET_BYTES {
        return Err(CallError::new(
            v1::ErrorCode::PayloadTooLarge,
            "credential value exceeds 16 KiB",
        ));
    }
    if value.contains('\r') || value.contains('\n') {
        return Err(CallError::new(
            v1::ErrorCode::BadRequest,
            "credential value must not contain CR or LF",
        ));
    }
    Ok(())
}

fn validate_backend(backend: &CredentialBackend) -> CallResult<()> {
    match backend {
        CredentialBackend::Env(name) => {
            if name.is_empty() {
                return Err(CallError::new(
                    v1::ErrorCode::Internal,
                    "env credential name must not be empty",
                ));
            }
            Ok(())
        }
        CredentialBackend::File(path) => {
            if !path.is_absolute() {
                return Err(CallError::new(
                    v1::ErrorCode::Internal,
                    "credential file path must be absolute",
                ));
            }
            if path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
            {
                return Err(CallError::new(
                    v1::ErrorCode::Internal,
                    "credential file path must not contain ..",
                ));
            }
            Ok(())
        }
    }
}

//! Credential isolation and secret hygiene. No process-global env is mutated.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use conex_core::{
    CallError, CallResult, CredentialKey, CredentialStore, PeerVerification, VerifiedPeer,
};
use conex_host::{
    CredentialBackend, CredentialBinding, EnvFileStore, SecretReader, SystemSecretReader,
};
use conex_proto::v1;

mod support {
    use super::*;

    pub struct CountingReader {
        env: Mutex<HashMap<String, String>>,
        files: Mutex<HashMap<PathBuf, String>>,
        reads: AtomicUsize,
    }

    impl CountingReader {
        pub fn new() -> Self {
            Self {
                env: Mutex::new(HashMap::new()),
                files: Mutex::new(HashMap::new()),
                reads: AtomicUsize::new(0),
            }
        }

        pub fn set_env(&self, name: &str, value: &str) {
            self.env
                .lock()
                .unwrap()
                .insert(name.to_string(), value.to_string());
        }

        pub fn set_file(&self, path: &Path, value: &str) {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_path_buf(), value.to_string());
        }

        pub fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl SecretReader for CountingReader {
        async fn read_env(&self, name: &str) -> CallResult<String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.env
                .lock()
                .unwrap()
                .get(name)
                .cloned()
                .ok_or_else(|| CallError::new(v1::ErrorCode::Forbidden, "missing env secret"))
        }

        async fn read_file(&self, path: &Path) -> CallResult<String> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.files
                .lock()
                .unwrap()
                .get(path)
                .cloned()
                .ok_or_else(|| CallError::new(v1::ErrorCode::Forbidden, "missing file secret"))
        }
    }

    pub fn key(tenant: &str, audience: &str, name: &str, principal: Option<&str>) -> CredentialKey {
        CredentialKey {
            tenant_id: tenant.into(),
            holder: "host".into(),
            provider_id: "catalog".into(),
            audience: audience.into(),
            name: name.into(),
            principal_id: principal.map(str::to_string),
        }
    }

    pub fn peer_for(audience: &str) -> VerifiedPeer {
        VerifiedPeer {
            target_id: "catalog".into(),
            audience: audience.into(),
            address: "198.51.100.7:443".parse().unwrap(),
            verification: PeerVerification::TlsServer,
        }
    }

    pub struct CredentialFixture {
        pub _dir: tempfile::TempDir,
        pub store: EnvFileStore,
        pub key: CredentialKey,
        pub reader: Arc<CountingReader>,
        pub file_path: PathBuf,
    }

    impl CredentialFixture {
        pub fn file_binding() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("catalog-key");
            let reader = Arc::new(CountingReader::new());
            reader.set_file(&path, "file-secret");
            let key = key("tenant-a", "catalog.example", "catalog/key", None);
            let store = EnvFileStore::with_reader(
                vec![CredentialBinding {
                    key: key.clone(),
                    backend: CredentialBackend::File(path.clone()),
                }],
                reader.clone(),
            )
            .unwrap();
            Self {
                _dir: dir,
                store,
                key,
                reader,
                file_path: path,
            }
        }

        pub fn env_binding() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let reader = Arc::new(CountingReader::new());
            reader.set_env("TENANT_A_CATALOG_KEY", "env-secret");
            let key = key("tenant-a", "catalog.example", "catalog/key", None);
            let store = EnvFileStore::with_reader(
                vec![CredentialBinding {
                    key: key.clone(),
                    backend: CredentialBackend::Env("TENANT_A_CATALOG_KEY".into()),
                }],
                reader.clone(),
            )
            .unwrap();
            Self {
                _dir: dir,
                store,
                key,
                reader,
                file_path: PathBuf::new(),
            }
        }

        pub fn peer(&self) -> VerifiedPeer {
            peer_for(&self.key.audience)
        }

        pub fn read_count(&self) -> usize {
            self.reader.reads()
        }
    }
}

use support::{CountingReader, CredentialFixture, key, peer_for};

#[tokio::test]
async fn audience_mismatch_does_not_load_secret() {
    let fixture = CredentialFixture::file_binding();
    let result = fixture
        .store
        .resolve(&fixture.key, &peer_for("other.example"))
        .await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Forbidden)
    );
    assert_eq!(fixture.read_count(), 0);
}

#[tokio::test]
async fn unknown_key_is_rejected_without_reading() {
    let fixture = CredentialFixture::file_binding();
    let other = key("tenant-z", "catalog.example", "catalog/key", None);
    let result = fixture.store.resolve(&other, &fixture.peer()).await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Forbidden)
    );
    assert_eq!(fixture.read_count(), 0);
}

#[tokio::test]
async fn principal_binding_mismatch_is_rejected_without_reading() {
    let fixture = CredentialFixture::file_binding();
    let other = key(
        "tenant-a",
        "catalog.example",
        "catalog/key",
        Some("someone-else"),
    );
    let result = fixture.store.resolve(&other, &fixture.peer()).await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Forbidden)
    );
    assert_eq!(fixture.read_count(), 0);
}

#[tokio::test]
async fn tenant_isolation_keeps_same_named_keys_apart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog-key");
    let reader = Arc::new(CountingReader::new());
    reader.set_file(&path, "tenant-a-secret");
    reader.set_env("TENANT_B_KEY", "tenant-b-secret");
    let key_a = key("tenant-a", "catalog.example", "catalog/key", None);
    let key_b = key("tenant-b", "catalog.example", "catalog/key", None);
    let store = EnvFileStore::with_reader(
        vec![
            CredentialBinding {
                key: key_a.clone(),
                backend: CredentialBackend::File(path),
            },
            CredentialBinding {
                key: key_b.clone(),
                backend: CredentialBackend::Env("TENANT_B_KEY".into()),
            },
        ],
        reader.clone(),
    )
    .unwrap();
    let peer = peer_for("catalog.example");
    assert_eq!(
        store.resolve(&key_a, &peer).await.unwrap().expose(),
        "tenant-a-secret"
    );
    assert_eq!(
        store.resolve(&key_b, &peer).await.unwrap().expose(),
        "tenant-b-secret"
    );
    assert_eq!(reader.reads(), 2);
}

#[tokio::test]
async fn successful_resolve_reads_once() {
    let fixture = CredentialFixture::file_binding();
    let secret = fixture
        .store
        .resolve(&fixture.key, &fixture.peer())
        .await
        .unwrap();
    assert_eq!(secret.expose(), "file-secret");
    assert_eq!(fixture.read_count(), 1);
    assert!(!format!("{secret:?}").contains("file-secret"));
}

#[tokio::test]
async fn file_change_is_read_again_without_caching() {
    let fixture = CredentialFixture::file_binding();
    fixture
        .reader
        .set_file(&fixture.file_path, "rotated-secret");
    let secret = fixture
        .store
        .resolve(&fixture.key, &fixture.peer())
        .await
        .unwrap();
    assert_eq!(secret.expose(), "rotated-secret");
}

#[tokio::test]
async fn empty_secret_is_rejected() {
    let fixture = CredentialFixture::env_binding();
    fixture.reader.set_env("TENANT_A_CATALOG_KEY", "");
    let result = fixture.store.resolve(&fixture.key, &fixture.peer()).await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::Forbidden)
    );
}

#[tokio::test]
async fn oversized_secret_is_rejected() {
    let fixture = CredentialFixture::env_binding();
    fixture
        .reader
        .set_env("TENANT_A_CATALOG_KEY", &"x".repeat(20 * 1024));
    let result = fixture.store.resolve(&fixture.key, &fixture.peer()).await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::PayloadTooLarge)
    );
}

#[tokio::test]
async fn newline_secret_is_rejected() {
    let fixture = CredentialFixture::env_binding();
    fixture
        .reader
        .set_env("TENANT_A_CATALOG_KEY", "line1\nline2");
    let result = fixture.store.resolve(&fixture.key, &fixture.peer()).await;
    assert_eq!(
        result.unwrap_err().code_enum(),
        Some(v1::ErrorCode::BadRequest)
    );
}

#[test]
fn duplicate_bindings_are_rejected() {
    let key = key("tenant-a", "catalog.example", "catalog/key", None);
    let bindings = vec![
        CredentialBinding {
            key: key.clone(),
            backend: CredentialBackend::Env("A".into()),
        },
        CredentialBinding {
            key,
            backend: CredentialBackend::Env("B".into()),
        },
    ];
    assert!(EnvFileStore::new(bindings).is_err());
}

#[test]
fn relative_backend_path_is_rejected() {
    let key = key("tenant-a", "catalog.example", "catalog/key", None);
    let bindings = vec![CredentialBinding {
        key,
        backend: CredentialBackend::File(PathBuf::from("relative")),
    }];
    assert!(EnvFileStore::new(bindings).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn system_reader_enforces_owner_only_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("secret");
    std::fs::write(&path, "value").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(SystemSecretReader.read_file(&path).await.is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(SystemSecretReader.read_file(&path).await.unwrap(), "value");
}

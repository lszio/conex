//! Approved credential backends — `env:NAME`, `env64:BASE64`, `file:PATH` —
//! with one set of rules for every crate that loads an operator-declared
//! secret. Backend strings live in operator config; secrets never do
//! (基线「不提交秘密」：配置里只允许间接引用).
//!
//! History this module replaces: the same rules had drifted across
//! `conex-access` (env64, symlink-following file reads), the agent's
//! `node_config` (strict file checks, no env64) and the agent runtime's
//! private copy (no file checks at all).

/// Shape check for an operator-declared backend: a known scheme with a
/// non-empty tail. Loading rules live in [`read_secret_bytes`].
pub fn is_valid_backend(source: &str) -> bool {
    source
        .strip_prefix("env:")
        .or_else(|| source.strip_prefix("env64:"))
        .or_else(|| source.strip_prefix("file:"))
        .is_some_and(|value| !value.trim().is_empty())
}

/// Shape check for text-secret backends only. `env64:` exists for binary
/// material (DER keys) routed to signing keys; bearer-style credentials
/// must not accept it, or a misconfigured backend is silently misread.
pub fn is_valid_text_backend(source: &str) -> bool {
    source
        .strip_prefix("env:")
        .or_else(|| source.strip_prefix("file:"))
        .is_some_and(|value| !value.trim().is_empty())
}

/// Load the secret bytes behind a backend declaration.
///
/// `file:` requires a regular, non-symlink file with mode 0600 and at most
/// 4096 bytes — checked via `symlink_metadata`, so a symlink is rejected
/// before its target is ever read. `env64:` base64-decodes the variable
/// (secret managers hold printable values; DER keys are binary). Errors are
/// operator-facing strings and deliberately never echo the secret input.
pub fn read_secret_bytes(backend: &str) -> Result<Vec<u8>, String> {
    if let Some(name) = backend.strip_prefix("env:") {
        return std::env::var(name)
            .map(|value| value.into_bytes())
            .map_err(|_| "approved credential env missing".to_owned());
    }
    if let Some(name) = backend.strip_prefix("env64:") {
        // Secret managers hold printable values; DER keys are binary. The
        // base64 variant lets an operator pass the pkcs8 document through
        // an environment variable without a temp file.
        let value =
            std::env::var(name).map_err(|_| "approved credential env missing".to_owned())?;
        return decode_env64(&value);
    }
    if let Some(path) = backend.strip_prefix("file:") {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(path)
            .map_err(|_| "approved credential file unavailable".to_owned())?;
        if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.len() > 4096 {
            return Err(
                "credential file must be regular, non-symlink, <=4096 bytes and 0600".into(),
            );
        }
        return std::fs::read(path).map_err(|_| "approved credential file unreadable".to_owned());
    }
    Err("credential source requires env:NAME, env64:BASE64 or file:PATH".into())
}

fn decode_env64(value: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(value.trim().as_bytes())
        .map_err(|_| "approved credential env is not valid base64".to_owned())
}

/// Text-secret flavor: UTF-8, trimmed, 1..=4096 bytes. This is the flavor
/// for bearer tokens and shared credentials; binary material (DER keys)
/// goes through [`read_secret_bytes`].
pub fn read_secret_string(backend: &str) -> Result<String, String> {
    let bytes = read_secret_bytes(backend)?;
    let secret = String::from_utf8(bytes)
        .map_err(|_| "credential must be UTF-8 text".to_owned())?
        .trim()
        .to_owned();
    if secret.is_empty() || secret.len() > 4096 {
        return Err("invalid credential size".into());
    }
    Ok(secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_shape_accepts_known_schemes_and_rejects_the_rest() {
        assert!(is_valid_backend("env:CONEX_TOKEN"));
        assert!(is_valid_backend("env64:AAECAw=="));
        assert!(is_valid_backend("file:/secrets/token"));
        assert!(!is_valid_backend("plain-token"));
        assert!(!is_valid_backend("env:"));
        assert!(!is_valid_backend("file:   "));
        // Binary material may use env64; text credentials may not.
        assert!(is_valid_text_backend("env:CONEX_TOKEN"));
        assert!(is_valid_text_backend("file:/secrets/token"));
        assert!(!is_valid_text_backend("env64:AAECAw=="));
        assert!(!is_valid_text_backend("plain-token"));
    }

    #[test]
    fn env64_decodes_printable_base64_into_binary() {
        assert_eq!(decode_env64("AAECAwQF").unwrap(), vec![0u8, 1, 2, 3, 4, 5]);
        assert_eq!(
            decode_env64("  AAECAwQF  ").unwrap(),
            vec![0u8, 1, 2, 3, 4, 5]
        );
        assert!(decode_env64("not-base64!!!").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn file_flavor_enforces_regular_0600_rejects_symlinks_and_trims() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        std::fs::write(&path, b"  token-value  ").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert_eq!(
            read_secret_string(&format!("file:{}", path.display())).unwrap(),
            "token-value"
        );

        // Group/world-readable files are refused.
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert!(read_secret_string(&format!("file:{}", path.display())).is_err());

        // A symlink is refused even when its target is 0600.
        let target = dir.path().join("real");
        std::fs::write(&target, b"token-value").unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read_secret_string(&format!("file:{}", link.display())).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn string_flavor_bounds_the_trimmed_length() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        std::fs::write(&path, vec![b'x'; 5000]).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(read_secret_string(&format!("file:{}", path.display())).is_err());
    }
}

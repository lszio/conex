//! Local account passwords (private entry route A): Argon2id hashing and the
//! login throttle. The hash is the only credential material ever stored — a
//! PHC string in the private store; plaintext passwords exist only inside one
//! request body. Route B (OIDC) identities skip this module entirely; the
//! accounts table records their kind so the two never mix.
//!
//! The throttle is deliberately in-memory: it must never outlive the process
//! that knows the accounts (a restart clears a stale backoff, and a persistent
//! one would let an attacker keep a victim locked across restarts).
#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::sync::Mutex;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use rand_core::OsRng;

use conex_proto::now_ms;

/// `argon2` crate defaults: Argon2id v19, m=19 MiB, t=2, p=1 (OWASP 2024
/// baseline). Salt and output length are the crate defaults too; the PHC
/// string self-describes every parameter, so tightening one later only
/// affects newly written hashes.
pub fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| format!("password hash: {error}"))
}

/// Verify against a stored PHC string. A malformed stored hash fails closed
/// (the account simply cannot log in until re-provisioned).
pub fn verify_password(password: &str, stored: &str) -> bool {
    PasswordHash::new(stored).ok().is_some_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

pub const MIN_PASSWORD_BYTES: usize = 8;
pub const MAX_PASSWORD_BYTES: usize = 1024;
pub const MIN_USERNAME_BYTES: usize = 1;
pub const MAX_USERNAME_BYTES: usize = 64;

/// Usernames are opaque login handles. The alphabet keeps them safe to embed
/// in logs, URLs and the UI without escaping; the authorization principal
/// stays the configured private principal, not the username.
pub fn valid_username(username: &str) -> bool {
    !username.is_empty()
        && username.len() <= MAX_USERNAME_BYTES
        && username
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

pub fn valid_password(password: &str) -> bool {
    password.len() >= MIN_PASSWORD_BYTES && password.len() <= MAX_PASSWORD_BYTES
}

/// Per-username exponential backoff on failed password logins, plus a global
/// per-minute attempt cap so cycling usernames does not bypass the per-key
/// throttle. Success clears the username's backoff. Single-admin reality
/// means one hot username; the global cap covers the rest.
pub struct LoginThrottle {
    inner: Mutex<ThrottleState>,
}

struct ThrottleState {
    failures: HashMap<String, Failure>,
    minute_window: Vec<u64>,
}

struct Failure {
    next_allowed_ms: u64,
    strikes: u32,
}

const BASE_BACKOFF_MS: u64 = 500;
const MAX_BACKOFF_MS: u64 = 15_000;
const MAX_ATTEMPTS_PER_MINUTE: usize = 60;
const FAILURE_CAPACITY: usize = 4096;

impl LoginThrottle {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ThrottleState {
                failures: HashMap::new(),
                minute_window: Vec::new(),
            }),
        }
    }

    /// Whether a login attempt for `username` may proceed right now. Accepted
    /// attempts are counted against the global minute cap.
    pub fn admit(&self, username: &str) -> bool {
        let now = now_ms();
        let mut state = self.inner.lock().expect("login throttle poisoned");
        state
            .minute_window
            .retain(|at| now.saturating_sub(*at) < 60_000);
        if state.minute_window.len() >= MAX_ATTEMPTS_PER_MINUTE {
            return false;
        }
        if let Some(failure) = state.failures.get(username)
            && now < failure.next_allowed_ms
        {
            return false;
        }
        state.minute_window.push(now);
        true
    }

    /// Record a failed attempt: the backoff for `username` doubles.
    pub fn punish(&self, username: &str) {
        let now = now_ms();
        let mut state = self.inner.lock().expect("login throttle poisoned");
        if state.failures.len() >= FAILURE_CAPACITY {
            state.failures.clear();
        }
        let failure = state
            .failures
            .entry(username.to_owned())
            .or_insert(Failure {
                next_allowed_ms: now,
                strikes: 0,
            });
        failure.strikes = failure.strikes.saturating_add(1);
        let backoff = BASE_BACKOFF_MS
            .saturating_mul(1 << failure.strikes.saturating_sub(1).min(6))
            .min(MAX_BACKOFF_MS);
        failure.next_allowed_ms = now + backoff;
    }

    /// Record a successful login: the username's backoff resets.
    pub fn absolve(&self, username: &str) {
        self.inner
            .lock()
            .expect("login throttle poisoned")
            .failures
            .remove(username);
    }
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_roundtrips_and_rejects_wrong() {
        let hash = hash_password("correct horse battery").expect("hash");
        assert!(hash.starts_with("$argon2id$"), "PHC string: {hash}");
        assert!(verify_password("correct horse battery", &hash));
        assert!(!verify_password("correct horse batterie", &hash));
        assert!(!verify_password(
            "correct horse battery",
            "$argon2id$garbage"
        ));
    }

    #[test]
    fn hashes_are_salted_per_call() {
        let first = hash_password("same input").expect("hash");
        let second = hash_password("same input").expect("hash");
        assert_ne!(first, second, "each hash carries its own salt");
    }

    #[test]
    fn username_and_password_bounds() {
        assert!(valid_username("owner"));
        assert!(valid_username("alice.cooper-2_x"));
        assert!(!valid_username(""));
        assert!(!valid_username("with space"));
        assert!(!valid_username("unicode-ü"));
        assert!(!valid_username(&"a".repeat(65)));
        assert!(!valid_password("short"));
        assert!(valid_password(&"p".repeat(8)));
        assert!(!valid_password(&"p".repeat(1025)));
    }

    #[test]
    fn throttle_backs_off_then_forgives() {
        let throttle = LoginThrottle::new();
        assert!(throttle.admit("owner"), "first attempt always admitted");
        throttle.punish("owner");
        assert!(!throttle.admit("owner"), "immediate retry is refused");
        throttle.absolve("owner");
        assert!(throttle.admit("owner"), "success clears the backoff");
    }

    #[test]
    fn throttle_backoff_grows_and_caps() {
        let throttle = LoginThrottle::new();
        for _ in 0..12 {
            throttle.punish("owner");
        }
        {
            let state = throttle.inner.lock().expect("state");
            let failure = state.failures.get("owner").expect("failure entry");
            let backoff = failure.next_allowed_ms.saturating_sub(now_ms());
            assert!(
                backoff <= MAX_BACKOFF_MS + 50,
                "backoff caps at {MAX_BACKOFF_MS}, got {backoff}"
            );
            assert_eq!(failure.strikes, 12);
        }
    }

    #[test]
    fn throttle_is_per_username_with_global_cap() {
        let throttle = LoginThrottle::new();
        throttle.punish("owner");
        assert!(throttle.admit("other"), "another username is unaffected");
        // "other" already took one slot of the minute window.
        for _ in 0..(MAX_ATTEMPTS_PER_MINUTE - 1) {
            assert!(throttle.admit(&format!("user-{}", rand_suffix())));
        }
        assert!(
            !throttle.admit("fresh-one"),
            "the global minute cap refuses further attempts"
        );
    }

    fn rand_suffix() -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        COUNTER.fetch_add(1, Ordering::Relaxed).to_string()
    }
}

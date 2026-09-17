//! OIDC id_token verification (design §15.1 notez borrow: issuer pinning +
//! JWKS cache + RSA PKCS#1 SHA-256 only).
//!
//! `jsonwebtoken` is deliberately not used: RS256 verification runs on
//! `ring` (already a workspace dependency) against JWKS keys supplied by
//! the operator (static JSON or a fetched jwks_uri). The verifier rejects
//! any other `alg`, mismatched `iss`/`aud`, expired/not-yet-valid `exp`/
//! `nbf`, missing `nonce`, and unknown/absent `kid`.
#![forbid(unsafe_code)]

use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaKeyPair, RsaPublicKeyComponents};
use serde::Deserialize;
use thiserror::Error;

/// Error code the HTTP layer maps to HTTP status.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum JwtError {
    #[error("malformed token: {0}")]
    Malformed(String),
    #[error("unsupported algorithm {0} (only RS256)")]
    UnsupportedAlgorithm(String),
    #[error("unknown key id")]
    UnknownKeyId,
    #[error("token expired")]
    Expired,
    #[error("token not yet valid")]
    NotYetValid,
    #[error("issuer mismatch: expected {expected}, got {got}")]
    IssuerMismatch { expected: String, got: String },
    #[error("audience mismatch: token audience {got} does not include {expected}")]
    AudienceMismatch { expected: String, got: String },
    #[error("missing nonce")]
    MissingNonce,
    #[error("signature verification failed")]
    BadSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdTokenClaims {
    pub subject: String,
    pub issuer: String,
    pub audiences: Vec<String>,
    pub nonce: Option<String>,
}

#[derive(Debug, Clone)]
struct Jwk {
    kid: String,
    n: Vec<u8>,
    e: Vec<u8>,
}

#[derive(Debug, Clone, Default)]
pub struct Jwks {
    keys: Vec<Jwk>,
}

impl Jwks {
    /// Parse a JWKS document (the `{"keys": [...]}` JSON).
    pub fn parse(json: &str) -> Result<Self, JwtError> {
        #[derive(Deserialize)]
        struct Doc {
            keys: Vec<RawKey>,
        }
        #[derive(Deserialize)]
        struct RawKey {
            #[serde(default)]
            kid: Option<String>,
            #[serde(default)]
            kty: Option<String>,
            n: String,
            e: String,
        }
        let decoded: Doc = serde_json::from_str(json)
            .map_err(|e| JwtError::Malformed(format!("invalid JWKS JSON: {e}")))?;
        let keys = decoded
            .keys
            .into_iter()
            .filter(|k| k.kty.as_deref().unwrap_or("RSA") == "RSA")
            .map(|k| {
                let n = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(&k.n)
                    .map_err(|_| JwtError::Malformed("JWKS n is not base64url".into()))?;
                let e = base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(&k.e)
                    .map_err(|_| JwtError::Malformed("JWKS e is not base64url".into()))?;
                let n = strip_leading_zeros(&n);
                let e = strip_leading_zeros(&e);
                Ok(Jwk {
                    kid: k.kid.unwrap_or_else(|| "<unnamed>".into()),
                    n,
                    e,
                })
            })
            .collect::<Result<Vec<_>, JwtError>>()?;
        if keys.is_empty() {
            return Err(JwtError::Malformed(
                "JWKS contains no RSA keys".to_string(),
            ));
        }
        Ok(Self { keys })
    }

    fn key(&self, kid: &str) -> Option<&Jwk> {
        self.keys.iter().find(|k| k.kid == kid)
    }
}

/// A loaded, immutable verification context.
pub struct OidcVerifier {
    /// Expected `iss` claim; must match exactly (issuer pinning).
    issuer: String,
    /// Expected `aud` claim; the token must include one of these.
    audiences: Vec<String>,
    /// Expected `nonce`; the token must carry it (default: required when
    /// configured via `expected_nonce`).
    expected_nonce: Option<String>,
    /// Key-use window: `exp` must be ≤ `now + skew`; `nbf` ≥ `now - skew`.
    clock_skew_secs: u64,
    jwks: Jwks,
}

impl OidcVerifier {
    pub fn new(
        issuer: impl Into<String>,
        audiences: Vec<String>,
        expected_nonce: Option<String>,
        jwks: Jwks,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            audiences,
            expected_nonce,
            clock_skew_secs: 60,
            jwks,
        }
    }

    /// Verify an id_token JWT and extract the validated claims.
    pub fn verify(&self, token: &str) -> Result<IdTokenClaims, JwtError> {
        // JWT = base64url(header).base64url(payload).base64url(signature)
        let mut parts = token.splitn(3, '.');
        let header = parts.next().ok_or_else(|| JwtError::Malformed("no header".into()))?;
        let payload = parts.next().ok_or_else(|| JwtError::Malformed("no payload".into()))?;
        let signature = parts.next().ok_or_else(|| JwtError::Malformed("no signature".into()))?;

        let header_bytes = b64u_decode(header)?;
        let payload_bytes = b64u_decode(payload)?;
        let signature_bytes = b64u_decode(signature)?;
        if signature_bytes.is_empty() {
            return Err(JwtError::Malformed("empty signature".into()));
        }
        let signed_input = format!("{header}.{payload}");

        #[derive(Deserialize)]
        struct Header {
            alg: String,
            #[serde(default)]
            kid: Option<String>,
        }
        let header: Header = serde_json::from_slice(&header_bytes)
            .map_err(|e| JwtError::Malformed(format!("bad header: {e}")))?;
        if header.alg != "RS256" {
            return Err(JwtError::UnsupportedAlgorithm(header.alg));
        }
        let kid = header.kid.ok_or(JwtError::UnknownKeyId)?;

        #[derive(Deserialize)]
        struct Claims {
            sub: String,
            iss: String,
            #[serde(default)]
            aud: serde_json::Value,
            #[serde(default)]
            nbf: Option<i64>,
            #[serde(default)]
            exp: i64,
            #[serde(default)]
            nonce: Option<String>,
        }
        let claims: Claims = serde_json::from_slice(&payload_bytes)
            .map_err(|e| JwtError::Malformed(format!("bad claims: {e}")))?;

        if claims.iss != self.issuer {
            return Err(JwtError::IssuerMismatch {
                expected: self.issuer.clone(),
                got: claims.iss,
            });
        }
        let audiences = parse_aud(&claims.aud);
        if !self.audiences.iter().any(|a| audiences.contains(a)) {
            return Err(JwtError::AudienceMismatch {
                expected: self.audiences.join(","),
                got: audiences.join(","),
            });
        }
        if let Some(expected_nonce) = &self.expected_nonce
            && claims.nonce.as_deref() != Some(expected_nonce.as_str())
        {
            return Err(JwtError::MissingNonce);
        }
        let now = now_unix();
        if claims.exp != 0 && (claims.exp as u64) < now.saturating_sub(self.clock_skew_secs) {
            return Err(JwtError::Expired);
        }
        if let Some(nbf) = claims.nbf
            && (nbf as u64) > now.saturating_add(self.clock_skew_secs)
        {
            return Err(JwtError::NotYetValid);
        }

        // Verify RS256 via ring against the JWKS key for `kid`.
        let jwk = self.jwks.key(&kid).ok_or(JwtError::UnknownKeyId)?;
        let public_key = RsaPublicKeyComponents { n: &jwk.n, e: &jwk.e };
        public_key
            .verify(
                &RSA_PKCS1_2048_8192_SHA256,
                signed_input.as_bytes(),
                &signature_bytes,
            )
            .map_err(|_| JwtError::BadSignature)?;

        Ok(IdTokenClaims {
            subject: claims.sub,
            issuer: claims.iss,
            audiences,
            nonce: claims.nonce,
        })
    }
}

fn strip_leading_zeros(bytes: &[u8]) -> Vec<u8> {
    let start = bytes.iter().position(|b| *b != 0).unwrap_or(0);
    bytes[start..].to_vec()
}

fn parse_aud(aud: &serde_json::Value) -> Vec<String> {
    match aud {
        serde_json::Value::String(s) => vec![s.clone()],
        serde_json::Value::Array(items) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        serde_json::Value::Null => vec![],
        _ => vec![aud.to_string()],
    }
}

fn b64u_decode(input: &str) -> Result<Vec<u8>, JwtError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(input.trim_end_matches('='))
        .map_err(|e| JwtError::Malformed(format!("not base64url: {e}")))
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Test signing helpers (id_token generation) — used by tests only.
// ---------------------------------------------------------------------------

/// Sign an RS256 JWT with a PKCS#8 private key (test/IdP fixture).
#[allow(deprecated)] // RSA_PKCS1_SHA256 is the standardized PKCS#1 v1.5 padding
pub fn sign_rs256(pkcs8_der: &[u8], header_kid: &str, alg: &str, payload: &str, rng: &ring::rand::SystemRandom) -> Result<String, String> {
    let key_pair = RsaKeyPair::from_pkcs8(pkcs8_der).map_err(|e| format!("bad pkcs8: {e}"))?;
    let header = format!(r#"{{"typ":"JWT","alg":"{alg}","kid":"{header_kid}"}}"#);
    let b64_header = b64u_encode(header.as_bytes());
    let b64_payload = b64u_encode(payload.as_bytes());
    let signed_input = format!("{b64_header}.{b64_payload}");
    let mut signature = vec![0u8; key_pair.public_modulus_len()];
    key_pair
        .sign(
            &ring::signature::RSA_PKCS1_SHA256,
            rng,
            signed_input.as_bytes(),
            &mut signature,
        )
        .map_err(|e| format!("sign: {e}"))?;
    Ok(format!("{signed_input}.{}", b64u_encode(&signature)))
}

pub fn b64u_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Convenience for tests: build a claims JSON payload.
pub fn claims_json(
    sub: &str,
    iss: &str,
    aud: &str,
    exp_offset_secs: i64,
    nonce: Option<&str>,
) -> String {
    let now = now_unix() as i64;
    let mut payload = format!(
        r#"{{"sub":"{sub}","iss":"{iss}","aud":"{aud}","exp":{exp},"nbf":{nbf}"#,
        exp = now + exp_offset_secs,
        nbf = now - 60,
    );
    if let Some(nonce) = nonce {
        payload.push_str(&format!(r#","nonce":"{nonce}""#));
    }
    payload.push('}');
    payload
}

pub const TEST_KID: &str = "UlERqg4IKXazWWTQCkUD7A";

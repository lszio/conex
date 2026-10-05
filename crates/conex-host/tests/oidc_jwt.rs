//! RS256 id_token verification (OIDC real-signature path).
//!
//! Signs tokens with a fixture RSA-2048 key (ring `RsaKeyPair`), verifies
//! with the `OidcVerifier` built from the matching JWKS, and asserts the
//! deviation cases: wrong issuer / wrong audience / expired / not-yet-valid
//! / HS256 / unknown kid / tampered signature / wrong nonce all rejected.

mod support;

use base64::Engine as _;
use conex_host::oidc_jwt::{Jwks, JwtError, OidcVerifier, b64u_encode, claims_json, now_unix};
use support::{TEST_JWKS_JSON, TEST_KID, sign_id_token};

fn fixture_jwks() -> Jwks {
    Jwks::parse(TEST_JWKS_JSON).expect("fixture jwks")
}

fn verifier() -> OidcVerifier {
    OidcVerifier::new(
        "https://issuer.example",
        vec!["conex-host".to_string()],
        Some("nohunter2".to_string()),
        fixture_jwks(),
    )
}

fn sign(payload: &str, kid: &str, alg: &str) -> String {
    sign_id_token(kid, payload, alg)
}

#[test]
fn valid_rs256_token_passes() {
    let verifier = verifier();
    let token = sign(
        &claims_json(
            "alice",
            "https://issuer.example",
            "conex-host",
            3600,
            Some("nohunter2"),
        ),
        TEST_KID,
        "RS256",
    );
    let claims = verifier.verify(&token).expect("verify ok");
    assert_eq!(claims.subject, "alice");
    assert_eq!(claims.issuer, "https://issuer.example");
    assert_eq!(claims.audiences, vec!["conex-host".to_string()]);
}

#[test]
fn wrong_issuer_rejected() {
    let verifier = verifier();
    let token = sign(
        &claims_json(
            "alice",
            "https://evil.example",
            "conex-host",
            3600,
            Some("nohunter2"),
        ),
        TEST_KID,
        "RS256",
    );
    assert_eq!(
        verifier.verify(&token),
        Err(JwtError::IssuerMismatch {
            expected: "https://issuer.example".into(),
            got: "https://evil.example".into(),
        })
    );
}

#[test]
fn wrong_audience_rejected() {
    let verifier = verifier();
    let token = sign(
        &claims_json(
            "alice",
            "https://issuer.example",
            "other-client",
            3600,
            Some("nohunter2"),
        ),
        TEST_KID,
        "RS256",
    );
    assert!(matches!(
        verifier.verify(&token),
        Err(JwtError::AudienceMismatch { .. })
    ));
}

#[test]
fn expired_token_rejected() {
    let verifier = verifier();
    let token = sign(
        &claims_json(
            "alice",
            "https://issuer.example",
            "conex-host",
            -7200,
            Some("nohunter2"),
        ),
        TEST_KID,
        "RS256",
    );
    assert_eq!(verifier.verify(&token), Err(JwtError::Expired));
}

#[test]
fn not_yet_valid_token_rejected() {
    let verifier = verifier();
    let now = now_unix() as i64;
    let payload = format!(
        r#"{{"sub":"alice","iss":"https://issuer.example","aud":"conex-host","exp":{exp},"nbf":{nbf},"nonce":"nohunter2"}}"#,
        exp = now + 3600,
        nbf = now + 7200,
    );
    let token = sign(&payload, TEST_KID, "RS256");
    assert_eq!(verifier.verify(&token), Err(JwtError::NotYetValid));
}

#[test]
fn non_rs256_rejected() {
    let verifier = verifier();
    let token = sign(
        &claims_json(
            "alice",
            "https://issuer.example",
            "conex-host",
            3600,
            Some("nohunter2"),
        ),
        TEST_KID,
        "HS256",
    );
    assert!(matches!(
        verifier.verify(&token),
        Err(JwtError::UnsupportedAlgorithm(alg)) if alg == "HS256"
    ));
}

#[test]
fn unknown_kid_rejected() {
    let verifier = verifier();
    let token = sign(
        &claims_json(
            "alice",
            "https://issuer.example",
            "conex-host",
            3600,
            Some("nohunter2"),
        ),
        "nope",
        "RS256",
    );
    assert_eq!(verifier.verify(&token), Err(JwtError::UnknownKeyId));
}

#[test]
fn tampered_signature_rejected() {
    let verifier = verifier();
    // Flip one byte inside the decoded signature so base64url stays valid.
    let mut token = sign(
        &claims_json(
            "alice",
            "https://issuer.example",
            "conex-host",
            3600,
            Some("nohunter2"),
        ),
        TEST_KID,
        "RS256",
    );
    let parts: Vec<&str> = token.split('.').collect();
    let mut sig = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[2])
        .expect("decode sig");
    sig[0] ^= 0x40;
    let tampered_sig = b64u_encode(&sig);
    token = format!("{}.{}.{}", parts[0], parts[1], tampered_sig);
    assert_eq!(verifier.verify(&token), Err(JwtError::BadSignature));
}

#[test]
fn wrong_nonce_rejected() {
    let verifier = verifier();
    let token = sign(
        &claims_json(
            "alice",
            "https://issuer.example",
            "conex-host",
            3600,
            Some("wrong"),
        ),
        TEST_KID,
        "RS256",
    );
    assert_eq!(verifier.verify(&token), Err(JwtError::MissingNonce));
}

#[test]
fn jwks_parse_rejects_empty() {
    assert!(Jwks::parse(r#"{"keys":[]}"#).is_err());
    assert!(Jwks::parse(r#"not json"#).is_err());
}

#[test]
fn b64u_round_trip() {
    let bytes = b"hello conex";
    let enc = b64u_encode(bytes);
    let dec = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(&enc)
        .expect("decode");
    assert_eq!(dec, bytes);
}

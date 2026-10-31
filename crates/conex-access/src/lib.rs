//! Private-access grants (docs/contracts/private-runtime.md §3.2-§3.3):
//! compact JWS JWTs signed with Ed25519. Only the hub holds the private key;
//! nodes install approved issuer public keys and can never mint a grant for
//! another node.
//!
//! Verification is strict end to end: canonical base64url, duplicate-key and
//! unknown-claim rejection, exact issuer/audience/tenant, integer JWT times
//! (this is JWT, not the Conex uint64-as-string wire rule), purpose-bound
//! lifetime ceilings, and `nbf <= now + 60` skew with `now < exp` as a hard
//! cutoff. Verification never consumes the `jti`: a grant stays usable for
//! every authorized operation until it expires or is revoked.
//!
//! `nodeEpoch`/`moduleGeneration` are token-internal claims surfaced on
//! [`VerifiedGrant`]; the node authorizer compares them against the current
//! boot epoch / module generation on every operation (contract §3.3), since
//! both can change while a grant is alive.

#![forbid(unsafe_code)]
use std::collections::BTreeMap;
use std::time::Duration;

use base64::Engine as _;
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use serde::Deserialize;

pub use conex_proto::{AccessGrantResponse, AccessGrantScope};

pub const MAX_TOKEN_BYTES: usize = 16384;
/// Maximum accepted clock skew for `nbf` (contract §3.3: at most 60s).
pub const MAX_CLOCK_SKEW_SECS: u64 = 60;
pub const PURPOSE_DIRECT: &str = "direct";
pub const PURPOSE_RELAY: &str = "relay";
/// Grant ceilings per purpose; issue refuses more, verify refuses longer.
pub const DIRECT_TTL: Duration = Duration::from_secs(3600);
pub const RELAY_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone, Copy)]
pub struct GrantPolicy {
    pub direct_ttl: Duration,
    pub relay_ttl: Duration,
}

impl Default for GrantPolicy {
    fn default() -> Self {
        Self {
            direct_ttl: DIRECT_TTL,
            relay_ttl: RELAY_TTL,
        }
    }
}

impl GrantPolicy {
    pub fn ceiling(&self, purpose: &str) -> Option<Duration> {
        match purpose {
            PURPOSE_DIRECT => Some(self.direct_ttl),
            PURPOSE_RELAY => Some(self.relay_ttl),
            _ => None,
        }
    }
}

/// Identity resolved by hub authentication (cookie+CSRF or dedicated bearer).
/// Never deserialized from a request body.
#[derive(Debug, Clone)]
pub struct VerifiedPrincipal {
    pub principal: String,
    pub tenant: String,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum GrantError {
    #[error("grant rejected: {0}")]
    Rejected(&'static str),
    #[error("grant internal: {0}")]
    Internal(String),
}

/// Signs grants. The pkcs8 document comes from an approved credential source
/// (`env:NAME`/`file:PATH`); the file must be regular, 0600 and small.
pub struct GrantIssuer {
    kid: String,
    issuer: String,
    policy: GrantPolicy,
    keypair: Ed25519KeyPair,
}

impl GrantIssuer {
    pub fn load(
        kid: &str,
        issuer: &str,
        pkcs8_secret_ref: &str,
        policy: GrantPolicy,
    ) -> Result<Self, GrantError> {
        if kid.is_empty() || issuer.is_empty() {
            return Err(GrantError::Internal("kid and issuer are required".into()));
        }
        let secret = read_secret(pkcs8_secret_ref)?;
        let keypair = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&secret)
            .map_err(|_| GrantError::Internal("signing key is not valid pkcs8".into()))?;
        Ok(Self {
            kid: kid.to_owned(),
            issuer: issuer.to_owned(),
            policy,
            keypair,
        })
    }

    /// `now_ms` is the hub clock in milliseconds; JWT times are whole seconds.
    /// An illegal ttl is an error, never a silent truncation: the caller asked
    /// for a lifetime the policy does not allow and must learn that.
    #[allow(clippy::too_many_arguments)]
    pub fn issue(
        &self,
        principal: &VerifiedPrincipal,
        node_id: &str,
        node_epoch: &str,
        scope: &AccessGrantScope,
        now_ms: u64,
        ttl: Duration,
    ) -> Result<AccessGrantResponse, GrantError> {
        if ttl.is_zero() {
            return Err(GrantError::Rejected("ttl must be positive"));
        }
        let ceiling = self
            .policy
            .ceiling(&scope.purpose)
            .ok_or(GrantError::Rejected("unknown grant purpose"))?;
        if ttl > ceiling {
            return Err(GrantError::Rejected("ttl exceeds the purpose ceiling"));
        }
        if scope.space_id.is_empty()
            || scope.provider_endpoint_id.is_empty()
            || scope.contract_id.is_empty()
            || scope.module_generation == 0
            || scope.methods.is_empty()
        {
            return Err(GrantError::Rejected("grant scope is incomplete"));
        }
        if principal.principal.is_empty() || principal.tenant.is_empty() {
            return Err(GrantError::Internal(
                "authenticated principal is empty".into(),
            ));
        }
        let now = now_ms / 1000;
        let exp = now.saturating_add(ttl.as_secs());
        let iat = json_int(now)?;
        let nbf = iat.clone();
        let exp = json_int(exp)?;
        let scope_value = serde_json::to_value(scope)
            .map_err(|error| GrantError::Internal(format!("scope encode: {error}")))?;
        let claims = Claims {
            iss: self.issuer.clone(),
            aud: node_id.to_owned(),
            sub: principal.principal.clone(),
            tenant: principal.tenant.clone(),
            node_epoch: node_epoch.to_owned(),
            iat,
            nbf,
            exp,
            jti: new_jti(),
            scope: scope_value,
        };
        let header = format!(
            r#"{{"alg":"EdDSA","typ":"JWT","kid":{}}}"#,
            json_string(&self.kid)
        );
        let payload = serde_json::to_string(&claims)
            .map_err(|error| GrantError::Internal(format!("claims encode: {error}")))?;
        let header64 = URL_SAFE.encode(header.as_bytes());
        let payload64 = URL_SAFE.encode(payload.as_bytes());
        let signing_input = format!("{header64}.{payload64}");
        let signature = self.keypair.sign(signing_input.as_bytes());
        Ok(AccessGrantResponse {
            token: format!("{signing_input}.{}", URL_SAFE.encode(signature.as_ref())),
            jti: claims.jti,
            node_id: node_id.to_owned(),
            node_epoch: node_epoch.to_owned(),
            expires_at_ms: exp_seconds_to_ms(&claims.exp),
        })
    }

    /// The public key nodes must install for this issuer/kid.
    pub fn public_key(&self) -> &[u8] {
        self.keypair.public_key().as_ref()
    }

    pub fn kid(&self) -> &str {
        &self.kid
    }
}

fn exp_seconds_to_ms(exp: &JsonInt) -> u64 {
    let secs: i64 = exp.0;
    u64::try_from(secs).unwrap_or(0).saturating_mul(1000)
}

#[derive(serde::Serialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: String,
    tenant: String,
    #[serde(rename = "nodeEpoch")]
    node_epoch: String,
    iat: JsonInt,
    nbf: JsonInt,
    exp: JsonInt,
    jti: String,
    scope: serde_json::Value,
}

/// A JSON integer kept distinct from floats: JWT times must serialize and
/// parse as whole integers, and stay inside the safe integer range.
#[derive(Debug, Clone, PartialEq)]
struct JsonInt(i64);

impl serde::Serialize for JsonInt {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.0)
    }
}

impl<'de> Deserialize<'de> for JsonInt {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Number::deserialize(deserializer)?;
        if value.as_f64().map(|f| f.fract() != 0.0).unwrap_or(true) {
            return Err(serde::de::Error::custom("not an integer"));
        }
        let raw = value.to_string();
        let parsed: i64 = raw
            .parse()
            .map_err(|_| serde::de::Error::custom("integer out of range"))?;
        // Safe integer range for interop with browser JSON consumers.
        if parsed.unsigned_abs() > ((1i64 << 53) - 1) as u64 {
            return Err(serde::de::Error::custom("integer outside the safe range"));
        }
        Ok(JsonInt(parsed))
    }
}

fn json_int(secs: u64) -> Result<JsonInt, GrantError> {
    if secs > ((1i64 << 53) - 1).try_into().unwrap_or(u64::MAX) {
        return Err(GrantError::Rejected("time outside the safe integer range"));
    }
    Ok(JsonInt(secs as i64))
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).expect("string encode")
}

fn new_jti() -> String {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .expect("system random source unavailable");
    URL_SAFE.encode(bytes)
}

/// Approved credential source, identical rules to the node's secret reader:
/// `env:NAME` or a regular 0600 file of at most 4096 bytes.
fn read_secret(backend: &str) -> Result<Vec<u8>, GrantError> {
    if let Some(name) = backend.strip_prefix("env:") {
        return std::env::var(name)
            .map(|value| value.into_bytes())
            .map_err(|_| GrantError::Internal("approved credential env missing".into()));
    }
    if let Some(path) = backend.strip_prefix("file:") {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata(path)
            .map_err(|_| GrantError::Internal("approved credential file unavailable".into()))?;
        if !metadata.is_file() || metadata.mode() & 0o077 != 0 || metadata.len() > 4096 {
            return Err(GrantError::Internal(
                "credential file must be regular, <=4096 bytes and 0600".into(),
            ));
        }
        return std::fs::read(path)
            .map_err(|_| GrantError::Internal("approved credential file unreadable".into()));
    }
    Err(GrantError::Internal(
        "credential source requires env:NAME or file:PATH".into(),
    ))
}

/// The result of strict verification. `scope` is the decoded grant scope;
/// `node_epoch`/`scope.module_generation` are claims, not truths — the node
/// rechecks both against runtime state on every operation.
#[derive(Debug, Clone)]
pub struct VerifiedGrant {
    pub issuer: String,
    pub audience: String,
    pub principal: String,
    pub tenant: String,
    pub node_epoch: String,
    pub scope: AccessGrantScope,
    pub iat: u64,
    pub nbf: u64,
    pub exp: u64,
    pub jti: String,
    pub kid: String,
}

impl VerifiedGrant {
    /// Milliseconds until this grant stops authorizing new operations.
    pub fn remaining_ms(&self, now_ms: u64) -> u64 {
        let exp_ms = self.exp.saturating_mul(1000);
        exp_ms.saturating_sub(now_ms)
    }
}

/// Verifies grants against one hub issuer. Public keys are installed by the
/// administrator; nothing here fetches keys from the network.
#[derive(Clone)]
pub struct GrantVerifier {
    issuer: String,
    tenant: String,
    node_id: String,
    keys: BTreeMap<String, Vec<u8>>,
}

impl GrantVerifier {
    /// `keys` maps an installed kid to the raw 32-byte Ed25519 public key.
    pub fn new(
        issuer: &str,
        tenant: &str,
        keys: impl IntoIterator<Item = (String, Vec<u8>)>,
        node_id: &str,
    ) -> Result<Self, GrantError> {
        if issuer.is_empty() || tenant.is_empty() || node_id.is_empty() {
            return Err(GrantError::Internal(
                "issuer, tenant and node_id are required".into(),
            ));
        }
        let mut installed = BTreeMap::new();
        for (kid, key) in keys {
            if kid.is_empty() {
                return Err(GrantError::Internal("kid must not be empty".into()));
            }
            if key.len() != 32 {
                return Err(GrantError::Internal(format!(
                    "public key for {kid} must be 32 bytes"
                )));
            }
            if installed.insert(kid, key).is_some() {
                return Err(GrantError::Internal("duplicate installed kid".into()));
            }
        }
        if installed.is_empty() {
            return Err(GrantError::Internal(
                "at least one approved public key is required".into(),
            ));
        }
        Ok(Self {
            issuer: issuer.to_owned(),
            tenant: tenant.to_owned(),
            node_id: node_id.to_owned(),
            keys: installed,
        })
    }

    pub fn verify(&self, token: &str, now_ms: u64) -> Result<VerifiedGrant, GrantError> {
        if token.len() > MAX_TOKEN_BYTES {
            return Err(GrantError::Rejected("token exceeds 16384 bytes"));
        }
        let mut segments = token.split('.');
        let (Some(header64), Some(payload64), Some(signature64), None) = (
            segments.next(),
            segments.next(),
            segments.next(),
            segments.next(),
        ) else {
            return Err(GrantError::Rejected(
                "token must have exactly three segments",
            ));
        };
        let header_bytes = canonical_segment(header64)?;
        let payload_bytes = canonical_segment(payload64)?;
        let signature = canonical_segment(signature64)?;

        let header = parse_flat_object(&header_bytes, &["alg", "typ", "kid"])?;
        let alg = string_field(&header, "alg")?;
        let typ = string_field(&header, "typ")?;
        let kid = string_field(&header, "kid")?;
        if alg != "EdDSA" || typ != "JWT" {
            return Err(GrantError::Rejected("alg and typ must be EdDSA/JWT"));
        }
        let Some(key) = self.keys.get(kid.as_str()) else {
            return Err(GrantError::Rejected("kid is not installed"));
        };
        let signing_input_length = header64.len() + 1 + payload64.len();
        // `token` is validated UTF-8 by canonical_segment inputs below, so the
        // ASCII prefix split at the segment boundaries is the signing input.
        let signing_input = &token.as_bytes()[..signing_input_length];
        UnparsedPublicKey::new(&ED25519, key)
            .verify(signing_input, &signature)
            .map_err(|_| GrantError::Rejected("signature verification failed"))?;

        let claims = parse_flat_object(&payload_bytes, CLAIM_KEYS)?;
        if string_field(&claims, "iss")? != self.issuer {
            return Err(GrantError::Rejected("issuer mismatch"));
        }
        if string_field(&claims, "aud")? != self.node_id {
            return Err(GrantError::Rejected("audience mismatch"));
        }
        if string_field(&claims, "tenant")? != self.tenant {
            return Err(GrantError::Rejected("tenant mismatch"));
        }
        let principal = string_field(&claims, "sub")?;
        let node_epoch = string_field(&claims, "nodeEpoch")?;
        let jti = string_field(&claims, "jti")?;
        if principal.is_empty() || node_epoch.is_empty() || jti.is_empty() {
            return Err(GrantError::Rejected(
                "principal, nodeEpoch and jti are required",
            ));
        }
        let iat = int_field(&claims, "iat")?;
        let nbf = int_field(&claims, "nbf")?;
        let exp = int_field(&claims, "exp")?;
        if iat.0 < 0 || !(iat.0 <= nbf.0 && nbf.0 <= exp.0) {
            return Err(GrantError::Rejected("iat must not exceed nbf and exp"));
        }
        let scope_value = claims
            .get("scope")
            .ok_or(GrantError::Rejected("scope claim is required"))?;
        let scope_json = to_serde_value(scope_value);
        let scope: AccessGrantScope = serde_json::from_value(scope_json)
            .map_err(|_| GrantError::Rejected("scope claim is not a valid AccessGrantScope"))?;
        let ceiling = GrantPolicy::default()
            .ceiling(&scope.purpose)
            .ok_or(GrantError::Rejected("unknown grant purpose"))?;
        let lifetime = (exp.0 - iat.0) as u64;
        if lifetime > ceiling.as_secs() {
            return Err(GrantError::Rejected(
                "grant lifetime exceeds the purpose ceiling",
            ));
        }
        let now =
            i64::try_from(now_ms / 1000).map_err(|_| GrantError::Rejected("now out of range"))?;
        if nbf.0 > now + MAX_CLOCK_SKEW_SECS as i64 {
            return Err(GrantError::Rejected("nbf is too far in the future"));
        }
        if now >= exp.0 {
            return Err(GrantError::Rejected("grant expired"));
        }
        Ok(VerifiedGrant {
            issuer: self.issuer.clone(),
            audience: self.node_id.clone(),
            principal,
            tenant: self.tenant.clone(),
            node_epoch,
            scope,
            iat: iat.0 as u64,
            nbf: nbf.0 as u64,
            exp: exp.0 as u64,
            jti,
            kid,
        })
    }
}

fn canonical_segment(segment: &str) -> Result<Vec<u8>, GrantError> {
    let decoded = URL_SAFE
        .decode(segment.as_bytes())
        .map_err(|_| GrantError::Rejected("segment is not canonical base64url"))?;
    if URL_SAFE.encode(&decoded) != segment {
        return Err(GrantError::Rejected("segment is not canonical base64url"));
    }
    Ok(decoded)
}

const CLAIM_KEYS: &[&str] = &[
    "iss",
    "aud",
    "sub",
    "tenant",
    "nodeEpoch",
    "scope",
    "iat",
    "nbf",
    "exp",
    "jti",
];

/// A strict JSON value: integers only (no floats, exponents or +/-), strings,
/// arrays and objects, with duplicate keys rejected at every depth.
#[derive(Debug, Clone, PartialEq)]
enum StrictValue {
    Str(String),
    Int(i64),
    Arr(Vec<StrictValue>),
    Obj(Vec<(String, StrictValue)>),
}

impl StrictValue {
    fn as_str(&self) -> Option<&str> {
        match self {
            StrictValue::Str(value) => Some(value),
            _ => None,
        }
    }
}

type StrictObject = BTreeMap<String, StrictValue>;

fn parse_flat_object(bytes: &[u8], keys: &[&str]) -> Result<StrictObject, GrantError> {
    let value = parse_strict(bytes)?;
    let StrictValue::Obj(entries) = value else {
        return Err(GrantError::Rejected("claims must be a JSON object"));
    };
    let mut object = StrictObject::new();
    for (key, value) in entries {
        if !keys.contains(&key.as_str()) {
            return Err(GrantError::Rejected("unknown claim key"));
        }
        if object.insert(key, value).is_some() {
            return Err(GrantError::Rejected("duplicate claim key"));
        }
    }
    Ok(object)
}

fn string_field(object: &StrictObject, key: &str) -> Result<String, GrantError> {
    object
        .get(key)
        .and_then(StrictValue::as_str)
        .map(str::to_owned)
        .ok_or(GrantError::Rejected("claim must be a string"))
}

fn int_field(object: &StrictObject, key: &str) -> Result<JsonInt, GrantError> {
    match object.get(key) {
        Some(StrictValue::Int(value)) => Ok(JsonInt(*value)),
        _ => Err(GrantError::Rejected("claim must be an integer")),
    }
}

/// The strict parser already bounded depth, duplicates and integers; this
/// only restates the accepted shapes as a `serde_json::Value` for the
/// generated `AccessGrantScope` decoder.
fn to_serde_value(value: &StrictValue) -> serde_json::Value {
    match value {
        StrictValue::Str(text) => serde_json::Value::String(text.clone()),
        StrictValue::Int(raw) => serde_json::Number::from_i128(i128::from(*raw))
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        StrictValue::Arr(items) => {
            serde_json::Value::Array(items.iter().map(to_serde_value).collect())
        }
        StrictValue::Obj(entries) => serde_json::Value::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), to_serde_value(value)))
                .collect(),
        ),
    }
}

/// Minimal strict JSON scanner for bounded JWT parts. It accepts exactly the
/// grammar the verifier needs and rejects anything else (floats, exponents,
/// duplicate keys at any depth, control characters, trailing data).
fn parse_strict(bytes: &[u8]) -> Result<StrictValue, GrantError> {
    let mut parser = StrictParser { bytes, position: 0 };
    let value = parser.value(0)?;
    parser.skip_ws();
    if parser.position != parser.bytes.len() {
        return Err(GrantError::Rejected("trailing bytes after JSON value"));
    }
    Ok(value)
}

struct StrictParser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> StrictParser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), GrantError> {
        if self.bump() == Some(byte) {
            Ok(())
        } else {
            Err(GrantError::Rejected("malformed JSON"))
        }
    }

    fn value(&mut self, depth: u8) -> Result<StrictValue, GrantError> {
        if depth > 8 {
            return Err(GrantError::Rejected("JSON nesting too deep"));
        }
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(StrictValue::Str(self.string()?)),
            Some(b'-' | b'0'..=b'9') => self.integer(),
            _ => Err(GrantError::Rejected("malformed JSON value")),
        }
    }

    fn object(&mut self, depth: u8) -> Result<StrictValue, GrantError> {
        self.expect(b'{')?;
        let mut entries: Vec<(String, StrictValue)> = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Ok(StrictValue::Obj(entries));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value(depth + 1)?;
            if entries.iter().any(|(existing, _)| existing == &key) {
                return Err(GrantError::Rejected("duplicate JSON key"));
            }
            entries.push((key, value));
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b'}') => return Ok(StrictValue::Obj(entries)),
                _ => return Err(GrantError::Rejected("malformed JSON object")),
            }
        }
    }

    fn array(&mut self, depth: u8) -> Result<StrictValue, GrantError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.position += 1;
            return Ok(StrictValue::Arr(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_ws();
            match self.bump() {
                Some(b',') => continue,
                Some(b']') => return Ok(StrictValue::Arr(items)),
                _ => return Err(GrantError::Rejected("malformed JSON array")),
            }
        }
    }

    fn string(&mut self) -> Result<String, GrantError> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.bump() {
                Some(b'"') => return Ok(out),
                Some(b'\\') => match self.bump() {
                    Some(b'"') => out.push('"'),
                    Some(b'\\') => out.push('\\'),
                    Some(b'/') => out.push('/'),
                    Some(b'b') => out.push('\u{0008}'),
                    Some(b'f') => out.push('\u{000C}'),
                    Some(b'n') => out.push('\n'),
                    Some(b'r') => out.push('\r'),
                    Some(b't') => out.push('\t'),
                    Some(b'u') => {
                        let mut code = 0u32;
                        for _ in 0..4 {
                            let byte = self
                                .bump()
                                .ok_or(GrantError::Rejected("truncated escape"))?;
                            let digit = (byte as char)
                                .to_digit(16)
                                .ok_or(GrantError::Rejected("bad unicode escape"))?;
                            code = code * 16 + digit;
                        }
                        out.push(
                            char::from_u32(code)
                                .ok_or(GrantError::Rejected("bad unicode escape"))?,
                        );
                    }
                    _ => return Err(GrantError::Rejected("invalid escape")),
                },
                Some(byte) if byte < 0x20 => {
                    return Err(GrantError::Rejected("raw control character in string"));
                }
                Some(byte) => {
                    // Multi-byte UTF-8 sequences pass through whole; the input
                    // came from canonical_segment which does not validate
                    // UTF-8, so re-check here.
                    if byte < 0x80 {
                        out.push(byte as char);
                    } else {
                        let start = self.position - 1;
                        let width = utf8_width(byte);
                        let end = start + width;
                        let chunk = self
                            .bytes
                            .get(start..end)
                            .ok_or(GrantError::Rejected("truncated UTF-8"))?;
                        let text = std::str::from_utf8(chunk)
                            .map_err(|_| GrantError::Rejected("invalid UTF-8 in string"))?;
                        out.push_str(text);
                        self.position = end;
                    }
                }
                None => return Err(GrantError::Rejected("unterminated string")),
            }
        }
    }

    fn integer(&mut self) -> Result<StrictValue, GrantError> {
        let start = self.position;
        if self.peek() == Some(b'-') {
            self.position += 1;
        }
        let digits_start = self.position;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
        let digits = self.position - digits_start;
        if digits == 0 {
            return Err(GrantError::Rejected("malformed number"));
        }
        if digits > 1 && self.bytes[digits_start] == b'0' {
            return Err(GrantError::Rejected("leading zero in number"));
        }
        if matches!(self.peek(), Some(b'.' | b'e' | b'E' | b'+')) {
            return Err(GrantError::Rejected("only integers are accepted"));
        }
        let text = std::str::from_utf8(&self.bytes[start..self.position])
            .map_err(|_| GrantError::Rejected("invalid number"))?;
        text.parse::<i64>()
            .map(StrictValue::Int)
            .map_err(|_| GrantError::Rejected("integer out of range"))
    }
}

fn utf8_width(byte: u8) -> usize {
    match byte {
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

const URL_SAFE: base64::engine::GeneralPurpose = base64::engine::general_purpose::URL_SAFE_NO_PAD;

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use serde_json::json;

    fn scope(purpose: &str) -> AccessGrantScope {
        AccessGrantScope {
            space_id: "space-1".into(),
            provider_endpoint_id: "endpoint-1".into(),
            contract_id: "conex.test/1".into(),
            module_generation: 3,
            methods: vec!["counter/read".into()],
            states: vec![],
            workspace_ids: vec![],
            conversation_ids: vec![],
            purpose: purpose.into(),
        }
    }

    fn temp_secret(dir: &std::path::Path, name: &str, bytes: &[u8]) -> String {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        format!("file:{}", path.display())
    }

    fn issuer(dir: &std::path::Path, kid: &str) -> GrantIssuer {
        let secret = temp_secret(dir, &format!("{kid}.pkcs8"), &test_pkcs8(kid));
        GrantIssuer::load(kid, "conex://hub.test", &secret, GrantPolicy::default()).unwrap()
    }

    /// Deterministic seeds (RFC 8032 style test material); the keypair and
    /// pkcs8 envelope are derived at startup, never loaded from a file.
    fn test_seed(kid: &str) -> [u8; 32] {
        match kid {
            "kid-1" => [1u8; 32],
            "kid-2" => [2u8; 32],
            _ => [3u8; 32],
        }
    }

    fn test_public(kid: &str) -> Vec<u8> {
        Ed25519KeyPair::from_seed_unchecked(&test_seed(kid))
            .unwrap()
            .public_key()
            .as_ref()
            .to_vec()
    }

    /// PKCS#8 v2 DER envelope for an Ed25519 key, matching ring's own
    /// `generate_pkcs8` layout: version, algorithm OID 1.3.101.112, the
    /// 32-byte seed, and the public key as `[1] IMPLICIT BIT STRING`.
    fn test_pkcs8(kid: &str) -> Vec<u8> {
        let seed = test_seed(kid);
        let mut out = vec![
            0x30, 0x51, 0x02, 0x01, 0x01, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        out.extend_from_slice(&seed);
        out.extend_from_slice(&[0x81, 0x21, 0x00]);
        out.extend_from_slice(&test_public(kid));
        out
    }

    fn verifier_with(_dir: &std::path::Path, kid: &str) -> GrantVerifier {
        GrantVerifier::new(
            "conex://hub.test",
            "tenant-1",
            [(kid.to_owned(), test_public(kid))],
            "node-1",
        )
        .unwrap()
    }

    fn issue_now(issuer: &GrantIssuer, purpose: &str, ttl: Duration) -> AccessGrantResponse {
        let principal = VerifiedPrincipal {
            principal: "user-1".into(),
            tenant: "tenant-1".into(),
        };
        issuer
            .issue(
                &principal,
                "node-1",
                "epoch-1",
                &scope(purpose),
                1_700_000_000_000,
                ttl,
            )
            .unwrap()
    }

    #[test]
    fn issue_then_verify_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let signer = issuer(dir.path(), "kid-1");
        let verifier = verifier_with(dir.path(), "kid-1");
        let grant = issue_now(&signer, PURPOSE_RELAY, Duration::from_secs(60));
        let verified = verifier.verify(&grant.token, 1_700_000_001_000).unwrap();
        assert_eq!(verified.principal, "user-1");
        assert_eq!(verified.tenant, "tenant-1");
        assert_eq!(verified.node_epoch, "epoch-1");
        assert_eq!(verified.scope.purpose, PURPOSE_RELAY);
        assert_eq!(verified.kid, "kid-1");
        assert_eq!(verified.exp, 1_700_000_060);
    }

    #[test]
    fn wrong_signing_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let signer = issuer(dir.path(), "kid-1");
        // Same kid label, different key: the installed trust anchor is wrong.
        let verifier_of_other = GrantVerifier::new(
            "conex://hub.test",
            "tenant-1",
            [("kid-1".to_owned(), test_public("kid-2"))],
            "node-1",
        )
        .unwrap();
        let grant = issue_now(&signer, PURPOSE_RELAY, Duration::from_secs(60));
        assert!(matches!(
            verifier_of_other.verify(&grant.token, 1_700_000_001_000),
            Err(GrantError::Rejected("signature verification failed"))
        ));
        // A second instance of the same key still verifies (determinism).
        let again = issuer(dir.path(), "kid-1");
        let token = issue_now(&again, PURPOSE_RELAY, Duration::from_secs(60));
        assert!(
            verifier_with(dir.path(), "kid-1")
                .verify(&token.token, 1_700_000_001_000)
                .is_ok()
        );
    }

    #[test]
    fn rfc8032_vector_verifies_with_the_raw_primitive() {
        // RFC 8032 §7.1 TEST 2 (independent of ring's own test suite).
        let public =
            hex::decode("3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c")
                .unwrap();
        let message = hex::decode("72").unwrap();
        let signature = hex::decode(
            "92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da\
             085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00",
        )
        .unwrap();
        UnparsedPublicKey::new(&ED25519, &public)
            .verify(&message, &signature)
            .expect("RFC 8032 vector must verify");
    }

    #[test]
    fn issue_rejects_illegal_ttl_instead_of_truncating() {
        let dir = tempfile::tempdir().unwrap();
        let signer = issuer(dir.path(), "kid-1");
        for bad in [
            Duration::ZERO,
            Duration::from_secs(301),
            Duration::from_secs(3601),
        ] {
            let purpose = if bad.as_secs() > 300 {
                PURPOSE_RELAY
            } else {
                PURPOSE_DIRECT
            };
            let error = match signer.issue(
                &VerifiedPrincipal {
                    principal: "u".into(),
                    tenant: "t".into(),
                },
                "node-1",
                "epoch-1",
                &scope(purpose),
                0,
                bad,
            ) {
                Err(error) => error,
                Ok(_) => panic!("ttl {bad:?} must be rejected"),
            };
            assert!(error.to_string().contains("ttl"), "unexpected: {error}");
        }
        assert!(
            signer
                .issue(
                    &VerifiedPrincipal {
                        principal: "u".into(),
                        tenant: "t".into()
                    },
                    "node-1",
                    "epoch-1",
                    &scope("broadcast"),
                    0,
                    Duration::from_secs(10),
                )
                .is_err()
        );
    }

    #[test]
    fn verify_rejects_tampered_and_malformed_tokens() {
        let dir = tempfile::tempdir().unwrap();
        let signer = issuer(dir.path(), "kid-1");
        let verifier = verifier_with(dir.path(), "kid-1");
        let grant = issue_now(&signer, PURPOSE_DIRECT, Duration::from_secs(600));
        let mut parts: Vec<String> = grant.token.split('.').map(str::to_owned).collect();

        // Flip one payload byte.
        let mut payload = B64.decode(&parts[1]).unwrap();
        payload[0] ^= 0x01;
        parts[1] = B64.encode(&payload);
        assert!(
            verifier
                .verify(&parts.join("."), 1_700_000_001_000)
                .is_err()
        );

        // Flip one signature byte.
        let parts: Vec<String> = grant.token.split('.').map(str::to_owned).collect();
        let mut signature = B64.decode(&parts[2]).unwrap();
        signature[0] ^= 0x01;
        let tampered = format!("{}.{}.{}", parts[0], parts[1], B64.encode(&signature));
        assert!(verifier.verify(&tampered, 1_700_000_001_000).is_err());

        // Extra segment / too short / oversized.
        let extra_segment = format!("{token}.x", token = grant.token);
        assert!(verifier.verify(&extra_segment, 0).is_err());
        assert!(verifier.verify("a.b", 0).is_err());
        let big = "a".repeat(MAX_TOKEN_BYTES + 1);
        assert!(verifier.verify(&big, 0).is_err());

        // Non-canonical base64url (padding and standard alphabet).
        let padded = format!("{}={}", grant.token.split('.').next().unwrap(), "");
        assert!(verifier.verify(&padded, 0).is_err());
        let plus = grant.token.replacen('A', "+", 1);
        assert!(verifier.verify(&plus, 0).is_err());
    }

    #[test]
    fn verify_rejects_header_and_claim_violations() {
        let dir = tempfile::tempdir().unwrap();
        let signer = issuer(dir.path(), "kid-1");
        let verifier = verifier_with(dir.path(), "kid-1");
        let grant = issue_now(&signer, PURPOSE_RELAY, Duration::from_secs(60));
        let parts: Vec<&str> = grant.token.split('.').collect();
        let payload = URL_SAFE.decode(parts[1]).unwrap();
        let claims = std::str::from_utf8(&payload).unwrap().to_owned();

        // Unknown claim / duplicate claim (signature no longer matches, but
        // strict parsing must reject before that matters).
        let injected = claims.replace("\"jti\":", "\"extra\":1,\"jti\":");
        assert_rejected_payload(&verifier, parts[0], &injected, parts[2]);
        let duplicated = claims.replace("\"iss\":", "\"iss\":\"conex://hub.test\",\"iss\":");
        assert_rejected_payload(&verifier, parts[0], &duplicated, parts[2]);
        // Missing required claim.
        let missing = claims.replace("\"jti\":", "\"\":");
        assert_rejected_payload(&verifier, parts[0], &missing, parts[2]);

        // alg=none and HS256 must fail even with a syntactically valid body.
        for alg in ["none", "HS256"] {
            let header = json_header(alg, "kid-1");
            let header64 = URL_SAFE.encode(header.as_bytes());
            let token = format!("{}.{}.{}", header64, parts[1], parts[2]);
            assert!(
                verifier.verify(&token, 1_700_000_001_000).is_err(),
                "alg={alg}"
            );
        }
    }

    fn json_header(alg: &str, kid: &str) -> String {
        serde_json::to_string(&serde_json::json!({"alg": alg, "typ": "JWT", "kid": kid})).unwrap()
    }

    fn forged_claims(overrides: serde_json::Value) -> String {
        let mut claims = serde_json::json!({
            "iss": "conex://hub.test",
            "aud": "node-1",
            "sub": "u",
            "tenant": "tenant-1",
            "nodeEpoch": "e",
            "scope": {
                "spaceId": "s", "providerEndpointId": "p", "contractId": "c",
                "moduleGeneration": "3", "methods": ["counter/read"], "purpose": "relay",
            },
            "iat": 1_700_000_000i64,
            "nbf": 1_700_000_000i64,
            "exp": 1_700_000_060i64,
            "jti": "j",
        });
        let serde_json::Value::Object(fields) = overrides else {
            panic!("overrides must be an object");
        };
        for (key, value) in fields {
            if value.is_null() {
                claims.as_object_mut().unwrap().remove(&key);
            } else {
                claims[key] = value;
            }
        }
        serde_json::to_string(&claims).unwrap()
    }

    /// An honest signature over attacker-chosen claims: the strict claim
    /// checks must hold even when the signature verifies.
    fn forged_token(dir: &std::path::Path, claims_json: &str, kid: &str) -> String {
        let _ = dir;
        let pair = Ed25519KeyPair::from_seed_unchecked(&test_seed(kid)).unwrap();
        let header64 = URL_SAFE.encode(json_header("EdDSA", kid).as_bytes());
        let payload64 = URL_SAFE.encode(claims_json.as_bytes());
        let signature = pair.sign(format!("{header64}.{payload64}").as_bytes());
        format!(
            "{header64}.{payload64}.{}",
            URL_SAFE.encode(signature.as_ref())
        )
    }

    fn assert_rejected_payload(
        verifier: &GrantVerifier,
        _header64: &str,
        payload_json: &str,
        _signature64: &str,
    ) {
        let token = forged_token(std::path::Path::new("."), payload_json, "kid-1");
        let error = verifier.verify(&token, 1_700_000_001_000).unwrap_err();
        assert!(
            matches!(error, GrantError::Rejected(_)),
            "expected strict rejection, got {error}"
        );
    }

    #[test]
    fn verify_rejects_time_and_identity_violations() {
        let dir = tempfile::tempdir().unwrap();
        let signer = issuer(dir.path(), "kid-1");
        let verifier = verifier_with(dir.path(), "kid-1");
        let principal = VerifiedPrincipal {
            principal: "u".into(),
            tenant: "tenant-1".into(),
        };

        // Expired: hard cutoff, no grace.
        let grant = signer
            .issue(
                &principal,
                "node-1",
                "e",
                &scope(PURPOSE_RELAY),
                1_700_000_000_000,
                Duration::from_secs(60),
            )
            .unwrap();
        assert_eq!(
            verifier
                .verify(&grant.token, 1_700_000_061_000)
                .unwrap_err(),
            GrantError::Rejected("grant expired")
        );
        // Boundary: exp is exclusive, the millisecond before passes.
        assert!(verifier.verify(&grant.token, 1_700_000_059_999).is_ok());

        // nbf more than 60s in the future.
        let future = signer
            .issue(
                &principal,
                "node-1",
                "e",
                &scope(PURPOSE_RELAY),
                1_700_000_000_000,
                Duration::from_secs(60),
            )
            .unwrap();
        assert_eq!(
            verifier
                .verify(&future.token, 1_690_000_000_000)
                .unwrap_err(),
            GrantError::Rejected("nbf is too far in the future")
        );

        // Wrong issuer / audience / tenant over an honestly signed body.
        assert!(
            verifier
                .verify(
                    &forged_token(
                        dir.path(),
                        &forged_claims(json!({"iss": "conex://other"})),
                        "kid-1"
                    ),
                    1_700_000_000_000
                )
                .is_err()
        );
        assert!(
            verifier
                .verify(
                    &forged_token(
                        dir.path(),
                        &forged_claims(json!({"aud": "node-9"})),
                        "kid-1"
                    ),
                    1_700_000_000_000
                )
                .is_err()
        );
        assert!(
            verifier
                .verify(
                    &forged_token(
                        dir.path(),
                        &forged_claims(json!({"tenant": "tenant-9"})),
                        "kid-1"
                    ),
                    1_700_000_000_000
                )
                .is_err()
        );
        // Missing claims entirely.
        assert!(
            verifier
                .verify(
                    &forged_token(dir.path(), &forged_claims(json!({"jti": null})), "kid-1"),
                    1_700_000_000_000
                )
                .is_err()
        );
        // iat after nbf.
        assert!(
            verifier
                .verify(
                    &forged_token(
                        dir.path(),
                        &forged_claims(json!({"iat": 1_700_000_050i64})),
                        "kid-1"
                    ),
                    1_700_000_000_000
                )
                .is_err()
        );
        // Unknown kid (unsigned: kid check precedes signature work).
        assert!(
            verifier
                .verify(
                    &forged_token(dir.path(), &forged_claims(json!({})), "kid-9"),
                    1_700_000_000_000
                )
                .is_err()
        );
    }

    #[test]
    fn verify_rejects_overlong_and_unknown_purpose_lifetimes() {
        let dir = tempfile::tempdir().unwrap();
        let verifier = verifier_with(dir.path(), "kid-1");
        // A relay grant that lives longer than the ceiling: issue refuses, so
        // forge the body to prove verify also refuses (defense in depth).
        let long = forged_token(
            dir.path(),
            &forged_claims(json!({"exp": 1_700_003_601i64})),
            "kid-1",
        );
        assert_eq!(
            verifier.verify(&long, 1_700_000_000_000).unwrap_err(),
            GrantError::Rejected("grant lifetime exceeds the purpose ceiling")
        );

        let unknown = forged_token(
            dir.path(),
            &forged_claims(
                json!({"scope": {"spaceId": "s", "providerEndpointId": "p", "contractId": "c", "moduleGeneration": "3", "methods": ["m"], "purpose": "broadcast"}}),
            ),
            "kid-1",
        );
        assert_eq!(
            verifier.verify(&unknown, 1_700_000_000_000).unwrap_err(),
            GrantError::Rejected("unknown grant purpose")
        );
    }
}

//! Inbound static bearer auth. Tokens are stored as SHA-256 digests only.
use conex_core::{CallError, CallResult, Caller};
use conex_proto::v1;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct InboundCaller {
    pub caller: Caller,
    pub audience: String,
}

#[derive(Debug, Clone)]
pub struct TokenRecord {
    pub token_hash: [u8; 32],
    pub principal_id: String,
    pub tenant_id: String,
    pub actor_peer_id: String,
    pub audience: String,
}

impl TokenRecord {
    pub fn from_plaintext(
        token: &str,
        principal_id: impl Into<String>,
        tenant_id: impl Into<String>,
        audience: impl Into<String>,
    ) -> Self {
        Self {
            token_hash: Sha256::digest(token.as_bytes()).into(),
            principal_id: principal_id.into(),
            tenant_id: tenant_id.into(),
            actor_peer_id: "inbound-http".into(),
            audience: audience.into(),
        }
    }
}

pub trait InboundAuth: Send + Sync {
    fn authenticate(&self, authorization: Option<&str>) -> CallResult<InboundCaller>;
}

pub struct StaticBearerAuth {
    tokens: Vec<TokenRecord>,
}

impl StaticBearerAuth {
    pub fn new(tokens: Vec<TokenRecord>) -> Self {
        Self { tokens }
    }
}

impl InboundAuth for StaticBearerAuth {
    fn authenticate(&self, authorization: Option<&str>) -> CallResult<InboundCaller> {
        let header = authorization.ok_or_else(|| unauthorized("missing Authorization header"))?;
        let token = header
            .strip_prefix("Bearer ")
            .ok_or_else(|| unauthorized("Authorization must use the Bearer scheme"))?;
        let digest: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        for record in &self.tokens {
            if constant_time_eq(&digest, &record.token_hash) {
                return Ok(InboundCaller {
                    caller: Caller {
                        principal_id: record.principal_id.clone(),
                        tenant_id: record.tenant_id.clone(),
                        actor_peer_id: record.actor_peer_id.clone(),
                    },
                    audience: record.audience.clone(),
                });
            }
        }
        Err(unauthorized("invalid bearer token"))
    }
}

fn unauthorized(message: &str) -> CallError {
    CallError::new(v1::ErrorCode::Unauthorized, message)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

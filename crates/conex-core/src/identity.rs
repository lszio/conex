//! Identity mapping. External keys are namespaced by source; sub alone is never a key.
use std::collections::HashMap;

use conex_proto::v1;

use crate::types::{CallError, CallResult, Caller, IdentityKey};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityBinding {
    pub key: IdentityKey,
    pub principal_id: String,
    pub tenant_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct IdentityMap {
    bindings: HashMap<IdentityKey, IdentityBinding>,
}

impl IdentityMap {
    pub fn new(bindings: Vec<IdentityBinding>) -> Self {
        let map = bindings.into_iter().map(|b| (b.key.clone(), b)).collect();
        Self { bindings: map }
    }

    /// Resolve a fully qualified identity key. There is no default principal and
    /// no caching by subject: the whole key must match.
    pub fn resolve(&self, key: &IdentityKey, actor_peer_id: &str) -> CallResult<Caller> {
        let binding = self
            .bindings
            .get(key)
            .ok_or_else(|| CallError::new(v1::ErrorCode::Unauthorized, "unknown identity"))?;
        Ok(Caller {
            principal_id: binding.principal_id.clone(),
            tenant_id: binding.tenant_id.clone(),
            actor_peer_id: actor_peer_id.to_string(),
        })
    }
}

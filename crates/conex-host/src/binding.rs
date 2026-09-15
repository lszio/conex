//! In-memory negotiation bindings. Short-lived, principal-bound, never renewed.
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use conex_core::{CallError, CallResult, Caller, Limits};
use conex_proto::v1;
use sha2::{Digest, Sha256};
use tokio::time::Instant;

pub const DEFAULT_TTL: Duration = Duration::from_secs(60);
pub const MAX_PER_PRINCIPAL: usize = 64;
pub const MAX_TOTAL: usize = 4096;

#[derive(Debug, Clone)]
pub struct NegotiatedBinding {
    pub binding_id: String,
    pub principal_id: String,
    pub tenant_id: String,
    pub audience: String,
    pub profile_id: String,
    pub plane: i32,
    pub provides: Vec<String>,
    pub limits: Limits,
    pub expires_at: Instant,
}

struct Inner {
    records: HashMap<String, NegotiatedBinding>,
    counter: u64,
}

pub struct BindingStore {
    profile_id: String,
    audience: String,
    ttl: Duration,
    max_per_principal: usize,
    max_total: usize,
    capabilities: HashMap<String, Vec<String>>,
    limits: Limits,
    inner: Mutex<Inner>,
}

impl BindingStore {
    pub fn new(
        profile_id: impl Into<String>,
        audience: impl Into<String>,
        capabilities: HashMap<String, Vec<String>>,
        limits: Limits,
    ) -> Self {
        Self {
            profile_id: profile_id.into(),
            audience: audience.into(),
            ttl: DEFAULT_TTL,
            max_per_principal: MAX_PER_PRINCIPAL,
            max_total: MAX_TOTAL,
            capabilities,
            limits,
            inner: Mutex::new(Inner {
                records: HashMap::new(),
                counter: 0,
            }),
        }
    }

    pub fn issue(
        &self,
        caller: &Caller,
        hello: &v1::HelloRequest,
    ) -> CallResult<v1::HelloResponse> {
        if hello.profile_id != self.profile_id {
            return Err(CallError::new(
                v1::ErrorCode::UnsupportedCapability,
                "unknown profile",
            ));
        }
        if hello.plane != v1::Plane::Broker as i32 {
            return Err(CallError::new(
                v1::ErrorCode::PlaneMismatch,
                "P0 only supports the broker plane",
            ));
        }
        let provides = self
            .capabilities
            .get(&caller.tenant_id)
            .cloned()
            .unwrap_or_default();
        let rejected: Vec<&String> = hello
            .requires
            .iter()
            .filter(|required| !provides.contains(required))
            .collect();
        if !rejected.is_empty() {
            return Err(CallError::new(
                v1::ErrorCode::UnsupportedCapability,
                "required capabilities are not available",
            ));
        }

        let mut inner = self.inner.lock().expect("binding lock poisoned");
        purge(&mut inner);
        let per_principal = inner
            .records
            .values()
            .filter(|binding| {
                binding.principal_id == caller.principal_id && binding.tenant_id == caller.tenant_id
            })
            .count();
        if per_principal >= self.max_per_principal {
            return Err(CallError::new(
                v1::ErrorCode::QuotaExceeded,
                "too many bindings for this principal",
            ));
        }
        if inner.records.len() >= self.max_total {
            return Err(CallError::new(
                v1::ErrorCode::QuotaExceeded,
                "too many bindings on this host",
            ));
        }

        inner.counter += 1;
        let binding_id = mint(&caller.principal_id, inner.counter);
        let expires_at = Instant::now() + self.ttl;
        inner.records.insert(
            binding_id.clone(),
            NegotiatedBinding {
                binding_id: binding_id.clone(),
                principal_id: caller.principal_id.clone(),
                tenant_id: caller.tenant_id.clone(),
                audience: self.audience.clone(),
                profile_id: self.profile_id.clone(),
                plane: v1::Plane::Broker as i32,
                provides: provides.clone(),
                limits: self.limits,
                expires_at,
            },
        );
        Ok(v1::HelloResponse {
            binding_id,
            expires_in_ms: self.ttl.as_millis() as u32,
            profile_id: self.profile_id.clone(),
            plane: v1::Plane::Broker as i32,
            provides,
            rejected_capabilities: vec![],
            limits: Some(wire_limits(self.limits)),
        })
    }

    pub fn validate(&self, caller: &Caller, binding_id: &str) -> CallResult<NegotiatedBinding> {
        let mut inner = self.inner.lock().expect("binding lock poisoned");
        purge(&mut inner);
        let binding = inner.records.get(binding_id).cloned().ok_or_else(|| {
            CallError::new(v1::ErrorCode::Unauthorized, "unknown or expired binding")
        })?;
        if binding.principal_id != caller.principal_id || binding.tenant_id != caller.tenant_id {
            return Err(CallError::new(
                v1::ErrorCode::Forbidden,
                "binding belongs to another principal",
            ));
        }
        // No renewal: validate never extends expires_at.
        Ok(binding)
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("binding lock poisoned")
            .records
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn purge(inner: &mut Inner) {
    let now = Instant::now();
    inner.records.retain(|_, binding| binding.expires_at > now);
}

fn mint(principal: &str, counter: u64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(principal.as_bytes());
    hasher.update(counter.to_le_bytes());
    if let Ok(now) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        hasher.update(now.as_nanos().to_le_bytes());
    }
    let digest = hasher.finalize();
    let bytes: &[u8] = digest.as_ref();
    format!("bnd-{}", &hex::encode(bytes)[..32])
}

fn wire_limits(limits: Limits) -> v1::Limits {
    v1::Limits {
        max_frame_bytes: limits.max_frame_bytes,
        max_inflight: limits.max_inflight,
        max_queued_bytes: limits.max_queued_bytes,
        timeout_ms: limits.timeout_ms,
    }
}

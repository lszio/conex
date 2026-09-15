//! Host assembly. The only place that wires ports together (design J1).
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::audit::AuditSink;
use crate::limits::{HostLimits, Limiter, ProviderBudget};
use crate::policy::Policy;
use crate::ports::{Connector, CredentialStore, Resolver};
use crate::registry::Registry;
use crate::target_policy::TargetPolicy;
use crate::types::CallResult;

pub struct Host {
    pub(crate) registry: Registry,
    pub(crate) policy: Arc<dyn Policy>,
    pub(crate) target_policy: Arc<TargetPolicy>,
    pub(crate) resolver: Arc<dyn Resolver>,
    pub(crate) connector: Arc<dyn Connector>,
    pub(crate) credentials: Arc<dyn CredentialStore>,
    pub(crate) audit: Arc<dyn AuditSink>,
    pub(crate) limiter: Limiter,
    pub(crate) providers: Mutex<HashMap<String, Arc<ProviderBudget>>>,
    pub(crate) counter: AtomicU64,
}

impl Host {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        registry: Registry,
        policy: Arc<dyn Policy>,
        target_policy: Arc<TargetPolicy>,
        resolver: Arc<dyn Resolver>,
        connector: Arc<dyn Connector>,
        credentials: Arc<dyn CredentialStore>,
        audit: Arc<dyn AuditSink>,
        limits: HostLimits,
    ) -> CallResult<Host> {
        Ok(Host {
            registry,
            policy,
            target_policy,
            resolver,
            connector,
            credentials,
            audit,
            limiter: Limiter::new(limits),
            providers: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(1),
        })
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub(crate) fn provider_budget(&self, endpoint_id: &str) -> Arc<ProviderBudget> {
        let mut providers = self
            .providers
            .lock()
            .expect("provider budget lock poisoned");
        providers
            .entry(endpoint_id.to_string())
            .or_insert_with(|| Arc::new(self.limiter.provider_budget()))
            .clone()
    }

    pub(crate) fn next_id(&self, prefix: &str) -> String {
        format!("{prefix}-{:x}", self.counter.fetch_add(1, Ordering::SeqCst))
    }
}

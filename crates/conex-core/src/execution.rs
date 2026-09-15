//! The single authorized execution path. Ordering follows design S8.4.
use std::time::Duration;

use conex_proto::v1;
use serde_json::Value;
use tokio::time::Instant;

use crate::aggregate::{AggregateResult, ProviderResult, TargetCall};
use crate::audit::{AuditEnd, AuditStart};
use crate::host::Host;
use crate::registry::{P0_PROTOCOL, P0_VERSION};
use crate::target_policy;
use crate::types::{CallContext, CallError, CallResult, Caller, Endpoint, ExecutionIo};

impl Host {
    pub async fn invoke(
        &self,
        caller: &Caller,
        endpoint_id: &str,
        method: &str,
        input: Value,
        timeout: Duration,
    ) -> CallResult<Value> {
        let started = Instant::now();
        let deadline = started + timeout;

        let endpoint = self
            .registry
            .endpoint(endpoint_id)
            .cloned()
            .ok_or_else(|| {
                CallError::new(
                    v1::ErrorCode::UnknownProvider,
                    format!("unknown provider endpoint {endpoint_id}"),
                )
            })?;
        let route = self
            .registry
            .route(endpoint_id, P0_PROTOCOL, P0_VERSION, method)
            .ok_or_else(|| {
                CallError::new(
                    v1::ErrorCode::UnknownMethod,
                    format!("unknown method {method}"),
                )
            })?;

        let prepared = match (route.contract.prepare)(&input) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.deny(caller, &endpoint, method, "", 0, "prepare", &error);
                return Err(error);
            }
        };

        if endpoint.plane != v1::Plane::Broker {
            let error = CallError::new(
                v1::ErrorCode::PlaneMismatch,
                "P0 only supports the broker plane",
            );
            self.deny(
                caller,
                &endpoint,
                method,
                &prepared.claim.resource_id,
                0,
                "authorize",
                &error,
            );
            return Err(error);
        }
        if caller.tenant_id != endpoint.tenant_id {
            let error = CallError::new(
                v1::ErrorCode::Forbidden,
                "tenant does not own this endpoint",
            );
            self.deny(
                caller,
                &endpoint,
                method,
                &prepared.claim.resource_id,
                0,
                "authorize",
                &error,
            );
            return Err(error);
        }

        let grant = match self.policy.authorize(caller, &endpoint, &prepared.claim) {
            Ok(grant) => grant,
            Err(error) => {
                self.deny(
                    caller,
                    &endpoint,
                    method,
                    &prepared.claim.resource_id,
                    0,
                    "authorize",
                    &error,
                );
                return Err(error);
            }
        };

        // Reserve audit capacity before any side effect.
        let terminal = self.audit.reserve(&self.audit_start(
            caller,
            &endpoint,
            method,
            &prepared.claim.resource_id,
            grant.policy_version,
            "audit_finish",
        ))?;
        self.record(
            caller,
            &endpoint,
            method,
            &prepared.claim.resource_id,
            grant.policy_version,
            "authorize",
            "allow",
            None,
        );

        // Admission: global then per-provider, both bounded by the remaining deadline.
        let bytes = serde_json::to_vec(&input)
            .map(|bytes| bytes.len())
            .unwrap_or(0);
        let _global = self.limiter.acquire_global(deadline).await?;
        let budget = self.provider_budget(endpoint_id);
        let _provider = budget.acquire(bytes, deadline).await?;

        let mut io = ExecutionIo {
            connection: None,
            secret: None,
        };
        if let Some(target) = &route.target {
            let hostname = target_policy::origin_hostname(&target.origin)?;
            let resolved = self.resolver.resolve(&hostname, deadline).await?;
            let allowed = self.target_policy.allow(caller, target, &resolved)?;
            self.record(
                caller,
                &endpoint,
                method,
                &prepared.claim.resource_id,
                grant.policy_version,
                "connect",
                "allow",
                None,
            );
            let connection = self.connector.connect(&allowed, deadline).await?;
            let peer = connection.peer().clone();
            self.record(
                caller,
                &endpoint,
                method,
                &prepared.claim.resource_id,
                grant.policy_version,
                "peer_verified",
                "allow",
                None,
            );

            if self.policy.version() != grant.policy_version {
                let error =
                    CallError::new(v1::ErrorCode::Forbidden, "policy changed during handshake");
                self.deny(
                    caller,
                    &endpoint,
                    method,
                    &prepared.claim.resource_id,
                    grant.policy_version,
                    "authorize",
                    &error,
                );
                return Err(error);
            }
            if let Err(error) = self.policy.authorize(caller, &endpoint, &prepared.claim) {
                self.deny(
                    caller,
                    &endpoint,
                    method,
                    &prepared.claim.resource_id,
                    grant.policy_version,
                    "authorize",
                    &error,
                );
                return Err(error);
            }
            self.record(
                caller,
                &endpoint,
                method,
                &prepared.claim.resource_id,
                grant.policy_version,
                "authorize",
                "allow",
                None,
            );

            if let Some(key) = &route.credential {
                io.secret = Some(self.credentials.resolve(key, &peer).await?);
            }
            self.record(
                caller,
                &endpoint,
                method,
                &prepared.claim.resource_id,
                grant.policy_version,
                "resolve",
                "allow",
                None,
            );
            io.connection = Some(connection);
        }

        let ctx = CallContext {
            caller: caller.clone(),
            endpoint_id: endpoint_id.to_string(),
            plane: endpoint.plane,
            method: method.to_string(),
            claim: prepared.claim.clone(),
            policy_version: grant.policy_version,
            deadline,
        };

        let outcome = tokio::time::timeout_at(
            deadline,
            route.handler.execute(&ctx, prepared.canonical, io),
        )
        .await;
        let mut result = match outcome {
            Ok(result) => result,
            Err(_) => Err(CallError::new(
                v1::ErrorCode::Timeout,
                "handler exceeded the deadline",
            )),
        };
        if let Ok(value) = &result
            && let Err(error) = (route.contract.validate_output)(value)
        {
            result = Err(error);
        }

        match result {
            Ok(value) => {
                self.record(
                    caller,
                    &endpoint,
                    method,
                    &prepared.claim.resource_id,
                    grant.policy_version,
                    "execute",
                    "allow",
                    None,
                );
                let _ = terminal.finish(AuditEnd {
                    phase: "audit_finish".into(),
                    outcome: "allow".into(),
                    error_code: None,
                    execution: "completed".into(),
                    latency_ms: started.elapsed().as_millis() as u64,
                });
                Ok(value)
            }
            Err(error) => {
                self.record(
                    caller,
                    &endpoint,
                    method,
                    &prepared.claim.resource_id,
                    grant.policy_version,
                    "execute",
                    "error",
                    Some(error.code()),
                );
                let _ = terminal.finish(AuditEnd {
                    phase: "audit_finish".into(),
                    outcome: "error".into(),
                    error_code: Some(error.code()),
                    execution: "completed".into(),
                    latency_ms: started.elapsed().as_millis() as u64,
                });
                Err(error)
            }
        }
    }

    pub async fn invoke_many(
        &self,
        caller: &Caller,
        calls: Vec<TargetCall>,
        timeout: Duration,
    ) -> AggregateResult {
        let deadline = Instant::now() + timeout;
        let mut provider_results = Vec::with_capacity(calls.len());
        for call in calls {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let result = self
                .invoke(
                    caller,
                    &call.endpoint_id,
                    &call.method,
                    call.input,
                    remaining,
                )
                .await;
            provider_results.push(ProviderResult {
                endpoint_id: call.endpoint_id,
                result,
            });
        }
        AggregateResult { provider_results }
    }

    fn audit_start(
        &self,
        caller: &Caller,
        endpoint: &Endpoint,
        method: &str,
        resource_scope: &str,
        policy_version: u64,
        phase: &str,
    ) -> AuditStart {
        AuditStart {
            event_id: self.next_id("event"),
            trace_id: self.next_id("trace"),
            principal_id: caller.principal_id.clone(),
            tenant_id: caller.tenant_id.clone(),
            actor_peer_id: caller.actor_peer_id.clone(),
            endpoint_id: endpoint.id.clone(),
            method: method.to_string(),
            resource_scope: resource_scope.to_string(),
            plane: endpoint.plane as i32,
            policy_version,
            phase: phase.to_string(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn record(
        &self,
        caller: &Caller,
        endpoint: &Endpoint,
        method: &str,
        resource_scope: &str,
        policy_version: u64,
        phase: &str,
        outcome: &str,
        error_code: Option<i32>,
    ) {
        let start = self.audit_start(
            caller,
            endpoint,
            method,
            resource_scope,
            policy_version,
            phase,
        );
        let _ = self.finish_phase(start, outcome, error_code);
    }

    #[allow(clippy::too_many_arguments)]
    fn deny(
        &self,
        caller: &Caller,
        endpoint: &Endpoint,
        method: &str,
        resource_scope: &str,
        policy_version: u64,
        phase: &str,
        error: &CallError,
    ) {
        self.record(
            caller,
            endpoint,
            method,
            resource_scope,
            policy_version,
            phase,
            "deny",
            Some(error.code()),
        );
    }

    fn finish_phase(
        &self,
        start: AuditStart,
        outcome: &str,
        error_code: Option<i32>,
    ) -> CallResult<()> {
        let phase = start.phase.clone();
        let reservation = self.audit.reserve(&start)?;
        reservation.finish(AuditEnd {
            phase,
            outcome: outcome.to_string(),
            error_code,
            execution: String::new(),
            latency_ms: 0,
        })
    }
}

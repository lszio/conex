//! Resource authorization. Default deny; every axis must match a rule.
use std::sync::RwLock;

use conex_proto::v1;

use crate::types::{CallError, CallResult, Caller, Endpoint, ResourceClaim};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyRule {
    pub principal_id: String,
    pub tenant_id: String,
    pub endpoint_id: String,
    pub actions: Vec<String>,
    pub root: String,
    pub subtree: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    pub policy_version: u64,
}

pub trait Policy: Send + Sync {
    fn authorize(
        &self,
        caller: &Caller,
        endpoint: &Endpoint,
        claim: &ResourceClaim,
    ) -> CallResult<Grant>;
    fn version(&self) -> u64;
}

#[derive(Debug)]
struct PolicyState {
    version: u64,
    rules: Vec<PolicyRule>,
}

/// Static rule table with atomically replaceable rules and a monotonic version.
pub struct StaticPolicy {
    inner: RwLock<PolicyState>,
}

impl StaticPolicy {
    pub fn new(rules: Vec<PolicyRule>) -> Self {
        Self {
            inner: RwLock::new(PolicyState { version: 1, rules }),
        }
    }

    /// Atomically replace the rule set and return the new version.
    pub fn replace_rules(&self, rules: Vec<PolicyRule>) -> u64 {
        let mut state = self.inner.write().expect("policy lock poisoned");
        state.version += 1;
        state.rules = rules;
        state.version
    }
}

impl Policy for StaticPolicy {
    fn authorize(
        &self,
        caller: &Caller,
        endpoint: &Endpoint,
        claim: &ResourceClaim,
    ) -> CallResult<Grant> {
        let state = self.inner.read().expect("policy lock poisoned");
        let allowed = state.rules.iter().any(|rule| {
            rule.principal_id == caller.principal_id
                && rule.tenant_id == caller.tenant_id
                && rule.endpoint_id == endpoint.id
                && rule.actions.iter().any(|action| action == &claim.action)
                && rule_covers(rule, claim)
        });
        if allowed {
            Ok(Grant {
                policy_version: state.version,
            })
        } else {
            Err(CallError::new(
                v1::ErrorCode::Forbidden,
                "resource is not permitted for this principal and action",
            ))
        }
    }

    fn version(&self) -> u64 {
        self.inner.read().expect("policy lock poisoned").version
    }
}

fn rule_covers(rule: &PolicyRule, claim: &ResourceClaim) -> bool {
    if claim.subtree {
        // A subtree request needs a rule that covers the whole requested subtree.
        rule.subtree && resource_within(&rule.root, &claim.resource_id)
    } else if rule.subtree {
        resource_within(&rule.root, &claim.resource_id)
    } else {
        rule.root == claim.resource_id
    }
}

/// True when the resource is inside the subtree rooted at root (segment-wise).
pub fn resource_within(root: &str, resource: &str) -> bool {
    if !is_valid_resource(root) || !is_valid_resource(resource) {
        return false;
    }
    if root.is_empty() {
        return true;
    }
    resource == root
        || resource
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// ResourceId grammar: UTF-8, slash-separated, no empty/./.. segments, no
/// backslash, NUL or absolute path. Empty string is the root (whole tree).
pub fn is_valid_resource(resource: &str) -> bool {
    if resource.is_empty() {
        return true;
    }
    if resource.starts_with('/') || resource.contains('\\') || resource.contains('\0') {
        return false;
    }
    resource
        .split('/')
        .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

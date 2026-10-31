//! Local operator identity comes only from the restricted Unix socket peer;
//! remote identity comes only from a verified hub grant (P5). No caller,
//! principal or permission is ever deserialized from a request body.
use std::collections::BTreeSet;

use conex_access::VerifiedGrant;
use conex_agent_domain::AuthorizedScope;

#[derive(Clone)]
pub struct TrustedCaller {
    principal: String,
    node_id: String,
}

impl TrustedCaller {
    pub(crate) fn local_operator(principal: String, node_id: String) -> Self {
        Self { principal, node_id }
    }
}

pub struct Target<'a> {
    pub node_id: &'a str,
    pub active: bool,
    pub generation: u64,
    pub expected_generation: u64,
    pub methods: &'a BTreeSet<String>,
}

/// The transport path a request arrived on. A grant's `purpose` must match
/// exactly: a direct grant cannot ride the relay and vice versa (N08).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Path {
    Direct,
    Relay,
}

impl Path {
    pub fn purpose(self) -> &'static str {
        match self {
            Path::Direct => "direct",
            Path::Relay => "relay",
        }
    }
}

/// No caller, principal or permission is deserialized from a request body.
/// P2's operator owns the installed node; P5 adds verified grants here.
pub fn authorize(
    caller: &TrustedCaller,
    target: Target<'_>,
    method: &str,
    resource: Option<&str>,
    _now: u64,
) -> Result<AuthorizedScope, String> {
    if caller.node_id != target.node_id || !target.methods.contains(method) {
        return Err("forbidden".into());
    }
    // P2 installs no Agent backend and accepts no arbitrary resource selectors.
    if resource.is_some() {
        return Err("forbidden".into());
    }
    if !target.active {
        return Err("unavailable".into());
    }
    if target.generation != target.expected_generation {
        return Err("stale_revision".into());
    }
    AuthorizedScope::new(&caller.principal).map_err(|error| error.to_string())
}

/// Per-operation grant authorization (contract §3.3): every call, read and
/// head update rechecks expiry, tenant, audience, epoch, generation, purpose
/// and the exact resource intersection — one successful upgrade never grants
/// the connection a permanent pass. Revocation is checked by the caller
/// against the node store before this runs (it needs async store access).
// Every parameter is a distinct authorization dimension from the frozen
// grant scope; grouping them would hide which axis is missing at a call site.
#[allow(clippy::too_many_arguments)]
pub fn authorize_grant(
    grant: &VerifiedGrant,
    target: Target<'_>,
    tenant: &str,
    node_epoch: &str,
    path: Path,
    method: &str,
    resources: &[(&str, &str)],
    now_ms: u64,
) -> Result<AuthorizedScope, String> {
    let scope = &grant.scope;
    if now_ms / 1000 >= grant.exp {
        return Err("grant_expired".into());
    }
    if grant.tenant != tenant {
        return Err("forbidden".into());
    }
    if grant.audience != target.node_id {
        return Err("forbidden".into());
    }
    if grant.node_epoch != node_epoch {
        return Err("forbidden".into());
    }
    if scope.module_generation != target.generation {
        return Err("forbidden".into());
    }
    if scope.purpose != path.purpose() {
        return Err("forbidden".into());
    }
    // Exact membership on both sides; "the intersection is not empty" never
    // authorizes the whole request.
    if !scope.methods.iter().any(|allowed| allowed == method) || !target.methods.contains(method) {
        return Err("forbidden".into());
    }
    if !target.active {
        return Err("unavailable".into());
    }
    if target.generation != target.expected_generation {
        return Err("stale_revision".into());
    }
    // Every requested resource dimension must be listed in the grant: an
    // empty list authorizes nothing in that dimension.
    let checks: [(&str, &[String]); 2] = [
        ("workspace", &scope.workspace_ids),
        ("conversation", &scope.conversation_ids),
    ];
    for (dimension, listed) in checks {
        let wanted: Vec<&str> = resources
            .iter()
            .filter(|(name, _)| *name == dimension)
            .map(|(_, value)| *value)
            .collect();
        if wanted.is_empty() {
            continue;
        }
        for value in wanted {
            if !listed.iter().any(|allowed| allowed == value) {
                return Err("forbidden".into());
            }
        }
    }
    AuthorizedScope::new(&grant.principal).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_authority_cannot_cross_node_or_generation() {
        let caller = TrustedCaller::local_operator("operator".into(), "node-a".into());
        let methods = BTreeSet::from(["counter/read".into()]);
        let target = |node, generation| Target {
            node_id: node,
            active: true,
            generation: 2,
            expected_generation: generation,
            methods: &methods,
        };
        assert!(authorize(&caller, target("node-a", 2), "counter/read", None, 0).is_ok());
        assert_eq!(
            authorize(&caller, target("node-b", 2), "counter/read", None, 0).unwrap_err(),
            "forbidden"
        );
        assert_eq!(
            authorize(&caller, target("node-a", 1), "counter/read", None, 0).unwrap_err(),
            "stale_revision"
        );
        assert_eq!(
            authorize(&caller, target("node-a", 2), "counter/add", None, 0).unwrap_err(),
            "forbidden"
        );
    }
}

#[cfg(test)]
mod grant_tests {
    use super::*;
    use conex_access::{AccessGrantScope, VerifiedGrant};

    fn grant(method: &str, purpose: &str) -> VerifiedGrant {
        VerifiedGrant {
            issuer: "conex://hub".into(),
            audience: "node-a".into(),
            principal: "user-1".into(),
            tenant: "tenant-1".into(),
            node_epoch: "epoch-1".into(),
            scope: AccessGrantScope {
                space_id: "space".into(),
                provider_endpoint_id: "endpoint".into(),
                contract_id: "conex.test/1".into(),
                module_generation: 2,
                methods: vec![method.into()],
                states: vec![],
                workspace_ids: vec!["ws-1".into()],
                conversation_ids: vec![],
                purpose: purpose.into(),
            },
            iat: 1_700_000_000,
            nbf: 1_700_000_000,
            exp: 1_700_000_600,
            jti: "jti-1".into(),
            kid: "kid-1".into(),
        }
    }

    fn target<'a>(owned: &'a BTreeSet<String>) -> Target<'a> {
        Target {
            node_id: "node-a",
            active: true,
            generation: 2,
            expected_generation: 2,
            methods: owned,
        }
    }

    fn methods(list: &[&str]) -> BTreeSet<String> {
        list.iter().map(|m| m.to_string()).collect()
    }

    #[test]
    fn direct_grant_invokes_within_scope() {
        let owned = methods(&["counter/read"]);
        let mk = || target(&owned);
        let g = grant("counter/read", "direct");
        assert!(
            authorize_grant(
                &g,
                mk(),
                "tenant-1",
                "epoch-1",
                Path::Direct,
                "counter/read",
                &[],
                1_700_000_001_000
            )
            .is_ok()
        );
    }

    #[test]
    fn purpose_and_dimensions_are_exact() {
        let owned = methods(&["counter/read"]);
        let mk = || target(&owned);
        let g = grant("counter/read", "direct");
        // Relay grant on the direct path and the reverse are both forbidden.
        assert_eq!(
            authorize_grant(
                &g,
                mk(),
                "tenant-1",
                "epoch-1",
                Path::Relay,
                "counter/read",
                &[],
                1_700_000_001_000
            )
            .unwrap_err(),
            "forbidden"
        );
        let relay = grant("counter/read", "relay");
        assert!(
            authorize_grant(
                &relay,
                mk(),
                "tenant-1",
                "epoch-1",
                Path::Relay,
                "counter/read",
                &[],
                1_700_000_001_000
            )
            .is_ok()
        );
        // A workspace outside the grant list is forbidden; a listed one passes.
        let with_resource = &[("workspace", "ws-9")];
        assert_eq!(
            authorize_grant(
                &g,
                mk(),
                "tenant-1",
                "epoch-1",
                Path::Direct,
                "counter/read",
                with_resource,
                1_700_000_001_000
            )
            .unwrap_err(),
            "forbidden"
        );
        let listed = &[("workspace", "ws-1")];
        assert!(
            authorize_grant(
                &g,
                mk(),
                "tenant-1",
                "epoch-1",
                Path::Direct,
                "counter/read",
                listed,
                1_700_000_001_000
            )
            .is_ok()
        );
    }

    #[test]
    fn expiry_epoch_generation_and_tenant_gate_every_call() {
        let owned = methods(&["counter/read"]);
        let mk = || target(&owned);
        let g = grant("counter/read", "direct");
        assert_eq!(
            authorize_grant(
                &g,
                mk(),
                "tenant-1",
                "epoch-1",
                Path::Direct,
                "counter/read",
                &[],
                1_700_000_600_000
            )
            .unwrap_err(),
            "grant_expired"
        );
        assert_eq!(
            authorize_grant(
                &g,
                mk(),
                "tenant-9",
                "epoch-1",
                Path::Direct,
                "counter/read",
                &[],
                1_700_000_001_000
            )
            .unwrap_err(),
            "forbidden"
        );
        assert_eq!(
            authorize_grant(
                &g,
                mk(),
                "tenant-1",
                "epoch-2",
                Path::Direct,
                "counter/read",
                &[],
                1_700_000_001_000
            )
            .unwrap_err(),
            "forbidden"
        );
        let stale_gen = Target {
            node_id: "node-a",
            active: true,
            generation: 3,
            expected_generation: 2,
            methods: &owned,
        };
        assert_eq!(
            authorize_grant(
                &g,
                stale_gen,
                "tenant-1",
                "epoch-1",
                Path::Direct,
                "counter/read",
                &[],
                1_700_000_001_000
            )
            .unwrap_err(),
            "forbidden"
        );
    }
}

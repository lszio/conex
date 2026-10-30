//! Local operator identity comes only from the restricted Unix socket peer.
use std::collections::BTreeSet;

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

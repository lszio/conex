//! Authorization: default deny, segment boundaries, subtree claims, versions.
use conex_core::policy::{Policy, PolicyRule, StaticPolicy, resource_within};
use conex_core::{Caller, Endpoint, Limits, ResourceClaim};
use conex_proto::v1;
use serde_json::Value;

fn caller(principal: &str, tenant: &str) -> Caller {
    Caller {
        principal_id: principal.into(),
        tenant_id: tenant.into(),
        actor_peer_id: "peer".into(),
    }
}

fn endpoint(id: &str, tenant: &str) -> Endpoint {
    Endpoint {
        id: id.into(),
        provider_id: id.into(),
        tenant_id: tenant.into(),
        plane: v1::Plane::Broker,
        provides: vec!["source/read".into()],
        limits: Limits::default(),
    }
}

fn claim(resource_id: &str, action: &str, subtree: bool) -> ResourceClaim {
    ResourceClaim {
        resource_id: resource_id.into(),
        action: action.into(),
        subtree,
    }
}

fn rule(
    principal: &str,
    tenant: &str,
    endpoint_id: &str,
    action: &str,
    root: &str,
    subtree: bool,
) -> PolicyRule {
    PolicyRule {
        principal_id: principal.into(),
        tenant_id: tenant.into(),
        endpoint_id: endpoint_id.into(),
        actions: vec![action.into()],
        root: root.into(),
        subtree,
    }
}

#[test]
fn default_deny() {
    let policy = StaticPolicy::new(vec![]);
    assert!(
        policy
            .authorize(
                &caller("p", "t"),
                &endpoint("e", "t"),
                &claim("a.md", "read", false)
            )
            .is_err()
    );
}

#[test]
fn subtree_rule_allows_child_and_denies_sibling_prefix() {
    let policy = StaticPolicy::new(vec![rule("p", "t", "e", "read", "team", true)]);
    assert!(
        policy
            .authorize(
                &caller("p", "t"),
                &endpoint("e", "t"),
                &claim("team/design.md", "read", false)
            )
            .is_ok()
    );
    assert!(
        policy
            .authorize(
                &caller("p", "t"),
                &endpoint("e", "t"),
                &claim("team-private/secret.md", "read", false)
            )
            .is_err()
    );
    assert!(
        policy
            .authorize(
                &caller("p", "t"),
                &endpoint("e", "t"),
                &claim("team/../secret.md", "read", false)
            )
            .is_err()
    );
}

#[test]
fn exact_rule_cannot_authorize_subtree_claim() {
    let policy = StaticPolicy::new(vec![rule("p", "t", "e", "read", "team/a.md", false)]);
    assert!(
        policy
            .authorize(
                &caller("p", "t"),
                &endpoint("e", "t"),
                &claim("team/a.md", "read", false)
            )
            .is_ok()
    );
    // A single-file permission must not authorize a whole-subtree scan.
    assert!(
        policy
            .authorize(
                &caller("p", "t"),
                &endpoint("e", "t"),
                &claim("team", "read", true)
            )
            .is_err()
    );
    // A subtree rule does authorize a subtree claim.
    let subtree = StaticPolicy::new(vec![rule("p", "t", "e", "read", "team", true)]);
    assert!(
        subtree
            .authorize(
                &caller("p", "t"),
                &endpoint("e", "t"),
                &claim("team", "read", true)
            )
            .is_ok()
    );
}

#[test]
fn every_axis_must_match() {
    let policy = StaticPolicy::new(vec![rule("p", "t", "e", "read", "team", true)]);
    assert!(
        policy
            .authorize(
                &caller("other", "t"),
                &endpoint("e", "t"),
                &claim("team/a.md", "read", false)
            )
            .is_err()
    );
    assert!(
        policy
            .authorize(
                &caller("p", "other"),
                &endpoint("e", "t"),
                &claim("team/a.md", "read", false)
            )
            .is_err()
    );
    assert!(
        policy
            .authorize(
                &caller("p", "t"),
                &endpoint("other", "t"),
                &claim("team/a.md", "read", false)
            )
            .is_err()
    );
    assert!(
        policy
            .authorize(
                &caller("p", "t"),
                &endpoint("e", "t"),
                &claim("team/a.md", "write", false)
            )
            .is_err()
    );
}

#[test]
fn replace_rules_bumps_version_monotonically() {
    let policy = StaticPolicy::new(vec![]);
    assert_eq!(policy.version(), 1);
    let granted = policy.authorize(
        &caller("p", "t"),
        &endpoint("e", "t"),
        &claim("a.md", "read", false),
    );
    assert!(granted.is_err());
    let v = policy.replace_rules(vec![rule("p", "t", "e", "read", "", true)]);
    assert_eq!(v, 2);
    assert_eq!(policy.version(), 2);
    let grant = policy
        .authorize(
            &caller("p", "t"),
            &endpoint("e", "t"),
            &claim("a.md", "read", false),
        )
        .unwrap();
    assert_eq!(grant.policy_version, 2);
    assert_eq!(policy.replace_rules(vec![]), 3);
}

#[test]
fn resource_within_shared_vectors() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/vectors/p0/policy.json"
    );
    let text = std::fs::read_to_string(path).expect("read policy vectors");
    let doc: Value = serde_json::from_str(&text).unwrap();
    let cases = doc["cases"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let root = case["root"].as_str().unwrap();
        let resource = case["resource"].as_str().unwrap();
        let within = case["within"].as_bool().unwrap();
        assert_eq!(resource_within(root, resource), within, "vector {name}");
    }
}

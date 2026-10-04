use std::sync::Arc;

use conex_host::agent::{
    AgentAuthorization, AgentRegistration, AuthorizedEndpoint, EndpointRegistration, HostSide,
};

fn registration(
    agent_id: &str,
    tenant_id: &str,
    endpoint_id: &str,
    method: &str,
    root: &str,
) -> AgentRegistration {
    AgentRegistration {
        agent_id: agent_id.into(),
        principal_id: agent_id.into(),
        tenant_id: tenant_id.into(),
        endpoints: vec![EndpointRegistration {
            endpoint_id: endpoint_id.into(),
            root: root.into(),
            methods: vec![method.into()],
        }],
        registered_at_ms: 1,
        last_heartbeat_at_ms: 1,
        host_origin: "conex://host.local".into(),
    }
}

fn side() -> HostSide {
    HostSide::with_authorizations(vec![AgentAuthorization {
        agent_id: "agent-a".into(),
        tenant_id: "tenant-a".into(),
        endpoints: vec![AuthorizedEndpoint {
            endpoint_id: "notes-a".into(),
            root: "team".into(),
            methods: vec!["source/read".into(), "source/list".into()],
        }],
    }])
}

#[test]
fn unknown_or_mismatched_agents_cannot_register() {
    let side = side();
    // Unknown agent id.
    assert!(
        side.register_agent_link(
            registration("unknown", "tenant-a", "notes-a", "source/read", "*"),
            1
        )
        .is_err()
    );
    // Tenant mismatch.
    assert!(
        side.register_agent_link(
            registration("agent-a", "tenant-b", "notes-a", "source/read", "*"),
            1
        )
        .is_err()
    );
    // Endpoint not configured for this agent.
    assert!(
        side.register_agent_link(
            registration("agent-a", "tenant-a", "notes-b", "source/read", "*"),
            1
        )
        .is_err()
    );
    // Claimed method outside the authorized set.
    let review = side.register_agent_link(
        registration("agent-a", "tenant-a", "notes-a", "source/search", "*"),
        1,
    );
    assert!(
        review.is_err(),
        "zero accepted endpoints must fail the registration"
    );
}

#[test]
fn endpoint_claims_are_reviewed_independently_and_roots_scoped() {
    let side = side();
    // One valid endpoint (narrowed root) plus one outside the authorized
    // root: the valid one is accepted, the invalid one is rejected with a
    // reason — the registration still succeeds (plan M2).
    let mut mixed = registration("agent-a", "tenant-a", "notes-a", "source/read", "team/sub");
    mixed.endpoints.push(EndpointRegistration {
        endpoint_id: "notes-a".into(),
        root: "elsewhere".into(),
        methods: vec!["source/read".into()],
    });
    // Two claims for the same endpoint id: deduplicated by review (both
    // valid, root narrowed) — accepted ids stay unique.
    let review = side.register_agent_link(mixed, 1).unwrap();
    assert_eq!(review.accepted_endpoint_ids, vec!["notes-a".to_string()]);
    assert!(
        review
            .rejected_capabilities
            .iter()
            .any(|(id, reason)| id == "notes-a" && reason.contains("root"))
    );
}

#[test]
fn registration_requires_the_configured_host_origin() {
    let side = side();
    let mut registration = registration("agent-a", "tenant-a", "notes-a", "source/read", "*");
    registration.host_origin = "https://wrong.example".into();
    assert!(side.register_agent_link(registration, 1).is_err());
}

#[test]
fn new_generation_owns_cleanup_and_heartbeat() {
    let side = Arc::new(side());
    side.register_agent_link(
        registration("agent-a", "tenant-a", "notes-a", "source/read", "*"),
        11,
    )
    .unwrap();
    side.register_agent_link(
        registration("agent-a", "tenant-a", "notes-a", "source/read", "*"),
        12,
    )
    .unwrap();
    assert!(
        side.register_agent_link(
            registration("agent-a", "tenant-a", "notes-a", "source/read", "*"),
            12
        )
        .is_err()
    );
    assert!(side.heartbeat_agent("agent-a", 11).is_err());
    assert!(side.heartbeat_agent("agent-a", 12).is_ok());
    side.disconnect_agent("agent-a", 11);
    assert!(side.heartbeat_agent("agent-a", 12).is_ok());
    side.disconnect_agent("agent-a", 12);
    assert!(side.heartbeat_agent("agent-a", 12).is_err());
}

#[test]
fn accepted_endpoints_drive_the_ready_projection() {
    let side = side();
    // Only notes-a is registered; notes-b must not count as ready.
    side.register_agent_link(
        registration("agent-a", "tenant-a", "notes-a", "source/read", "*"),
        1,
    )
    .unwrap();
    assert!(side.agents.accepts_endpoint("agent-a", "notes-a"));
    assert!(!side.agents.accepts_endpoint("agent-a", "notes-b"));
    side.disconnect_agent("agent-a", 1);
    assert!(!side.agents.accepts_endpoint("agent-a", "notes-a"));
}

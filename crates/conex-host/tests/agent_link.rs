use std::sync::Arc;

use conex_host::agent::{AgentAuthorization, AgentRegistration, HostSide};

fn registration(agent_id: &str, tenant_id: &str, provider: &str, method: &str, resource: &str) -> AgentRegistration {
    AgentRegistration {
        agent_id: agent_id.into(),
        principal_id: agent_id.into(),
        tenant_id: tenant_id.into(),
        provider_ids: vec![provider.into()],
        methods: vec![method.into()],
        resources: vec![resource.into()],
        registered_at_ms: 1,
        last_heartbeat_at_ms: 1,
        host_origin: "conex://host.local".into(),
    }
}

fn side() -> HostSide {
    HostSide::with_authorizations(vec![AgentAuthorization {
        agent_id: "agent-a".into(),
        tenant_id: "tenant-a".into(),
        provider_ids: vec!["notes".into()],
        methods: vec!["source/read".into()],
        resources: vec!["*".into()],
    }])
}

#[test]
fn unknown_or_mismatched_agents_cannot_register() {
    let side = side();
    assert!(side.register_agent_link(registration("unknown", "tenant-a", "notes", "source/read", "a.md"), 1).is_err());
    assert!(side.register_agent_link(registration("agent-a", "tenant-b", "notes", "source/read", "a.md"), 1).is_err());
    assert!(side.register_agent_link(registration("agent-a", "tenant-a", "other", "source/read", "a.md"), 1).is_err());
}

#[test]
fn registration_requires_the_configured_host_origin() {
    let side = side();
    let mut registration = registration("agent-a", "tenant-a", "notes", "source/read", "a.md");
    registration.host_origin = "https://wrong.example".into();
    assert!(side.register_agent_link(registration, 1).is_err());
}

#[test]
fn new_generation_owns_cleanup_and_heartbeat() {
    let side = Arc::new(side());
    side.register_agent_link(registration("agent-a", "tenant-a", "notes", "source/read", "a.md"), 11).unwrap();
    side.register_agent_link(registration("agent-a", "tenant-a", "notes", "source/read", "a.md"), 12).unwrap();
    assert!(side.register_agent_link(registration("agent-a", "tenant-a", "notes", "source/read", "a.md"), 12).is_err());
    assert!(side.heartbeat_agent("agent-a", 11).is_err());
    assert!(side.heartbeat_agent("agent-a", 12).is_ok());
    side.disconnect_agent("agent-a", 11);
    assert!(side.heartbeat_agent("agent-a", 12).is_ok());
    side.disconnect_agent("agent-a", 12);
    assert!(side.heartbeat_agent("agent-a", 12).is_err());
}

use conex_proto::{self, AgentEndpoint, AgentRegisterRequest, EndpointListRequest, EndpointListResult, EndpointSummary};
use prost::Message;

#[test]
fn endpoint_catalog_round_trips_and_preserves_optional_cursor() {
    let value = EndpointListResult {
        endpoints: vec![EndpointSummary {
            endpoint_id: "notes-remote".into(),
            display_name: "Remote notes".into(),
            provider_id: "notes".into(),
            attachment_kind: conex_proto::AttachmentKind::ReverseAgent as i32,
            agent_id: Some("agent-a".into()),
            region: Some("private".into()),
            connection_state: conex_proto::ConnectionState::Ready as i32,
            available_methods: vec!["source/list".into(), "source/read".into()],
            authorized_scopes: vec![conex_proto::AuthorizedScope {
                method: "source/read".into(),
                root: "team".into(),
                subtree: true,
            }],
        }],
        next_after_endpoint_id: Some("notes-remote".into()),
    };
    let decoded = EndpointListResult::decode(value.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(decoded, value);
    assert_eq!(
        conex_proto::AttachmentKind::try_from(decoded.endpoints[0].attachment_kind),
        Ok(conex_proto::AttachmentKind::ReverseAgent)
    );
    assert_eq!(decoded.next_after_endpoint_id.as_deref(), Some("notes-remote"));

    let request = EndpointListRequest {
        limit: Some(50),
        after_endpoint_id: None,
    };
    assert_eq!(EndpointListRequest::decode(request.encode_to_vec().as_slice()).expect("decode"), request);
}

#[test]
fn agent_registration_contains_identity_and_no_secret_field() {
    let request = AgentRegisterRequest {
        agent_id: "agent-a".into(),
        tenant_id: "tenant-a".into(),
        endpoints: vec![AgentEndpoint {
            endpoint_id: "notes".into(),
            root: "team".into(),
            methods: vec!["source/read".into()],
        }],
        provides: vec!["source/read".into()],
        host_origin: Some("https://host.example".into()),
    };
    let decoded = AgentRegisterRequest::decode(request.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(decoded.agent_id, "agent-a");
    assert_eq!(decoded.endpoints[0].methods, ["source/read"]);
    assert!(!format!("{decoded:?}").contains("token"));
}

#[test]
fn empty_catalog_round_trips_without_a_cursor() {
    let value = EndpointListResult {
        endpoints: Vec::new(),
        next_after_endpoint_id: None,
    };
    let decoded = EndpointListResult::decode(value.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(decoded, value);
    assert!(decoded.endpoints.is_empty());
    assert!(decoded.next_after_endpoint_id.is_none());
}

#[test]
fn invalid_enum_value_is_preserved_on_decode_and_rejected_by_typed_reader() {
    let value = EndpointSummary {
        endpoint_id: "notes".into(),
        display_name: String::new(),
        provider_id: "notes".into(),
        attachment_kind: 99,
        agent_id: None,
        region: None,
        connection_state: conex_proto::ConnectionState::Ready as i32,
        available_methods: Vec::new(),
        authorized_scopes: Vec::new(),
    };
    let decoded =
        EndpointSummary::decode(value.encode_to_vec().as_slice()).expect("decode unknown enum");
    assert_eq!(decoded.attachment_kind, 99);
    assert!(conex_proto::AttachmentKind::try_from(decoded.attachment_kind).is_err());
}

#[test]
fn registration_vectors_keep_missing_identity_and_duplicate_endpoints_visible() {
    let value = AgentRegisterRequest {
        agent_id: String::new(),
        tenant_id: "tenant-a".into(),
        endpoints: vec![
            AgentEndpoint {
                endpoint_id: "notes".into(),
                root: "team".into(),
                methods: vec!["source/read".into()],
            },
            AgentEndpoint {
                endpoint_id: "notes".into(),
                root: "other".into(),
                methods: vec!["source/list".into()],
            },
        ],
        provides: vec!["source/read".into()],
        host_origin: None,
    };
    let decoded =
        AgentRegisterRequest::decode(value.encode_to_vec().as_slice()).expect("decode registration");
    assert!(decoded.agent_id.is_empty());
    assert_eq!(decoded.endpoints.len(), 2);
    assert_eq!(decoded.endpoints[0].endpoint_id, decoded.endpoints[1].endpoint_id);
}

#[test]
fn partial_scope_and_control_negotiation_round_trip() {
    let scope = conex_proto::AuthorizedScope {
        method: "source/read".into(),
        root: "team".into(),
        subtree: false,
    };
    let decoded = conex_proto::AuthorizedScope::decode(scope.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(decoded, scope);

    let capabilities = conex_proto::NegotiatedCapabilities {
        provides: vec!["source/read".into()],
        requires: vec!["source/list".into()],
        rejected_capabilities: vec![conex_proto::RejectedCapability {
            method: "source/write".into(),
            direction: "provides".into(),
            reason: "read-only".into(),
        }],
    };
    let decoded =
        conex_proto::NegotiatedCapabilities::decode(capabilities.encode_to_vec().as_slice()).expect("decode");
    assert_eq!(decoded, capabilities);

    let identity = conex_proto::LinkIdentity {
        link_id: "link-1".into(),
        peer_id: "agent-a".into(),
        tenant_id: "tenant-a".into(),
    };
    assert_eq!(
        conex_proto::LinkIdentity::decode(identity.encode_to_vec().as_slice()).expect("decode"),
        identity
    );
}

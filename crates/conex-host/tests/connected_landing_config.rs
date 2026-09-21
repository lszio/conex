use std::fs;

use conex_host::config::HostConfig;
use tempfile::tempdir;

fn write_config(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("host.toml");
    fs::write(&path, body).expect("write config");
    (dir, path)
}

fn base() -> &'static str {
    r#"
listen = "127.0.0.1:8787"
allow_loopback_http = true
web_origin = "http://127.0.0.1:8787"
web_root = "."

[[tokens]]
token_hash = "0000000000000000000000000000000000000000000000000000000000000000"
principal_id = "alice"
tenant_id = "tenant-a"
audience = "host.local"
role = "ui"

[[tokens]]
token_hash = "1111111111111111111111111111111111111111111111111111111111111111"
principal_id = "agent-a"
tenant_id = "tenant-a"
audience = "host.local"
role = "agent"

[[agents]]
id = "agent-a"
tenant_id = "tenant-a"
credential_name = "agent-a-token"
credential_backend = "env:CONEX_AGENT_A_TOKEN"

[[endpoints]]
id = "notes-remote"
tenant_id = "tenant-a"
provider_id = "notes"
kind = "source-remote"
provides = ["source/list", "source/read"]
agent_id = "agent-a"
root = "team"
"#
}

#[test]
fn connected_landing_config_is_additive_and_validates() {
    let (_dir, path) = write_config(base());
    let config = HostConfig::load(&path).expect("connected config should parse");
    assert_eq!(config.agents[0].id, "agent-a");
    assert_eq!(config.endpoints[0].agent_id.as_deref(), Some("agent-a"));
}

#[test]
fn invalid_token_role_is_rejected() {
    let (_dir, path) = write_config(&base().replace("role = \"ui\"", "role = \"root\""));
    let error = HostConfig::load(&path).expect_err("unknown role must fail");
    assert!(error.message().contains("unknown token role"));
}

#[test]
fn duplicate_agent_id_is_rejected() {
    let body = format!("{}\n[[agents]]\nid = \"agent-a\"\ntenant_id = \"tenant-a\"\n", base());
    let (_dir, path) = write_config(&body);
    let error = HostConfig::load(&path).expect_err("duplicate agent must fail");
    assert!(error.message().contains("duplicate agent"));
}

#[test]
fn remote_endpoint_requires_supported_method_and_authorized_agent() {
    let unsupported = base().replace("source/list\", \"source/read\"]", "source/write\"]");
    let (_dir, path) = write_config(&unsupported);
    let error = HostConfig::load(&path).expect_err("source/write must fail");
    assert!(error.message().contains("unsupported remote method"));

    let cross_tenant = base().replace("tenant_id = \"tenant-a\"\ncredential_name", "tenant_id = \"tenant-b\"\ncredential_name");
    let (_dir, path) = write_config(&cross_tenant);
    let error = HostConfig::load(&path).expect_err("cross-tenant binding must fail");
    assert!(error.message().contains("tenant"));
}

#[test]
fn remote_endpoint_requires_agent_and_root() {
    let missing_agent = base().replace("agent_id = \"agent-a\"\n", "");
    let (_dir, path) = write_config(&missing_agent);
    let error = HostConfig::load(&path).expect_err("remote endpoint agent is required");
    assert!(error.message().contains("requires agent_id"));

    let missing_root = base().replace("root = \"team\"\n", "");
    let (_dir, path) = write_config(&missing_root);
    let error = HostConfig::load(&path).expect_err("remote endpoint root is required");
    assert!(error.message().contains("requires root"));
}

#[test]
fn policy_must_reference_a_known_endpoint() {
    let body = format!(
        "{}\n[[policy]]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"missing\"\nactions = [\"read\"]\n",
        base()
    );
    let (_dir, path) = write_config(&body);
    let error = HostConfig::load(&path).expect_err("unknown policy endpoint must fail");
    assert!(error.message().contains("unknown endpoint"));
}


#[test]
fn preauthorized_agent_requires_credentials_and_matching_agent_token() {
    let token = r#"[[tokens]]
token_hash = "1111111111111111111111111111111111111111111111111111111111111111"
principal_id = "agent-a"
tenant_id = "tenant-a"
audience = "host.local"
role = "agent"
"#;
    let (_dir, path) = write_config(&base().replace(token, ""));
    let error = HostConfig::load(&path).expect_err("agent token is required");
    assert!(error.message().contains("matching role=agent token"));

    let (_dir, path) = write_config(&base().replace("credential_backend = \"env:CONEX_AGENT_A_TOKEN\"\n", ""));
    let error = HostConfig::load(&path).expect_err("agent credential backend is required");
    assert!(error.message().contains("credential_backend"));
}
#[test]
fn web_origin_must_be_a_bare_origin_without_userinfo_path_query_or_slash() {
    for invalid in [
        "http://example.test/",
        "http://example.test/ui",
        "http://user@example.test",
        "http://example.test?x=1",
        "http://example.test:bad",
    ] {
        let body = base().replace("http://127.0.0.1:8787", invalid);
        let (_dir, path) = write_config(&body);
        let error = HostConfig::load(&path).expect_err("invalid web origin must fail");
        assert!(error.message().contains("web_origin"), "{invalid}: {error:?}");
    }
}

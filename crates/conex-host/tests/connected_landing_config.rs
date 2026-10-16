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
    let body = format!(
        "{}\n[[agents]]\nid = \"agent-a\"\ntenant_id = \"tenant-a\"\n",
        base()
    );
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

    let cross_tenant = base().replace(
        "tenant_id = \"tenant-a\"\ncredential_name",
        "tenant_id = \"tenant-b\"\ncredential_name",
    );
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

    let (_dir, path) =
        write_config(&base().replace("credential_backend = \"env:CONEX_AGENT_A_TOKEN\"\n", ""));
    let error = HostConfig::load(&path).expect_err("agent credential backend is required");
    assert!(error.message().contains("credential_backend"));
}
#[test]
fn web_guest_config_validates_identity_and_tenant_coherence() {
    // A coherent guest section parses and carries the configured identity.
    let ok = format!(
        "{}\n[web_guest]\nprincipal_id = \"visitor\"\ntenant_id = \"tenant-a\"\nmax_sessions = 16\nidle_ttl_ms = 300000\nmax_issue_per_minute = 30\n\n[[policy]]\nprincipal_id = \"visitor\"\ntenant_id = \"tenant-a\"\nendpoint_id = \"notes-remote\"\nactions = [\"read\"]\n",
        base()
    );
    let (_dir, path) = write_config(&ok);
    let config = HostConfig::load(&path).expect("coherent guest config parses");
    let guest = config.web_guest.as_ref().expect("guest section");
    assert_eq!(guest.principal_id, "visitor");
    assert_eq!(guest.tenant_id, "tenant-a");
    assert_eq!(guest.max_sessions, 16);

    // Guest tenant contradicting a policy entry for the same principal is
    // rejected at load (plan M1.1): the policy still binds tenant-a.
    let contradictory = ok.replace(
        "principal_id = \"visitor\"\ntenant_id = \"tenant-a\"\nmax_sessions",
        "principal_id = \"visitor\"\ntenant_id = \"tenant-b\"\nmax_sessions",
    );
    let (_dir, path) = write_config(&contradictory);
    let error = HostConfig::load(&path).expect_err("guest tenant contradiction must fail");
    assert!(error.message().contains("contradicts policy tenant"));

    // The anonymous principal must not collide with a token principal.
    let colliding = format!(
        "{}\n[web_guest]\nprincipal_id = \"alice\"\ntenant_id = \"tenant-a\"\n",
        base()
    );
    let (_dir, path) = write_config(&colliding);
    let error = HostConfig::load(&path).expect_err("guest principal collision must fail");
    assert!(error.message().contains("must not collide"));
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
        assert!(
            error.message().contains("web_origin"),
            "{invalid}: {error:?}"
        );
    }
}

#[test]
fn share_limits_are_additive_and_defaulted() {
    let (_dir, path) = write_config(base());
    let config = HostConfig::load(&path).expect("config without [share] should parse");
    assert!(config.share.is_none(), "the scene is opt-in by section");
    // A host that configures P1 gets a sibling share root without a new line,
    // and the limits stay at their defaults until an operator names them.
    let limits = config.share_limits();
    assert_eq!(limits.max_file_bytes, 8 * 1024 * 1024);
    assert_eq!(limits.max_group_bytes, 32 * 1024 * 1024);
    assert_eq!(limits.max_files_per_client, 32);
    assert!(limits.max_clips_per_client > 0);
    assert!(limits.clip_ttl_ms > 0);
    assert!(
        config.share_storage().is_none(),
        "no content_root and no [share]: no bytes to put anywhere"
    );
}

#[test]
fn share_root_defaults_to_a_sibling_of_the_content_root() {
    let body = base().replace(
        "web_root = \".\"",
        "content_root = \"/var/lib/conex/content\"\nsession_root = \"/var/lib/conex/session\"\noperation_root = \"/var/lib/conex/operation\"",
    );
    let (_dir, path) = write_config(&body);
    let config = HostConfig::load(&path).expect("p1 config should parse");
    let (root, lease_ms) = config.share_storage().expect("a share root is derived");
    assert_eq!(root, std::path::Path::new("/var/lib/conex/share"));
    assert!(lease_ms > 0);
}

#[test]
fn a_named_share_root_is_used_instead_of_the_derived_one() {
    let body = base().replace(
        "web_root = \".\"",
        "content_root = \"/var/lib/conex/content\"\nsession_root = \"/var/lib/conex/session\"\noperation_root = \"/var/lib/conex/operation\"\n\n[share]\nshare_root = \"/srv/conex/share\"\nmax_file_bytes = 1073741824\nmax_clips_per_client = 5\nclip_ttl_ms = 60000",
    );
    let (_dir, path) = write_config(&body);
    let config = HostConfig::load(&path).expect("share config should parse");
    let (root, _) = config.share_storage().expect("a share root is named");
    assert_eq!(root, std::path::Path::new("/srv/conex/share"));
    let limits = config.share_limits();
    assert_eq!(limits.max_file_bytes, 1073741824);
    assert_eq!(limits.max_clips_per_client, 5);
    assert_eq!(limits.clip_ttl_ms, 60000);
}

#[test]
fn the_share_root_may_not_be_the_content_root() {
    // The two scenes reclaim differently: the blob channel's garbage collector
    // takes blocks no committed root reaches, which is exactly what an
    // in-flight upload looks like. One root would let either delete the
    // other's staging.
    let body = base().replace(
        "web_root = \".\"",
        "content_root = \"/var/lib/conex/content\"\nsession_root = \"/var/lib/conex/session\"\noperation_root = \"/var/lib/conex/operation\"\n\n[share]\nshare_root = \"/var/lib/conex/content\"",
    );
    let (_dir, path) = write_config(&body);
    let error = HostConfig::load(&path).expect_err("a shared root must be refused");
    assert!(
        error.message().contains("share_root"),
        "{:?}",
        error.message()
    );
}

#[test]
fn a_group_cap_below_the_file_cap_follows_it_instead_of_refusing() {
    // Raising the per-file cap without touching the group cap used to be a
    // config error, which is a footgun: the operator meant "bigger files" and
    // got a refusal that did not say why.
    let body = base().replace(
        "web_root = \".\"",
        "content_root = \"/var/lib/conex/content\"\nsession_root = \"/var/lib/conex/session\"\noperation_root = \"/var/lib/conex/operation\"\n\n[share]\nshare_root = \"/tmp/s\"\nmax_file_bytes = 1073741824",
    );
    let (_dir, path) = write_config(&body);
    let config = HostConfig::load(&path).expect("a bigger file cap should parse");
    let limits = config.share_limits();
    assert_eq!(limits.max_file_bytes, 1073741824);
    assert_eq!(
        limits.max_group_bytes, 1073741824,
        "the group has room for one file"
    );
}

#[test]
fn share_limits_are_checked_for_sense_not_for_taste() {
    for (section, needle) in [
        (
            "share_root = \"/tmp/s\"\nmax_file_bytes = 0",
            "max_file_bytes",
        ),
        (
            "share_root = \"/tmp/s\"\nmax_files_per_client = 0",
            "max_files_per_client",
        ),
        (
            "share_root = \"/tmp/s\"\nmax_clips_per_client = 0",
            "max_clips_per_client",
        ),
        ("share_root = \"/tmp/s\"\nclip_ttl_ms = 0", "clip_ttl_ms"),
    ] {
        let body = base().replace("web_root = \".\"", &format!("content_root = \"/var/lib/conex/content\"\nsession_root = \"/var/lib/conex/session\"\noperation_root = \"/var/lib/conex/operation\"\n\n[share]\n{section}"));
        let (_dir, path) = write_config(&body);
        let error = HostConfig::load(&path).expect_err("an impossible limit must fail");
        assert!(error.message().contains(needle), "{needle}: {error:?}");
    }
}

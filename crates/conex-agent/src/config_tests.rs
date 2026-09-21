use super::config::AgentConfig;
use std::fs;
use tempfile::tempdir;

fn write(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("agent.toml");
    fs::write(&path, body).expect("write config");
    (dir, path)
}

fn base(root: &str) -> String {
    format!(
        r#"
agent_id = "agent-a"
tenant_id = "tenant-a"
host_url = "wss://host.example/wss"
token_name = "agent-token"
token_backend = "env:CONEX_AGENT_TOKEN"

[[endpoints]]
endpoint_id = "notes"
root = "{root}"
methods = ["source/list", "source/read"]
"#
    )
}

#[test]
fn agent_config_validates_read_only_endpoint_mapping() {
    let (_dir, path) = write(&base("."));
    let config = AgentConfig::load(&path).expect("agent config should parse");
    assert_eq!(config.endpoints[0].endpoint_id, "notes");
}

#[test]
fn agent_config_rejects_missing_root_and_unsupported_method() {
    let (_dir, path) = write(&base(""));
    let error = AgentConfig::load(&path).expect_err("empty root must fail");
    assert!(error.contains("requires root"));

    let (_dir, path) = write(&base(".").replace("source/read", "source/write"));
    let error = AgentConfig::load(&path).expect_err("source/write must fail");
    assert!(error.contains("unsupported method"));
}

#[test]
fn agent_config_rejects_duplicate_endpoint_ids() {
    let body = format!("{}\n[[endpoints]]\nendpoint_id = \"notes\"\nroot = \".\"\nmethods = [\"source/read\"]\n", base("."));
    let (_dir, path) = write(&body);
    let error = AgentConfig::load(&path).expect_err("duplicate endpoint must fail");
    assert!(error.contains("duplicate endpoint"));
}


#[test]
fn agent_config_accepts_only_wss_except_explicit_loopback_ws() {
    let https = base(".").replace("wss://host.example/wss", "https://host.example/wss");
    let (_dir, path) = write(&https);
    let error = AgentConfig::load(&path).expect_err("https must not be an agent WS URL");
    assert!(error.contains("wss://"));
    let loopback = base(".")
        .replace(
            "[[endpoints]]",
            "allow_loopback_ws = true\n\n[[endpoints]]",
        )
        .replace("wss://host.example/wss", "ws://127.0.0.1:8787/wss");
    let (_dir, path) = write(&loopback);
    AgentConfig::load(&path).expect("explicit loopback ws should be accepted");
}

#[test]
fn agent_config_rejects_non_loopback_authorities_and_empty_token_sources() {
    for host in [
        "ws://127.0.0.1.evil.com/wss",
        "ws://localhost.evil.com/wss",
        "ws://127.0.0.2/wss",
    ] {
        let body = base(".")
            .replace(
                "[[endpoints]]",
                "allow_loopback_ws = true\n\n[[endpoints]]",
            )
            .replace("wss://host.example/wss", host);
        let (_dir, path) = write(&body);
        assert!(AgentConfig::load(&path).is_err(), "{host} must be rejected");
    }
    for source in ["env:", "file:"] {
        let body = base(".").replace("env:CONEX_AGENT_TOKEN", source);
        let (_dir, path) = write(&body);
        let error = AgentConfig::load(&path).expect_err("empty token source must fail");
        assert!(error.contains("token_backend"));
    }
    let (_dir, path) = write(&base(".").replace("token_name = \"agent-token\"", "token_name = \"\""));
    let error = AgentConfig::load(&path).expect_err("empty token name must fail");
    assert!(error.contains("token_name"));
}
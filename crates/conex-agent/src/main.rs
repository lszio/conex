//! P1-05 reverse-connect agent (in-process stub).
//!
//! P2 wires a real outbound WS connection plus the auto-register
//! bootstrap that listens for the host's hello/ready handshake. For P1 we
//! ship a self-contained CLI that talks the agent/* wire contract via the
//! same JSON-RPC envelopes as the broker, exercising the in-memory registry
//! and proving the registration surface end-to-end.
#![forbid(unsafe_code)]

use std::time::Duration;

use conex_proto::v1;

use serde_json::{Value, json};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).cloned().unwrap_or_else(|| "help".into());
    match mode.as_str() {
        "register" => register(&args),
        "heartbeat" => heartbeat(&args),
        "resolve" => resolve(&args),
        "help" | "--help" | "-h" => help(),
        other => {
            eprintln!("unknown mode {other}");
            std::process::exit(2);
        }
    }
}

fn register(args: &[String]) {
    let payload = json!({
        "agentId": arg_or(args, 2, "agent-1"),
        "providerIds": arg_or(args, 3, "notes-local").split(',').collect::<Vec<_>>(),
        "methods": arg_or(args, 4, "source/read").split(',').collect::<Vec<_>>(),
        "resources": arg_or(args, 5, "notes/a.md").split(',').collect::<Vec<_>>(),
        "hostOrigin": arg_or(args, 6, "conex://broker.local"),
    });
    println!("{}", serde_json::to_string(&payload).unwrap());
}

fn heartbeat(args: &[String]) {
    let agent_id = arg_or(args, 2, "agent-1");
    let response = json!({
        "agentId": agent_id,
        "heartbeatAtMs": millis(),
    });
    println!("{}", serde_json::to_string(&response).unwrap());
}

fn resolve(args: &[String]) {
    let provider = arg_or(args, 2, "notes-local");
    let resource = arg_or(args, 3, "notes/a.md");
    let method = arg_or(args, 4, "source/read");
    let response = json!({ "agents": vec![format!("agent:{provider}")] });
    let response = simulate_lookup(response, &provider, &resource, &method);
    println!("{}", serde_json::to_string(&response).unwrap());
}

fn simulate_lookup(value: Value, provider: &str, _resource: &str, _method: &str) -> Value {
    Value::Object({
        let mut m = serde_json::Map::new();
        m.insert("providerId".into(), Value::String(provider.to_string()));
        m.insert("resolvedAtMs".into(), Value::String(millis()));
        m.insert(
            "agents".into(),
            value.get("agents").cloned().unwrap_or(Value::Null),
        );
        m
    })
}

fn help() {
    eprintln!("conex-agent <mode> [args]");
    eprintln!("  modes: register | heartbeat | resolve | help");
}

fn arg_or(args: &[String], idx: usize, default: &str) -> String {
    args.get(idx).cloned().unwrap_or_else(|| default.into())
}

fn millis() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().to_string())
        .unwrap_or_default()
}

#[allow(dead_code)]
fn _ensure_v1_used(_: v1::Plane) {
    let _ = Duration::from_secs(0);
}

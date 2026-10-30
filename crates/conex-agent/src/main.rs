#![forbid(unsafe_code)]

pub mod access;
pub mod application;
pub mod assembly;
pub mod builtins;
pub mod config;
pub mod local_api;
pub mod node;
pub mod node_config;
pub mod node_types;
pub mod process;
pub mod publication;
pub mod runtime;

#[cfg(test)]
mod config_tests;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next();
    if mode.as_deref() == Some("--module-worker") && args.next().is_none() {
        if let Err(error) = process::worker().await {
            eprintln!("module worker failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    if mode.as_deref() == Some("--node-config") {
        let Some(path) = args.next() else {
            eprintln!("--node-config requires a path");
            std::process::exit(2);
        };
        if args.next().is_some() {
            eprintln!("unexpected node arguments");
            std::process::exit(2);
        }
        let result = match node_config::NodeConfig::load(std::path::Path::new(&path)) {
            Ok(config) => runtime::run_node(config).await,
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            eprintln!("node runtime failed: {error}");
            std::process::exit(1);
        }
        return;
    }
    let config_path = match mode.as_deref() {
        Some("--config") => args.next(),
        Some("-c") => args.next(),
        Some("help") | Some("--help") | Some("-h") | None => {
            eprintln!("conex-agent --config PATH");
            return;
        }
        Some(other) => {
            eprintln!("unknown argument {other}; use --config PATH");
            std::process::exit(2);
        }
    };
    let Some(config_path) = config_path else {
        eprintln!("--config requires a path");
        std::process::exit(2);
    };
    let config = match config::AgentConfig::load(std::path::Path::new(&config_path)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("invalid agent config: {error}");
            std::process::exit(2);
        }
    };
    if let Err(error) = runtime::run(config).await {
        eprintln!("agent runtime failed: {error}");
        std::process::exit(1);
    }
}

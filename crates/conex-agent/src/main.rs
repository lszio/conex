#![forbid(unsafe_code)]

pub mod config;
pub mod runtime;

#[cfg(test)]
mod config_tests;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let config_path = match args.next().as_deref() {
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

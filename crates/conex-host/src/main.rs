//! Standalone host: load a TOML config and serve the authenticated /rpc entry.
use std::path::Path;

use conex_host::config::HostConfig;
use conex_host::serve::serve;

#[tokio::main]
async fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "examples/p0/host.toml".to_string());
    let config = match HostConfig::load(Path::new(&path)) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("config error: {}", error.message());
            std::process::exit(1);
        }
    };
    if let Err(error) = serve(config).await {
        eprintln!("serve error: {}", error.message());
        std::process::exit(1);
    }
}

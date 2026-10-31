//! Standalone host: load a TOML config and serve the authenticated /rpc entry.
use std::path::Path;

use conex_host::config::HostConfig;
use conex_host::private_access::{
    BOOTSTRAP_TTL_MS, bootstrap_hash, ensure_private_dir, generate_bootstrap_code,
    put_bootstrap_blocking,
};
use conex_host::serve::serve;

/// `conex-host private-bootstrap --config <owned-config>`: mint one 256-bit
/// single-use bootstrap code, persist only its hash, print the plaintext
/// once. The code is never written to a file or log.
fn private_bootstrap(config_path: Option<&str>) {
    let Some(config_path) = config_path
        .and_then(|value| {
            value
                .strip_prefix("--config=")
                .map(str::to_owned)
                .or(Some(value.to_owned()))
        })
        .filter(|value| !value.is_empty())
    else {
        eprintln!("private-bootstrap requires --config PATH");
        std::process::exit(2);
    };
    let config = match HostConfig::load(Path::new(config_path.as_str())) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("config error: {}", error.message());
            std::process::exit(1);
        }
    };
    let Some(private) = config.private.as_ref() else {
        eprintln!("config has no [private] section");
        std::process::exit(1);
    };
    if let Err(error) = ensure_private_dir(private.store_path.parent().unwrap_or(Path::new("."))) {
        eprintln!("bootstrap error: {}", error.message());
        std::process::exit(1);
    }
    let code = generate_bootstrap_code();
    let expires_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
        + BOOTSTRAP_TTL_MS;
    if let Err(error) =
        put_bootstrap_blocking(&private.store_path, &bootstrap_hash(&code), expires_at_ms)
    {
        eprintln!("bootstrap error: {}", error.message());
        std::process::exit(1);
    }
    println!("{code}");
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let first = args
        .next()
        .unwrap_or_else(|| "examples/p0/host.toml".to_string());
    if first == "private-bootstrap" {
        private_bootstrap(args.next().as_deref());
        return;
    }
    let path = first;
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

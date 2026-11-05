//! Module-principal acceptance: real host + two real nodes; approved module
//! consumer, republish chain, and unapproved/cross-restart revocation.
use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;

pub fn run() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("workspace root")?
        .to_path_buf();
    let status = Command::new("cargo")
        .args(["build", "-p", "conex-host", "-p", "conex-agent", "--locked"])
        .current_dir(&root)
        .status()
        .context("build module-principals binaries")?;
    if !status.success() {
        bail!("module-principals build failed");
    }
    let status = Command::new("bun")
        .arg("sdk/typescript/tests/module_principals_probe.ts")
        .env("CONEX_NODE_BINARY", root.join("target/debug/conex-agent"))
        .env("CONEX_HOST_BINARY", root.join("target/debug/conex-host"))
        .current_dir(&root)
        .status()
        .context("run module-principals probe")?;
    if !status.success() {
        bail!("module-principals acceptance failed");
    }
    Ok(())
}

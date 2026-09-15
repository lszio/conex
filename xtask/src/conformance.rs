//! Drive Rust and TypeScript over the same shared vectors.
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn run(vectors: &str) -> Result<()> {
    if !Path::new(vectors).is_dir() {
        bail!("vectors directory not found: {vectors}");
    }
    let root = repo_root()?;
    run_step(
        &root,
        "rust vectors",
        "cargo",
        &[
            "test",
            "-p",
            "conex-proto",
            "--test",
            "scalars",
            "--test",
            "wire",
            "--test",
            "cid",
        ],
    )?;
    run_step(
        &root,
        "ts vectors",
        "bun",
        &[
            "test",
            "sdk/typescript/tests/scalars.test.ts",
            "sdk/typescript/tests/wire.test.ts",
            "sdk/typescript/tests/cid.test.ts",
        ],
    )?;
    println!("conformance: Rust and TS agree on {vectors}");
    Ok(())
}

pub(crate) fn run_step(root: &Path, name: &str, program: &str, args: &[&str]) -> Result<()> {
    println!("== {name} ==");
    let status = Command::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .with_context(|| format!("spawn {program}"))?;
    if !status.success() {
        bail!("{name} failed ({} {args:?})", program);
    }
    Ok(())
}

pub(crate) fn repo_root() -> Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest.join("..").canonicalize()?)
}

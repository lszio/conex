//! Drive Rust and TypeScript over the same shared vectors.
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn run(vectors: &str) -> Result<()> {
    if !Path::new(vectors).is_dir() {
        bail!("vectors directory not found: {vectors}");
    }
    let root = repo_root()?;
    // P1-01 contract freeze: shared chunking golden CIDs.
    run_step(
        &root,
        "rust p1 chunking",
        "cargo",
        &["test", "-p", "conex-proto", "--test", "chunking"],
    )?;
    run_step(
        &root,
        "ts p1 chunking",
        "bun",
        &["test", "sdk/typescript/tests/chunking.test.ts"],
    )?;
    run_step(
        &root,
        "rust p1 contracts",
        "cargo",
        &["test", "-p", "conex-proto", "--test", "p1_contracts"],
    )?;
    run_step(
        &root,
        "ts p1 contracts",
        "bun",
        &["test", "sdk/typescript/tests/p1_contracts.test.ts"],
    )?;
    // P1-06 blob storage: lifecycle + crash matrix + GC against blob.json.
    run_step(
        &root,
        "rust conex-content blob",
        "cargo",
        &["test", "-p", "conex-content", "--test", "blob"],
    )?;
    // P1-08 operations: dedup / execution / state machine against
    // conformance/vectors/p1/operation.json.
    run_step(
        &root,
        "rust conex-core operation",
        "cargo",
        &["test", "-p", "conex-core", "--test", "operation"],
    )?;
    // P1-07 blob transfer: inline eligibility + 1 GiB roundtrip.
    run_step(
        &root,
        "rust conex-core session",
        "cargo",
        &["test", "-p", "conex-core", "--test", "session"],
    )?;
    run_step(
        &root,
        "rust conex-content transfer",
        "cargo",
        &[
            "test",
            "-p",
            "conex-content",
            "--test",
            "transfer",
            "--",
            "--test-threads=1",
        ],
    )?;
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

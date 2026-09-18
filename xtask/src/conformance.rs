//! Drive Rust and TypeScript over the same shared vectors.
//!
//! `--vectors <dir>` selects the *step set* for that directory: the P0
//! vectors drive the cross-language codec tests, the P1 vectors drive the
//! chunking/blob/stream/operation/session/transfer behaviour suites. The
//! runner no longer executes one fixed list regardless of the directory
//! (which ran the same steps twice and re-ran the 1 GiB transfer test).
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub fn run(vectors: &str) -> Result<()> {
    if !Path::new(vectors).is_dir() {
        bail!("vectors directory not found: {vectors}");
    }
    let root = repo_root()?;
    let steps: Vec<(&str, &str, Vec<&str>)> = match vectors {
        "conformance/vectors/p0" => p0_steps(),
        "conformance/vectors/p1" => p1_steps(),
        other => bail!("no conformance step set for {other}"),
    };
    for (name, program, args) in steps {
        run_step(&root, name, program, &args)?;
    }
    println!("conformance: Rust and TS agree on {vectors}");
    Ok(())
}

/// P0: cross-language codec vectors (scalars/wire/cid) + policy vectors.
fn p0_steps() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    vec![
        (
            "rust p0 vectors",
            "cargo",
            vec![
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
        ),
        (
            "ts p0 vectors",
            "bun",
            vec![
                "test",
                "sdk/typescript/tests/scalars.test.ts",
                "sdk/typescript/tests/wire.test.ts",
                "sdk/typescript/tests/cid.test.ts",
            ],
        ),
        (
            "rust p0 policy",
            "cargo",
            vec!["test", "-p", "conex-core", "--test", "policy"],
        ),
    ]
}

/// P1: canonical content addressing, blob storage/transfer, stream, session
/// and operation slices. Every step drives a real implementation against the
/// shared vectors or its own behavioural matrix.
fn p1_steps() -> Vec<(&'static str, &'static str, Vec<&'static str>)> {
    vec![
        (
            "manifest goldens",
            "python3",
            vec!["conformance/tools/gen_manifest_goldens.py", "--check"],
        ),
        (
            "rust p1 chunking",
            "cargo",
            vec!["test", "-p", "conex-proto", "--test", "chunking"],
        ),
        (
            "ts p1 chunking",
            "bun",
            vec!["test", "sdk/typescript/tests/chunking.test.ts"],
        ),
        (
            "rust conex-content blob",
            "cargo",
            vec!["test", "-p", "conex-content", "--test", "blob"],
        ),
        (
            "rust conex-content transfer",
            "cargo",
            vec![
                "test",
                "-p",
                "conex-content",
                "--test",
                "transfer",
                "--",
                "--test-threads=1",
            ],
        ),
        (
            "rust conex-core stream",
            "cargo",
            vec!["test", "-p", "conex-core", "--test", "stream"],
        ),
        (
            "rust conex-core operation",
            "cargo",
            vec!["test", "-p", "conex-core", "--test", "operation"],
        ),
        (
            "rust conex-core session",
            "cargo",
            vec!["test", "-p", "conex-core", "--test", "session"],
        ),
    ]
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

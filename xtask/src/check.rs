//! Aggregate the protocol and SDK gates; a non-zero step fails the whole run.
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

type Step = (&'static str, &'static str, &'static [&'static str]);

const STEPS: &[Step] = &[
    ("fmt", "cargo", &["fmt", "--all", "--", "--check"]),
    (
        "clippy",
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    ),
    ("cargo test", "cargo", &["test", "--workspace", "--locked"]),
    ("bun install", "bun", &["install", "--frozen-lockfile"]),
    ("typecheck", "bun", &["run", "typecheck"]),
    ("bun test", "bun", &["test", "sdk/typescript/tests"]),
    (
        "generate --check",
        "cargo",
        &["xtask", "generate", "--check"],
    ),
    (
        "conformance",
        "cargo",
        &[
            "xtask",
            "conformance",
            "--vectors",
            "conformance/vectors/p0",
        ],
    ),
    (
        "conformance p1",
        "cargo",
        &[
            "xtask",
            "conformance",
            "--vectors",
            "conformance/vectors/p1",
        ],
    ),
    (
        "conformance s1",
        "cargo",
        &[
            "xtask",
            "conformance",
            "--vectors",
            "conformance/vectors/s1",
        ],
    ),
    (
        "end-to-end (real host + sdk)",
        "cargo",
        &["xtask", "e2e", "--suite", "p0-ts"],
    ),
    (
        "S1 capabilities (real host + TS/Rust SDKs)",
        "cargo",
        &["xtask", "e2e", "--suite", "s1-capabilities"],
    ),
    (
        "S2 plugins (real host + plugin runtimes)",
        "cargo",
        &["xtask", "e2e", "--suite", "s2-plugins"],
    ),
];

pub fn run() -> Result<()> {
    let root = repo_root()?;
    let mut failures: Vec<&str> = Vec::new();
    for (name, program, args) in STEPS {
        println!("\n=== {name} ===");
        let status = Command::new(program)
            .args(*args)
            .current_dir(&root)
            .status()
            .with_context(|| format!("spawn {program}"))?;
        if !status.success() {
            failures.push(name);
        }
    }
    if !failures.is_empty() {
        bail!("xtask check failed: {}", failures.join(", "));
    }
    println!("\nxtask check: all steps passed");
    Ok(())
}

fn repo_root() -> Result<std::path::PathBuf> {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    Ok(manifest.join("..").canonicalize()?)
}

#[allow(dead_code)]
fn relative(root: &Path, path: &Path) -> std::path::PathBuf {
    path.strip_prefix(root)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| path.to_path_buf())
}

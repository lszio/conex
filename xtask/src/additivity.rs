//! Enforce that adding a second provider needs no core/proto/source/host change.
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

const DENIED_PREFIXES: &[&str] = &[
    "crates/conex-core/",
    "crates/conex-proto/",
    "crates/conex-source/",
    "crates/conex-host/",
    "schema/conex/v1/",
];

/// Explicit registration-only exceptions under a denied prefix (none in P0).
const ALLOWED: &[&str] = &[];

pub fn run(base_file: &str) -> Result<()> {
    if !Path::new(base_file).exists() {
        bail!("base file not found: {base_file}");
    }
    let base = fs::read_to_string(base_file)
        .context("read base file")?
        .trim()
        .to_string();
    if base.is_empty() {
        bail!("base file is empty: {base_file}");
    }
    git(&["cat-file", "-e", &format!("{base}^{{commit}}")]).context("unknown base commit")?;
    let ancestor = Command::new("git")
        .args(["merge-base", "--is-ancestor", &base, "HEAD"])
        .status()
        .context("run git merge-base")?;
    if !ancestor.success() {
        bail!("dirty base: {base} is not an ancestor of HEAD");
    }

    let mut changed: BTreeSet<String> = BTreeSet::new();
    for args in [
        vec!["diff", "--name-only", &base, "HEAD"],
        vec!["diff", "--name-only"],
        vec!["diff", "--name-only", "--cached"],
        vec!["ls-files", "--others", "--exclude-standard"],
    ] {
        for line in git(&args)?.lines() {
            let path = line.trim();
            if !path.is_empty() {
                changed.insert(path.to_string());
            }
        }
    }

    let problems: Vec<&String> = changed
        .iter()
        .filter(|path| {
            DENIED_PREFIXES
                .iter()
                .any(|prefix| path.starts_with(prefix))
        })
        .filter(|path| !ALLOWED.contains(&path.as_str()))
        .collect();
    if !problems.is_empty() {
        bail!(
            "additivity violation: core/proto/source/host/schema files changed since {base}:\n  {}",
            problems
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join("\n  ")
        );
    }
    println!(
        "check-additivity: {base} ok ({} changed paths, none under core/proto/source/host/schema)",
        changed.len()
    );
    Ok(())
}

fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("run git {args:?}"))?;
    if !output.status.success() {
        bail!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

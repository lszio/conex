//! conex build/verification tasks: `cargo xtask <command>`.
mod additivity;
mod e2e;
mod generate;
mod schema;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");
    let result = match command {
        "generate" => generate::run(args.iter().any(|a| a == "--check")),
        "check-additivity" => {
            let base = args
                .iter()
                .position(|a| a == "--base-file")
                .and_then(|index| args.get(index + 1))
                .map(String::as_str)
                .unwrap_or("target/p0-additivity-base");
            additivity::run(base)
        }
        "e2e" => {
            let suite = args
                .iter()
                .position(|a| a == "--suite")
                .and_then(|index| args.get(index + 1))
                .map(String::as_str)
                .unwrap_or("p0-ts");
            e2e::run(suite)
        }
        "" | "help" | "--help" | "-h" => {
            eprintln!("usage: cargo xtask <generate [--check]>");
            return ExitCode::SUCCESS;
        }
        other => Err(anyhow::anyhow!("unknown xtask command: {other}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("xtask {command} failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}

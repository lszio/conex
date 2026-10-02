//! conex build/verification tasks: `cargo xtask <command>`.
mod additivity;
mod check;
mod conformance;
mod e2e;
mod generate;
mod landing_demo;
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
        "conformance" => {
            let vectors = args
                .iter()
                .position(|a| a == "--vectors")
                .and_then(|index| args.get(index + 1))
                .map(String::as_str)
                .unwrap_or("conformance/vectors/p0");
            conformance::run(vectors)
        }
        "landing-demo" => {
            if args
                .iter()
                .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
            {
                eprintln!("usage: cargo xtask landing-demo");
                Ok(())
            } else {
                landing_demo::run()
            }
        }
        "check" => check::run(),
        "" | "help" | "--help" | "-h" => {
            eprintln!("usage: cargo xtask <generate [--check]>");
            eprintln!("       cargo xtask landing-demo");
            eprintln!("       cargo xtask e2e --suite connected-landing");
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

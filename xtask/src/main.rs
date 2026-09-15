//! conex build/verification tasks: `cargo xtask <command>`.
mod generate;
mod schema;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");
    let result = match command {
        "generate" => generate::run(args.iter().any(|a| a == "--check")),
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

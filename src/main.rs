use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use dockerlint::{lint_dockerfile, Finding, Severity};

#[derive(Parser)]
#[command(
    name = "dockerlint",
    about = "Lints a Dockerfile for common anti-patterns — a hadolint alternative with no Haskell runtime"
)]
struct Cli {
    /// Path to the Dockerfile to lint.
    path: PathBuf,
    /// Emit findings as a JSON array instead of human-readable lines.
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let content = match fs::read_to_string(&cli.path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("dockerlint: error: reading {}: {e}", cli.path.display());
            return ExitCode::from(2);
        }
    };

    let findings = lint_dockerfile(&content);
    report(&cli.path.display().to_string(), &findings, cli.json);

    if findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn report(path: &str, findings: &[Finding], json: bool) {
    if json {
        let out = serde_json::to_string_pretty(findings).unwrap_or_else(|_| "[]".to_string());
        println!("{out}");
        return;
    }
    if findings.is_empty() {
        println!("dockerlint: {path}: no issues found");
        return;
    }
    for f in findings {
        let sev = match f.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        println!("{path}:{}: [{sev}] {} - {}", f.line, f.rule, f.message);
    }
    eprintln!("\ndockerlint: {} issue(s) found in {path}", findings.len());
}

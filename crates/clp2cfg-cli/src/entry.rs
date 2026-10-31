//! The console binary's entry point, and the real clipboard / standard input shared with the
//! GUI binary's headless mode. Kept out of `main.rs` so that stays a thin wiring file.

use crate::args::Args;
use crate::headless::{run, HeadlessRequest, InputEnv, Report};
use clap::Parser;
use clp2cfg_core::apply::{SystemCatalog, SystemIo};
use std::io::Write;
use std::process::ExitCode;

/// The OS clipboard and standard input.
pub struct SystemInputs;

impl InputEnv for SystemInputs {
    fn clipboard_text(&self) -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }

    fn stdin_text(&self) -> std::io::Result<String> {
        std::io::read_to_string(std::io::stdin())
    }
}

/// The arguments of this process (exits with clap's message on bad usage, `--help`, `--version`).
pub fn parse_args() -> Args {
    Args::parse()
}

/// Run the headless apply / dry-run the arguments describe, against the real machine.
pub fn run_headless(args: &Args) -> Report {
    match HeadlessRequest::try_from(args) {
        Ok(request) => run(&request, &SystemInputs, &SystemCatalog, &SystemIo),
        Err(e) => Report::usage(e.to_string()),
    }
}

/// `clp2cfg-cli` (console): always headless, prints the result and returns the exit code.
pub fn cli_main() -> ExitCode {
    let report = run_headless(&parse_args());
    // 出力先が閉じられていても（パイプ切断など）異常終了させない
    let _ = std::io::stdout().write_all(report.stdout.as_bytes());
    let _ = std::io::stderr().write_all(report.stderr.as_bytes());
    ExitCode::from(report.exit_code)
}

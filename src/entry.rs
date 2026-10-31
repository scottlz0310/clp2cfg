//! Entry points of the two binaries, kept out of `main.rs` so that stays a thin wiring file.
//! This is also where the real clipboard, standard input, client detection and file system are
//! plugged into the frontends.

use crate::app::{load_app_icon, Clp2cfgApp, StartupOptions};
use crate::apply::{SystemCatalog, SystemIo};
use crate::args::{Args, Mode};
use crate::headless::{read_input, run, HeadlessRequest, InputEnv, Report};
use crate::snippet::filter::FilterOptions;
use clap::Parser;
use std::io::Write;
use std::process::ExitCode;

/// The OS clipboard and standard input.
struct SystemInputs;

impl InputEnv for SystemInputs {
    fn clipboard_text(&self) -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }

    fn stdin_text(&self) -> std::io::Result<String> {
        std::io::read_to_string(std::io::stdin())
    }
}

/// `clp2cfg` (GUI subsystem on Windows): opens the window, or runs headless silently when
/// `--apply` / `--dry-run` is given (the exit code is the only output).
pub fn gui_main() -> ExitCode {
    let args = Args::parse();
    match args.mode() {
        Mode::Headless(_) => ExitCode::from(run_headless(&args).exit_code),
        Mode::Gui => run_gui(&args),
    }
}

/// `clp2cfg-cli` (console): always headless, prints the result and returns the exit code.
pub fn cli_main() -> ExitCode {
    let report = run_headless(&Args::parse());
    // 出力先が閉じられていても（パイプ切断など）異常終了させない
    let _ = std::io::stdout().write_all(report.stdout.as_bytes());
    let _ = std::io::stderr().write_all(report.stderr.as_bytes());
    ExitCode::from(report.exit_code)
}

fn run_headless(args: &Args) -> Report {
    match HeadlessRequest::try_from(args) {
        Ok(request) => run(&request, &SystemInputs, &SystemCatalog, &SystemIo),
        Err(e) => Report::usage(e.to_string()),
    }
}

fn run_gui(args: &Args) -> ExitCode {
    tracing_subscriber::fmt::init();

    let filters = FilterOptions::from(args);
    let startup = StartupOptions {
        initial_input: args
            .input
            .as_deref()
            .map(|input| read_input(Some(input), &SystemInputs).map(|(text, _)| text)),
        replace_npx_with_bunx: filters.replace_npx_with_bunx,
        replace_pipx_with_uvx: filters.replace_pipx_with_uvx,
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([960.0, 640.0])
        .with_min_inner_size([640.0, 480.0])
        .with_title("clp2cfg - MCP Server Config Merger");
    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        persist_window: false,
        ..Default::default()
    };

    match eframe::run_native(
        "clp2cfg",
        native_options,
        Box::new(move |cc| Ok(Box::new(Clp2cfgApp::new(cc, startup)))),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("clp2cfg: {e}");
            ExitCode::FAILURE
        }
    }
}

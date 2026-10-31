//! Entry point of the GUI binary, kept out of `main.rs` so that stays a thin wiring file.

use crate::app::{load_app_icon, Clp2cfgApp, StartupOptions};
use clp2cfg_cli::args::{Args, Mode};
use clp2cfg_cli::entry::{parse_args, run_headless, SystemInputs};
use clp2cfg_cli::headless::read_input;
use clp2cfg_core::snippet::filter::FilterOptions;
use std::process::ExitCode;

/// `clp2cfg` (GUI subsystem on Windows): opens the window, or runs headless silently when
/// `--apply` / `--dry-run` is given (the exit code is the only output).
pub fn gui_main() -> ExitCode {
    let args = parse_args();
    match args.mode() {
        Mode::Headless(_) => ExitCode::from(run_headless(&args).exit_code),
        Mode::Gui => run_gui(&args),
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

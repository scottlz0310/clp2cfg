#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clp2cfg::app::{load_app_icon, Clp2cfgApp};

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([960.0, 640.0])
        .with_min_inner_size([640.0, 480.0])
        .with_title("clp2cfg - MCP Server Config Merger");

    if let Some(icon) = load_app_icon() {
        viewport = viewport.with_icon(icon);
    }

    let native_options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "clp2cfg",
        native_options,
        Box::new(|cc| Ok(Box::new(Clp2cfgApp::new(cc)))),
    )
}

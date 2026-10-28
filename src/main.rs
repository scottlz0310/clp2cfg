use clp2cfg::app::Clp2cfgApp;

fn main() -> eframe::Result<()> {
    tracing_subscriber::fmt::init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_min_inner_size([640.0, 480.0])
            .with_title("clp2cfg - MCP Server Config Merger"),
        ..Default::default()
    };

    eframe::run_native(
        "clp2cfg",
        native_options,
        Box::new(|cc| Ok(Box::new(Clp2cfgApp::new(cc)))),
    )
}

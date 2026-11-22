//! Exo for HEY — a fast, local-first, keyboard-driven native desktop client.

fn main() -> eframe::Result<()> {
    let _ = fastframe_log::Logging::new("Exo", env!("CARGO_PKG_VERSION"))
        .filter("info")
        .init();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Exo for HEY")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Exo for HEY",
        options,
        Box::new(|cc| Ok(Box::new(exo::ExoApp::new(cc)) as Box<dyn eframe::App>)),
    )
}

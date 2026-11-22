//! Exo for HEY — a fast, local-first, keyboard-driven native desktop client.

fn main() -> eframe::Result<()> {
    let _ = fastframe_log::Logging::new("Exo", env!("CARGO_PKG_VERSION"))
        .filter("info")
        .init();

    // The window draws Exo's own 41px tab strip under the native macOS traffic lights, so the
    // content runs behind a transparent titlebar. The default size is the original app's
    // 1280×800 canvas.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Exo for HEY")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([880.0, 600.0])
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false),
        ..Default::default()
    };

    eframe::run_native(
        "Exo for HEY",
        options,
        Box::new(|cc| Ok(Box::new(exo::ExoApp::new(cc)) as Box<dyn eframe::App>)),
    )
}

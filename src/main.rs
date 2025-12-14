mod app;

use app::TextViewerApp;
use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Large Text Viewer"),
        ..Default::default()
    };

    eframe::run_native(
        "Large Text Viewer",
        options,
        Box::new(|_cc| Ok(Box::new(TextViewerApp::default()))),
    )
}

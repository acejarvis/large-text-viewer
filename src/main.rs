mod app;

use app::TextViewerApp;
use eframe::egui;
use std::env;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = env::args().collect();
    let initial_file: Option<PathBuf> = if args.len() > 1 {
        Some(PathBuf::from(&args[1]))
    } else {
        None
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("Large Text Viewer"),
        ..Default::default()
    };

    eframe::run_native(
        "Large Text Viewer",
        options,
        Box::new(move |_cc| Ok(Box::new(TextViewerApp::new(initial_file.clone())))),
    )
}

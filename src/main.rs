mod ai;
mod analysis;
mod app;
mod folds;
mod help;
mod knowledge;
mod models;
mod providers;
mod eval;
mod storage;

pub(crate) const USER_AGENT: &str = "the-hedgehog/0.1.0-preview";

use eframe::egui;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_maximized(true),
        ..Default::default()
    };

    eframe::run_native(
        "The Hedgehog",
        native_options,
        Box::new(|cc| Ok(Box::new(app::DashboardApp::new(cc)))),
    )
}

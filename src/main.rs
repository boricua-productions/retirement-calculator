// ============================================================================
// Retirement Calculator — Rust/egui edition
// ============================================================================

use eframe::egui;
use retirement_calculator::ui::app::RetirementApp;

fn main() -> eframe::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp(None)
        .format_target(false)
        .init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Retirement Calculator")
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Retirement Calculator",
        native_options,
        Box::new(|_cc| Ok(Box::new(RetirementApp::default()))),
    )
}

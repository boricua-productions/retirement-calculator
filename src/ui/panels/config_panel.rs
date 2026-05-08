use egui::{CollapsingHeader, Ui};

/// Renders a collapsible left-side configuration panel.
/// This panel is informational for the current run — parameters are loaded from JSON.
/// A future version could allow direct editing of config fields here.
pub fn show(ui: &mut Ui, scenario_name: &Option<String>) {
    ui.heading("Configuration");
    ui.add_space(6.0);

    match scenario_name {
        Some(name) => {
            ui.label(format!("📄 Scenario: {}", name));
        }
        None => {
            ui.label(egui::RichText::new("No scenario loaded.").color(egui::Color32::GRAY));
        }
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);

    CollapsingHeader::new("ℹ Usage").default_open(true).show(ui, |ui| {
        ui.label("1. Click 'Open Scenario' to load a JSON config file.");
        ui.label("2. Click 'Run Simulation' to execute.");
        ui.label("3. View results in the tabs on the right.");
    });

    ui.add_space(6.0);

    CollapsingHeader::new("📋 JSON Schema Reminder").show(ui, |ui| {
        ui.label("Key sections in your scenario JSON:");
        ui.label("• simulation_settings — dates, economics, contributions");
        ui.label("• rsu_awards — grant date, ticker, shares, vesting schedule");
        ui.label("• market_prices_usd — manual price overrides (0 = auto-fallback)");
        ui.label("• holdings — taxable, roth_ira, japan_dc portfolios");
    });
}

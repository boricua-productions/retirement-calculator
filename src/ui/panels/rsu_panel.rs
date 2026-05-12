use egui::{Color32, RichText, ScrollArea, Ui};
use crate::engine::rsu_engine::RsuEngine;
use crate::models::snapshot::SimResults;

/// Renders the RSU vesting schedule and summary.
pub fn show(ui: &mut Ui, rsu_engine: &Option<RsuEngine>, results: &Option<SimResults>) {
    ui.heading("RSU Vesting Schedule");
    ui.add_space(6.0);

    let Some(engine) = rsu_engine else {
        ui.label(RichText::new("Load a scenario with RSU awards to see this panel.").color(Color32::GRAY));
        return;
    };

    let end_date = results.as_ref()
        .and_then(|r| r.annual_summary.last())
        .map(|s| {
            chrono::NaiveDate::from_ymd_opt(s.year, 12, 31).unwrap_or_default()
        })
        .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(2080, 12, 31).unwrap());

    // Summary of vested/unvested at end of simulation.
    let summary = engine.vested_and_unvested(end_date);
    if !summary.is_empty() {
        ui.label(RichText::new("End-of-Simulation RSU Status:").strong());
        egui::Grid::new("rsu_summary_grid")
            .num_columns(3)
            .spacing([30.0, 4.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Ticker").strong());
                ui.label(RichText::new("Vested Shares").strong());
                ui.label(RichText::new("Unvested Shares").strong());
                ui.end_row();

                for (ticker, status) in &summary {
                    ui.label(ticker);
                    ui.label(format!("{:.2}", status.vested));
                    ui.label(format!("{:.2}", status.unvested));
                    ui.end_row();
                }
            });
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
    }

    // Full vesting schedule.
    ui.label(RichText::new("Full Vesting Schedule:").strong());
    ui.add_space(4.0);

    ScrollArea::vertical().id_salt("rsu_schedule_scroll").show(ui, |ui| {
        egui::Grid::new("rsu_schedule_grid")
            .num_columns(3)
            .spacing([30.0, 3.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Date").strong());
                ui.label(RichText::new("Ticker").strong());
                ui.label(RichText::new("Shares").strong());
                ui.end_row();

                for event in &engine.vesting_schedule {
                    ui.label(event.date.to_string());
                    ui.label(&event.ticker);
                    ui.label(format!("{:.2}", event.shares));
                    ui.end_row();
                }
            });
    });
}

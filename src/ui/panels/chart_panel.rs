use egui::Ui;
use egui_plot::{Bar, BarChart, Legend, Line, Plot, PlotPoints};
use crate::models::snapshot::SimResults;

/// Renders the portfolio growth and cash flow gap charts using egui_plot.
pub fn show(ui: &mut Ui, results: &Option<SimResults>) {
    let Some(res) = results else {
        ui.label("Run a simulation to see charts.");
        return;
    };

    if res.annual_summary.is_empty() {
        ui.label("No data to chart.");
        return;
    }

    let available_height = ui.available_height();
    let chart_height = (available_height / 2.0 - 20.0).max(200.0);

    // ── Portfolio chart ────────────────────────────────────────────────────────
    ui.label(egui::RichText::new("Portfolio Value Over Time (USD)").strong());

    Plot::new("portfolio_chart")
        .height(chart_height)
        .legend(Legend::default())
        .x_axis_label("Year")
        .y_axis_label("USD")
        .show(ui, |plot_ui| {
            let brokerage: PlotPoints = res.annual_summary.iter()
                .map(|s| [s.year as f64, s.brokerage_usd])
                .collect();
            let roth: PlotPoints = res.annual_summary.iter()
                .map(|s| [s.year as f64, s.roth_usd])
                .collect();

            plot_ui.line(Line::new(brokerage).name("Taxable ($)").width(2.0));
            plot_ui.line(Line::new(roth).name("Roth IRA ($)").width(2.0));
        });

    ui.add_space(8.0);

    // ── Cash flow gap chart ────────────────────────────────────────────────────
    ui.label(egui::RichText::new("Annual Cash Flow Gap (JPY)").strong());

    Plot::new("gap_chart")
        .height(chart_height)
        .legend(Legend::default())
        .x_axis_label("Year")
        .y_axis_label("JPY")
        .show(ui, |plot_ui| {
            let bars: Vec<Bar> = res.annual_summary.iter()
                .map(|s| {
                    let color = if s.gap_jpy >= 0.0 {
                        egui::Color32::from_rgb(46, 160, 67)
                    } else {
                        egui::Color32::from_rgb(200, 50, 50)
                    };
                    Bar::new(s.year as f64, s.gap_jpy).fill(color).width(0.8)
                })
                .collect();

            plot_ui.bar_chart(BarChart::new(bars).name("Gap (¥)"));

            // Zero line reference.
            let zero_line: PlotPoints = {
                let first = res.annual_summary.first().map(|s| s.year as f64).unwrap_or(2031.0);
                let last = res.annual_summary.last().map(|s| s.year as f64).unwrap_or(2080.0);
                vec![[first, 0.0], [last, 0.0]].into_iter().collect()
            };
            plot_ui.line(
                Line::new(zero_line)
                    .color(egui::Color32::WHITE)
                    .width(1.0)
                    .name("Break-even"),
            );
        });
}

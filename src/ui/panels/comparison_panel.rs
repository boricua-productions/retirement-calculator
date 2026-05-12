use egui::{Color32, RichText, Ui};
use crate::models::snapshot::SimResults;
use crate::simulation::monte_carlo::MarcoPoloResults;

fn fmt_usd(v: f64) -> String { format!("${:.0}", v) }
fn fmt_jpy(v: f64) -> String { format!("¥{:.0}", v) }
fn fmt_pct(v: f64) -> String { format!("{:.1}%", v * 100.0) }

/// Renders the Comparison tab: side-by-side Baseline vs Comparison metrics,
/// plus the Marco Polo (Monte Carlo) P10/P50/P90 trajectories when available.
pub fn show(
    ui: &mut Ui,
    baseline: &Option<SimResults>,
    comparison: &Option<SimResults>,
    marco_polo: &Option<MarcoPoloResults>,
) {
    ui.push_id("comparison_view", |ui| {
        show_inner(ui, baseline, comparison, marco_polo);
    });
}

fn show_inner(
    ui: &mut Ui,
    baseline: &Option<SimResults>,
    comparison: &Option<SimResults>,
    marco_polo: &Option<MarcoPoloResults>,
) {
    // ── Marco Polo section (always shown when data exists) ────────────────────
    if let Some(mp) = marco_polo {
        ui.heading("🎲 Marco Polo — Monte Carlo Results");
        ui.label(
            RichText::new(format!(
                "{} iterations · μ = {} · σ = {}",
                mp.iterations,
                fmt_pct(mp.mean_return),
                fmt_pct(mp.volatility),
            ))
            .small()
            .color(Color32::GRAY),
        );
        ui.add_space(8.0);

        // Table: Year | P10 | P50 | P90
        egui::Grid::new("mp_grid")
            .num_columns(4)
            .spacing([24.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label(RichText::new("Year").strong());
                ui.label(RichText::new("P10 — Worst Case").strong().color(Color32::from_rgb(220, 100, 100)));
                ui.label(RichText::new("P50 — Median").strong().color(Color32::from_rgb(100, 180, 220)));
                ui.label(RichText::new("P90 — Best Case").strong().color(Color32::from_rgb(100, 220, 100)));
                ui.end_row();

                for i in 0..mp.years.len() {
                    ui.label(format!("{}", mp.years[i]));
                    ui.label(RichText::new(fmt_usd(mp.p10[i])).color(Color32::from_rgb(220, 100, 100)));
                    ui.label(RichText::new(fmt_usd(mp.p50[i])).color(Color32::from_rgb(100, 180, 220)));
                    ui.label(RichText::new(fmt_usd(mp.p90[i])).color(Color32::from_rgb(100, 220, 100)));
                    ui.end_row();
                }
            });

        ui.add_space(16.0);
        ui.separator();
        ui.add_space(8.0);
    }

    // ── Scenario comparison section ────────────────────────────────────────────
    ui.heading("🔀 Scenario Comparison");

    match (baseline, comparison) {
        (None, None) => {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(
                        "Run a Baseline simulation first.\n\
                         Then load a Comparison scenario via '📂 Open Comparison'."
                    )
                    .color(Color32::GRAY),
                );
            });
            return;
        }
        (Some(_), None) => {
            ui.label(
                RichText::new("Load a Comparison scenario via '📂 Open Comparison' and run it.")
                    .color(Color32::GRAY),
            );
            ui.add_space(8.0);
        }
        _ => {}
    }

    let col_w = 200.0;

    egui::Grid::new("cmp_grid")
        .num_columns(3)
        .spacing([col_w * 0.1, 6.0])
        .striped(true)
        .show(ui, |ui| {
            // Header
            ui.label("");
            ui.label(RichText::new("Baseline").strong().color(Color32::from_rgb(100, 180, 220)));
            ui.label(RichText::new("Comparison").strong().color(Color32::from_rgb(220, 160, 60)));
            ui.end_row();

            let row = |ui: &mut Ui, label: &str, b_val: String, c_val: String| {
                ui.label(RichText::new(label).strong());
                ui.label(b_val);
                ui.label(c_val);
                ui.end_row();
            };

            let snap_b = baseline.as_ref().and_then(|r| r.annual_summary.last());
            let snap_c = comparison.as_ref().and_then(|r| r.annual_summary.last());

            let get_b = |f: &dyn Fn(&crate::models::snapshot::AnnualSnapshot) -> String| -> String {
                snap_b.map(f).unwrap_or_else(|| "—".into())
            };
            let get_c = |f: &dyn Fn(&crate::models::snapshot::AnnualSnapshot) -> String| -> String {
                snap_c.map(f).unwrap_or_else(|| "—".into())
            };

            row(ui, "Simulation Years",
                baseline.as_ref().map(|r| r.annual_summary.len().to_string()).unwrap_or_else(|| "—".into()),
                comparison.as_ref().map(|r| r.annual_summary.len().to_string()).unwrap_or_else(|| "—".into()),
            );
            row(ui, "Final Year",
                get_b(&|s| s.year.to_string()),
                get_c(&|s| s.year.to_string()),
            );
            row(ui, "Ending Taxable Portfolio",
                get_b(&|s| fmt_usd(s.brokerage_usd)),
                get_c(&|s| fmt_usd(s.brokerage_usd)),
            );
            row(ui, "Ending Roth IRA",
                get_b(&|s| fmt_usd(s.roth_usd)),
                get_c(&|s| fmt_usd(s.roth_usd)),
            );
            row(ui, "Ending DC Plan",
                get_b(&|s| fmt_jpy(s.dc_jpy)),
                get_c(&|s| fmt_jpy(s.dc_jpy)),
            );
            row(ui, "Final FX Rate",
                get_b(&|s| format!("{:.2} ¥/$", s.usd_jpy)),
                get_c(&|s| format!("{:.2} ¥/$", s.usd_jpy)),
            );

            // Ending wealth (taxable + roth, in USD)
            let ew_b = snap_b.map(|s| s.brokerage_usd + s.roth_usd).unwrap_or(0.0);
            let ew_c = snap_c.map(|s| s.brokerage_usd + s.roth_usd).unwrap_or(0.0);
            let diff = ew_c - ew_b;
            let diff_color = if diff >= 0.0 { Color32::from_rgb(100, 220, 100) } else { Color32::from_rgb(220, 100, 100) };

            ui.label(RichText::new("Ending Wealth (USD)").strong());
            ui.label(fmt_usd(ew_b));
            if comparison.is_some() {
                ui.label(RichText::new(format!("{} ({:+.0})", fmt_usd(ew_c), diff)).color(diff_color));
            } else {
                ui.label("—");
            }
            ui.end_row();

            // Cumulative NHI paid
            let nhi_b: f64 = baseline.as_ref()
                .map(|r| r.annual_summary.iter().map(|s| s.nhi_obligation_jpy).sum())
                .unwrap_or(0.0);
            let nhi_c: f64 = comparison.as_ref()
                .map(|r| r.annual_summary.iter().map(|s| s.nhi_obligation_jpy).sum())
                .unwrap_or(0.0);
            row(ui, "Total NHI Paid",
                if baseline.is_some() { fmt_jpy(nhi_b) } else { "—".into() },
                if comparison.is_some() { fmt_jpy(nhi_c) } else { "—".into() },
            );

            // Solvency warnings
            row(ui, "Solvency Warnings",
                baseline.as_ref().map(|r| r.gap_warnings.len().to_string()).unwrap_or_else(|| "—".into()),
                comparison.as_ref().map(|r| r.gap_warnings.len().to_string()).unwrap_or_else(|| "—".into()),
            );
        });
}

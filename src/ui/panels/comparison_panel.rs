use egui::{Color32, RichText, Ui};
use crate::models::snapshot::{AnnualSnapshot, SimResults};
use crate::simulation::monte_carlo::MarcoPoloResults;

fn fmt_usd(v: f64) -> String { format!("${:.0}", v) }
fn fmt_jpy(v: f64) -> String { format!("¥{:.0}", v) }
fn fmt_pct(v: f64) -> String { format!("{:.1}%", v * 100.0) }
fn fmt_x(v: f64)   -> String { format!("{:.2}×", v) }

fn total_wealth_usd(s: &AnnualSnapshot) -> f64 {
    s.brokerage_usd + s.roth_usd + s.dc_jpy / s.usd_jpy.max(1.0)
}

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

fn section_label(ui: &mut Ui, label: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(label).strong().color(Color32::from_rgb(160, 200, 240)));
    ui.label("");
    ui.label("");
    ui.end_row();
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

    // ── Pre-compute derived metrics ───────────────────────────────────────────
    let snap_b = baseline.as_ref().and_then(|r| r.annual_summary.last());
    let snap_c = comparison.as_ref().and_then(|r| r.annual_summary.last());

    let verdict_b = baseline.as_ref().map(|r| r.retirement_verdict());
    let verdict_c = comparison.as_ref().map(|r| r.retirement_verdict());

    // Average dividend coverage ratio (years where ratio > 0)
    let avg_dcr = |res: &SimResults| -> f64 {
        let v: Vec<f64> = res.annual_summary.iter()
            .filter(|s| s.div_coverage_ratio > 0.0)
            .map(|s| s.div_coverage_ratio)
            .collect();
        if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
    };

    // Cumulative US federal + state tax (USD)
    let cum_us_tax = |res: &SimResults| -> f64 {
        res.annual_summary.iter().map(|s| s.us_tax_charged_usd).sum()
    };

    // Cumulative Japan resident tax + NHI (JPY)
    let cum_jp_tax = |res: &SimResults| -> f64 {
        res.annual_summary.iter()
            .map(|s| s.japan_tax_charged_jpy + s.nhi_obligation_jpy)
            .sum()
    };

    // Worst single-year wealth drawdown (USD, negative = loss)
    let worst_drawdown = |res: &SimResults| -> f64 {
        res.annual_summary.windows(2).map(|w| {
            total_wealth_usd(&w[1]) - total_wealth_usd(&w[0])
        }).fold(0.0_f64, f64::min)
    };

    // Years of expense coverage at final year (final_wealth / annual_base_expenses)
    let years_headroom = |res: &SimResults| -> f64 {
        let last = match res.annual_summary.last() { Some(s) => s, None => return 0.0 };
        let fx = last.usd_jpy.max(1.0);
        let annual_exp_usd = last.base_exp_jpy * 12.0 / fx;
        if annual_exp_usd < 1.0 { return 0.0; }
        total_wealth_usd(last) / annual_exp_usd
    };

    // ── Comparison grid ────────────────────────────────────────────────────────
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

            let dash = || "—".to_string();

            // ── Plan Health ──────────────────────────────────────────────────
            section_label(ui, "Plan Health");

            // Verdict
            let render_verdict = |ui: &mut Ui, vopt: &Option<crate::models::snapshot::RetirementVerdict>| {
                match vopt {
                    None => { ui.label("—"); }
                    Some(v) => {
                        let ((_, _, _), (fg_r, fg_g, fg_b)) = v.tier.colors();
                        let color = Color32::from_rgb(fg_r, fg_g, fg_b);
                        ui.label(RichText::new(
                            format!("{} {} — {}", v.tier.icon(), v.tier.label(), v.summary))
                            .color(color));
                    }
                }
            };
            ui.label(RichText::new("Verdict").strong());
            render_verdict(ui, &verdict_b);
            render_verdict(ui, &verdict_c);
            ui.end_row();

            row(ui, "Simulation Years",
                baseline.as_ref().map(|r| r.annual_summary.len().to_string()).unwrap_or_else(dash),
                comparison.as_ref().map(|r| r.annual_summary.len().to_string()).unwrap_or_else(dash),
            );
            row(ui, "Final Year",
                snap_b.map(|s| s.year.to_string()).unwrap_or_else(dash),
                snap_c.map(|s| s.year.to_string()).unwrap_or_else(dash),
            );

            // Solvency warnings count
            let warn_b = baseline.as_ref().map(|r| r.gap_warnings.len()).unwrap_or(0);
            let warn_c = comparison.as_ref().map(|r| r.gap_warnings.len()).unwrap_or(0);
            row(ui, "Solvency Warnings",
                baseline.as_ref().map(|_| warn_b.to_string()).unwrap_or_else(dash),
                comparison.as_ref().map(|_| warn_c.to_string()).unwrap_or_else(dash),
            );

            // ── Portfolio ────────────────────────────────────────────────────
            section_label(ui, "Portfolio (End of Horizon)");

            row(ui, "Ending Taxable Portfolio",
                snap_b.map(|s| fmt_usd(s.brokerage_usd)).unwrap_or_else(dash),
                snap_c.map(|s| fmt_usd(s.brokerage_usd)).unwrap_or_else(dash),
            );
            row(ui, "Ending Roth IRA",
                snap_b.map(|s| fmt_usd(s.roth_usd)).unwrap_or_else(dash),
                snap_c.map(|s| fmt_usd(s.roth_usd)).unwrap_or_else(dash),
            );
            row(ui, "Ending DC Plan",
                snap_b.map(|s| fmt_jpy(s.dc_jpy)).unwrap_or_else(dash),
                snap_c.map(|s| fmt_jpy(s.dc_jpy)).unwrap_or_else(dash),
            );
            row(ui, "Final FX Rate",
                snap_b.map(|s| format!("{:.2} ¥/$", s.usd_jpy)).unwrap_or_else(dash),
                snap_c.map(|s| format!("{:.2} ¥/$", s.usd_jpy)).unwrap_or_else(dash),
            );

            // Ending wealth with diff color
            let ew_b = snap_b.map(|s| total_wealth_usd(s)).unwrap_or(0.0);
            let ew_c = snap_c.map(|s| total_wealth_usd(s)).unwrap_or(0.0);
            let diff = ew_c - ew_b;
            let diff_color = if diff >= 0.0 {
                Color32::from_rgb(100, 220, 100)
            } else {
                Color32::from_rgb(220, 100, 100)
            };

            ui.label(RichText::new("Ending Wealth (USD-equiv)").strong());
            ui.label(if baseline.is_some() { fmt_usd(ew_b) } else { "—".into() });
            if comparison.is_some() {
                ui.label(RichText::new(format!("{} ({:+.0})", fmt_usd(ew_c), diff)).color(diff_color));
            } else {
                ui.label("—");
            }
            ui.end_row();

            // Years of expense headroom
            ui.label(RichText::new("Years of Expense Headroom").strong());
            ui.label(baseline.as_ref().map(|r| format!("{:.1} yr", years_headroom(r))).unwrap_or_else(dash));
            if comparison.is_some() {
                let yh_b = baseline.as_ref().map(years_headroom).unwrap_or(0.0);
                let yh_c = comparison.as_ref().map(years_headroom).unwrap_or(0.0);
                let yh_diff = yh_c - yh_b;
                let col = if yh_diff >= 0.0 { Color32::from_rgb(100, 220, 100) } else { Color32::from_rgb(220, 100, 100) };
                ui.label(RichText::new(format!("{:.1} yr ({:+.1})", yh_c, yh_diff)).color(col));
            } else {
                ui.label(comparison.as_ref().map(|r| format!("{:.1} yr", years_headroom(r))).unwrap_or_else(dash));
            }
            ui.end_row();

            // ── Cash Flow ────────────────────────────────────────────────────
            section_label(ui, "Cash Flow");

            // Average div coverage
            ui.label(RichText::new("Avg Dividend Coverage").strong());
            ui.label(baseline.as_ref().map(|r| fmt_x(avg_dcr(r))).unwrap_or_else(dash));
            if comparison.is_some() {
                let dc_b = baseline.as_ref().map(|r| avg_dcr(r)).unwrap_or(0.0);
                let dc_c = comparison.as_ref().map(|r| avg_dcr(r)).unwrap_or(0.0);
                let col = if dc_c >= dc_b { Color32::from_rgb(100, 220, 100) } else { Color32::from_rgb(220, 100, 100) };
                ui.label(RichText::new(format!("{} ({:+.2}×)", fmt_x(dc_c), dc_c - dc_b)).color(col));
            } else {
                ui.label(comparison.as_ref().map(|r| fmt_x(avg_dcr(r))).unwrap_or_else(dash));
            }
            ui.end_row();

            // Worst-year drawdown
            ui.label(RichText::new("Worst-Year Drawdown (USD)").strong());
            ui.label(baseline.as_ref().map(|r| fmt_usd(worst_drawdown(r))).unwrap_or_else(dash));
            if comparison.is_some() {
                let wd_b = baseline.as_ref().map(|r| worst_drawdown(r)).unwrap_or(0.0);
                let wd_c = comparison.as_ref().map(|r| worst_drawdown(r)).unwrap_or(0.0);
                // Less negative = better
                let col = if wd_c >= wd_b { Color32::from_rgb(100, 220, 100) } else { Color32::from_rgb(220, 100, 100) };
                ui.label(RichText::new(format!("{} ({:+.0})", fmt_usd(wd_c), wd_c - wd_b)).color(col));
            } else {
                ui.label(comparison.as_ref().map(|r| fmt_usd(worst_drawdown(r))).unwrap_or_else(dash));
            }
            ui.end_row();

            // ── Tax Drag ─────────────────────────────────────────────────────
            section_label(ui, "Cumulative Tax Drag");

            // US tax
            ui.label(RichText::new("US Federal + State Tax (USD)").strong());
            ui.label(baseline.as_ref().map(|r| fmt_usd(cum_us_tax(r))).unwrap_or_else(dash));
            if comparison.is_some() {
                let t_b = baseline.as_ref().map(|r| cum_us_tax(r)).unwrap_or(0.0);
                let t_c = comparison.as_ref().map(|r| cum_us_tax(r)).unwrap_or(0.0);
                let col = if t_c <= t_b { Color32::from_rgb(100, 220, 100) } else { Color32::from_rgb(220, 100, 100) };
                ui.label(RichText::new(format!("{} ({:+.0})", fmt_usd(t_c), t_c - t_b)).color(col));
            } else {
                ui.label(comparison.as_ref().map(|r| fmt_usd(cum_us_tax(r))).unwrap_or_else(dash));
            }
            ui.end_row();

            // Japan tax + NHI
            ui.label(RichText::new("Japan Resident Tax + NHI (JPY)").strong());
            ui.label(baseline.as_ref().map(|r| fmt_jpy(cum_jp_tax(r))).unwrap_or_else(dash));
            if comparison.is_some() {
                let t_b = baseline.as_ref().map(|r| cum_jp_tax(r)).unwrap_or(0.0);
                let t_c = comparison.as_ref().map(|r| cum_jp_tax(r)).unwrap_or(0.0);
                let col = if t_c <= t_b { Color32::from_rgb(100, 220, 100) } else { Color32::from_rgb(220, 100, 100) };
                ui.label(RichText::new(format!("{} ({:+.0})", fmt_jpy(t_c), t_c - t_b)).color(col));
            } else {
                ui.label(comparison.as_ref().map(|r| fmt_jpy(cum_jp_tax(r))).unwrap_or_else(dash));
            }
            ui.end_row();

            // Total NHI (kept for backward compat)
            let nhi_b: f64 = baseline.as_ref()
                .map(|r| r.annual_summary.iter().map(|s| s.nhi_obligation_jpy).sum())
                .unwrap_or(0.0);
            let nhi_c: f64 = comparison.as_ref()
                .map(|r| r.annual_summary.iter().map(|s| s.nhi_obligation_jpy).sum())
                .unwrap_or(0.0);
            row(ui, "  of which: NHI (JPY)",
                if baseline.is_some() { fmt_jpy(nhi_b) } else { "—".into() },
                if comparison.is_some() { fmt_jpy(nhi_c) } else { "—".into() },
            );
        });

    // ── Solvency warnings expandable lists (outside the grid) ─────────────────
    ui.add_space(12.0);
    let show_warnings = |ui: &mut Ui, label: &str, res: &SimResults, color: Color32| {
        // V8.7 — Show only real cash shortfalls; exclude DcCapExceeded / notices.
        let cash_gaps: Vec<_> = res.gap_warnings.iter().filter(|w| w.is_cash_gap()).collect();
        if cash_gaps.is_empty() { return; }
        egui::CollapsingHeader::new(
            RichText::new(format!("{} ⚠ {} solvency warning(s)", label, cash_gaps.len()))
                .color(color)
        )
        .id_salt(format!("cmp_warn_{}", label))
        .show(ui, |ui| {
            for (i, w) in cash_gaps.iter().enumerate().take(10) {
                ui.label(RichText::new(format!(
                    "{}. {} — gap ¥{:.0} (absorbed by: {})",
                    i + 1, w.date, w.gap_jpy.abs(), w.absorbed_by
                )).small().color(Color32::from_rgb(255, 200, 80)));
            }
            if cash_gaps.len() > 10 {
                ui.label(RichText::new(format!(
                    "… and {} more (see Annual Table for full detail)",
                    cash_gaps.len() - 10
                )).small().color(Color32::GRAY));
            }
        });
    };

    if let Some(b) = baseline.as_ref() {
        show_warnings(ui, "Baseline", b, Color32::from_rgb(100, 180, 220));
    }
    if let Some(c) = comparison.as_ref() {
        show_warnings(ui, "Comparison", c, Color32::from_rgb(220, 160, 60));
    }
}

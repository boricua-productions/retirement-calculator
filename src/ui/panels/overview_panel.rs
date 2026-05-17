use std::path::Path;
use std::time::{Duration, Instant};

use egui::{Color32, RichText, Ui};

use crate::engine::rsu_engine::RsuEngine;
use crate::models::snapshot::SimResults;
use crate::reporter;

const STATUS_ID: &str = "overview_export_status";
const STATUS_TTL: Duration = Duration::from_secs(4);

#[derive(Clone)]
struct ExportStatus {
    message: String,
    is_success: bool,
    when: Instant,
}

/// Renders the Overview tab: key metrics, solvency status, and export controls.
pub fn show(ui: &mut Ui, results: &Option<SimResults>, rsu_engine: &Option<RsuEngine>) {
    let Some(res) = results else {
        ui.centered_and_justified(|ui| {
            ui.label(RichText::new("Run a simulation to see results.").color(Color32::GRAY));
        });
        return;
    };

    ui.heading("Simulation Overview");
    ui.add_space(8.0);

    egui::Grid::new("overview_grid")
        .num_columns(2)
        .spacing([40.0, 6.0])
        .show(ui, |ui| {
            let years = res.annual_summary.len();
            let last = res.annual_summary.last();

            ui.label(RichText::new("Years Simulated:").strong());
            ui.label(format!("{}", years));
            ui.end_row();

            if let Some(snap) = last {
                ui.label(RichText::new("Final Year:").strong());
                ui.label(format!("{}", snap.year));
                ui.end_row();

                ui.label(RichText::new("Final Taxable Portfolio:").strong());
                ui.label(fmt_usd(snap.brokerage_usd));
                ui.end_row();

                ui.label(RichText::new("Final Roth IRA:").strong());
                ui.label(fmt_usd(snap.roth_usd));
                ui.end_row();

                ui.label(RichText::new("Final Japan DC:").strong());
                ui.label(fmt_jpy(snap.dc_jpy));
                ui.end_row();

                ui.label(RichText::new("Final Exchange Rate:").strong());
                ui.label(format!("{:.2} JPY/USD", snap.usd_jpy));
                ui.end_row();
            }

            // Stage 02 — Show effective filing status so user can confirm NRA profile was applied.
            ui.label(RichText::new("Effective Filing Status:").strong());
            ui.label(RichText::new(&res.effective_filing_status)
                .color(Color32::from_rgb(180, 220, 255)));
            ui.end_row();

            let gap_warnings = res.gap_warnings.len();
            ui.label(RichText::new("Solvency Warnings:").strong());
            if gap_warnings == 0 {
                ui.label(RichText::new("✅ None — fully solvent").color(Color32::GREEN));
            } else {
                ui.label(
                    RichText::new(format!("⚠ {} quarters with negative cash flow", gap_warnings))
                        .color(Color32::YELLOW),
                );
            }
            ui.end_row();

            // V7.7.2 — RSU margin-call banner.
            let unpaid_rsu = res.annual_summary.last()
                .map(|s| s.unpaid_rsu_tax_liability_usd)
                .unwrap_or(0.0);
            if !res.rsu_sell_to_cover_warnings.is_empty() || unpaid_rsu > 0.0 {
                ui.label(RichText::new("RSU Tax Shortfall:").strong());
                ui.label(
                    RichText::new(format!(
                        "🔴 {} margin call(s) — ${:.0} unpaid IRS liability",
                        res.rsu_sell_to_cover_warnings.len(),
                        unpaid_rsu,
                    ))
                    .color(Color32::RED),
                );
                ui.end_row();
            }

            // Stage 05 — PFIC basis drift banner.
            let total_pfic_mtm: f64 = res.annual_summary.iter().map(|s| s.pfic_mtm_income_usd).sum();
            if total_pfic_mtm > 0.0 || !res.pfic_basis_drift_warnings.is_empty() {
                ui.label(RichText::new("PFIC §1296 MTM Drag:").strong());
                let drift_msg = if res.pfic_basis_drift_warnings.is_empty() {
                    format!("${:.0} lifetime phantom income", total_pfic_mtm)
                } else {
                    format!(
                        "${:.0} lifetime phantom income — {} drift event(s) self-healed",
                        total_pfic_mtm,
                        res.pfic_basis_drift_warnings.len(),
                    )
                };
                let color = if res.pfic_basis_drift_warnings.is_empty() {
                    Color32::YELLOW
                } else {
                    Color32::from_rgb(255, 165, 0)
                };
                ui.label(RichText::new(drift_msg).color(color));
                ui.end_row();
            }

            // Stage 04 — Show pre/post shock net-worth rows for any shock year.
            let shock_years: Vec<_> = res.annual_summary.iter()
                .filter(|s| s.pre_shock_net_worth_jpy.is_some())
                .collect();
            for snap in &shock_years {
                if let (Some(pre), Some(post)) = (snap.pre_shock_net_worth_jpy, snap.post_shock_net_worth_jpy) {
                    ui.label(RichText::new(format!("{} Dual-Shock:", snap.year)).strong().color(Color32::YELLOW));
                    ui.label(RichText::new(format!("Pre ¥{:.0} → Post ¥{:.0}", pre, post))
                        .color(Color32::YELLOW));
                    ui.end_row();
                }
            }

            ui.label(RichText::new("Tax Jurisdiction:").strong());
            ui.label(res.tax_jurisdiction.to_string());
            ui.end_row();

            ui.label(RichText::new("Investment Location:").strong());
            ui.label(res.investment_location.to_string());
            ui.end_row();

            if let Some(t) = &res.transition_report {
                ui.label(RichText::new("Retirement Transition:").strong());
                ui.label(format!(
                    "{} | Pre: {} → Post: {}",
                    t.date,
                    fmt_usd(t.pre_val),
                    fmt_usd(t.post_val)
                ));
                ui.end_row();
            }
        });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);

    let positive_gaps = res.annual_summary.iter().filter(|s| s.gap_jpy >= 0.0).count();
    let negative_gaps = res.annual_summary.iter().filter(|s| s.gap_jpy < 0.0).count();
    let ret_year = res.annual_summary.first().map(|s| s.year).unwrap_or(0);

    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("✅ Surplus years: {}", positive_gaps)).color(Color32::GREEN));
        ui.add_space(16.0);
        ui.label(RichText::new(format!("❌ Deficit years: {}", negative_gaps)).color(Color32::RED));
        ui.add_space(16.0);
        ui.label(format!("Simulation starts: {}", ret_year));
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);

    // ── Japan tax summary ─────────────────────────────────────────────────────
    let total_restax: f64 = res.annual_summary.iter().map(|s| s.res_tax_jpy).sum();
    let total_nhi: f64 = res.annual_summary.iter().map(|s| s.nhi_obligation_jpy).sum();
    let total_rsu: f64 = res.annual_summary.iter().map(|s| s.rsu_vest_usd).sum();

    egui::Grid::new("overview_tax_grid")
        .num_columns(2)
        .spacing([40.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Total Resident Tax Paid:").strong());
            ui.label(fmt_jpy(total_restax));
            ui.end_row();

            ui.label(RichText::new("Total NHI Paid:").strong());
            ui.label(fmt_jpy(total_nhi));
            ui.end_row();

            ui.label(RichText::new("Total RSU Income:").strong());
            ui.label(fmt_usd(total_rsu));
            ui.end_row();

            // Dividend Coverage Ratio — average over post-retirement years (V6.0)
            let post_ret: Vec<_> = res.annual_summary.iter()
                .filter(|s| s.div_coverage_ratio > 0.0)
                .collect();
            if !post_ret.is_empty() {
                let avg_dcr = post_ret.iter().map(|s| s.div_coverage_ratio).sum::<f64>()
                    / post_ret.len() as f64;
                let dcr_color = if avg_dcr >= 1.0 {
                    Color32::from_rgb(100, 220, 100)
                } else if avg_dcr >= 0.5 {
                    Color32::YELLOW
                } else {
                    Color32::from_rgb(220, 100, 100)
                };
                ui.label(RichText::new("Avg Dividend Coverage Ratio:").strong());
                ui.label(RichText::new(format!("{:.2}×", avg_dcr)).color(dcr_color));
                ui.end_row();
            }
        });

    ui.add_space(14.0);
    ui.separator();
    ui.add_space(8.0);

    // ── Reporting & Export ────────────────────────────────────────────────────
    ui.label(RichText::new("Reporting & Export").strong().size(16.0));
    ui.add_space(6.0);

    let can_export = rsu_engine.is_some();
    let status_id = egui::Id::new(STATUS_ID);

    ui.horizontal(|ui| {
        // 💾 Export Text Report — user picks file name and location
        let text_btn = ui.add_enabled(can_export, egui::Button::new("💾 Export Text Report"));
        if text_btn.clicked() {
            if let Some(engine) = rsu_engine {
                let picked = rfd::FileDialog::new()
                    .set_title("Save Text Report")
                    .set_file_name("Retirement_Summary.txt")
                    .add_filter("Text Files", &["txt"])
                    .add_filter("All Files", &["*"])
                    .save_file();

                if let Some(path) = picked {
                    let status = match write_text_report_to(res, engine, &path) {
                        Ok(()) => ExportStatus {
                            message: format!(
                                "✅ Saved → {}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            ),
                            is_success: true,
                            when: Instant::now(),
                        },
                        Err(e) => ExportStatus {
                            message: format!("❌ Export failed: {}", e),
                            is_success: false,
                            when: Instant::now(),
                        },
                    };
                    ui.ctx().data_mut(|d| d.insert_temp(status_id, status));
                }
            }
        }

        ui.add_space(8.0);

        // 📊 Export Audit CSV — user picks file name and location
        let csv_btn = ui.add_enabled(can_export, egui::Button::new("📊 Export Audit CSV"));
        if csv_btn.clicked() {
            let picked = rfd::FileDialog::new()
                .set_title("Save Audit CSV")
                .set_file_name("simulation_audit.csv")
                .add_filter("CSV Files", &["csv"])
                .add_filter("All Files", &["*"])
                .save_file();

            if let Some(path) = picked {
                let status = match write_audit_csv_to(res, &path) {
                    Ok(()) => ExportStatus {
                        message: format!(
                            "✅ Saved → {}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        is_success: true,
                        when: Instant::now(),
                    },
                    Err(e) => ExportStatus {
                        message: format!("❌ CSV export failed: {}", e),
                        is_success: false,
                        when: Instant::now(),
                    },
                };
                ui.ctx().data_mut(|d| d.insert_temp(status_id, status));
            }
        }

        ui.add_space(8.0);

        // 📋 Copy to Clipboard
        let clip_btn = ui.add_enabled(can_export, egui::Button::new("📋 Copy to Clipboard"));
        if clip_btn.clicked() {
            let text = reporter::format_clipboard_text(res, rsu_engine.as_ref());
            ui.ctx().copy_text(text);
            let status = ExportStatus {
                message: "✅ Summary copied to clipboard!".into(),
                is_success: true,
                when: Instant::now(),
            };
            ui.ctx().data_mut(|d| d.insert_temp(status_id, status));
        }
    });

    // ── Feedback label (success = green, error = red, auto-expires) ───────────
    let maybe_status = ui.ctx().data(|d| d.get_temp::<ExportStatus>(status_id));
    if let Some(status) = maybe_status {
        if status.when.elapsed() < STATUS_TTL {
            ui.add_space(4.0);
            let color = if status.is_success { Color32::GREEN } else { Color32::RED };
            ui.label(RichText::new(&status.message).color(color));
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
    }

    ui.add_space(4.0);
    ui.label(
        RichText::new("Reports are also auto-saved to output/ after every simulation run.")
            .small()
            .color(Color32::GRAY),
    );
}

// ── File I/O helpers ──────────────────────────────────────────────────────────

fn write_text_report_to(
    results: &SimResults,
    rsu_engine: &RsuEngine,
    path: &Path,
) -> std::io::Result<()> {
    let text = reporter::format_text_report(results, rsu_engine);
    std::fs::write(path, text)?;
    Ok(())
}

fn write_audit_csv_to(results: &SimResults, path: &Path) -> std::io::Result<()> {
    let csv = reporter::format_csv(results);
    std::fs::write(path, csv)?;
    Ok(())
}

// ── Number formatting ─────────────────────────────────────────────────────────

fn fmt_usd(v: f64) -> String {
    format!("${}", add_commas(v as i64))
}

fn fmt_jpy(v: f64) -> String {
    format!("¥{}", add_commas(v as i64))
}

fn add_commas(n: i64) -> String {
    let sign = if n < 0 { "-" } else { "" };
    let s = format!("{}", n.unsigned_abs());
    let mut result = String::new();
    let len = s.len();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(c);
    }
    format!("{}{}", sign, result)
}

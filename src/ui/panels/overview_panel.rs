use std::path::Path;
use std::time::{Duration, Instant};

use egui::{Color32, RichText, Ui};

use crate::engine::rsu_engine::RsuEngine;
use crate::models::assets::AccountLocation;
use crate::models::snapshot::{AccountSnapshotEvent, SimResults};
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

    // ── Year-range helpers (used throughout the panel) ────────────────────────
    let first_year = res.annual_summary.first().map(|s| s.year).unwrap_or(0);
    let last_year  = res.annual_summary.last().map(|s| s.year).unwrap_or(0);
    let range = format!("{}–{}", first_year, last_year);

    ui.heading("Simulation Overview");
    ui.add_space(8.0);

    // ── Task (g) — Plain-English retirement verdict ───────────────────────────
    let verdict = evaluate_scenario(res);
    let (bg, fg) = if verdict.works {
        (Color32::from_rgb(20, 60, 30), Color32::from_rgb(170, 255, 180))
    } else {
        (Color32::from_rgb(70, 25, 25), Color32::from_rgb(255, 180, 180))
    };
    egui::Frame::none()
        .fill(bg)
        .inner_margin(egui::Margin::symmetric(12.0, 10.0))
        .rounding(egui::Rounding::same(6.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(if verdict.works { "✅ Retirement Verdict" } else { "❌ Retirement Verdict" })
                    .strong().size(18.0).color(fg),
            );
            ui.label(RichText::new(&verdict.summary).size(15.0).color(fg));
            ui.add_space(6.0);
            ui.label(RichText::new("Why:").strong().color(fg));
            for r in &verdict.reasons {
                ui.label(RichText::new(format!("  • {}", r)).color(fg));
            }
            if !verdict.recommendations.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new("Recommendations:").strong().color(fg));
                for rec in &verdict.recommendations {
                    ui.label(RichText::new(format!("  → {}", rec)).color(fg));
                }
            }
        });
    ui.add_space(12.0);

    // ── Key metrics grid ──────────────────────────────────────────────────────
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

            // Task (c) — Effective filing status with country flags
            ui.label(RichText::new("Effective US Filing Status:").strong());
            ui.label(
                RichText::new(format!("🇺🇸 {}", res.effective_filing_status))
                    .color(Color32::from_rgb(180, 220, 255)),
            );
            ui.end_row();

            ui.label(RichText::new("Japan Tax Profile:").strong());
            ui.label(
                RichText::new(format!(
                    "🇯🇵 Permanent resident — Prefecture: {} / City: {}",
                    res.prefecture, res.city
                ))
                .color(Color32::from_rgb(180, 255, 200)),
            );
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

    // ── Task (b) — Cash-flow summary with plain-English definitions ───────────
    let positive_gaps = res.annual_summary.iter().filter(|s| s.gap_jpy >= 0.0).count();
    let negative_gaps = res.annual_summary.iter().filter(|s| s.gap_jpy < 0.0).count();

    ui.label(RichText::new("Cash-flow Summary").strong().size(15.0));
    ui.add_space(2.0);
    ui.label(
        RichText::new(
            "A surplus year is one where total net income (after tax) exceeded total expenses — \
             the portfolio grew or stayed flat. A deficit year is one where expenses exceeded \
             income, meaning savings/buffers had to absorb the gap."
        )
        .small()
        .color(Color32::from_rgb(180, 180, 180)),
    );
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("✅ Surplus years: {}", positive_gaps)).color(Color32::GREEN));
        ui.add_space(16.0);
        ui.label(RichText::new(format!("❌ Deficit years: {}", negative_gaps)).color(Color32::RED));
        ui.add_space(16.0);
        ui.label(format!("Range: {}–{}", first_year, last_year));
    });

    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);

    // ── Task (e) — Tax summary with country flags + year ranges ──────────────
    let total_restax: f64 = res.annual_summary.iter().map(|s| s.res_tax_jpy).sum();
    let total_nhi: f64 = res.annual_summary.iter().map(|s| s.nhi_obligation_jpy).sum();
    let total_rsu: f64 = res.annual_summary.iter().map(|s| s.rsu_vest_usd).sum();
    let total_kaigo_premium: f64 = res.annual_summary.iter().map(|s| s.kaigo_hoken_premium_jpy).sum();
    let total_kaigo_care: f64 = res.annual_summary.iter().map(|s| s.kaigo_out_of_pocket_jpy).sum();
    let total_kaigo = total_kaigo_premium + total_kaigo_care;

    egui::Grid::new("overview_tax_grid")
        .num_columns(2)
        .spacing([40.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new(format!("🇯🇵 Japan — Total Resident Tax Paid ({})", range)).strong());
            ui.label(fmt_jpy(total_restax));
            ui.end_row();

            ui.label(RichText::new(format!("🇯🇵 Japan — Total NHI Paid ({})", range)).strong());
            ui.label(fmt_jpy(total_nhi));
            ui.end_row();

            if total_kaigo > 0.0 {
                ui.label(RichText::new(format!("🇯🇵 Japan — Long-Term Care Cost 介護保険 ({})", range)).strong());
                if total_kaigo_care > 0.0 {
                    ui.label(format!(
                        "{} (premium: {}, care: {})",
                        fmt_jpy(total_kaigo),
                        fmt_jpy(total_kaigo_premium),
                        fmt_jpy(total_kaigo_care)
                    ));
                } else {
                    ui.label(fmt_jpy(total_kaigo));
                }
                ui.end_row();
            }

            ui.label(RichText::new(format!("🇺🇸 US — Total RSU Income ({})", range)).strong());
            ui.label(fmt_usd(total_rsu));
            ui.end_row();

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
                ui.label(
                    RichText::new(format!(
                        "Avg Dividend Coverage Ratio (post-retirement, {})",
                        post_ret_year_range(&res.annual_summary)
                    ))
                    .strong(),
                );
                ui.label(RichText::new(format!("{:.2}×", avg_dcr)).color(dcr_color));
                ui.end_row();
            }
        });

    // ── Task (d) — Investment accounts by location ────────────────────────────
    ui.add_space(8.0);
    ui.label(RichText::new("Investment Accounts by Location").strong().size(15.0));
    ui.add_space(2.0);
    egui::Grid::new("inv_location_by_account")
        .num_columns(3)
        .striped(true)
        .spacing([20.0, 4.0])
        .show(ui, |ui| {
            ui.label(RichText::new("Account").strong());
            ui.label(RichText::new("Country").strong());
            ui.label(RichText::new(format!("Final Value ({})", last_year)).strong());
            ui.end_row();

            for row in res.account_snapshots.iter()
                .filter(|r| r.event == AccountSnapshotEvent::FinalYear)
            {
                ui.label(&row.account_name);
                ui.label(country_label(row.location));
                if row.currency == "JPY" {
                    ui.label(fmt_jpy(row.total_value_native));
                } else {
                    ui.label(fmt_usd(row.total_value_native));
                }
                ui.end_row();
            }
        });

    // ── Task (f) — Account snapshots at Retirement / Rebalance / FinalYear ───
    ui.add_space(10.0);
    ui.label(RichText::new("Account Snapshots — Retirement and Rebalance").strong().size(15.0));
    ui.add_space(2.0);
    ui.label(
        RichText::new(
            "Composition of every investment account at the retirement date and at each \
             rebalance date. Country and tax jurisdiction shown per account."
        )
        .small()
        .color(Color32::from_rgb(180, 180, 180)),
    );
    ui.add_space(4.0);

    // Collect unique (event, date) pairs, sorted.
    let events: Vec<_> = {
        let mut set = std::collections::BTreeSet::new();
        for r in &res.account_snapshots {
            set.insert((r.event, r.date));
        }
        set.into_iter().collect()
    };

    for (event, date) in events {
        let title = match event {
            AccountSnapshotEvent::Retirement => format!("🏁 Retirement — {}", date),
            AccountSnapshotEvent::Rebalance  => format!("⚖ Rebalance — {}", date),
            AccountSnapshotEvent::FinalYear  => format!("📅 Final Year — {}", date),
        };
        egui::CollapsingHeader::new(title)
            .default_open(event == AccountSnapshotEvent::Retirement)
            .show(ui, |ui| {
                for row in res.account_snapshots.iter()
                    .filter(|r| r.event == event && r.date == date)
                {
                    ui.label(
                        RichText::new(format!(
                            "{} — {} ({})",
                            row.account_name,
                            country_label(row.location),
                            row.tax_jurisdiction,
                        ))
                        .strong(),
                    );
                    let native_str = if row.currency == "JPY" {
                        fmt_jpy(row.total_value_native)
                    } else {
                        fmt_usd(row.total_value_native)
                    };
                    ui.label(format!(
                        "  Total: {} (≈ {} / {})",
                        native_str,
                        fmt_usd(row.total_value_usd),
                        fmt_jpy(row.total_value_jpy),
                    ));
                    if !row.composition.is_empty() {
                        egui::Grid::new(format!("comp_{}_{}_{}", row.account_name, event as u8, date))
                            .num_columns(4)
                            .striped(true)
                            .spacing([16.0, 2.0])
                            .show(ui, |ui| {
                                for h in &["Ticker", "Qty", "Price", "% of acct"] {
                                    ui.label(RichText::new(*h).small().strong());
                                }
                                ui.end_row();
                                for a in &row.composition {
                                    ui.label(&a.ticker);
                                    ui.label(format!("{:.3}", a.quantity));
                                    ui.label(format!("${:.2}", a.price_native));
                                    ui.label(format!("{:.1}%", a.pct_of_account * 100.0));
                                    ui.end_row();
                                }
                            });
                    }
                    ui.add_space(4.0);
                }
            });
    }

    // ── Stage 07 — Wealth Transferred to Heirs ───────────────────────────────
    if let Some(summary) = &res.estate_summary {
        ui.add_space(14.0);
        ui.separator();
        ui.add_space(8.0);

        ui.label(RichText::new("Wealth Transferred to Heirs").strong().size(16.0));
        ui.add_space(4.0);

        egui::Grid::new("estate_grid")
            .num_columns(2)
            .spacing([40.0, 4.0])
            .show(ui, |ui| {
                ui.label(RichText::new("Gross Estate (JPY):").strong());
                ui.label(fmt_jpy(summary.total_estate_jpy));
                ui.end_row();

                ui.label(RichText::new("Gross Estate (USD):").strong());
                ui.label(fmt_usd(summary.total_estate_usd));
                ui.end_row();

                ui.label(RichText::new("Japan Sōzoku-zei (相続税):").strong());
                ui.label(RichText::new(format!(
                    "{} ({:.1}% of estate)",
                    fmt_jpy(summary.japan_sozoku_zei_jpy),
                    summary.japan_sozoku_zei_pct,
                )).color(Color32::from_rgb(255, 140, 60)));
                ui.end_row();

                ui.label(RichText::new("US Estate Tax (gross):").strong());
                ui.label(RichText::new(format!(
                    "{} ({:.1}% of estate)",
                    fmt_usd(summary.us_estate_tax_usd),
                    summary.us_estate_tax_pct,
                )).color(Color32::from_rgb(255, 140, 60)));
                ui.end_row();

                if summary.treaty_credit_usd > 0.0 {
                    ui.label(RichText::new("US-Japan Treaty Credit:").strong());
                    ui.label(RichText::new(format!(
                        "−{} (credit applied)",
                        fmt_usd(summary.treaty_credit_usd),
                    )).color(Color32::from_rgb(100, 200, 120)));
                    ui.end_row();

                    ui.label(RichText::new("Net US Estate Tax:").strong());
                    ui.label(RichText::new(fmt_usd(summary.net_us_estate_tax_usd))
                        .color(Color32::from_rgb(255, 140, 60)));
                    ui.end_row();
                }

                ui.separator();
                ui.separator();
                ui.end_row();

                ui.label(RichText::new("Net to Heirs (JPY):").strong());
                ui.label(RichText::new(fmt_jpy(summary.net_to_heirs_jpy)).color(Color32::GREEN));
                ui.end_row();

                ui.label(RichText::new("Net to Heirs (USD):").strong());
                ui.label(RichText::new(fmt_usd(summary.net_to_heirs_usd)).color(Color32::GREEN));
                ui.end_row();
            });
    }

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

// ── Task (g) — Scenario verdict ───────────────────────────────────────────────

#[derive(Clone)]
struct ScenarioVerdict {
    works: bool,
    summary: String,
    reasons: Vec<String>,
    recommendations: Vec<String>,
}

fn evaluate_scenario(res: &SimResults) -> ScenarioVerdict {
    let total_years = res.annual_summary.len();
    let deficit_years = res.annual_summary.iter().filter(|s| s.gap_jpy < 0.0).count();
    let warnings = res.gap_warnings.len();
    let unpaid_rsu = res.annual_summary.last()
        .map(|s| s.unpaid_rsu_tax_liability_usd)
        .unwrap_or(0.0);
    let exit_tax_hit = res.annual_summary.iter().any(|s| s.exit_tax_triggered);
    let bridge_exhausted_years = res.annual_summary.iter()
        .filter(|s| s.bridge_exhausted)
        .count();

    let dcr_post: Vec<f64> = res.annual_summary.iter()
        .filter(|s| s.div_coverage_ratio > 0.0)
        .map(|s| s.div_coverage_ratio)
        .collect();
    let avg_dcr = if dcr_post.is_empty() { 0.0 }
                  else { dcr_post.iter().sum::<f64>() / dcr_post.len() as f64 };

    let final_value_usd = res.annual_summary.last().map(|s| {
        s.brokerage_usd + s.roth_usd + (s.dc_jpy / s.usd_jpy)
    }).unwrap_or(0.0);

    let mut reasons = Vec::new();
    let mut recs = Vec::new();

    let deficit_ratio = if total_years > 0 {
        deficit_years as f64 / total_years as f64
    } else {
        0.0
    };

    let works = warnings == 0
        && unpaid_rsu < 1_000.0
        && bridge_exhausted_years == 0
        && deficit_ratio < 0.20
        && final_value_usd > 0.0;

    if works {
        reasons.push(format!(
            "✅ {} of {} years had no solvency warning.", total_years, total_years));
        reasons.push(format!(
            "✅ Final portfolio (USD-equiv): ${:.0} — still positive at end of horizon.",
            final_value_usd));
        if avg_dcr >= 1.0 {
            reasons.push(format!(
                "✅ Dividends alone covered expenses on average ({:.2}× coverage).", avg_dcr));
        } else if avg_dcr >= 0.5 {
            reasons.push(format!(
                "ℹ Dividends covered {:.0}% of expenses — the rest came from drawdowns.",
                avg_dcr * 100.0));
        }
        return ScenarioVerdict {
            works: true,
            summary: "This scenario supports your retirement.".into(),
            reasons,
            recommendations: vec![],
        };
    }

    if warnings > 0 {
        reasons.push(format!(
            "❌ {} quarter(s) ran negative — income did not cover expenses.", warnings));
        recs.push("Increase the bridge fund target or war-chest target before retirement.".into());
        recs.push("Delay retirement by 1–2 years to accumulate more buffer capital.".into());
    }
    if bridge_exhausted_years > 0 {
        reasons.push(format!(
            "❌ Bridge fund was exhausted in {} year(s) — forced portfolio sells occurred.",
            bridge_exhausted_years));
        recs.push("Raise the bridge fund cap so it lasts through deficit years.".into());
    }
    if unpaid_rsu >= 1_000.0 {
        reasons.push(format!(
            "❌ ${:.0} in RSU-vest IRS liability remained unpaid (sell-to-cover deficit).",
            unpaid_rsu));
        recs.push("Withhold a higher % at vest, or set aside cash before RSU vest dates.".into());
    }
    if deficit_ratio >= 0.20 {
        reasons.push(format!(
            "❌ {:.0}% of simulated years were deficit years (income < expenses).",
            deficit_ratio * 100.0));
        recs.push("Reduce planned base expenses or boost expected dividend yield.".into());
    }
    if exit_tax_hit {
        reasons.push(
            "⚠ Japan Exit Tax (Article 60-2) would trigger if you leave Japan with current assets.".into());
        recs.push("Consult a Japan-licensed tax advisor before any departure planning.".into());
    }
    if final_value_usd <= 0.0 {
        reasons.push("❌ Portfolio reaches zero before the end of the simulation horizon.".into());
        recs.push("Lower withdrawal rate or extend earning years.".into());
    }

    ScenarioVerdict {
        works: false,
        summary: "This scenario does NOT support your retirement as configured.".into(),
        reasons,
        recommendations: recs,
    }
}

// ── Task (e) helper — post-retirement year range for DCR label ───────────────

fn post_ret_year_range(snaps: &[crate::models::snapshot::AnnualSnapshot]) -> String {
    let first = snaps.iter().find(|s| s.div_coverage_ratio > 0.0);
    let last  = snaps.iter().rev().find(|s| s.div_coverage_ratio > 0.0);
    match (first, last) {
        (Some(a), Some(b)) => format!("{}–{}", a.year, b.year),
        _ => "n/a".into(),
    }
}

// ── Task (d) helper — country label for AccountLocation ──────────────────────

fn country_label(loc: AccountLocation) -> String {
    match loc {
        AccountLocation::Us    => "🇺🇸 United States".into(),
        AccountLocation::Japan => "🇯🇵 Japan".into(),
        AccountLocation::Both  => "🇺🇸 + 🇯🇵 (both)".into(),
        AccountLocation::None  => "—".into(),
    }
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

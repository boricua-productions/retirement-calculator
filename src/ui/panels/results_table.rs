use egui::{Color32, RichText, ScrollArea, Ui};
use crate::models::snapshot::SimResults;

/// Format with thousands separators.
fn c(n: f64) -> String {
    let sign = if n < 0.0 { "-" } else { "" };
    let s = format!("{:.0}", n.abs());
    let mut result = String::new();
    let len = s.len();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 { result.push(','); }
        result.push(ch);
    }
    format!("{}{}", sign, result)
}

/// Renders the scrollable annual breakdown table using plain egui Grid.
pub fn show(ui: &mut Ui, results: &Option<SimResults>) {
    let Some(res) = results else {
        ui.label(RichText::new("No results yet.").color(Color32::GRAY));
        return;
    };

    if res.annual_summary.is_empty() {
        ui.label("No annual data in results.");
        return;
    }

    ScrollArea::both().id_salt("annual_table_scroll").show(ui, |ui| {
        // V8.7: 25 columns (21 original + 2 DC + 2 tax visibility)
        egui::Grid::new("annual_table")
            .num_columns(25)
            .spacing([8.0, 3.0])
            .striped(true)
            .show(ui, |ui| {
                // Header row
                for h in &[
                    "Year", "FX (¥/$)", "Brokerage ($)", "Roth ($)",
                    "DC Bal (¥)", "DC Payout (¥)",         // Issue 3: DC columns
                    "Div Net ($)", "FERS Net ($)", "VA Net ($)",
                    "Total Inc (¥)", "Base Exp (¥)", "NHI (¥)", "Nenkin (¥)",
                    "ResTax (¥)", "Total Exp (¥)", "Gap (¥)",
                    "US PreFTC ($)", "FTC ($)", "Japan ResTax (¥)", "Japan CGT (¥)", // Issue 4
                    "War Chest (¥)", "Bridge ($)", "Div Cover",
                    "FX Penalty (¥)", "Months@Min",
                ] {
                    ui.label(RichText::new(*h).strong().small());
                }
                ui.end_row();

                for snap in &res.annual_summary {
                    let gap_color = if snap.gap_jpy >= 0.0 { Color32::GREEN } else { Color32::RED };
                    // V8.7: show pre-FTC gross; it is non-zero even when net is $0 due to FTC.
                    let us_tax_color = if snap.us_tax_pre_ftc_usd > 0.0 { Color32::YELLOW } else { Color32::GRAY };
                    let jp_res_color = if snap.japan_tax_charged_jpy > 0.0 { Color32::YELLOW } else { Color32::GRAY };
                    let jp_cgt_color = if snap.japan_cap_gains_tax_jpy > 0.0 { Color32::from_rgb(255,180,60) } else { Color32::GRAY };
                    let dcr_color = if snap.div_coverage_ratio >= 1.0 {
                        Color32::from_rgb(100, 220, 100)
                    } else if snap.div_coverage_ratio >= 0.5 {
                        Color32::YELLOW
                    } else {
                        Color32::GRAY
                    };
                    let dc_payout_color = if snap.dc_payout_net_jpy > 0.0 {
                        Color32::from_rgb(120, 200, 255)
                    } else {
                        Color32::GRAY
                    };

                    // Stage 04: highlight years with combined recession + FX shock
                    let is_shock_year = snap.pre_shock_net_worth_jpy.is_some();
                    let year_color = if is_shock_year { Color32::YELLOW } else { Color32::WHITE };
                    let shock_tooltip = snap.pre_shock_net_worth_jpy.map(|pre| {
                        let post = snap.post_shock_net_worth_jpy.unwrap_or(pre);
                        format!(
                            "Two shock events this year (recession + FX). \
                             Pre-shock: ¥{:.0} → Post-shock: ¥{:.0}",
                            pre, post
                        )
                    });
                    let year_label = RichText::new(snap.year.to_string()).color(year_color);
                    let yr_resp = ui.label(year_label);
                    if let Some(tip) = shock_tooltip {
                        yr_resp.on_hover_text(tip);
                    }
                    ui.label(format!("{:.2}", snap.usd_jpy));
                    ui.label(format!("${}", c(snap.brokerage_usd)));
                    ui.label(format!("${}", c(snap.roth_usd)));
                    // DC columns (Issue 3)
                    ui.label(format!("¥{}", c(snap.dc_jpy)));
                    ui.label(RichText::new(format!("¥{}", c(snap.dc_payout_net_jpy))).color(dc_payout_color));
                    // Income columns
                    ui.label(format!("${}", c(snap.div_net_usd)));
                    ui.label(format!("${}", c(snap.fers_net_usd)));
                    ui.label(format!("${}", c(snap.va_net_usd)));
                    ui.label(format!("¥{}", c(snap.total_inc_net_jpy)));
                    ui.label(format!("¥{}", c(snap.base_exp_jpy)));
                    ui.label(format!("¥{}", c(snap.nhi_obligation_jpy)));
                    ui.label(format!("¥{}", c(snap.nenkin_jpy)));
                    ui.label(format!("¥{}", c(snap.res_tax_jpy)));
                    ui.label(format!("¥{}", c(snap.total_exp_jpy)));
                    ui.label(RichText::new(format!("¥{}", c(snap.gap_jpy))).color(gap_color));
                    // Tax columns (Issue 4): pre-FTC gross, FTC credit, Japan resident tax, Japan CGT
                    ui.label(RichText::new(format!("${}", c(snap.us_tax_pre_ftc_usd))).color(us_tax_color))
                        .on_hover_text("US federal tax computed before the Foreign Tax Credit is applied. FTC column shows credit applied.");
                    ui.label(format!("${}", c(snap.ftc_applied_usd)))
                        .on_hover_text("Foreign Tax Credit (§904) applied against US liability this year. US net tax = Pre-FTC − FTC.");
                    ui.label(RichText::new(format!("¥{}", c(snap.japan_tax_charged_jpy))).color(jp_res_color))
                        .on_hover_text("Japan resident tax (住民税) paid this year.");
                    ui.label(RichText::new(format!("¥{}", c(snap.japan_cap_gains_tax_jpy))).color(jp_cgt_color))
                        .on_hover_text("Japan dividend + capital-gains tax (20.315%) paid this year. This is the dominant Japan tax in retirement; credited against US liability via FTC.");
                    // Buffer columns
                    ui.label(format!("¥{}", c(snap.war_chest_jpy)));
                    ui.label(format!("${}", c(snap.bridge_fund_usd)));
                    if snap.div_coverage_ratio > 0.0 {
                        ui.label(RichText::new(format!("{:.2}×", snap.div_coverage_ratio)).color(dcr_color));
                    } else {
                        ui.label(RichText::new("—").color(Color32::GRAY));
                    }
                    let fx_pen_color = if snap.fx_penalty_jpy > 0.0 { Color32::YELLOW } else { Color32::GRAY };
                    ui.label(RichText::new(format!("¥{}", c(snap.fx_penalty_jpy))).color(fx_pen_color));
                    let min_color = if snap.months_at_min_target > 0 { Color32::RED } else { Color32::GRAY };
                    ui.label(RichText::new(snap.months_at_min_target.to_string()).color(min_color));
                    ui.end_row();
                }
            });
    });

    // Solvency warnings section — V8.7: separate real cash gaps from config notices.
    let cash_gaps: Vec<_> = res.gap_warnings.iter().filter(|w| w.is_cash_gap()).collect();
    let notices:   Vec<_> = res.gap_warnings.iter().filter(|w| !w.is_cash_gap()).collect();
    if !cash_gaps.is_empty() {
        ui.add_space(12.0);
        ui.separator();
        ui.label(RichText::new(format!("⚠ {} Solvency Warnings (quarters with negative cash flow)", cash_gaps.len()))
            .color(Color32::YELLOW).strong());
        ui.add_space(4.0);
        ScrollArea::vertical().id_salt("gap_warnings_scroll").max_height(150.0).show(ui, |ui| {
            for w in &cash_gaps {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&w.date).monospace());
                    ui.label(format!("  Gap: ¥{}", c(w.gap_jpy)));
                    ui.label(format!("  Bridge: ${}", c(w.bridge_fund_left_usd)));
                    ui.label(format!("  Absorbed: {}", w.absorbed_by));
                });
            }
        });
    }
    if !notices.is_empty() {
        ui.add_space(8.0);
        ui.separator();
        ui.label(RichText::new(format!("ℹ {} Configuration Notices", notices.len()))
            .color(Color32::GRAY).strong());
        ui.add_space(2.0);
        ScrollArea::vertical().id_salt("config_notices_scroll").max_height(100.0).show(ui, |ui| {
            for w in &notices {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&w.date).monospace().color(Color32::GRAY));
                    ui.label(RichText::new(format!("  {}: {}", w.absorbed_by, w.notes)).small().color(Color32::GRAY));
                });
            }
        });
    }
}

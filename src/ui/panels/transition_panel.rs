use chrono::Datelike;
use egui::{Color32, RichText, ScrollArea, Ui};
use crate::models::snapshot::SimResults;

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

fn c2(n: f64) -> String {
    let sign = if n < 0.0 { "-" } else { "" };
    let s = format!("{:.2}", n.abs());
    let (int_part, dec_part) = s.split_once('.').unwrap_or((&s, "00"));
    let mut result = String::new();
    let len = int_part.len();
    for (i, ch) in int_part.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 { result.push(','); }
        result.push(ch);
    }
    format!("{}{}.{}", sign, result, dec_part)
}

/// Renders the retirement transition rebalance report.
pub fn show(ui: &mut Ui, results: &Option<SimResults>) {
    ui.heading("Retirement Transition Report");
    ui.label(
        RichText::new("Account: 🇺🇸 Taxable Brokerage (US-domiciled)")
            .strong()
            .color(Color32::from_rgb(180, 220, 255)),
    );
    ui.add_space(8.0);

    let Some(res) = results else {
        ui.label(RichText::new("No results yet.").color(Color32::GRAY));
        return;
    };

    let Some(t) = &res.transition_report else {
        ui.label(RichText::new("⚠ Retirement transition event did not trigger. Check date settings.")
            .color(Color32::YELLOW));
        return;
    };

    // A. Account balance before/after.
    ui.label(RichText::new("A. Taxable Account (before → after)").strong());
    ui.horizontal(|ui| {
        ui.label(format!("Before: ${}", c2(t.pre_val)));
        ui.label("  →  ");
        ui.label(format!("After: ${}", c2(t.post_val)));
        ui.label(format!("  |  Yield: {:.2}%", t.yield_post * 100.0));
    });

    ui.add_space(10.0);

    // B. Source and use of funds.
    ui.label(RichText::new("B. Source & Use of Funds").strong());
    let alloc = &t.allocation;
    egui::Grid::new("transition_funds_grid")
        .num_columns(2)
        .spacing([20.0, 4.0])
        .show(ui, |ui| {
            let total_source: f64 = t.sells.iter().map(|s| s.proceeds).sum();
            ui.label(RichText::new("SOURCE: Portfolio liquidation (🇺🇸 Taxable Brokerage)").strong());
            ui.label(format!("${}", c2(total_source)));
            ui.end_row();

            ui.label("USE: 🇺🇸 US Capital Gains Tax");
            ui.label(format!("-${}  (Total: ${} | Pre-funded: ${})",
                c2(alloc.us_tax_paid_from_portfolio), c2(alloc.us_tax_bill), c2(alloc.us_tax_pre)));
            ui.end_row();

            let wc_sym = if alloc.wc_currency == "USD" { "$" } else { "¥" };
            let wc_country = if alloc.wc_currency == "USD" { "🇺🇸 US" } else { "🇯🇵 Japan" };
            ui.label(format!("USE: War Chest Fill ({})", wc_country));
            if alloc.wc_paid_from_portfolio_usd == 0.0 && alloc.wc_target == 0.0 {
                ui.label("$0 (disabled)");
            } else {
                ui.label(format!("-${}  (Target: {}{})",
                    c2(alloc.wc_paid_from_portfolio_usd), wc_sym, c(alloc.wc_target)));
            }
            ui.end_row();

            let bridge_country = if alloc.bridge_fund_currency == "USD" { "🇺🇸 US" } else { "🇯🇵 Japan" };
            ui.label(format!("USE: Bridge Fund Fill ({})", bridge_country));
            if alloc.bridge_pull_usd == 0.0 && alloc.bridge_total_jpy == 0.0 {
                ui.label("$0 (disabled)");
            } else {
                ui.label(format!("-${}  (Total needed: ¥{})",
                    c2(alloc.bridge_pull_usd), c(alloc.bridge_total_jpy)));
            }
            ui.end_row();

            ui.label("USE: Reinvestment");
            ui.label(format!("-${}", c2(alloc.reinvested_cash)));
            ui.end_row();
        });

    ui.add_space(10.0);

    // C. Transactions.
    ui.label(RichText::new("C. Transaction Log — all entries occur in the 🇺🇸 Taxable Brokerage account").strong());
    ui.label("Sold (assets liquidated from Taxable Brokerage):");

    ScrollArea::vertical().max_height(180.0).id_salt("sells_scroll").show(ui, |ui| {
        egui::Grid::new("sells_grid")
            .num_columns(5)
            .spacing([15.0, 3.0])
            .striped(true)
            .show(ui, |ui| {
                for h in &["Ticker", "Action", "Qty Sold", "Price", "Proceeds"] {
                    ui.label(RichText::new(*h).strong());
                }
                ui.end_row();
                for s in &t.sells {
                    ui.label(&s.ticker);
                    ui.label(&s.action);
                    ui.label(format!("{:.3}", s.qty_sold));
                    ui.label(format!("${:.2}", s.price));
                    ui.label(format!("${}", c2(s.proceeds)));
                    ui.end_row();
                }
            });
    });

    ui.add_space(6.0);
    ui.label("Bought (assets purchased into Taxable Brokerage):");
    egui::Grid::new("buys_grid")
        .num_columns(3)
        .spacing([15.0, 3.0])
        .show(ui, |ui| {
            for h in &["Ticker", "Qty Bought", "Cost"] {
                ui.label(RichText::new(*h).strong());
            }
            ui.end_row();
            for b in &t.buys {
                ui.label(&b.ticker);
                ui.label(format!("{:.3}", b.qty_bought));
                ui.label(format!("${}", c2(b.cost)));
                ui.end_row();
            }
        });

    ui.add_space(10.0);

    // D. Tax breakdown.
    ui.label(RichText::new("D. Estimated Tax Bills — by Country").strong());
    let bd = &alloc.us_tax_breakdown;
    egui::Grid::new("tax_breakdown_grid")
        .num_columns(2)
        .spacing([20.0, 4.0])
        .show(ui, |ui| {
            let _g0 = bd.get("gains_at_0_pct").copied().unwrap_or(0.0);
            let g15 = bd.get("gains_at_15_pct").copied().unwrap_or(0.0);
            let g20 = bd.get("gains_at_20_pct").copied().unwrap_or(0.0);
            let niit = bd.get("niit_on_gains").copied().unwrap_or(0.0);

            ui.label("🇺🇸 US — Gains @ 0%:"); ui.label(format!("${} → Tax: $0.00", c(_g0))); ui.end_row();
            ui.label("🇺🇸 US — Gains @ 15%:"); ui.label(format!("${} → Tax: ${}", c(g15), c2(g15 * 0.15))); ui.end_row();
            ui.label("🇺🇸 US — Gains @ 20%:"); ui.label(format!("${} → Tax: ${}", c(g20), c2(g20 * 0.20))); ui.end_row();
            ui.label("🇺🇸 US — NIIT (3.8% surtax):"); ui.label(format!("${}", c2(niit))); ui.end_row();
            ui.label(RichText::new("🇺🇸 US — Total Federal Tax Due:").strong());
            ui.label(RichText::new(format!("${}", c2(alloc.us_tax_bill))).strong());
            ui.end_row();
            ui.label(format!("🇯🇵 Japan — Resident Tax ({}+1):", t.date.year()));
            ui.label(format!("¥{}", c(alloc.jp_tax_bill)));
            ui.end_row();
        });
}

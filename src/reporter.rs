use std::collections::HashMap;
use std::fs;

use chrono::{Datelike, Local};
use log::{info, warn};

use crate::engine::rsu_engine::RsuEngine;
use crate::models::snapshot::SimResults;

// ─── Number-formatting helpers ────────────────────────────────────────────────

/// Integer with thousands commas: 1_234_567 → "1,234,567"
fn c(n: f64) -> String {
    let sign = if n < 0.0 { "-" } else { "" };
    let s = format!("{:.0}", n.abs());
    let mut result = String::new();
    let len = s.len();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    format!("{}{}", sign, result)
}

/// Two decimal places with thousands commas: 1_234.56 → "1,234.56"
fn c2(n: f64) -> String {
    let sign = if n < 0.0 { "-" } else { "" };
    let s = format!("{:.2}", n.abs());
    let (int_part, dec_part) = s.split_once('.').unwrap_or((&s, "00"));
    let mut result = String::new();
    let len = int_part.len();
    for (i, ch) in int_part.chars().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    format!("{}{}.{}", sign, result, dec_part)
}

// ─── Section / row helpers (text report) ─────────────────────────────────────

fn section(out: &mut String, title: &str) {
    let fill = "━".repeat(38_usize.saturating_sub(title.len()));
    out.push_str(&format!("━━━ {} {}━\n", title, fill));
}

fn row(out: &mut String, label: &str, value: &str) {
    out.push_str(&format!("  {:<32} {}\n", format!("{}:", label), value));
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Formats a short, clipboard-ready summary of the key simulation metrics.
/// Called by the Overview panel's "Copy Summary to Clipboard" button.
pub fn format_clipboard_text(results: &SimResults, rsu_engine: Option<&RsuEngine>) -> String {
    let location_str = if results.city.is_empty() || results.city == "Other (Standard Rate)" {
        format!("{} (Standard Rate)", results.prefecture)
    } else {
        format!("{}, {}", results.city, results.prefecture)
    };
    let mut lines: Vec<String> = vec![
        "RETIREMENT SIMULATION SUMMARY".to_string(),
        "━".repeat(44),
        format!("Tax Jurisdiction:       {}", results.tax_jurisdiction),
        format!("Investment Location:    {}", results.investment_location),
        format!("Japan Location:         {}", location_str),
        String::new(),
    ];

    if let Some(snap) = results.annual_summary.last() {
        lines.push(format!("Final Year:             {}", snap.year));
        lines.push(format!("Taxable Portfolio:      ${}", c(snap.brokerage_usd)));
        lines.push(format!("Roth IRA:               ${}", c(snap.roth_usd)));
        lines.push(format!("Japan DC:               ¥{}", c(snap.dc_jpy)));
        lines.push(format!("USD/JPY Rate:           {:.2} ¥/$", snap.usd_jpy));
    }

    lines.push(String::new());

    // DC distribution summary
    let total_dc_gross: f64 = results.annual_summary.iter().map(|s| s.dc_payout_gross_jpy).sum();
    let total_dc_tax:   f64 = results.annual_summary.iter().map(|s| s.dc_payout_tax_jpy).sum();
    let total_dc_net:   f64 = results.annual_summary.iter().map(|s| s.dc_payout_net_jpy).sum();
    if total_dc_gross > 0.0 {
        lines.push(format!("DC Gross Distributed:   ¥{}", c(total_dc_gross)));
        lines.push(format!("DC Japan Tax:           ¥{}", c(total_dc_tax)));
        lines.push(format!("DC Net Delivered:       ¥{}", c(total_dc_net)));
    }

    lines.push(String::new());

    // Solvency
    let deficit_snaps: Vec<_> = results.annual_summary.iter().filter(|s| s.gap_jpy < 0.0).collect();
    let surplus_count = results.annual_summary.iter().filter(|s| s.gap_jpy >= 0.0).count();
    if deficit_snaps.is_empty() {
        lines.push(format!("Solvency:               ✅ Fully Solvent ({} surplus years)", surplus_count));
    } else {
        lines.push(format!("Solvency:               ⚠ First deficit year: {}", deficit_snaps[0].year));
        lines.push(format!("Deficit Years:          {}", deficit_snaps.len()));
    }
    lines.push(format!("Gap Warnings:           {} quarters", results.gap_warnings.len()));
    lines.push(String::new());

    // Japan taxes
    let total_restax: f64 = results.annual_summary.iter().map(|s| s.res_tax_jpy).sum();
    let total_nhi: f64 = results.annual_summary.iter().map(|s| s.nhi_obligation_jpy).sum();
    lines.push(format!("Total Resident Tax:     ¥{}", c(total_restax)));
    lines.push(format!("Total NHI:              ¥{}", c(total_nhi)));
    lines.push(String::new());

    // RSU
    let total_rsu: f64 = results.annual_summary.iter().map(|s| s.rsu_vest_usd).sum();
    lines.push(format!("Total RSU Income:       ${}", c(total_rsu)));
    if let Some(engine) = rsu_engine {
        let mut ticker_shares: HashMap<&str, f64> = HashMap::new();
        for event in &engine.vesting_schedule {
            *ticker_shares.entry(event.ticker.as_str()).or_insert(0.0) += event.shares;
        }
        if !ticker_shares.is_empty() {
            let mut pairs: Vec<_> = ticker_shares.iter().collect();
            pairs.sort_by_key(|(t, _)| *t);
            let summary: Vec<String> = pairs
                .iter()
                .map(|(t, s)| format!("{}:{:.0}sh", t, s))
                .collect();
            lines.push(format!("RSU by Ticker:          {}", summary.join("  ")));
        }
        // FY2026 highlight
        let fy2026_total: f64 = engine.vesting_schedule.iter()
            .filter(|e| e.date.year() == 2026)
            .map(|e| e.shares)
            .sum();
        if fy2026_total > 0.0 {
            lines.push(format!("FY2026 VIP Bonus:       {:.2} shares vested", fy2026_total));
        }
    }

    // Transition
    if let Some(t) = &results.transition_report {
        lines.push(String::new());
        lines.push(format!("Transition Portfolio:   ${} → ${}", c2(t.pre_val), c2(t.post_val)));
        lines.push(format!("US Cap Gains Tax:       ${}", c2(t.allocation.us_tax_bill)));
        lines.push(format!("JP Resident Tax (Y+1):  ¥{}", c(t.allocation.jp_tax_bill)));
    }

    lines.push(String::new());
    lines.push(format!("Generated: {}", Local::now().format("%Y-%m-%d %H:%M:%S")));

    lines.join("\n")
}

/// Generates the full text report string for writing to `Retirement_Summary.txt`.
pub fn format_text_report(results: &SimResults, rsu_engine: &RsuEngine) -> String {
    let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let w = 60_usize;
    let border = "═".repeat(w);

    let mut out = String::new();
    out.push_str(&format!("╔{}╗\n", border));
    out.push_str(&format!("║{:^w$}║\n", "RETIREMENT SIMULATION SUMMARY REPORT", w = w));
    out.push_str(&format!("║{:^w$}║\n", format!("Generated: {}", now), w = w));
    out.push_str(&format!("╚{}╝\n\n", border));

    // ── 0. Retirement Verdict ─────────────────────────────────────────────────
    let rv = results.retirement_verdict();
    section(&mut out, "RETIREMENT VERDICT");
    row(&mut out, "Tier",    &format!("{} {}", rv.tier.icon(), rv.tier.label()));
    row(&mut out, "Summary", &rv.summary);
    if let Some(yr) = rv.first_gap_year {
        row(&mut out, "First Real Cash Gap", &yr.to_string());
    }

    // ── 1. Simulation Configuration ──────────────────────────────────────────
    out.push('\n');
    section(&mut out, "SIMULATION CONFIGURATION");
    row(&mut out, "Tax Jurisdiction",    &results.tax_jurisdiction.to_string());
    row(&mut out, "Investment Location", &results.investment_location.to_string());

    // ── 1. Final Portfolio Values ─────────────────────────────────────────────
    section(&mut out, "FINAL PORTFOLIO VALUES");
    if let Some(snap) = results.annual_summary.last() {
        row(&mut out, "Final Year", &snap.year.to_string());
        row(&mut out, "Taxable (Brokerage)", &format!("${}", c(snap.brokerage_usd)));
        row(&mut out, "Roth IRA", &format!("${}", c(snap.roth_usd)));
        row(&mut out, "Japan DC", &format!("¥{}", c(snap.dc_jpy)));
        row(&mut out, "Exchange Rate", &format!("{:.2} ¥/$", snap.usd_jpy));
    } else {
        out.push_str("  (no data)\n");
    }

    // ── 1b. Japan DC / iDeCo Distribution ────────────────────────────────────
    out.push('\n');
    section(&mut out, "JAPAN DC / iDeCo DISTRIBUTION");
    let total_dc_gross: f64 = results.annual_summary.iter().map(|s| s.dc_payout_gross_jpy).sum();
    let total_dc_tax:   f64 = results.annual_summary.iter().map(|s| s.dc_payout_tax_jpy).sum();
    let total_dc_net:   f64 = results.annual_summary.iter().map(|s| s.dc_payout_net_jpy).sum();
    if total_dc_gross <= 0.0 {
        out.push_str("  (no distribution — DC still accumulating or not triggered)\n");
    } else {
        row(&mut out, "Lifetime Gross Distributed", &format!("¥{}", c(total_dc_gross)));
        row(&mut out, "Lifetime Japan Tax Withheld", &format!("¥{}", c(total_dc_tax)));
        row(&mut out, "Lifetime Net Delivered", &format!("¥{}", c(total_dc_net)));
        row(&mut out, "Effective Japan DC Tax Rate",
            &format!("{:.1}%", if total_dc_gross > 0.0 { total_dc_tax / total_dc_gross * 100.0 } else { 0.0 }));
        out.push_str("\n  US Saving Clause note: DC distributions are US-taxable ordinary\n");
        out.push_str("  income (Article 1(4)); Japan tax credited via FTC (Form 1116).\n");
        out.push_str("  When 退職所得控除 ≈ zeroes Japan tax, the US taxes the full lump sum.\n");
        // Per-year payout detail
        let payout_years: Vec<_> = results.annual_summary.iter()
            .filter(|s| s.dc_payout_gross_jpy > 0.0).collect();
        if !payout_years.is_empty() {
            out.push_str("\n  Per-year DC Payout:\n");
            out.push_str(&format!("    {:<6}  {:>16}  {:>16}  {:>14}\n",
                "Year", "Gross (¥)", "Tax (¥)", "Net (¥)"));
            out.push_str(&format!("    {}\n", "─".repeat(54)));
            for snap in &payout_years {
                out.push_str(&format!("    {:<6}  {:>16}  {:>16}  {:>14}\n",
                    snap.year,
                    c(snap.dc_payout_gross_jpy),
                    c(snap.dc_payout_tax_jpy),
                    c(snap.dc_payout_net_jpy)));
            }
        }
    }

    // ── 2. Solvency Analysis ──────────────────────────────────────────────────
    out.push('\n');
    section(&mut out, "SOLVENCY ANALYSIS");
    let surplus_years = results.annual_summary.iter().filter(|s| s.gap_jpy >= 0.0).count();
    let deficit_snaps: Vec<_> = results.annual_summary.iter()
        .filter(|s| s.gap_jpy < 0.0).collect();
    // V8.7 — split real cash gaps from config notices (DcCapExceeded / RothMfsPhaseOutExceeded).
    let cash_gap_warnings: Vec<_> = results.gap_warnings.iter().filter(|w| w.is_cash_gap()).collect();
    let config_notices:    Vec<_> = results.gap_warnings.iter().filter(|w| !w.is_cash_gap()).collect();

    if deficit_snaps.is_empty() {
        row(&mut out, "Status", &format!("✅ Fully Solvent ({} surplus years)", surplus_years));
    } else {
        row(&mut out, "Status", &format!("⚠  First deficit year: {}", deficit_snaps[0].year));
        row(&mut out, "Surplus Years", &surplus_years.to_string());
        row(&mut out, "Deficit Years", &deficit_snaps.len().to_string());
    }
    row(&mut out, "Quarterly Gap Warnings", &cash_gap_warnings.len().to_string());
    if !cash_gap_warnings.is_empty() {
        out.push_str("\n  Warning Detail:\n");
        for w_item in &cash_gap_warnings {
            out.push_str(&format!("    {} | Gap: ¥{:>12} | Bridge Left: ${:>10.2} | {}\n",
                w_item.date,
                c(w_item.gap_jpy),
                w_item.bridge_fund_left_usd,
                w_item.absorbed_by,
            ));
        }
    }
    if !config_notices.is_empty() {
        row(&mut out, "Configuration Notices", &config_notices.len().to_string());
        for n_item in &config_notices {
            out.push_str(&format!("    {} | {} — {}\n",
                n_item.date, n_item.absorbed_by, n_item.notes));
        }
    }

    // ── 2b. Buffer Summary ────────────────────────────────────────────────────
    out.push('\n');
    section(&mut out, "BUFFER USAGE SUMMARY");
    let wc_years_used = results.annual_summary.iter().filter(|s| s.war_chest_used_jpy > 1.0).count();
    let forced_liq_years = results.annual_summary.iter().filter(|s| s.forced_liquidations_usd > 0.01).count();
    let belt_months: u32 = results.annual_summary.iter().map(|s| s.months_at_min_target).sum();
    let bridge_ex_years = results.annual_summary.iter().filter(|s| s.bridge_exhausted).count();
    row(&mut out, "War Chest Drawn (years)",     &wc_years_used.to_string());
    row(&mut out, "Bridge Fund Exhausted (years)", &bridge_ex_years.to_string());
    row(&mut out, "Forced Liquidations (years)",  &forced_liq_years.to_string());
    row(&mut out, "Belt-tighten Months",          &belt_months.to_string());
    if wc_years_used > 0 || forced_liq_years > 0 || bridge_ex_years > 0 {
        out.push_str("\n  Year-by-year buffer detail:\n");
        out.push_str(&format!("  {:>4}  {:>14}  {:>12}  {:>16}  {:>10}\n",
            "Year", "WarChest(¥)", "Bridge($)", "ForcedLiq($)", "Mo@Min"));
        out.push_str(&format!("  {}\n", "─".repeat(62)));
        for snap in &results.annual_summary {
            if snap.war_chest_used_jpy > 1.0 || snap.forced_liquidations_usd > 0.01 || snap.bridge_exhausted {
                out.push_str(&format!("  {:>4}  {:>14}  {:>12}  {:>16}  {:>10}\n",
                    snap.year,
                    c(snap.war_chest_used_jpy),
                    c(snap.bridge_fund_usd),
                    c2(snap.forced_liquidations_usd),
                    snap.months_at_min_target));
            }
        }
    }

    // ── 3. Totalization Income Summary ───────────────────────────────────────
    out.push('\n');
    section(&mut out, "TOTALIZATION INCOME SUMMARY");
    let total_va: f64  = results.annual_summary.iter().map(|s| s.va_net_usd).sum();
    let total_ss: f64  = results.annual_summary.iter().map(|s| s.ss_payout_usd).sum();
    let total_nen: f64 = results.annual_summary.iter().map(|s| s.nenkin_income_jpy).sum();
    row(&mut out, "Total VA Benefit (lifetime)",     &format!("${}", c(total_va)));
    row(&mut out, "Total SS Payout (lifetime)",      &format!("${}", c(total_ss)));
    row(&mut out, "Total Nenkin Payout (lifetime)",  &format!("¥{}", c(total_nen)));
    if results.annual_summary.iter().any(|s| s.feie_applied) {
        let feie_years = results.annual_summary.iter().filter(|s| s.feie_applied).count();
        row(&mut out, "FEIE Applied (years)",        &feie_years.to_string());
    }

    // ── 4. Dual-Jurisdiction Tax Summary ─────────────────────────────────────
    out.push('\n');
    section(&mut out, "DUAL-JURISDICTION TAX SUMMARY");
    let total_us_net:   f64 = results.annual_summary.iter().map(|s| s.us_tax_charged_usd).sum();
    let total_us_gross: f64 = results.annual_summary.iter().map(|s| s.us_tax_pre_ftc_usd).sum();
    let total_ftc:      f64 = results.annual_summary.iter().map(|s| s.ftc_applied_usd).sum();
    let total_jp_cgt:   f64 = results.annual_summary.iter().map(|s| s.japan_cap_gains_tax_jpy).sum();
    let total_jp_tax:   f64 = results.annual_summary.iter().map(|s| s.japan_tax_charged_jpy).sum();
    let total_restax:   f64 = results.annual_summary.iter().map(|s| s.res_tax_jpy).sum();
    let total_nhi:      f64 = results.annual_summary.iter().map(|s| s.nhi_obligation_jpy).sum();
    let total_nenkin:   f64 = results.annual_summary.iter().map(|s| s.nenkin_jpy).sum();
    let total_friction: f64 = results.annual_summary.iter().map(|s| s.tax_friction_usd).sum();
    row(&mut out, "[US Tax Pre-FTC] Lifetime",  &format!("${}", c2(total_us_gross)));
    row(&mut out, "[FTC Applied] Lifetime",     &format!("${}", c2(total_ftc)));
    row(&mut out, "[US Tax Net after FTC] Total", &format!("${}", c2(total_us_net)));
    out.push_str("  (US net tax is near $0 because Japan investment tax is credited via §904 FTC.)\n");
    row(&mut out, "[Japan Inv/CGT Tax] Total",  &format!("¥{}", c(total_jp_cgt)));
    row(&mut out, "[Japan Resident Tax] Total", &format!("¥{}", c(total_jp_tax)));
    row(&mut out, "Total Resident Tax Paid",    &format!("¥{}", c(total_restax)));
    row(&mut out, "Total NHI Paid",             &format!("¥{}", c(total_nhi)));
    row(&mut out, "Total Nenkin Paid",          &format!("¥{}", c(total_nenkin)));
    row(&mut out, "[Tax Friction] Lifetime ($)", &format!("${}", c2(total_friction)));
    out.push_str("\n  Annual Tax Detail:\n");
    out.push_str(&format!("    {:<6}  {:>14}  {:>12}  {:>18}  {:>18}\n",
        "Year", "USPreFTC ($)", "FTC ($)", "Japan CGT (¥)", "Japan ResTax (¥)"));
    out.push_str(&format!("    {}\n", "─".repeat(72)));
    for snap in &results.annual_summary {
        if snap.us_tax_pre_ftc_usd > 0.0
            || snap.japan_tax_charged_jpy > 0.0
            || snap.japan_cap_gains_tax_jpy > 0.0
        {
            out.push_str(&format!("    {:<6}  {:>14}  {:>12}  {:>18}  {:>18}\n",
                snap.year,
                format!("${}", c2(snap.us_tax_pre_ftc_usd)),
                format!("${}", c2(snap.ftc_applied_usd)),
                format!("¥{}", c(snap.japan_cap_gains_tax_jpy)),
                format!("¥{}", c(snap.japan_tax_charged_jpy)),
            ));
        }
    }

    // ── 5. RSU Vesting Summary ────────────────────────────────────────────────
    out.push('\n');
    section(&mut out, "RSU VESTING SUMMARY");
    let total_rsu_usd: f64 = results.annual_summary.iter().map(|s| s.rsu_vest_usd).sum();
    row(&mut out, "Total RSU Income (lifetime)", &format!("${}", c(total_rsu_usd)));

    // Shares by ticker
    let mut ticker_shares: HashMap<String, f64> = HashMap::new();
    for event in &rsu_engine.vesting_schedule {
        *ticker_shares.entry(event.ticker.clone()).or_insert(0.0) += event.shares;
    }
    if !ticker_shares.is_empty() {
        out.push_str("\n  Total Vested Shares by Ticker:\n");
        let mut tickers: Vec<_> = ticker_shares.iter().collect();
        tickers.sort_by_key(|(t, _)| (*t).clone());
        for (ticker, shares) in &tickers {
            out.push_str(&format!("    {:<10}  {:>10.2} shares\n", ticker, shares));
        }
    }

    // Annual RSU income (only years with vesting)
    out.push_str("\n  Annual RSU Income (USD):\n");
    let mut any_rsu = false;
    for snap in &results.annual_summary {
        if snap.rsu_vest_usd > 0.0 {
            out.push_str(&format!("    {}: ${}\n", snap.year, c(snap.rsu_vest_usd)));
            any_rsu = true;
        }
    }
    if !any_rsu {
        out.push_str("    (none)\n");
    }

    // FY2026 VIP Bonus detail
    out.push_str("\n  FY2026 Vesting Events (VIP Bonus Year):\n");
    let fy2026: Vec<_> = rsu_engine.vesting_schedule.iter()
        .filter(|e| e.date.year() == 2026)
        .collect();
    if fy2026.is_empty() {
        out.push_str("    (no vesting events in FY2026)\n");
    } else {
        let total_fy2026: f64 = fy2026.iter().map(|e| e.shares).sum();
        for event in &fy2026 {
            out.push_str(&format!("    {} | {:>8.2} shares | {}\n",
                event.date, event.shares, event.ticker));
        }
        out.push_str(&format!("    ─────────────────────────────\n"));
        out.push_str(&format!("    FY2026 Total:  {:>8.2} shares\n", total_fy2026));
    }

    // ── 6. Retirement Transition Report ──────────────────────────────────────
    if let Some(t) = &results.transition_report {
        out.push('\n');
        section(&mut out, "RETIREMENT TRANSITION EVENT");
        row(&mut out, "Date", &t.date.to_string());
        row(&mut out, "Exchange Rate", &format!("{:.2} ¥/$", t.fx_rate));
        row(&mut out, "Portfolio Pre-Rebalance", &format!("${}", c2(t.pre_val)));
        row(&mut out, "Portfolio Post-Rebalance", &format!("${}", c2(t.post_val)));
        row(&mut out, "Portfolio Yield (Post)", &format!("{:.2}%", t.yield_post * 100.0));
        out.push('\n');
        let alloc = &t.allocation;
        row(&mut out, "US Cap Gains Tax Bill", &format!("${}", c2(alloc.us_tax_bill)));
        row(&mut out, "  ST Gains Realized", &format!("${}", c2(alloc.total_st_gains)));
        row(&mut out, "  LT Gains Realized", &format!("${}", c2(alloc.total_lt_gains)));
        row(&mut out, "  NIIT", &format!("${}", c2(alloc.total_niit)));
        row(&mut out, "JP Resident Tax (Year+1)", &format!("¥{}", c(alloc.jp_tax_bill)));
        row(&mut out, "War Chest Funded", &format!("${}", c2(alloc.wc_paid_from_portfolio_usd)));
        row(&mut out, "Bridge Fund Pulled", &format!("${}", c2(alloc.bridge_pull_usd)));
        row(&mut out, "Reinvested into Portfolio", &format!("${}", c2(alloc.reinvested_cash)));

        if !t.sells.is_empty() {
            out.push_str("\n  Sell Transactions:\n");
            out.push_str(&format!("    {:<8}  {:<16}  {:>10}  {:>10}  {:>12}\n",
                "Ticker", "Action", "Qty Sold", "Price", "Proceeds"));
            out.push_str(&format!("    {}\n", "─".repeat(62)));
            for s in &t.sells {
                out.push_str(&format!("    {:<8}  {:<16}  {:>10.3}  {:>10.2}  ${:>11}\n",
                    s.ticker, s.action, s.qty_sold, s.price, c2(s.proceeds)));
            }
        }
        if !t.buys.is_empty() {
            out.push_str("\n  Buy Transactions:\n");
            out.push_str(&format!("    {:<8}  {:>12}  {:>12}\n", "Ticker", "Qty Bought", "Cost"));
            out.push_str(&format!("    {}\n", "─".repeat(38)));
            for b in &t.buys {
                out.push_str(&format!("    {:<8}  {:>12.3}  ${:>11}\n",
                    b.ticker, b.qty_bought, c2(b.cost)));
            }
        }
    }

    // ── 7. Annual Summary Table (abbreviated) ─────────────────────────────────
    out.push('\n');
    section(&mut out, "ANNUAL INCOME vs EXPENSE SUMMARY");
    out.push_str(&format!("  {:>4}  {:>8}  {:>14}  {:>14}  {:>14}  {:>10}\n",
        "Year", "FX(¥/$)", "Inc Net(¥)", "Exp Total(¥)", "Gap(¥)", "RSU($)"));
    out.push_str(&format!("  {}\n", "─".repeat(68)));
    for snap in &results.annual_summary {
        let gap_marker = if snap.gap_jpy >= 0.0 { " " } else { "⚠" };
        out.push_str(&format!("  {:>4}  {:>8.2}  {:>14}  {:>14}  {:>13}{} {:>10}\n",
            snap.year,
            snap.usd_jpy,
            c(snap.total_inc_net_jpy),
            c(snap.total_exp_jpy),
            c(snap.gap_jpy),
            gap_marker,
            c(snap.rsu_vest_usd),
        ));
    }

    out.push_str(&format!("\n{}\n", "─".repeat(w)));
    let cash_gap_count = results.gap_warnings.iter().filter(|w| w.is_cash_gap()).count();
    let notice_count   = results.gap_warnings.len() - cash_gap_count;
    out.push_str(&format!(
        "End of Report — {} simulated years  |  {} solvency gap(s)  |  {} config notice(s)\n",
        results.annual_summary.len(), cash_gap_count, notice_count));

    out
}

/// V7.7 — Returns the CSV column header line (§1.3 Zero-Footprint check).
/// Used by tests to verify no prohibited regulatory terms appear in CSV output.
pub fn csv_headers() -> Vec<&'static str> {
    CSV_HEADER_LINE.split(',').collect()
}

const CSV_HEADER_LINE: &str =
    "Year,FX_JPY_per_USD,Brokerage_USD,Roth_USD,DC_JPY,\
     DivGross_USD,DivNet_USD,FERSNet_USD,VA_Benefit_USD,RSUVest_USD,\
     SS_Payout_USD,Nenkin_Income_JPY,\
     TotalIncNet_USD,TotalIncNet_JPY,\
     BaseExp_JPY,NHI_JPY,Nenkin_JPY,ResTax_JPY,TotalExp_JPY,\
     Gap_JPY,USTaxCharged_USD,JapanTaxCharged_JPY,FEIE_Applied,\
     BridgeFund_USD,WarChest_JPY,WarChestUsed_JPY,ExtTaxPaid_USD,\
     BridgeExhausted,ForcedLiquidations_USD,FTC_Carryover_USD,Purchasing_Power_USD,\
     DivCoverageRatio,JapanCapGainsTax_JPY,StateCapGainsTax_USD,\
     FXPenalty_JPY,MonthsAtMin,\
     TotalGrossReturn_USD,TotalNetReturn_USD,TaxFriction_USD,\
     Dist_Dividend_USD,Dist_Interest_USD,Dist_CapGains_USD,Dist_Special_USD,Dist_ROC_USD,\
     RSUTaxShortfall_USD,\
     DC_Payout_Gross_JPY,DC_Payout_Tax_JPY,DC_Payout_Net_JPY,\
     USTaxPreFTC_USD,FTCApplied_USD";

/// Formats the full annual breakdown as a CSV string.
pub fn format_csv(results: &SimResults) -> String {
    let mut csv = String::from(CSV_HEADER_LINE);
    csv.push('\n');
    for s in &results.annual_summary {
        csv.push_str(&format!(
            "{},{:.2},{:.2},{:.2},{:.0},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.0},\
             {:.2},{:.0},\
             {:.0},{:.0},{:.0},{:.0},{:.0},{:.0},{:.2},{:.0},{},\
             {:.2},{:.0},{:.0},{:.2},{},{:.2},{:.2},{:.2},{:.4},{:.0},{:.2},{:.0},{},\
             {:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},{:.2},\
             {:.0},{:.0},{:.0},\
             {:.2},{:.2}\n",
            s.year, s.usd_jpy,
            s.brokerage_usd, s.roth_usd, s.dc_jpy,
            s.div_gross_usd, s.div_net_usd, s.fers_net_usd, s.va_net_usd, s.rsu_vest_usd,
            s.ss_payout_usd, s.nenkin_income_jpy,
            s.total_inc_net_usd, s.total_inc_net_jpy,
            s.base_exp_jpy, s.nhi_obligation_jpy, s.nenkin_jpy, s.res_tax_jpy,
            s.total_exp_jpy, s.gap_jpy,
            s.us_tax_charged_usd, s.japan_tax_charged_jpy,
            if s.feie_applied { "Y" } else { "N" },
            s.bridge_fund_usd, s.war_chest_jpy, s.war_chest_used_jpy, s.ext_tax_paid_usd,
            if s.bridge_exhausted { "Y" } else { "N" },
            s.forced_liquidations_usd, s.ftc_carryover_usd,
            s.purchasing_power_usd, s.div_coverage_ratio,
            s.japan_cap_gains_tax_jpy, s.state_cap_gains_tax_usd,
            s.fx_penalty_jpy, s.months_at_min_target,
            s.total_gross_return_usd, s.total_net_return_usd, s.tax_friction_usd,
            s.dist_dividend_usd, s.dist_interest_usd, s.dist_cap_gains_usd,
            s.dist_special_usd, s.dist_roc_usd,
            s.unpaid_rsu_tax_liability_usd,
            s.dc_payout_gross_jpy, s.dc_payout_tax_jpy, s.dc_payout_net_jpy,
            s.us_tax_pre_ftc_usd, s.ftc_applied_usd,
        ));
    }
    csv
}

/// Writes both `output/Retirement_Summary.txt` and `output/simulation_data.csv`.
/// Called automatically from the UI after every successful simulation run.
pub fn write_reports(results: &SimResults, rsu_engine: &RsuEngine) {
    if let Err(e) = fs::create_dir_all("output") {
        warn!("[Reporter] Cannot create output/ directory: {}", e);
        return;
    }

    // ── Text report ────────────────────────────────────────────────────────────
    let text = format_text_report(results, rsu_engine);
    match fs::write("output/Retirement_Summary.txt", &text) {
        Ok(_) => info!("[Reporter] Written: output/Retirement_Summary.txt ({} bytes)", text.len()),
        Err(e) => warn!("[Reporter] Failed to write Retirement_Summary.txt: {}", e),
    }

    // ── CSV dump ───────────────────────────────────────────────────────────────
    let csv = format_csv(results);
    match fs::write("output/simulation_data.csv", &csv) {
        Ok(_) => info!("[Reporter] Written: output/simulation_data.csv ({} rows)", results.annual_summary.len()),
        Err(e) => warn!("[Reporter] Failed to write simulation_data.csv: {}", e),
    }
}

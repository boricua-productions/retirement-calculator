use chrono::Datelike;
use log::{info, warn};

use crate::engine::market_data::MarketDataService;
use crate::engine::rsu_engine::RsuEngine;
use crate::engine::tax::japan_tax::JapanTaxEngine;
use crate::engine::tax::us_tax::TaxEngine;
use crate::handlers::cashflow_manager::cover_usd_deficit_from_buffers;
use crate::models::config::Config;
use crate::models::snapshot::RsuSellToCoverWarning;
use crate::simulation::state::SimState;

/// Processes RSU vesting events for the current month.
///
/// For each vesting event:
///   1. Looks up the current market price of the ticker.
///   2. Calculates the marginal US tax on the vesting value.
///   3. Either:
///      - "SALARY": records tax as externally paid, buys full vest value into Taxable.
///      - "SELL_TO_COVER" (realism=false): buys only the post-US-tax net (legacy).
///      - "SELL_TO_COVER" (realism=true): computes combined US+Japan tax; if vest
///        cannot cover the bill, drains Bridge Fund → War Chest → T8 liquidation,
///        records any residual as an unpaid IRS liability.
///   4. Updates `stats.year_rsu_vest_usd` and `stats.acc_ord_inc`.
///
/// Mirrors Python's `RsuVestingHandler.handle_rsu_vesting()`.
pub fn handle_rsu_vesting(
    state: &mut SimState,
    cfg: &Config,
    rsu_engine: &RsuEngine,
    tax_engine: &TaxEngine,
    estimate_annual_ord_income: impl Fn(&SimState, i32) -> f64,
) {
    let current_date = state.date;
    let yr = current_date.year();

    let events: Vec<_> = rsu_engine
        .events_for_month(current_date)
        .into_iter()
        .map(|e| (e.date, e.shares, e.ticker.clone()))
        .collect();

    for (_, shares, ticker) in events {
        // V7.7 — Per-award overrides (first award matching ticker wins).
        let rsu_overrides = rsu_engine.awards_iter().find(|a| a.ticker == ticker);

        // Price precedence: existing Taxable Asset > RSU stored value > fallback.
        let price = state
            .accounts
            .get("Taxable")
            .and_then(|acc| acc.assets.get(&ticker))
            .map(|a| a.price)
            .or_else(|| rsu_overrides.and_then(|a| a.unit_value))
            .unwrap_or_else(|| MarketDataService::fallback_price(&ticker));

        let vest_value = shares * price;
        if vest_value <= 0.0 {
            continue;
        }

        // Calculate marginal US tax using the incremental method.
        let current_ord = estimate_annual_ord_income(state, yr);
        let current_cap = state.stats.acc_div_inc;

        let tax_pre = tax_engine.calculate_liability(yr, current_ord, 0.0, current_cap).total_tax;
        let tax_post = tax_engine
            .calculate_liability(yr, current_ord + vest_value, 0.0, current_cap)
            .total_tax;
        let us_tax_liability = tax_post - tax_pre;

        let buy_amount = if cfg.rsu_tax_handling == "SELL_TO_COVER" {
            if cfg.rsu_sell_to_cover_realism {
                // ── V7.7.2 Realism Path ───────────────────────────────────────
                // Compute marginal Japan income tax + resident tax on the vest.
                let fx = state.current_fx;
                let vest_jpy = vest_value * fx;
                let age = yr - cfg.birth_date.year();
                // Dec 31 snapshot per NTA spec (扶養控除 not prorated mid-year).
                let num_deps: u32 = {
                    let dec31 = chrono::NaiveDate::from_ymd_opt(yr, 12, 31).unwrap();
                    cfg.family_unit.dependents.iter().filter(|dep| {
                        let birth = dep.birth_date.unwrap_or_else(|| {
                            chrono::NaiveDate::from_ymd_opt(dep.birth_year, 1, 1).unwrap()
                        });
                        let dep_age = {
                            let y = dec31.year() - birth.year();
                            if (dec31.month(), dec31.day()) < (birth.month(), birth.day()) { y - 1 } else { y }
                        };
                        dep_age < 18
                    }).count() as u32
                };
                let salary_jpy = state.stats.year_salary_jpy;
                // year_rsu_vest_jpy accumulates DURING the year; use the value
                // before this vest event (added below after buy).
                let prior_rsu_jpy = state.stats.year_rsu_vest_jpy;

                // Japan income tax (所得税) marginal on vest.
                let jp_inc_pre = JapanTaxEngine::calculate_income_tax(
                    salary_jpy + prior_rsu_jpy, 0.0, 0.0, age, num_deps,
                );
                let jp_inc_post = JapanTaxEngine::calculate_income_tax(
                    salary_jpy + prior_rsu_jpy + vest_jpy, 0.0, 0.0, age, num_deps,
                );
                // Japan resident tax (住民税) marginal on vest (standard 10%).
                let jp_res_pre = JapanTaxEngine::calculate_resident_tax(
                    salary_jpy + prior_rsu_jpy, 0.0, 0.0, age, num_deps, 0.10, 6_000.0,
                );
                let jp_res_post = JapanTaxEngine::calculate_resident_tax(
                    salary_jpy + prior_rsu_jpy + vest_jpy, 0.0, 0.0, age, num_deps, 0.10, 6_000.0,
                );
                let jp_tax_jpy = (jp_inc_post - jp_inc_pre) + (jp_res_post - jp_res_pre);
                let jp_tax_usd = jp_tax_jpy / fx;

                let combined_tax_usd = us_tax_liability + jp_tax_usd;

                if vest_value >= combined_tax_usd {
                    // Normal: vest proceeds cover the combined tax bill.
                    let net = vest_value - combined_tax_usd;
                    info!(
                        "   [RSU] Vesting {:.2} {} for ${:.2} (SELL_TO_COVER realism). \
                         US tax ${:.2}, JP tax ¥{:.0} (${:.2}), net ${:.2}",
                        shares, ticker, vest_value, us_tax_liability, jp_tax_jpy, jp_tax_usd, net,
                    );
                    net
                } else {
                    // Deficit: vest price post-recession cannot fund the tax bill.
                    let deficit_usd = combined_tax_usd - vest_value;
                    warn!(
                        "   [RSU] MARGIN CALL: {:.2} {} vest ${:.2} < combined tax ${:.2} \
                         (US ${:.2} + JP ¥{:.0}). Deficit ${:.2} — activating fallback cascade.",
                        shares, ticker, vest_value, combined_tax_usd,
                        us_tax_liability, jp_tax_jpy, deficit_usd,
                    );

                    let penalty = cfg.fx_spread_penalty.clamp(0.0, 0.99);
                    let uncovered = cover_usd_deficit_from_buffers(state, cfg, deficit_usd, penalty);

                    if uncovered > 0.0 {
                        state.unpaid_rsu_tax_liability_usd += uncovered;
                        state.rsu_sell_to_cover_warnings.push(RsuSellToCoverWarning {
                            date: current_date.format("%Y-%m-%d").to_string(),
                            ticker: ticker.clone(),
                            vest_value_usd: vest_value,
                            combined_tax_usd,
                            deficit_usd,
                            uncovered_usd: uncovered,
                        });
                        warn!(
                            "   [RSU] UNPAID TAX LIABILITY: ${:.2} added (cumulative ${:.2}).",
                            uncovered, state.unpaid_rsu_tax_liability_usd,
                        );
                    }

                    // All shares sold to pay tax; nothing left to reinvest.
                    0.0
                }
            } else {
                // ── Legacy permissive path ────────────────────────────────────
                let net = (vest_value - us_tax_liability).max(0.0);
                info!(
                    "   [RSU] Vesting {:.2} {} for ${:.2} (SELL_TO_COVER legacy, Tax: ${:.2})",
                    shares, ticker, vest_value, us_tax_liability,
                );
                net
            }
        } else {
            // SALARY: tax paid externally from paycheck.
            state.stats.tax_paid_external += us_tax_liability;
            info!(
                "   [RSU] Vesting {:.2} {} for ${:.2} (SALARY, Tax: ${:.2} external)",
                shares, ticker, vest_value, us_tax_liability,
            );
            vest_value
        };

        // Growth precedence: global cfg (from brokerage) > RSU stored > fallback.
        let growth_rate = cfg.growth_rates_annual.get(&ticker).copied()
            .or_else(|| rsu_overrides.and_then(|a| a.growth_rate))
            .unwrap_or_else(|| MarketDataService::fallback_growth(&ticker));

        let fallback_price = price;
        if let Some(taxable) = state.accounts.get_mut("Taxable") {
            taxable.buy(&ticker, buy_amount, current_date, fallback_price, growth_rate);
            // V7.7 — Attach RSU return profile to the new Asset if none is set yet.
            if let Some(profile) = rsu_overrides.and_then(|a| a.return_profile.clone()) {
                if let Some(asset) = taxable.assets.get_mut(&ticker) {
                    if asset.return_profile.is_none() {
                        asset.return_profile = Some(profile);
                    }
                }
            }
        }

        state.stats.year_rsu_vest_usd += vest_value;
        state.stats.year_rsu_vest_jpy += vest_value * state.current_fx;
        state.stats.acc_ord_inc += vest_value;
    }
}

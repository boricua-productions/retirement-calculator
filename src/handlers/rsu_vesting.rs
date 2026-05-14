use chrono::Datelike;
use log::info;

use crate::engine::market_data::MarketDataService;
use crate::engine::rsu_engine::RsuEngine;
use crate::engine::tax::us_tax::TaxEngine;
use crate::models::config::Config;
use crate::simulation::state::SimState;

/// Processes RSU vesting events for the current month.
///
/// For each vesting event:
///   1. Looks up the current market price of the ticker.
///   2. Calculates the marginal US tax on the vesting value.
///   3. Either:
///      - "SALARY": records tax as externally paid, buys full vest value into Taxable.
///      - "SELL_TO_COVER": buys only the post-tax net into Taxable.
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

        // Calculate marginal tax using the incremental method.
        let current_ord = estimate_annual_ord_income(state, yr);
        let current_cap = state.stats.acc_div_inc;

        let tax_pre = tax_engine.calculate_liability(yr, current_ord, 0.0, current_cap).total_tax;
        let tax_post = tax_engine
            .calculate_liability(yr, current_ord + vest_value, 0.0, current_cap)
            .total_tax;
        let tax_liability = tax_post - tax_pre;

        let buy_amount = if cfg.rsu_tax_handling == "SELL_TO_COVER" {
            let net = (vest_value - tax_liability).max(0.0);
            info!(
                "   [RSU] Vesting {:.2} {} for ${:.2} (SELL_TO_COVER, Tax: ${:.2})",
                shares, ticker, vest_value, tax_liability
            );
            net
        } else {
            // SALARY: tax paid externally from paycheck.
            state.stats.tax_paid_external += tax_liability;
            info!(
                "   [RSU] Vesting {:.2} {} for ${:.2} (SALARY, Tax: ${:.2} external)",
                shares, ticker, vest_value, tax_liability
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

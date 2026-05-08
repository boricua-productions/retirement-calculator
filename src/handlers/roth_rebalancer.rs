use chrono::{Datelike, NaiveDate};
use log::info;

use crate::engine::market_data::MarketDataService;
use crate::models::config::Config;
use crate::simulation::state::SimState;

/// Executes the one-shot Roth IRA rebalance at age 59.5.
///
/// 1. Liquidates all non-target assets (not VTI or SCHD).
/// 2. Sells overweight VTI or SCHD.
/// 3. Buys underweight assets proportionally with the cash proceeds.
///
/// The rebalance is tax-free since it occurs inside a Roth IRA.
/// Mirrors Python's `RothRebalancer.execute_roth_rebalance()`.
pub fn execute_roth_rebalance(state: &mut SimState, cfg: &Config) {
    let current_date = state.date;
    info!("   [EVENT] Executing Roth IRA rebalance at {}...", current_date);

    let _roth = match state.accounts.get("Roth") {
        Some(a) if !a.assets.is_empty() => (),
        _ => {
            info!("   [Roth Rebalance] Roth account empty or missing. Skipping.");
            return;
        }
    };

    let total_value = state.accounts["Roth"].total_value(state.current_fx);
    if total_value <= 0.0 {
        info!("   [Roth Rebalance] Roth account value is zero. Nothing to rebalance.");
        return;
    }

    let target_vti = total_value * cfg.roth_rebalance_target_vti_pct;
    let target_schd = total_value * cfg.roth_rebalance_target_schd_pct;

    let mut cash_proceeds = 0.0;

    // Pass 1: sell overweight / non-target assets.
    let tickers: Vec<String> = state.accounts["Roth"].assets.keys().cloned().collect();
    for ticker in &tickers {
        let current_value = state.accounts["Roth"].assets.get(ticker.as_str())
            .map(|a| a.market_value())
            .unwrap_or(0.0);

        if ticker != "VTI" && ticker != "SCHD" {
            info!("   [Roth Rebalance] Liquidating non-target {}: ${:.2}", ticker, current_value);
            let gain = state.accounts.get_mut("Roth").unwrap()
                .liquidate_asset(ticker, current_date);
            cash_proceeds += gain.proceeds;
        } else if ticker == "VTI" && current_value > target_vti {
            let sell = current_value - target_vti;
            info!("   [Roth Rebalance] Selling overweight VTI: ${:.2}", sell);
            let gain = state.accounts.get_mut("Roth").unwrap()
                .sell_value(ticker, sell, current_date);
            cash_proceeds += gain.proceeds;
        } else if ticker == "SCHD" && current_value > target_schd {
            let sell = current_value - target_schd;
            info!("   [Roth Rebalance] Selling overweight SCHD: ${:.2}", sell);
            let gain = state.accounts.get_mut("Roth").unwrap()
                .sell_value(ticker, sell, current_date);
            cash_proceeds += gain.proceeds;
        }
    }

    if cash_proceeds <= 0.0 {
        return;
    }

    // Pass 2: buy underweight assets proportionally.
    let vti_cur = state.accounts["Roth"].assets.get("VTI").map(|a| a.market_value()).unwrap_or(0.0);
    let schd_cur = state.accounts["Roth"].assets.get("SCHD").map(|a| a.market_value()).unwrap_or(0.0);
    let vti_needed = (target_vti - vti_cur).max(0.0);
    let schd_needed = (target_schd - schd_cur).max(0.0);
    let total_needed = vti_needed + schd_needed;

    if total_needed > 0.0 {
        let vti_buy = (vti_needed / total_needed) * cash_proceeds;
        let schd_buy = (schd_needed / total_needed) * cash_proceeds;

        if vti_buy > 0.0 {
            info!("   [Roth Rebalance] Buying VTI: ${:.2}", vti_buy);
            let p = MarketDataService::fallback_price("VTI");
            let g = cfg.growth_rates_annual.get("VTI").copied().unwrap_or(0.08);
            state.accounts.get_mut("Roth").unwrap().buy("VTI", vti_buy, current_date, p, g);
        }
        if schd_buy > 0.0 {
            info!("   [Roth Rebalance] Buying SCHD: ${:.2}", schd_buy);
            let p = MarketDataService::fallback_price("SCHD");
            let g = cfg.growth_rates_annual.get("SCHD").copied().unwrap_or(0.09);
            state.accounts.get_mut("Roth").unwrap().buy("SCHD", schd_buy, current_date, p, g);
        }
    }

    info!("   [Roth Rebalance] Complete.");
}

/// Returns the date when the Roth rebalance should trigger: birth_date + 59 years + 6 months.
/// Mirrors Python's `birth_date + relativedelta(years=59, months=6)`.
pub fn roth_rebalance_trigger_date(birth_date: NaiveDate) -> NaiveDate {
    let year = birth_date.year() + 59;
    // Add 6 months to the year-offset date.
    let base = NaiveDate::from_ymd_opt(year, birth_date.month(), birth_date.day())
        .unwrap_or(birth_date);
    // Add 6 months safely.
    let month = base.month() + 6;
    if month > 12 {
        NaiveDate::from_ymd_opt(base.year() + 1, month - 12, base.day()).unwrap_or(base)
    } else {
        NaiveDate::from_ymd_opt(base.year(), month, base.day()).unwrap_or(base)
    }
}

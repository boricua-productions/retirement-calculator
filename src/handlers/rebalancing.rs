use chrono::{Datelike, NaiveDate};
use log::info;

use crate::engine::market_data::MarketDataService;
use crate::handlers::cashflow_manager::JAPAN_CAPITAL_GAINS_RATE;
use crate::models::assets::AccountRebalanceStrategy;
use crate::models::config::Config;
use crate::simulation::state::SimState;

pub fn is_rebalance_month(date: NaiveDate, frequency_months: u32) -> bool {
    // Use an absolute month index so non-divisors of 12 (e.g. 5, 7, 13) still
    // fire at consistent intervals instead of silently misfiring or never
    // firing at all.
    let freq = frequency_months.max(1) as i64;
    let abs_month = date.year() as i64 * 12 + (date.month() as i64 - 1);
    abs_month % freq == 0
}

/// Target-state rebalancing engine (V6.0, extended V8.3 to per-account).
///
/// Sells overweight positions and buys underweight ones for each account that
/// has target allocations set. Capital gains on sells are approximated at 15%
/// LTCG for taxable accounts; tax-advantaged accounts (Roth, IRA, NISA, iDeCo,
/// DC) pay no rebalancing tax.
pub fn handle_rebalancing(state: &mut SimState, cfg: &Config) {
    if !cfg.rebalance_enabled || cfg.target_allocations.is_empty() {
        return;
    }
    if !is_rebalance_month(state.date, cfg.rebalance_frequency_months) {
        return;
    }

    let current_date = state.date;
    let fx = state.current_fx;

    // Collect account names to avoid borrowing cfg and state simultaneously.
    let accounts_to_rebalance: Vec<(String, std::collections::HashMap<String, f64>)> =
        cfg.target_allocations
            .iter()
            .filter(|(_, alloc)| !alloc.is_empty())
            .map(|(acct, alloc)| (acct.clone(), alloc.clone()))
            .collect();

    for (account_name, alloc_map) in &accounts_to_rebalance {
        let total_value = state.accounts.get(account_name.as_str())
            .map(|a| a.total_value(fx))
            .unwrap_or(0.0);
        if total_value <= 0.0 { continue; }

        // Tax-advantaged accounts: Roth, traditional IRA, NISA, iDeCo, DC Plan.
        let is_tax_advantaged = matches!(account_name.as_str(),
            "Roth" | "IRA" | "NISA" | "iDeCo" | "DC");

        let deltas: Vec<(String, f64)> = alloc_map.iter().map(|(ticker, &weight)| {
            let current = state.accounts.get(account_name.as_str())
                .and_then(|a| a.assets.get(ticker.as_str()))
                .map(|a| a.market_value())
                .unwrap_or(0.0);
            (ticker.clone(), total_value * weight - current)
        }).collect();

        // Pass 1: sells.
        let mut proceeds = 0.0f64;
        for (ticker, delta) in &deltas {
            if *delta >= 0.0 { continue; }
            let sell_amount = {
                let avail = state.accounts.get(account_name.as_str())
                    .and_then(|a| a.assets.get(ticker.as_str()))
                    .map(|a| a.market_value())
                    .unwrap_or(0.0);
                (-delta).min(avail)
            };
            if sell_amount < 1.0 { continue; }

            if let Some(acc) = state.accounts.get_mut(account_name.as_str()) {
                let gain_bd = acc.sell_value(ticker, sell_amount, current_date);
                let tax = if is_tax_advantaged {
                    0.0
                } else {
                    gain_bd.long_term_gain.max(0.0) * 0.15
                    + gain_bd.short_term_gain.max(0.0) * 0.22
                };
                let net = gain_bd.proceeds - tax;
                state.stats.year_cap_gains += gain_bd.total_gain().max(0.0);
                proceeds += net;
                info!("   [Rebalance:{}] Sold ${:.0} of {} (LTG ${:.0}, tax ${:.0})",
                    account_name, sell_amount, ticker, gain_bd.long_term_gain, tax);
            }
        }

        // Pass 2: buys, proportional to shortfall.
        let total_buy_need: f64 = deltas.iter()
            .filter(|(_, d)| *d > 0.0)
            .map(|(_, d)| *d)
            .sum::<f64>()
            .max(1e-9);

        for (ticker, delta) in &deltas {
            if *delta <= 0.0 || proceeds < 1.0 { continue; }
            let buy_amount = (proceeds * (*delta / total_buy_need)).min(proceeds);
            if buy_amount < 1.0 { continue; }

            let price = state.accounts.get(account_name.as_str())
                .and_then(|a| a.assets.get(ticker.as_str()))
                .map(|a| a.price)
                .unwrap_or_else(|| MarketDataService::fallback_price(ticker));
            let growth = cfg.growth_rates_annual.get(ticker.as_str())
                .copied()
                .unwrap_or_else(|| MarketDataService::fallback_growth(ticker));
            let account_fx = state.accounts.get(account_name.as_str())
                .map(|a| if matches!(a.currency, crate::models::assets::Currency::Jpy) { 1.0 } else { fx })
                .unwrap_or(fx);

            if let Some(acc) = state.accounts.get_mut(account_name.as_str()) {
                acc.buy_with_fx(ticker, buy_amount, current_date, price, growth, account_fx);
            }
            proceeds -= buy_amount;
            info!("   [Rebalance:{}] Bought ${:.0} of {}", account_name, buy_amount, ticker);
        }
    }
}

/// V7.7 — Execute a per-account rebalance strategy (§2.3).
///
/// Sells over-weight positions vs the strategy's target weights and buys
/// under-weight ones with the net proceeds. Capital-gains tax is estimated
/// at the §5.1 rate for the account's jurisdiction:
///   - `Both` accounts: Japan 20.315% + US 15% LTCG (both applied on gross gain).
///   - `Us`-only accounts: US 15% LTCG on long-term gains, 22% on short-term.
///   - `Japan`-only accounts: Japan 20.315%.
///   - `None` accounts: 0% (tax-advantaged — NISA, iDeCo, DC, Roth).
pub fn execute_account_rebalance_strategy(
    state: &mut SimState,
    cfg: &Config,
    account_name: &str,
    strategy: &AccountRebalanceStrategy,
) {
    if !strategy.enabled || strategy.targets.is_empty() {
        return;
    }

    let fx = state.current_fx;
    let current_date = state.date;

    let total_value = state.accounts.get(account_name)
        .map(|a| a.total_value(fx))
        .unwrap_or(0.0);
    if total_value <= 0.0 {
        return;
    }

    let jurisdiction = state.accounts.get(account_name)
        .map(|a| a.tax_jurisdiction)
        .unwrap_or_default();

    let deltas: Vec<(String, f64)> = strategy.targets.iter().map(|t| {
        let current = state.accounts.get(account_name)
            .and_then(|a| a.assets.get(&t.ticker))
            .map(|a| a.market_value())
            .unwrap_or(0.0);
        let target = total_value * t.weight;
        (t.ticker.clone(), target - current)
    }).collect();

    // Pass 1: sells.
    let mut proceeds = 0.0_f64;
    for (ticker, delta) in &deltas {
        if *delta >= 0.0 { continue; }
        let sell_amount = {
            let avail = state.accounts.get(account_name)
                .and_then(|a| a.assets.get(ticker.as_str()))
                .map(|a| a.market_value())
                .unwrap_or(0.0);
            (-delta).min(avail)
        };
        if sell_amount < 1.0 { continue; }

        if let Some(acc) = state.accounts.get_mut(account_name) {
            let gain_bd = acc.sell_value(ticker, sell_amount, current_date);
            let us_tax = match jurisdiction {
                crate::models::assets::AccountJurisdiction::Us
                | crate::models::assets::AccountJurisdiction::Both => {
                    gain_bd.long_term_gain.max(0.0) * 0.15
                    + gain_bd.short_term_gain.max(0.0) * 0.22
                }
                _ => 0.0,
            };
            let jp_tax_usd = match jurisdiction {
                crate::models::assets::AccountJurisdiction::Japan
                | crate::models::assets::AccountJurisdiction::Both => {
                    gain_bd.total_gain().max(0.0) * JAPAN_CAPITAL_GAINS_RATE
                }
                _ => 0.0,
            };
            let net = gain_bd.proceeds - us_tax - jp_tax_usd;
            state.stats.year_cap_gains += gain_bd.total_gain().max(0.0);
            if jp_tax_usd > 0.0 {
                state.stats.year_japan_cap_gains_tax_jpy += jp_tax_usd * fx;
            }
            proceeds += net;
            info!("   [AcctRebalance:{}] Sold ${:.0} of {} (gain ${:.0}, tax ${:.0})",
                account_name, sell_amount, ticker, gain_bd.total_gain(), us_tax + jp_tax_usd);
        }
    }

    // Pass 2: buys proportional to shortfall.
    let total_buy_need: f64 = deltas.iter()
        .filter(|(_, d)| *d > 0.0)
        .map(|(_, d)| *d)
        .sum::<f64>()
        .max(1e-9);

    for (ticker, delta) in &deltas {
        if *delta <= 0.0 || proceeds < 1.0 { continue; }
        let buy_amount = (proceeds * (*delta / total_buy_need)).min(proceeds);
        if buy_amount < 1.0 { continue; }

        let price = state.accounts.get(account_name)
            .and_then(|a| a.assets.get(ticker.as_str()))
            .map(|a| a.price)
            .unwrap_or_else(|| MarketDataService::fallback_price(ticker));
        let growth = cfg.growth_rates_annual.get(ticker.as_str())
            .copied()
            .unwrap_or_else(|| MarketDataService::fallback_growth(ticker));

        let account_fx = state.accounts.get(account_name)
            .map(|a| if matches!(a.currency, crate::models::assets::Currency::Jpy) { 1.0 } else { fx })
            .unwrap_or(fx);
        if let Some(acc) = state.accounts.get_mut(account_name) {
            acc.buy_with_fx(ticker, buy_amount, current_date, price, growth, account_fx);
        }
        proceeds -= buy_amount;
        info!("   [AcctRebalance:{}] Bought ${:.0} of {}", account_name, buy_amount, ticker);
    }
}

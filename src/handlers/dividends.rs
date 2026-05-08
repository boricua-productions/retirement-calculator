use chrono::{Datelike, NaiveDate};
use log::info;

use crate::engine::market_data::MarketDataService;
use crate::engine::tax::us_tax::TaxEngine;
use crate::handlers::cashflow_manager::JAPAN_CAPITAL_GAINS_RATE;
use crate::models::assets::DividendCurrency;
use crate::models::config::{Config, TaxProtocol};
use crate::simulation::state::SimState;

/// V7.1 — Processes lumpy dividends for all accounts.
///
/// Each asset now specifies `dividend_months: Vec<u32>` (the calendar months in
/// which it pays). An asset only fires in a month that appears in that list — no
/// dividend smoothing. The gross per event is:
///   `market_value × annual_yield / dividend_months.len()`
///
/// Currency routing (JPY-first, no FX churn):
///   - USD dividends (DividendCurrency::Usd): taxed via US marginal rate,
///     net stored in `state.current_month_div_net_usd`.
///   - JPY dividends (DividendCurrency::Jpy): taxed at Japan 20.315% CG rate
///     (0% for DC account — tax-free), net stored in
///     `state.current_month_div_net_jpy`.
///
/// Roth / DC DRIP: always reinvested, no tax, respects each asset's
/// `dividend_months` list.
///
/// Returns `(net_usd, net_jpy)` — the caller stores both into SimState.
pub fn handle_dividends(
    state: &mut SimState,
    cfg: &Config,
    tax_engine: &TaxEngine,
    estimate_annual_ord_income: impl Fn(&SimState, i32) -> f64,
) -> (f64, f64) {
    let current_date = state.date;
    let yr  = current_date.year();
    let mo  = current_date.month();

    let is_schd_pivot = cfg.buy_schd_last_year && {
        let pivot_start = subtract_one_year_approx(cfg.retirement_date);
        current_date >= pivot_start && current_date < cfg.retirement_date
    };

    let is_retired = current_date >= cfg.retirement_date;
    let mut div_net_usd = 0.0_f64;
    let mut div_net_jpy = 0.0_f64;

    // ── Taxable account dividends ─────────────────────────────────────────────
    // Collect per-asset data to avoid borrow-checker conflicts during mutation.
    let taxable_events: Vec<(String, f64, f64, f64, bool, Option<String>, DividendCurrency, Vec<u32>)> = {
        let taxable = match state.accounts.get("Taxable") {
            Some(a) => a,
            None => return (0.0, 0.0),
        };
        taxable.assets.values()
            .filter(|a| {
                a.yield_rate > 0.0
                    && a.qty() > 0.0
                    && a.price > 0.0
                    && a.dividend_months.contains(&mo)   // lumpy: only paying months
            })
            .map(|a| (
                a.ticker.clone(),
                a.qty(),
                a.price,
                a.yield_rate,
                a.drip_enabled,
                a.dividend_reinvest_target.clone(),
                a.dividend_currency.clone(),
                a.dividend_months.clone(),
            ))
            .collect()
    };

    for (ticker, qty, price, yield_rate, drip_enabled, reinvest_target, div_currency, div_months) in taxable_events {
        let n_events = div_months.len().max(1) as f64;
        let gross    = qty * price * (yield_rate / n_events);

        match div_currency {
            DividendCurrency::Usd => {
                state.stats.year_div_gross += gross;
                state.stats.acc_div_inc    += gross;

                let est_ord = estimate_annual_ord_income(state, yr);
                let gains_pre  = state.stats.year_div_gross + state.stats.year_cap_gains - gross;
                let gains_post = state.stats.year_div_gross + state.stats.year_cap_gains;
                let tax_due = if cfg.tax_jurisdiction != TaxProtocol::JapanOnly {
                    let pre  = tax_engine.calculate_liability(yr, est_ord, 0.0, gains_pre).total_tax;
                    let post = tax_engine.calculate_liability(yr, est_ord, 0.0, gains_post).total_tax;
                    (post - pre).max(0.0)
                } else {
                    0.0
                };

                if !is_retired {
                    if tax_due > 0.0 {
                        state.stats.tax_paid_external += tax_due;
                        state.stats.year_div_tax += tax_due;
                    }
                    let reinvest_ticker = if is_schd_pivot {
                        "SCHD".to_string()
                    } else {
                        reinvest_target.unwrap_or_else(|| ticker.clone())
                    };
                    if drip_enabled || is_schd_pivot {
                        let fp = MarketDataService::fallback_price(&reinvest_ticker);
                        let fg = MarketDataService::fallback_growth(&reinvest_ticker);
                        if let Some(taxable) = state.accounts.get_mut("Taxable") {
                            taxable.buy(&reinvest_ticker, gross, current_date, fp, fg);
                        }
                    } else {
                        div_net_usd += gross;
                    }
                } else {
                    let net = gross - tax_due;
                    div_net_usd += net;
                    if tax_due > 0.0 {
                        state.stats.year_tax_routed += tax_due;
                        state.stats.year_div_tax    += tax_due;
                    }
                    info!("   [DIV USD] {} div=${:.2} tax=${:.2} net=${:.2}", ticker, gross, tax_due, net);
                }
            }

            DividendCurrency::Jpy => {
                // JPY dividends: taxed at Japan 20.315% (or 0% for NISA/iDeCo-like holdings
                // that carry AccountJurisdiction::None). We approximate NISA by checking
                // whether the Taxable account's jurisdiction is None; otherwise apply CG rate.
                let is_tax_free = state.accounts.get("Taxable")
                    .map(|a| a.tax_jurisdiction == crate::models::assets::AccountJurisdiction::None)
                    .unwrap_or(false);

                let tax_jpy = if is_retired && !is_tax_free {
                    gross * JAPAN_CAPITAL_GAINS_RATE
                } else {
                    0.0
                };
                let net_jpy = gross - tax_jpy;

                // gross in JPY → convert to USD-equivalent for year_div_gross stat.
                let fx = state.current_fx;
                state.stats.year_div_gross += gross / fx;
                state.stats.acc_div_inc    += gross / fx;

                if !is_retired {
                    // Pre-retirement: DRIP in native JPY (buy more of the same JPY asset).
                    if drip_enabled || is_schd_pivot {
                        let fp = MarketDataService::fallback_price(&ticker);
                        let fg = MarketDataService::fallback_growth(&ticker);
                        if let Some(taxable) = state.accounts.get_mut("Taxable") {
                            taxable.buy(&ticker, gross, current_date, fp, fg);
                        }
                    } else {
                        div_net_jpy += gross;
                    }
                } else {
                    div_net_jpy += net_jpy;
                    if tax_jpy > 0.0 {
                        state.stats.year_japan_cap_gains_tax_jpy += tax_jpy;
                    }
                    info!("   [DIV JPY] {} div=¥{:.0} tax=¥{:.0} net=¥{:.0}", ticker, gross, tax_jpy, net_jpy);
                }
            }
        }
    }

    // ── Roth DRIP (always reinvest, no tax, respects dividend_months) ─────────
    {
        let events: Vec<(String, f64, f64, f64, Vec<u32>)> = {
            let acc = match state.accounts.get("Roth") {
                Some(a) => a,
                None => { return finish_dc_drip(state, current_date, mo, div_net_usd, div_net_jpy); }
            };
            acc.assets.values()
                .filter(|a| a.yield_rate > 0.0 && a.qty() > 0.0 && a.dividend_months.contains(&mo))
                .map(|a| (a.ticker.clone(), a.qty(), a.price, a.yield_rate, a.dividend_months.clone()))
                .collect()
        };
        for (ticker, qty, price, yield_rate, div_months) in events {
            let n = div_months.len().max(1) as f64;
            let div = qty * price * (yield_rate / n);
            if div > 0.0 {
                let fp = MarketDataService::fallback_price(&ticker);
                let fg = MarketDataService::fallback_growth(&ticker);
                if let Some(acc) = state.accounts.get_mut("Roth") {
                    acc.buy(&ticker, div, current_date, fp, fg);
                }
            }
        }
    }

    finish_dc_drip(state, current_date, mo, div_net_usd, div_net_jpy)
}

/// Processes DC DRIP then returns the (usd, jpy) dividend tuple.
fn finish_dc_drip(
    state: &mut SimState,
    current_date: NaiveDate,
    mo: u32,
    div_net_usd: f64,
    div_net_jpy: f64,
) -> (f64, f64) {
    // ── Japan DC DRIP (always reinvest, tax-free, respects dividend_months) ───
    {
        let events: Vec<(String, f64, f64, f64, Vec<u32>)> = {
            let acc = match state.accounts.get("DC") {
                Some(a) => a,
                None => return (div_net_usd, div_net_jpy),
            };
            acc.assets.values()
                .filter(|a| a.yield_rate > 0.0 && a.qty() > 0.0 && a.dividend_months.contains(&mo))
                .map(|a| (a.ticker.clone(), a.qty(), a.price, a.yield_rate, a.dividend_months.clone()))
                .collect()
        };
        for (ticker, qty, price, yield_rate, div_months) in events {
            let n = div_months.len().max(1) as f64;
            let div = qty * price * (yield_rate / n);
            if div > 0.0 {
                let fp = MarketDataService::fallback_price(&ticker);
                let fg = MarketDataService::fallback_growth(&ticker);
                if let Some(acc) = state.accounts.get_mut("DC") {
                    acc.buy(&ticker, div, current_date, fp, fg);
                }
            }
        }
    }
    (div_net_usd, div_net_jpy)
}

fn subtract_one_year_approx(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year() - 1, date.month(), date.day())
        .or_else(|| NaiveDate::from_ymd_opt(date.year() - 1, date.month(), date.day() - 1))
        .unwrap_or(date)
}

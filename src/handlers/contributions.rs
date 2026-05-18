use chrono::{Datelike, NaiveDate};
use log::info;

use crate::engine::cashflow_engine::CashFlowEngine;
use crate::engine::market_data::MarketDataService;
use crate::models::config::{BufferFundingTiming, Config, SpouseProfile};
use crate::models::snapshot::SolvencyWarning;
use crate::simulation::state::SimState;

/// Stage 12 — Subtract months from a date, returning the first day of the resulting month.
fn subtract_months(date: NaiveDate, months: u32) -> NaiveDate {
    let total_months = (date.year() * 12 + date.month() as i32) - months as i32;
    let year = total_months / 12;
    let month = (total_months % 12) as u32;
    let month = if month == 0 { 12 } else { month };
    let year = if month == 12 { year - 1 } else { year };
    NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(date)
}

/// Stage 12 — Calculate number of months between two dates (rounded up).
fn months_between(start: NaiveDate, end: NaiveDate) -> u32 {
    let start_months = start.year() * 12 + start.month() as i32;
    let end_months = end.year() * 12 + end.month() as i32;
    (end_months - start_months).max(0) as u32
}

/// Processes all pre-retirement contributions for the current month:
///   1. Monthly VTI/SCHD buy from VA income surplus.
///   2. Roth IRA annual contribution (lump sum at start of year or remainder in first month).
///   3. Japanese DC / iDeCo monthly contribution.
///
/// Returns immediately if we are on or past retirement_date.
/// Mirrors Python's `ContributionHandler.handle_contributions()`.
pub fn handle_contributions(
    state: &mut SimState,
    cfg: &Config,
    cf_engine: &CashFlowEngine,
) {
    let current_date = state.date;
    let yr = current_date.year();
    let mo = current_date.month();

    if current_date >= cfg.retirement_date {
        return;
    }

    // ── 1. Monthly VTI contribution from VA income surplus ─────────────────────
    let skip_first_month = current_date == cfg.start_date && current_date.day() > 3;
    if !skip_first_month {
        let income = cf_engine.get_incomes_usd(current_date);
        let vti_contribution = income.va_usd - cfg.va_contribution_buffer_usd;
        if vti_contribution > 0.0 {
            let ticker = &cfg.monthly_contribution_ticker;
            let fallback_p = MarketDataService::fallback_price(ticker);
            let fallback_g = cfg.growth_rates_annual.get(ticker.as_str())
                .copied()
                .unwrap_or_else(|| MarketDataService::fallback_growth(ticker));

            let fx = state.current_fx;
            if let Some(taxable) = state.accounts.get_mut("Taxable") {
                taxable.buy_with_fx(ticker, vti_contribution, current_date, fallback_p, fallback_g, fx);
            }
            state.stats.year_monthly_contribution += vti_contribution;
        }
    }

    // ── 2. Roth IRA contribution ───────────────────────────────────────────────
    // NRA-MFS Roth phase-out: MAGI $0–$10k window (IRC §408A(c)(3)(B)(ii)).
    // A working professional filing MFS almost always exceeds the $10k ceiling,
    // so Roth contributions are suppressed. Warn once per year (January).
    let nra_mfs_roth_blocked = cfg.spouse_profile == SpouseProfile::NraMfs
        && cfg.total_annual_compensation_usd > 10_000.0;
    if nra_mfs_roth_blocked && mo == 1 {
        info!(
            "   [NRA-MFS] Roth contribution skipped {}: MAGI ${:.0} > $10k MFS ceiling (IRC §408A).",
            yr, cfg.total_annual_compensation_usd
        );
        state.gap_warnings.push(SolvencyWarning {
            date:  format!("{}-01-01", yr),
            fx_rate: state.current_fx,
            qtr_income_jpy:   0.0,
            qtr_expenses_jpy: 0.0,
            gap_jpy:          0.0,
            bridge_fund_left_usd: state.bridge_fund_usd,
            absorbed_by: "RothMfsPhaseOutExceeded".into(),
            notes: format!(
                "NRA-MFS: Roth IRA skipped. Estimated MAGI ${:.0} exceeds $10,000 MFS \
                 phase-out ceiling (IRC §408A(c)(3)(B)(ii)). Consider backdoor Roth.",
                cfg.total_annual_compensation_usd
            ),
        });
    }

    let target = &cfg.monthly_contribution_ticker;
    let roth_fallback_p = MarketDataService::fallback_price(target);
    let roth_fallback_g = cfg.growth_rates_annual.get(target.as_str())
        .copied()
        .unwrap_or_else(|| MarketDataService::fallback_growth(target));

    if !nra_mfs_roth_blocked {
        let fx = state.current_fx;
        if yr == cfg.start_date.year() {
            // First year of simulation: contribute remaining limit in the first month.
            if mo == cfg.start_date.month() {
                let remaining = if cfg.roth_contribution_made_this_year {
                    0.0
                } else {
                    (state.ira_limit - cfg.roth_contribution_so_far).max(0.0)
                };
                if remaining > 0.0 {
                    info!(
                        "   [INFO] Roth IRA remainder for {}: ${:.2} (limit ${:.0} - so far ${:.0})",
                        yr, remaining, state.ira_limit, cfg.roth_contribution_so_far
                    );
                    if let Some(roth) = state.accounts.get_mut("Roth") {
                        roth.buy_with_fx(target, remaining, current_date, roth_fallback_p, roth_fallback_g, fx);
                    }
                }
            }
        } else if mo == 1 {
            // Subsequent years: contribute full limit in January.
            if let Some(roth) = state.accounts.get_mut("Roth") {
                roth.buy_with_fx(target, state.ira_limit, current_date, roth_fallback_p, roth_fallback_g, fx);
            }
        }
    }

    // ── 3. Japan DC / iDeCo monthly contribution (JPY denominated) ────────────
    // DC account uses JPY, so fx=1.0 keeps basis in JPY.
    let dc_ticker = "TAWARA";
    let dc_fallback_p = MarketDataService::fallback_price(dc_ticker);
    let dc_fallback_g = cfg.dc_growth_rate;
    if let Some(dc) = state.accounts.get_mut("DC") {
        dc.buy_with_fx(dc_ticker, cfg.dc_monthly_jpy, current_date, dc_fallback_p, dc_fallback_g, 1.0);
    }

    // ── 4. User-defined accumulation rules (V6.0) ─────────────────────────────
    for rule in &cfg.accumulation_rules {
        if rule.stop_at_retirement && current_date >= cfg.retirement_date {
            continue;
        }
        let freq = rule.frequency_months.max(1);
        if mo % freq != 0 {
            continue;
        }
        let ticker = &rule.ticker;
        let price = MarketDataService::fallback_price(ticker);
        let growth = rule.growth_pct_override
            .unwrap_or_else(|| cfg.growth_rates_annual.get(ticker.as_str())
                .copied()
                .unwrap_or_else(|| MarketDataService::fallback_growth(ticker)));
        let fx = state.current_fx;
        if let Some(acc) = state.accounts.get_mut(&rule.account) {
            acc.buy_with_fx(ticker, rule.monthly_amount, current_date, price, growth, fx);
            state.stats.year_monthly_contribution += rule.monthly_amount;
        }
    }

    // ── 5. Gradual buffer accumulation (Stage 12) ────────────────────────────
    // During the ramp period leading up to retirement, divert monthly income
    // into the buffer accumulators. This reduces the lump liquidation at
    // transition and therefore reduces capital gains tax in the retirement year.

    let mut total_skim_usd = 0.0;

    if cfg.war_chest_enabled
        && cfg.war_chest_funding_timing == BufferFundingTiming::GraduallyBeforeRetirement
    {
        let ramp_start = subtract_months(cfg.retirement_date, cfg.war_chest_ramp_months);
        if current_date >= ramp_start {
            let wc_target_jpy = if cfg.war_chest_currency == "USD" {
                cfg.war_chest_target_usd * state.current_fx
            } else {
                cfg.war_chest_target_jpy
            };
            let remaining_gap = (wc_target_jpy
                - cfg.pre_funded_war_chest_jpy
                - state.war_chest_accumulating_jpy)
                .max(0.0);
            let months_left = months_between(current_date, cfg.retirement_date).max(1);
            let skim_jpy = remaining_gap / months_left as f64;
            state.war_chest_accumulating_jpy += skim_jpy;
            state.stats.year_buffer_accumulation_jpy += skim_jpy;
            total_skim_usd += skim_jpy / state.current_fx;
        }
    }

    if cfg.bridge_fund_enabled
        && cfg.bridge_fund_funding_timing == BufferFundingTiming::GraduallyBeforeRetirement
    {
        let ramp_start = subtract_months(cfg.retirement_date, cfg.bridge_fund_ramp_months);
        if current_date >= ramp_start {
            // Bridge target calculation (same formula as retirement_transition.rs uses)
            let exp_breakdown = cf_engine.get_expenses_breakdown(current_date, state.current_fx);
            let income = cf_engine.get_incomes_usd(current_date);
            let guaranteed_income_jpy = (income.va_usd + income.fers_usd) * state.current_fx;
            let shortfall_monthly = (exp_breakdown.total_desired - guaranteed_income_jpy).max(0.0);
            let bridge_target = shortfall_monthly * cfg.bridge_months_target as f64;
            let nhi_buffer = cfg.nhi_spike_monthly_jpy * 12.0;
            let bridge_general_target = bridge_target.max(nhi_buffer);

            let bridge_target_usd = bridge_general_target / state.current_fx;
            let pre_funded_bridge_usd = if cfg.bridge_fund_currency == "USD" {
                cfg.pre_funded_bridge_usd
            } else {
                cfg.pre_funded_bridge_jpy / state.current_fx
            };
            let remaining_gap = (bridge_target_usd
                - pre_funded_bridge_usd
                - state.bridge_fund_accumulating_usd)
                .max(0.0);
            let months_left = months_between(current_date, cfg.retirement_date).max(1);
            let skim_usd = remaining_gap / months_left as f64;
            state.bridge_fund_accumulating_usd += skim_usd;
            state.stats.year_buffer_accumulation_usd += skim_usd;
            total_skim_usd += skim_usd;
        }
    }

    // Reduce VTI contribution by the total skim amount (cash building has priority)
    if total_skim_usd > 0.0 && !skip_first_month {
        if let Some(taxable) = state.accounts.get_mut("Taxable") {
            let ticker = &cfg.monthly_contribution_ticker;
            // Find the most recent buy of this ticker today and reduce it
            if let Some(asset) = taxable.assets.get_mut(ticker) {
                if let Some(last_lot) = asset.lots.last_mut() {
                    if last_lot.purchase_date == current_date {
                        let reduction = total_skim_usd.min(last_lot.basis);
                        let shares_to_remove = reduction / (last_lot.basis / last_lot.qty);
                        last_lot.qty -= shares_to_remove;
                        last_lot.basis -= reduction;
                        state.stats.year_monthly_contribution -= reduction;

                        // If the lot is now empty or nearly empty, remove it
                        if last_lot.qty < 0.001 {
                            asset.lots.pop();
                        }
                    }
                }
            }
        }
    }
}

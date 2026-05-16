use chrono::Datelike;
use log::info;

use crate::engine::cashflow_engine::CashFlowEngine;
use crate::engine::market_data::MarketDataService;
use crate::models::config::{Config, SpouseProfile};
use crate::models::snapshot::SolvencyWarning;
use crate::simulation::state::SimState;

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

            if let Some(taxable) = state.accounts.get_mut("Taxable") {
                taxable.buy(ticker, vti_contribution, current_date, fallback_p, fallback_g);
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
                        roth.buy(target, remaining, current_date, roth_fallback_p, roth_fallback_g);
                    }
                }
            }
        } else if mo == 1 {
            // Subsequent years: contribute full limit in January.
            if let Some(roth) = state.accounts.get_mut("Roth") {
                roth.buy(target, state.ira_limit, current_date, roth_fallback_p, roth_fallback_g);
            }
        }
    }

    // ── 3. Japan DC / iDeCo monthly contribution (JPY denominated) ────────────
    // DC account uses JPY, so no FX conversion needed.
    let dc_ticker = "TAWARA";
    let dc_fallback_p = MarketDataService::fallback_price(dc_ticker);
    let dc_fallback_g = cfg.dc_growth_rate;
    if let Some(dc) = state.accounts.get_mut("DC") {
        dc.buy(dc_ticker, cfg.dc_monthly_jpy, current_date, dc_fallback_p, dc_fallback_g);
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
        if let Some(acc) = state.accounts.get_mut(&rule.account) {
            acc.buy(ticker, rule.monthly_amount, current_date, price, growth);
            state.stats.year_monthly_contribution += rule.monthly_amount;
        }
    }
}

use chrono::{Datelike, NaiveDate};
use log::{info, warn};

use crate::engine::cashflow_engine::CashFlowEngine;
use crate::engine::market_data::MarketDataService;
use crate::engine::tax::japan_tax::JapanTaxEngine;
use crate::models::config::{Config, TaxProtocol, WaterfallStrategy, WithdrawalStrategy};
use crate::models::constants::SimConstants;
use crate::models::snapshot::SolvencyWarning;
use crate::simulation::state::SimState;

/// V7.0 — Japan capital-gains tax rate (所得税15.315% + 住民税5%).
pub const JAPAN_CAPITAL_GAINS_RATE: f64 = 0.20315;

/// V7.1 — Flat FX spread penalty applied on every USD→JPY conversion in the waterfall.
/// Applied at Tiers 4, 5, 6, and 8. Rate: 0.5% (overridable via cfg.fx_spread_penalty).
pub const DEFAULT_FX_SPREAD_PENALTY: f64 = 0.005;

/// Convert USD to JPY applying the configured spread penalty.
/// Returns (jpy_after_penalty, penalty_jpy_lost).
#[inline]
fn convert_usd_to_jpy(usd: f64, fx: f64, penalty: f64) -> (f64, f64) {
    let gross_jpy  = usd * fx;
    let penalty_jpy = gross_jpy * penalty;
    (gross_jpy - penalty_jpy, penalty_jpy)
}

/// Strategy dispatcher — routes to the correct waterfall based on config.
pub fn manage_monthly_cashflow(
    state: &mut SimState,
    cfg: &Config,
    cf_engine: &CashFlowEngine,
    estimate_annual_ord_income: impl Fn(&SimState, i32) -> f64,
    yr: i32,
    mo: u32,
    is_qtr: bool,
) {
    match cfg.withdrawal_waterfall {
        WaterfallStrategy::Defensive => manage_monthly_cashflow_defensive(
            state, cfg, cf_engine, estimate_annual_ord_income, yr, mo, is_qtr,
        ),
        WaterfallStrategy::Cautious => manage_monthly_cashflow_cautious(
            state, cfg, cf_engine, estimate_annual_ord_income, yr, mo, is_qtr,
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  V7.1 DEFENSIVE WATERFALL (8-Tier JPY-First)
// ─────────────────────────────────────────────────────────────────────────────

/// Manages post-retirement monthly cashflow using the V7.1 Defensive waterfall.
///
/// Priority: exhaust JPY sources first (no FX cost), then USD sources (with FX
/// spread penalty), then tighten the belt, then liquidate stocks.
///
/// Tier 0 — JPY Floor Income:   Nenkin + DC Payout (native JPY).
/// Tier 1 — JPY Dividends:      Only assets paying this calendar month.
/// Tier 2 — reserved.
/// Tier 3 — JPY War Chest:      Cash reserve, no FX cost.
/// Tier 4 — USD Floor Income:   FERS, VA, SS, SSDI (→JPY with 0.5% FX penalty).
/// Tier 5 — USD Dividends:      Only paying this month (→JPY with 0.5% penalty).
/// Tier 6 — USD Bridge Fund:    Cash reserve (→JPY with 0.5% FX penalty).
/// Tier 7 — Belt-tighten:       Drop target from Base to Minimum floor.
/// Tier 8 — Liquidate Stocks:   Highest-JPY-basis first (→JPY with 0.5% penalty).
///
/// Reset semantics: target_base_jpy is re-evaluated fresh every month; a month
/// that fires Tier 7 does not stick — next month re-attempts Base spending.
fn manage_monthly_cashflow_defensive(
    state: &mut SimState,
    cfg: &Config,
    cf_engine: &CashFlowEngine,
    estimate_annual_ord_income: impl Fn(&SimState, i32) -> f64,
    yr: i32,
    mo: u32,
    is_qtr: bool,
) {
    let current_date = state.date;
    let fx      = state.current_fx;
    // Clamp penalty to [0, 0.99] to prevent div-zero in gross-up denominators.
    let penalty = cfg.fx_spread_penalty.clamp(0.0, 0.99);

    // ── Income sources ────────────────────────────────────────────────────────
    let income       = cf_engine.get_incomes_usd(current_date);
    let va_usd       = income.va_usd;
    let fers_usd     = income.fers_usd;
    let ss_usd       = income.ss_usd;
    let ssdi_usd     = income.ssdi_usd;
    let nenkin_jpy   = income.nenkin_income_jpy;

    // US federal withholding on FERS.
    let fers_tax = compute_fers_tax(state, cfg, yr, fers_usd, &estimate_annual_ord_income);
    state.stats.year_va_net            += va_usd;
    state.stats.year_fers_gross        += fers_usd;
    state.stats.year_fers_tax          += fers_tax;
    state.stats.year_ss_payout_usd     += ss_usd;
    state.stats.year_ssdi_gross_usd    += ssdi_usd;
    state.stats.year_nenkin_income_jpy += nenkin_jpy;

    let mil_usd = cfg.military_retired.as_ref()
        .filter(|m| m.jurisdiction != TaxProtocol::TaxFree)
        .map(|m| m.monthly_usd)
        .unwrap_or(0.0);

    // ── Expense targets ───────────────────────────────────────────────────────
    let exp = cf_engine.get_expenses_breakdown(current_date);
    let nhi_delta = compute_nhi_delta(state, cfg, yr, mo, &exp);

    state.stats.year_total_exp_jpy += exp.base_desired + nhi_delta + exp.nenkin + exp.restax;
    state.stats.year_exp_base      += exp.base_desired;
    state.stats.year_exp_nhi       += exp.nhi + nhi_delta;
    state.stats.year_exp_nenkin    += exp.nenkin;
    state.stats.year_exp_restax    += exp.restax;

    let target_base_jpy = exp.base_desired + nhi_delta + exp.nenkin + exp.restax;
    let target_min_jpy  = exp.base_floor   + nhi_delta + exp.nenkin + exp.restax;

    // ── DC Payout (JPY-native, Tier 0) ───────────────────────────────────────
    let dc_payout_jpy = compute_dc_payout_jpy(state, cfg, current_date);

    // ── Tier 0: JPY Floor Income (Nenkin + DC Payout) ────────────────────────
    // Pure JPY sources: no FX conversion, no spread penalty.
    let t0_jpy  = nenkin_jpy + dc_payout_jpy;
    let mut gap = target_base_jpy;
    let t0_used = t0_jpy.min(gap);
    gap -= t0_used;
    let t0_surplus_jpy = t0_jpy - t0_used;

    // ── Tier 1: JPY Dividends (this month's lumpy events only) ───────────────
    let t1_net_jpy = state.current_month_div_net_jpy;
    let t1_used    = t1_net_jpy.min(gap);
    gap -= t1_used;
    let t1_surplus_jpy = t1_net_jpy - t1_used;

    // Tier 2: reserved.

    // ── Tier 3: JPY War Chest ─────────────────────────────────────────────────
    if gap > 0.0 {
        let draw = state.war_chest_jpy.min(gap);
        state.war_chest_jpy -= draw;
        gap -= draw;
        state.stats.year_wc_used += draw;
        if draw > 0.0 {
            info!("   [T3-WC] Drew ¥{:.0} from War Chest (remaining ¥{:.0})", draw, state.war_chest_jpy);
        }
    }

    // ── Tier 4: USD Floor Income (FERS, VA, SS, SSDI) → JPY with FX penalty ──
    let usd_pension = va_usd + (fers_usd - fers_tax) + ss_usd + ssdi_usd + mil_usd;
    let mut t4_surplus_usd = usd_pension;
    if gap > 0.0 && t4_surplus_usd > 0.0 {
        let needed_usd = gap / (fx * (1.0 - penalty));
        let spent_usd  = needed_usd.min(t4_surplus_usd);
        let (jpy_in, pen_jpy) = convert_usd_to_jpy(spent_usd, fx, penalty);
        let actual_gap_filled = jpy_in.min(gap);
        gap -= actual_gap_filled;
        t4_surplus_usd -= spent_usd;
        state.stats.year_fx_penalty_jpy += pen_jpy;
    }

    // ── Tier 5: USD Dividends (this month only) → JPY with FX penalty ─────────
    let t5_net_usd = state.current_month_div_net_usd;
    let mut t5_surplus_usd = t5_net_usd;
    if gap > 0.0 && t5_surplus_usd > 0.0 {
        let needed_usd = gap / (fx * (1.0 - penalty));
        let spent_usd  = needed_usd.min(t5_surplus_usd);
        let (jpy_in, pen_jpy) = convert_usd_to_jpy(spent_usd, fx, penalty);
        let actual_gap_filled = jpy_in.min(gap);
        gap -= actual_gap_filled;
        t5_surplus_usd -= spent_usd;
        state.stats.year_fx_penalty_jpy += pen_jpy;
    }

    // ── Tier 6: USD Bridge Fund → JPY with FX penalty ─────────────────────────
    if gap > 0.0 && state.bridge_fund_usd > 0.0 {
        let needed_usd = gap / (fx * (1.0 - penalty));
        let spent_usd  = needed_usd.min(state.bridge_fund_usd);
        let (jpy_in, pen_jpy) = convert_usd_to_jpy(spent_usd, fx, penalty);
        let actual_gap_filled = jpy_in.min(gap);
        gap -= actual_gap_filled;
        state.bridge_fund_usd -= spent_usd;
        state.stats.year_fx_penalty_jpy += pen_jpy;

        if state.bridge_fund_usd <= 0.01 {
            state.stats.year_bridge_exhausted = true;
            if !state.bridge_exhausted_logged {
                warn!("   [T6] Bridge Fund depleted ({}).", current_date.format("%Y-%m"));
                state.bridge_exhausted_logged = true;
            }
        }
    }

    // ── Tier 7: Belt-tightening — drop target to Minimum ─────────────────────
    let mut target_dropped = false;
    if gap > 0.0 {
        let savings = target_base_jpy - target_min_jpy;
        let gap_reduction = savings.min(gap);
        gap -= gap_reduction;
        target_dropped = true;
        state.stats.year_months_target_dropped += 1;
        warn!("   [T7] Target dropped to Minimum (¥{:.0}) — ¥{:.0} gap remaining.",
            target_min_jpy, gap);
    }

    // ── Tier 8: Liquidate Stocks (Highest-Basis-JPY first) → JPY with penalty ─
    if gap > 0.0 {
        let needed_usd = gap / (fx * (1.0 - penalty));
        // Signal the deficit into bridge (negative) so v7_liquidate can fill it.
        state.bridge_fund_usd -= needed_usd;
        v7_liquidate_for_deficit(state, cfg);
        // Convert whatever was recovered (net into bridge) back to JPY for gap closure.
        if state.bridge_fund_usd >= 0.0 {
            // Full recovery.
            gap = 0.0;
        } else {
            // Partial: residual deficit — solvency warning fires below.
            let residual_jpy = state.bridge_fund_usd.abs() * fx;
            gap = residual_jpy;
            state.bridge_fund_usd = 0.0;
        }
        // Debit FX spread cost on the recovered proceeds — consistent with Tiers 4-6.
        let recovered_usd = needed_usd - gap / (fx * (1.0 - penalty)).max(f64::EPSILON);
        if recovered_usd > 0.0 {
            let pen_jpy = recovered_usd * fx * penalty;
            state.stats.year_fx_penalty_jpy += pen_jpy;
            // Actual cost extracted from the bridge to match Tier 4-6 debit semantics.
            state.bridge_fund_usd -= pen_jpy / fx;
        }
    }

    let actual_target = if target_dropped { target_min_jpy } else { target_base_jpy };
    let actual_spend_jpy = (actual_target - gap).max(0.0);

    // ── Native-currency surplus deposit (no FX cross-contamination) ───────────
    // JPY surpluses → War Chest; USD surpluses → Bridge Fund.
    // During recession, park 100% in buffers; no reinvestment.
    let jpy_surplus = t0_surplus_jpy + t1_surplus_jpy;
    let usd_surplus = t4_surplus_usd + t5_surplus_usd;

    if !state.recession_active {
        deposit_jpy_surplus(state, cfg, jpy_surplus);
        deposit_usd_surplus(state, cfg, usd_surplus, current_date, target_base_jpy);
    } else {
        state.war_chest_jpy   += jpy_surplus;
        state.bridge_fund_usd += usd_surplus;
    }

    // ── Quarterly income tracking ──────────────────────────────────────────────
    // qtr_inc counts only earned income streams (not buffer draws).
    let monthly_earned_jpy = t0_jpy * 1.0  // Tier 0 always JPY
        + t1_net_jpy
        + convert_usd_to_jpy(usd_pension, fx, 0.0).0   // before penalty for income stat
        + convert_usd_to_jpy(t5_net_usd,  fx, 0.0).0;
    state.qtr_inc_jpy += monthly_earned_jpy;
    state.qtr_exp_jpy += actual_spend_jpy;

    if is_qtr {
        check_quarterly_solvency(state);
        state.qtr_inc_jpy = 0.0;
        state.qtr_exp_jpy = 0.0;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  V7.0 CAUTIOUS WATERFALL (renamed, field-updated)
// ─────────────────────────────────────────────────────────────────────────────

/// V7.0 Cautious waterfall — legacy behaviour.
/// Cuts spending to actual income before tapping buffers. Used for backward-
/// compatible scenario comparison when withdrawal_waterfall = "cautious".
fn manage_monthly_cashflow_cautious(
    state: &mut SimState,
    cfg: &Config,
    cf_engine: &CashFlowEngine,
    estimate_annual_ord_income: impl Fn(&SimState, i32) -> f64,
    yr: i32,
    mo: u32,
    is_qtr: bool,
) {
    let current_date = state.date;

    let income      = cf_engine.get_incomes_usd(current_date);
    let va_usd      = income.va_usd;
    let fers_usd    = income.fers_usd;
    let ss_usd      = income.ss_usd;
    let ssdi_usd    = income.ssdi_usd;
    let nenkin_jpy  = income.nenkin_income_jpy;

    let fers_tax = compute_fers_tax(state, cfg, yr, fers_usd, &estimate_annual_ord_income);
    state.stats.year_va_net            += va_usd;
    state.stats.year_fers_gross        += fers_usd;
    state.stats.year_fers_tax          += fers_tax;
    state.stats.year_ss_payout_usd     += ss_usd;
    state.stats.year_ssdi_gross_usd    += ssdi_usd;
    state.stats.year_nenkin_income_jpy += nenkin_jpy;

    let mil_usd = cfg.military_retired.as_ref()
        .filter(|m| m.jurisdiction != TaxProtocol::TaxFree)
        .map(|m| m.monthly_usd)
        .unwrap_or(0.0);

    let pension_net_usd = va_usd + (fers_usd - fers_tax) + ss_usd + ssdi_usd + mil_usd
        + (nenkin_jpy / state.current_fx);

    let exp = cf_engine.get_expenses_breakdown(current_date);
    let nhi_delta = compute_nhi_delta(state, cfg, yr, mo, &exp);

    state.stats.year_total_exp_jpy += exp.base_desired + nhi_delta + exp.nenkin + exp.restax;
    state.stats.year_exp_base      += exp.base_desired;
    state.stats.year_exp_nhi       += exp.nhi + nhi_delta;
    state.stats.year_exp_nenkin    += exp.nenkin;
    state.stats.year_exp_restax    += exp.restax;

    // DC Payout (converted to USD for the cautious legacy path).
    let dc_payout_usd = compute_dc_payout_usd(state, cfg, current_date);

    let total_new_money_usd = pension_net_usd + state.current_month_div_net_usd + dc_payout_usd;
    let total_new_money_jpy = total_new_money_usd * state.current_fx;

    let desired_spend_jpy = exp.base_desired + nhi_delta + exp.nenkin + exp.restax;
    let floor_spend_jpy   = exp.base_floor   + nhi_delta + exp.nenkin + exp.restax;

    let actual_spend_jpy = if total_new_money_jpy >= desired_spend_jpy {
        desired_spend_jpy
    } else if total_new_money_jpy >= floor_spend_jpy {
        total_new_money_jpy
    } else {
        floor_spend_jpy
    };

    state.bridge_fund_usd += total_new_money_usd;
    state.bridge_fund_usd -= actual_spend_jpy / state.current_fx;

    if state.bridge_fund_usd < 0.0 {
        state.stats.year_bridge_exhausted = true;
        if !state.bridge_exhausted_logged && state.war_chest_jpy > 0.0 {
            warn!("   [ALERT] Bridge Fund depleted. Switching to War Chest in {}.", current_date.format("%Y-%m"));
            state.bridge_exhausted_logged = true;
        }

        let deficit_jpy = state.bridge_fund_usd.abs() * state.current_fx;
        if state.war_chest_jpy >= deficit_jpy {
            state.war_chest_jpy -= deficit_jpy;
            state.stats.year_wc_used += deficit_jpy;
            state.bridge_fund_usd = 0.0;
        } else {
            let used = state.war_chest_jpy;
            state.stats.year_wc_used += used;
            state.bridge_fund_usd += used / state.current_fx;
            state.war_chest_jpy = 0.0;
            v7_liquidate_for_deficit(state, cfg);
        }
    } else if !state.recession_active {
        // Surplus: fill war chest (JPY), then reinvest remainder.
        let min_operating_usd = desired_spend_jpy / state.current_fx;
        if state.bridge_fund_usd > min_operating_usd {
            let available = state.bridge_fund_usd - min_operating_usd;
            let gap_jpy = (cfg.war_chest_target_jpy - state.war_chest_jpy).max(0.0);
            if gap_jpy > 0.0 {
                let fill_amt = available.min(gap_jpy / state.current_fx);
                state.war_chest_jpy   += fill_amt * state.current_fx;
                state.bridge_fund_usd -= fill_amt;
            }

            let target_bridge_usd = desired_spend_jpy * cfg.bridge_months_target as f64 / state.current_fx;
            let investable = state.bridge_fund_usd - target_bridge_usd;
            if investable > 0.0 {
                let vti_amt  = investable * cfg.target_vti_pct;
                let schd_amt = investable * cfg.target_schd_pct;
                let vti_p    = MarketDataService::fallback_price("VTI");
                let vti_g    = cfg.growth_rates_annual.get("VTI").copied().unwrap_or(0.08);
                let schd_p   = MarketDataService::fallback_price("SCHD");
                let schd_g   = cfg.growth_rates_annual.get("SCHD").copied().unwrap_or(0.09);
                if let Some(taxable) = state.accounts.get_mut("Taxable") {
                    let spent_vti  = taxable.buy("VTI",  vti_amt,  current_date, vti_p,  vti_g);
                    let spent_schd = taxable.buy("SCHD", schd_amt, current_date, schd_p, schd_g);
                    state.bridge_fund_usd -= spent_vti + spent_schd;
                }
            }
        }
    }

    state.qtr_inc_jpy += total_new_money_jpy;
    state.qtr_exp_jpy += actual_spend_jpy;

    if is_qtr {
        check_quarterly_solvency(state);
        state.qtr_inc_jpy = 0.0;
        state.qtr_exp_jpy = 0.0;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Surplus deposit helpers (Defensive path)
// ─────────────────────────────────────────────────────────────────────────────

fn deposit_jpy_surplus(state: &mut SimState, cfg: &Config, jpy_surplus: f64) {
    if jpy_surplus <= 0.0 { return; }
    let gap = (cfg.war_chest_target_jpy - state.war_chest_jpy).max(0.0);
    let fill = jpy_surplus.min(gap);
    if fill > 0.0 {
        state.war_chest_jpy += fill;
    }
    // Any JPY surplus beyond WC target: leave in war chest (simplification —
    // JPY equity purchases would require a JPY brokerage, out of scope for V7.1).
    state.war_chest_jpy += jpy_surplus - fill;
}

fn deposit_usd_surplus(
    state: &mut SimState,
    cfg: &Config,
    usd_surplus: f64,
    current_date: NaiveDate,
    desired_spend_jpy: f64,
) {
    if usd_surplus <= 0.0 { return; }
    state.bridge_fund_usd += usd_surplus;

    let target_bridge_usd = desired_spend_jpy * cfg.bridge_months_target as f64 / state.current_fx;
    let investable = state.bridge_fund_usd - target_bridge_usd;
    if investable <= 0.0 { return; }

    let vti_amt  = investable * cfg.target_vti_pct;
    let schd_amt = investable * cfg.target_schd_pct;
    let vti_p    = MarketDataService::fallback_price("VTI");
    let vti_g    = cfg.growth_rates_annual.get("VTI").copied().unwrap_or(0.08);
    let schd_p   = MarketDataService::fallback_price("SCHD");
    let schd_g   = cfg.growth_rates_annual.get("SCHD").copied().unwrap_or(0.09);
    if let Some(taxable) = state.accounts.get_mut("Taxable") {
        let spent_vti  = taxable.buy("VTI",  vti_amt,  current_date, vti_p,  vti_g);
        let spent_schd = taxable.buy("SCHD", schd_amt, current_date, schd_p, schd_g);
        state.bridge_fund_usd -= spent_vti + spent_schd;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Shared helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Compute FERS monthly federal withholding (returns 0.0 when not applicable).
fn compute_fers_tax(
    state: &SimState,
    cfg: &Config,
    yr: i32,
    fers_usd: f64,
    estimate: &impl Fn(&SimState, i32) -> f64,
) -> f64 {
    if fers_usd > 0.0
        && cfg.fers_jurisdiction != TaxProtocol::JapanOnly
        && cfg.fers_jurisdiction != TaxProtocol::TaxFree
    {
        let est_annual = estimate(state, yr);
        let tax_est = crate::engine::tax::us_tax::TaxEngine::new(cfg.tax_rules.clone())
            .calculate_liability(yr, est_annual, 0.0, 0.0).total_tax;
        if est_annual > 0.0 { tax_est / 12.0 } else { 0.0 }
    } else {
        0.0
    }
}

/// Compute the NHI monthly delta above the embedded baseline.
fn compute_nhi_delta(
    state: &SimState,
    cfg: &Config,
    yr: i32,
    mo: u32,
    exp: &crate::engine::cashflow_engine::ExpenseBreakdown,
) -> f64 {
    if cfg.tax_jurisdiction == TaxProtocol::UsOnly {
        return 0.0;
    }
    let current_fiscal_year = if mo >= 4 { yr } else { yr - 1 };
    let income_basis_year   = current_fiscal_year - 1;
    let total_nhi_monthly = if let Some(&basis_fers_usd) = state.fers_history.get(&income_basis_year) {
        if basis_fers_usd > 0.0 {
            let basis_fers_jpy = basis_fers_usd * state.current_fx;
            JapanTaxEngine::estimate_nhi_from_fers(basis_fers_jpy) / 12.0
        } else {
            exp.nhi
        }
    } else {
        exp.nhi
    };
    (total_nhi_monthly - SimConstants::EMBEDDED_NHI_MONTHLY_JPY).max(0.0)
}

/// Compute DC payout in JPY for the Defensive path (Tier 0, JPY-native).
fn compute_dc_payout_jpy(state: &mut SimState, cfg: &Config, current_date: NaiveDate) -> f64 {
    let payout_eligibility = {
        let age = cfg.dc_payout_start_age as i32;
        let y = cfg.birth_date.year() + age;
        NaiveDate::from_ymd_opt(y, cfg.birth_date.month(), cfg.birth_date.day())
            .unwrap_or(cfg.birth_date)
    };

    if !state.dc_payout_active && current_date >= payout_eligibility {
        info!("   >>> EVENT: DC Payout Triggered at Age {} ({})", cfg.dc_payout_start_age, cfg.dc_payout_method);
        state.dc_payout_active = true;
        if cfg.dc_payout_method == "LUMP_SUM" {
            if let Some(dc_acc) = state.accounts.get_mut("DC") {
                let g = dc_acc.liquidate_all(current_date);
                state.dc_months_remaining = 0;
                info!("        Action: Lump Sum Payout ¥{:.0}", g.proceeds);
                return g.proceeds;  // proceeds already in JPY
            }
        }
    }

    if state.dc_payout_active && cfg.dc_payout_method == "ANNUITY_20YR" && state.dc_months_remaining > 0 {
        let dc_balance_jpy = state.accounts.get("DC")
            .map(|a| a.total_value(state.current_fx))
            .unwrap_or(0.0);
        if dc_balance_jpy > 0.0 {
            let monthly_payout_jpy = dc_balance_jpy / state.dc_months_remaining as f64;
            if let Some(dc_acc) = state.accounts.get_mut("DC") {
                let gain = dc_acc.sell_value("TAWARA", monthly_payout_jpy, current_date);
                // qtr_inc_jpy is updated by the caller via monthly_earned_jpy (t0_jpy);
                // do not add here to avoid double-counting.
                state.dc_months_remaining -= 1;
                return gain.proceeds;  // JPY proceeds from JPY-denominated DC account
            }
        }
    }
    0.0
}

/// Compute DC payout in USD for the Cautious legacy path.
fn compute_dc_payout_usd(state: &mut SimState, cfg: &Config, current_date: NaiveDate) -> f64 {
    let payout_eligibility = {
        let age = cfg.dc_payout_start_age as i32;
        let y = cfg.birth_date.year() + age;
        NaiveDate::from_ymd_opt(y, cfg.birth_date.month(), cfg.birth_date.day())
            .unwrap_or(cfg.birth_date)
    };

    if !state.dc_payout_active && current_date >= payout_eligibility {
        info!("   >>> EVENT: DC Payout Triggered at Age {} ({})", cfg.dc_payout_start_age, cfg.dc_payout_method);
        state.dc_payout_active = true;
        if cfg.dc_payout_method == "LUMP_SUM" {
            if let Some(dc_acc) = state.accounts.get_mut("DC") {
                let g = dc_acc.liquidate_all(current_date);
                let usd = g.proceeds / state.current_fx;
                state.dc_months_remaining = 0;
                info!("        Action: Lump Sum Payout ${:.2}", usd);
                return usd;
            }
        }
    }

    if state.dc_payout_active && cfg.dc_payout_method == "ANNUITY_20YR" && state.dc_months_remaining > 0 {
        let dc_balance_jpy = state.accounts.get("DC")
            .map(|a| a.total_value(state.current_fx))
            .unwrap_or(0.0);
        if dc_balance_jpy > 0.0 {
            let monthly_payout_jpy = dc_balance_jpy / state.dc_months_remaining as f64;
            if let Some(dc_acc) = state.accounts.get_mut("DC") {
                let gain = dc_acc.sell_value("TAWARA", monthly_payout_jpy, current_date);
                // qtr_inc_jpy is updated by the caller via total_new_money_jpy;
                // do not add here to avoid double-counting.
                state.dc_months_remaining -= 1;
                return gain.proceeds / state.current_fx;
            }
        }
    }
    0.0
}

// ─────────────────────────────────────────────────────────────────────────────
//  V7.0 Liquidation engine (highest-JPY-basis-first)
// ─────────────────────────────────────────────────────────────────────────────

/// V7.0 — Liquidate Taxable holdings to cover a cash-buffer deficit, in
/// **highest-JPY-basis-first order**, with both Japan capital-gains tax (20.315%
/// of jpy_proceeds − jpy_basis) and US state tax (rate × usd_realised_gain)
/// folded into the per-share gross-up. This minimises early-year realised JPY
/// gains so the portfolio survives longer.
pub fn v7_liquidate_for_deficit(state: &mut SimState, cfg: &Config) {
    let needed = state.bridge_fund_usd.abs();
    if needed <= 0.0 {
        return;
    }

    if cfg.withdrawal_strategy == WithdrawalStrategy::DividendOnly {
        warn!(
            "   [ALERT] {} | Deficit ${:.2} but withdrawal_strategy=DividendOnly — no shares sold.",
            state.date, needed,
        );
        return;
    }

    warn!(
        "   [V7 LIQUIDATION] {} | Deficit ${:.2} — selling Taxable, highest JPY basis first.",
        state.date, needed,
    );

    let fx         = state.current_fx;
    let state_rate = cfg.us_state_tax_rate.max(0.0);
    let current_date = state.date;

    let sorted_tickers: Vec<String> = {
        let mut pairs: Vec<(String, f64)> = state.accounts.get("Taxable")
            .map(|a| a.assets.iter()
                .map(|(t, asset)| (t.clone(), asset.jpy_basis_per_share(fx)))
                .collect())
            .unwrap_or_default();
        pairs.sort_by(|(_, a), (_, b)| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        pairs.into_iter().map(|(t, _)| t).collect()
    };

    let mut total_japan_tax_jpy   = 0.0_f64;
    let mut total_state_tax_usd   = 0.0_f64;
    let mut total_shares_sold_usd = 0.0_f64;

    for ticker in sorted_tickers {
        if state.bridge_fund_usd >= 0.0 { break; }

        let (price, jpy_basis_per_share, usd_basis_per_share, available_qty) = {
            let asset = match state.accounts.get("Taxable").and_then(|a| a.assets.get(&ticker)) {
                Some(a) if a.qty() > 0.0 && a.price > 0.0 => a,
                _ => continue,
            };
            let q = asset.qty();
            (asset.price, asset.jpy_basis_per_share(fx), asset.basis() / q, q)
        };

        let jpy_proceeds_per_share = price * fx;
        let jpy_gain_per_share     = (jpy_proceeds_per_share - jpy_basis_per_share).max(0.0);
        let usd_gain_per_share     = (price - usd_basis_per_share).max(0.0);

        let japan_tax_per_share_jpy = jpy_gain_per_share * JAPAN_CAPITAL_GAINS_RATE;
        let state_tax_per_share_usd = usd_gain_per_share * state_rate;

        let net_per_share_usd = price
            - (japan_tax_per_share_jpy / fx)
            - state_tax_per_share_usd;

        if net_per_share_usd <= 0.0 { continue; }

        let shortfall_usd  = state.bridge_fund_usd.abs();
        let target_shares  = (shortfall_usd / net_per_share_usd).min(available_qty);
        if target_shares <= 0.0 { continue; }

        let amount_to_sell_usd = target_shares * price;
        let gain = match state.accounts.get_mut("Taxable") {
            Some(acct) => acct.sell_value(&ticker, amount_to_sell_usd, current_date),
            None => continue,
        };

        if gain.proceeds <= 0.0 { continue; }

        let shares_sold    = gain.proceeds / price;
        let jpy_basis_sold = shares_sold * jpy_basis_per_share;
        let jpy_proceeds   = gain.proceeds * fx;
        let jpy_gain       = (jpy_proceeds - jpy_basis_sold).max(0.0);
        let japan_tax_jpy  = jpy_gain * JAPAN_CAPITAL_GAINS_RATE;
        let state_tax_usd  = gain.total_gain().max(0.0) * state_rate;

        let net_to_buffer = gain.proceeds - (japan_tax_jpy / fx) - state_tax_usd;

        state.bridge_fund_usd += net_to_buffer;
        state.stats.year_cap_gains               += gain.total_gain();
        state.stats.year_forced_liquidations_usd += gain.proceeds;
        state.total_forced_liquidations_usd      += gain.proceeds;
        state.stats.year_japan_cap_gains_tax_jpy += japan_tax_jpy;
        state.stats.year_state_cap_gains_tax_usd += state_tax_usd;

        total_japan_tax_jpy   += japan_tax_jpy;
        total_state_tax_usd   += state_tax_usd;
        total_shares_sold_usd += gain.proceeds;

        info!(
            "        Sold ${:.2} of {} | JPYbasis ¥{:.0}/sh | JapanCG ¥{:.0} | StateCG ${:.2}",
            gain.proceeds, ticker, jpy_basis_per_share, japan_tax_jpy, state_tax_usd,
        );
    }

    // Roth / advantaged accounts as last resort — proceeds are tax-free.
    if state.bridge_fund_usd < 0.0 {
        let roth_tickers: Vec<String> = state.accounts.get("Roth")
            .map(|a| a.assets.keys().cloned().collect())
            .unwrap_or_default();
        for ticker in roth_tickers {
            if state.bridge_fund_usd >= 0.0 { break; }
            let needed_usd = state.bridge_fund_usd.abs();
            let gain = state.accounts.get_mut("Roth").unwrap()
                .sell_value(&ticker, needed_usd, current_date);
            if gain.proceeds > 0.0 {
                state.bridge_fund_usd += gain.proceeds;
                state.stats.year_forced_liquidations_usd += gain.proceeds;
                state.total_forced_liquidations_usd      += gain.proceeds;
                total_shares_sold_usd += gain.proceeds;
                info!("        [Roth fallback] Sold ${:.2} of {} (tax-free).", gain.proceeds, ticker);
            }
        }
    }

    info!(
        "   [V7 LIQUIDATION SUMMARY] Sold ${:.2} | Japan CG ¥{:.0} | State CG ${:.2}",
        total_shares_sold_usd, total_japan_tax_jpy, total_state_tax_usd,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
//  Quarterly solvency check
// ─────────────────────────────────────────────────────────────────────────────

fn check_quarterly_solvency(state: &mut SimState) {
    let gap = state.qtr_inc_jpy - state.qtr_exp_jpy;
    if gap < 0.0 {
        let absorbed_by = if state.bridge_fund_usd > 0.0 {
            "Bridge Fund ($)".into()
        } else if state.war_chest_jpy > 0.0 {
            "War Chest (¥)".into()
        } else {
            "DEFICIT".into()
        };

        state.gap_warnings.push(SolvencyWarning {
            date: state.date.format("%Y-%m-%d").to_string(),
            fx_rate: state.current_fx,
            qtr_income_jpy: state.qtr_inc_jpy,
            qtr_expenses_jpy: state.qtr_exp_jpy,
            gap_jpy: gap,
            bridge_fund_left_usd: state.bridge_fund_usd,
            absorbed_by,
            notes: "Quarterly Cashflow Negative".into(),
        });
    }
}

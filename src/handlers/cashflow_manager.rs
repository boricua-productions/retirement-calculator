use chrono::{Datelike, NaiveDate};
use log::{info, warn};

use crate::engine::cashflow_engine::CashFlowEngine;
use crate::engine::market_data::MarketDataService;
use crate::engine::tax::japan_tax::JapanTaxEngine;
use crate::models::config::{Config, TaxProtocol, WaterfallStrategy, WithdrawalRegime, WithdrawalStrategy};
use crate::models::constants::SimConstants;
use crate::models::snapshot::SolvencyWarning;
use crate::simulation::state::SimState;

/// V7.0 — Japan capital-gains tax rate (所得税15.315% + 住民税5%).
pub const JAPAN_CAPITAL_GAINS_RATE: f64 = 0.20315;

/// V7.1 — Flat FX spread penalty applied on every USD→JPY conversion in the waterfall.
/// Applied at Tiers 4, 5, 6, and 8. Rate: 0.5% (overridable via cfg.fx_spread_penalty).
pub const DEFAULT_FX_SPREAD_PENALTY: f64 = 0.005;

/// V7.4 — Dynamic-mode preemptive trigger.
/// If the projected minimum of either buffer over the next
/// `MODE_B_LOOKAHEAD_MONTHS` falls below `MODE_B_PREEMPT_FLOOR × target`, Mode B
/// fires a Tier-8 sale to restore both buffers to full target.
pub const MODE_B_LOOKAHEAD_MONTHS: u32 = 4;
pub const MODE_B_PREEMPT_FLOOR:    f64 = 0.50;

/// Convert USD to JPY applying the configured spread penalty.
/// Returns (jpy_after_penalty, penalty_jpy_lost).
#[inline]
fn convert_usd_to_jpy(usd: f64, fx: f64, penalty: f64) -> (f64, f64) {
    let gross_jpy  = usd * fx;
    let penalty_jpy = gross_jpy * penalty;
    (gross_jpy - penalty_jpy, penalty_jpy)
}

/// V7.7.2 — Covers a USD tax shortfall using the ordered fallback chain:
///   1. Bridge Fund USD (direct drain, no conversion cost)
///   2. War Chest JPY → USD (converted with the standard FX spread `penalty`)
///   3. Tier 8 Taxable stock liquidation (highest-JPY-basis-first)
///
/// Returns the portion of `deficit_usd` that could **not** be covered after
/// exhausting all three sources. A non-zero return signals an unpaid liability.
pub fn cover_usd_deficit_from_buffers(
    state: &mut SimState,
    cfg: &Config,
    mut deficit_usd: f64,
    penalty: f64,
) -> f64 {
    let fx = state.current_fx;

    // ── 1. Bridge Fund USD ────────────────────────────────────────────────────
    if deficit_usd > 0.0 && state.bridge_fund_usd > 0.0 {
        let draw = deficit_usd.min(state.bridge_fund_usd);
        state.bridge_fund_usd -= draw;
        deficit_usd -= draw;
        info!("   [RSU-STC-1] Drew ${:.2} from Bridge Fund (remaining ${:.2}).", draw, state.bridge_fund_usd);
    }

    // ── 2. War Chest JPY → USD (with FX spread penalty) ──────────────────────
    if deficit_usd > 0.0 && state.war_chest_jpy > 0.0 {
        let jpy_needed_gross = deficit_usd * fx / (1.0 - penalty).max(f64::EPSILON);
        let drawn_jpy = jpy_needed_gross.min(state.war_chest_jpy);
        let usd_net = drawn_jpy * (1.0 - penalty) / fx;
        let pen_jpy = drawn_jpy * penalty;
        state.war_chest_jpy -= drawn_jpy;
        state.stats.year_wc_used += drawn_jpy;
        state.stats.year_fx_penalty_jpy += pen_jpy;
        deficit_usd = (deficit_usd - usd_net).max(0.0);
        info!("   [RSU-STC-2] Drew ¥{:.0} from War Chest → ${:.2} net (FX penalty ¥{:.0}).",
            drawn_jpy, usd_net, pen_jpy);
    }

    // ── 3. Tier 8 stock liquidation ───────────────────────────────────────────
    if deficit_usd > 0.0 {
        let target_jpy = deficit_usd * fx;
        let recovered_jpy = liquidate_for_jpy_target(state, cfg, target_jpy, fx, penalty);
        let recovered_usd = recovered_jpy / fx;
        let covered = recovered_usd.min(deficit_usd);
        deficit_usd = (deficit_usd - covered).max(0.0);
        info!("   [RSU-STC-3] T8 liquidation recovered ${:.2} (¥{:.0}).", recovered_usd, recovered_jpy);
    }

    deficit_usd
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

    state.stats.year_total_exp_jpy += exp.base_desired + nhi_delta + exp.nenkin + exp.restax + exp.education;
    state.stats.year_exp_base      += exp.base_desired;
    state.stats.year_exp_nhi       += exp.nhi + nhi_delta;
    state.stats.year_exp_nenkin    += exp.nenkin;
    state.stats.year_exp_restax    += exp.restax;

    let target_base_jpy = exp.base_desired + nhi_delta + exp.nenkin + exp.restax;
    let target_min_jpy  = exp.base_floor   + nhi_delta + exp.nenkin + exp.restax;

    // ── V7.3 — Tier 2.5: Education Fund draw (BYPASS main waterfall) ─────────
    // Education-tagged expenses pull from the dedicated Education bucket first.
    // If the bucket is empty, the residual falls through to a Tier-8 sale
    // sized exactly to the shortfall (no other tier touches it).
    if cfg.enable_education_savings && exp.education > 0.0 {
        process_education_expense(state, cfg, exp.education, fx, penalty);
    }

    // ── DC Payout (JPY-native, Tier 0) ───────────────────────────────────────
    let dc_payout_jpy = compute_dc_payout_jpy(state, cfg, current_date);

    // ── V7.3 — Tier 0.5: Jido Teate (児童手当) child allowance ────────────────
    // Bi-monthly: paid in even calendar months at 2× the per-month rate (¥15k
    // for ages 0-3, ¥10k for 3-18). No income cap modeled. Pure JPY inflow.
    let t0_5_jpy = compute_jido_teate_jpy(cfg, current_date);
    state.stats.year_jido_teate_jpy += t0_5_jpy;

    // ── Tier 0: JPY Floor Income (Nenkin + DC Payout + Jido Teate) ───────────
    // Pure JPY sources: no FX conversion, no spread penalty.
    let t0_jpy  = nenkin_jpy + dc_payout_jpy + t0_5_jpy;
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

    // ── V7.3/V7.4 — Regime-aware Tier 7/8 dispatch ───────────────────────────
    //
    // Shielded (Mode A): EXPLICITLY Tier 7 BEFORE Tier 8.
    //   1. Tier 7 (Belt-tightening): if any gap remains after T0–T6, OR if
    //      both cash buffers are at zero, drop the month's spend target from
    //      Base to Minimum. `gap` is reduced by (target_base − target_min).
    //   2. Tier 8 (Stock Liquidation): fires ONLY when a residual gap remains
    //      after T7. The sale is sized against the *minimum* gap (post-T7),
    //      NOT the base gap — so even a partially-funded month sells just
    //      enough equity to cover the floor, never the desired base spend.
    //   Outcome: equity is preserved unless the minimum floor itself cannot
    //   be funded by all earlier tiers (T0–T7) combined.
    //
    // Dynamic (Mode B) — V7.4 PREEMPTIVE: project the next 4 months. If
    // either buffer is on track to dip below 50% of target, sell to restore
    // both buffers to FULL target (less next-month dividend look-ahead).
    // Otherwise just cover the current monthly gap. Falls back to a T7-style
    // Minimum drop only if the sale itself underperforms.
    let mut target_dropped = false;
    match cfg.withdrawal_regime {
        WithdrawalRegime::Shielded => {
            let cash_zero = state.war_chest_jpy <= 0.01 && state.bridge_fund_usd <= 0.01;
            let belt_tighten = gap > 0.0 || cash_zero;
            if belt_tighten {
                let savings = target_base_jpy - target_min_jpy;
                let gap_reduction = savings.min(gap.max(0.0));
                gap = (gap - gap_reduction).max(0.0);
                target_dropped = true;
                state.stats.year_months_target_dropped += 1;
                if gap_reduction > 0.0 {
                    warn!("   [T7-A] Shielded: target dropped to Minimum (¥{:.0}) — gap reduced by ¥{:.0}, ¥{:.0} remaining.",
                        target_min_jpy, gap_reduction, gap);
                } else if cash_zero {
                    warn!("   [T7-A] Shielded: cash buffers at zero — month runs at Minimum (¥{:.0}).",
                        target_min_jpy);
                }
            }
            if gap > 0.0 {
                gap = liquidate_for_jpy_gap(state, cfg, gap, fx, penalty);
            }
        }
        WithdrawalRegime::Dynamic => {
            // V7.4 — Preemptive restocking.
            //
            // The V7.3 Dynamic branch always sold to top buffers back to target,
            // but `liquidate_for_jpy_target` silently no-op'd when the bridge
            // already had headroom — so the war chest could drain to ¥0 mid-year
            // before any actual sale happened.
            //
            // V7.4 separates the *trigger* from the *sizing*:
            //   - TRIGGER (preemptive): project WC and Bridge balances forward
            //     `MODE_B_LOOKAHEAD_MONTHS` (default 4). If either is on track
            //     to dip below 50% of its target, fire a sale.
            //   - SIZING: restore both buffers to FULL target, minus the
            //     next-month dividend look-ahead so we don't over-sell against
            //     an imminent inflow.
            //   - The accompanying fix to `liquidate_for_jpy_target` guarantees
            //     the sale actually executes regardless of current bridge state.
            let wc_gap_jpy     = (cfg.war_chest_target_jpy - state.war_chest_jpy).max(0.0);
            let bridge_target_usd = target_base_jpy * cfg.bridge_months_target as f64 / fx;
            let bridge_gap_usd = (bridge_target_usd - state.bridge_fund_usd).max(0.0);
            let bridge_gap_jpy = bridge_gap_usd * fx;

            let wc_floor_jpy     = cfg.war_chest_target_jpy * MODE_B_PREEMPT_FLOOR;
            let bridge_floor_usd = bridge_target_usd        * MODE_B_PREEMPT_FLOOR;

            // V7.5 — Defect 1.4: pass monthly non-spend drains (T9 gift + edu skim).
            let monthly_non_spend_drain = {
                let annual_gift = if cfg.enable_gift_sink {
                    cfg.annual_gift_jpy_per_recipient * cfg.gift_recipient_count as f64
                } else { 0.0 };
                let edu_drain = if cfg.enable_education_savings {
                    cfg.edu_savings_jpy_monthly
                } else { 0.0 };
                annual_gift / 12.0 + edu_drain
            };
            let (proj_min_wc, proj_min_bridge_usd) =
                project_buffer_minimums(state, target_base_jpy, monthly_non_spend_drain, fx, penalty, MODE_B_LOOKAHEAD_MONTHS);

            let preemptive_trigger =
                proj_min_wc < wc_floor_jpy || proj_min_bridge_usd < bridge_floor_usd;

            // Look-ahead: expected next-month dividend net (JPY-equivalent).
            let lookahead_jpy = project_next_month_dividends_jpy(state, fx, penalty);

            // If neither buffer is at risk in the lookahead window, do NOT
            // proactively sell — only cover an actual current gap.
            let restock_jpy = if preemptive_trigger {
                wc_gap_jpy + bridge_gap_jpy
            } else {
                0.0
            };
            let sale_target_jpy = (gap + restock_jpy - lookahead_jpy).max(0.0);
            if sale_target_jpy > 0.0 {
                let recovered_jpy = liquidate_for_jpy_target(state, cfg, sale_target_jpy, fx, penalty);
                // First close the monthly gap, then route remainder into buffers.
                let to_gap = recovered_jpy.min(gap);
                gap -= to_gap;
                let remainder_jpy = recovered_jpy - to_gap;
                if remainder_jpy > 0.0 {
                    // Refill JPY war chest first, then USD bridge — both capped
                    // at their respective gaps so neither buffer can over-fill.
                    let wc_fill = remainder_jpy.min(wc_gap_jpy);
                    state.war_chest_jpy += wc_fill;
                    let bridge_fill_jpy = (remainder_jpy - wc_fill).min(bridge_gap_jpy);
                    if bridge_fill_jpy > 0.0 {
                        // Convert with the same FX penalty already debited at sale time.
                        state.bridge_fund_usd += bridge_fill_jpy / fx;
                    }
                }
            }
            // If a gap still remains (Dynamic ran short), drop to minimum for
            // observability — without this fallback Mode B can silently underspend.
            if gap > 0.0 {
                let savings = target_base_jpy - target_min_jpy;
                let gap_reduction = savings.min(gap);
                gap -= gap_reduction;
                target_dropped = true;
                state.stats.year_months_target_dropped += 1;
                warn!("   [T7-B] Dynamic: short on restock — target dropped to Minimum (¥{:.0}), ¥{:.0} residual.",
                    target_min_jpy, gap);
            }
        }
    }

    let actual_target = if target_dropped { target_min_jpy } else { target_base_jpy };
    let actual_spend_jpy = (actual_target - gap).max(0.0);

    // ── Native-currency surplus deposit (no FX cross-contamination) ───────────
    // V7.3: JPY surplus is first skimmed into the Tier 2.5 Education Fund up
    // to `edu_savings_jpy_monthly`. Remainder follows the V7.1 surplus rules.
    let jpy_surplus_raw = t0_surplus_jpy + t1_surplus_jpy;

    // ── V7.5 — Tier 9: Estate Planning Gift Sink ─────────────────────────────
    // Fires once per year (December) to model legal-year donation semantics.
    let t9_jpy_drawn = if cfg.enable_gift_sink && state.date.month() == 12 {
        process_tier9_gift_sink(state, cfg, jpy_surplus_raw)
    } else {
        0.0
    };
    let jpy_surplus_raw = jpy_surplus_raw - t9_jpy_drawn;

    let jpy_surplus     = skim_education_savings(state, cfg, jpy_surplus_raw);
    let usd_surplus     = t4_surplus_usd + t5_surplus_usd;

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
        let jpy_gain_per_share     = jpy_proceeds_per_share - jpy_basis_per_share;
        let usd_gain_per_share     = (price - usd_basis_per_share).max(0.0);

        // V7.5 — Defect 1.1: preserve signed JPY gain for loss carry-forward tracking.
        let japan_tax_per_share_jpy = if jpy_gain_per_share >= 0.0 {
            jpy_gain_per_share * JAPAN_CAPITAL_GAINS_RATE
        } else {
            0.0
        };
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
        // V7.5 — Defect 1.1: preserve signed JPY gain; accumulate losses for carry-forward.
        let jpy_gain_signed = jpy_proceeds - jpy_basis_sold;
        let japan_tax_jpy  = if jpy_gain_signed >= 0.0 {
            jpy_gain_signed * JAPAN_CAPITAL_GAINS_RATE
        } else {
            state.stats.year_japan_cap_loss_jpy += jpy_gain_signed.abs();
            0.0
        };
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

// ─────────────────────────────────────────────────────────────────────────────
//  V7.3 — Family & Education helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Tier 0.5 — Jido Teate (児童手当) child allowance, paid bi-monthly in even
/// calendar months. The per-month rate depends on the child's age in that
/// specific month:
///   age 0 – under 3  → ¥15,000 / month
///   age 3 – under 18 → ¥10,000 / month
///
/// V7.4 — Accrual is computed PER COVERED MONTH (not at payment date). The
/// even-month payment X bundles the rate that applied in month (X-1) plus the
/// rate that applied in month X. This eliminates the ¥2,500–¥10,000 transition-
/// year drift that V7.3 produced when the age-3 or age-18 boundary fell
/// between the two months a single bundled payment covered.
fn compute_jido_teate_jpy(cfg: &Config, current_date: NaiveDate) -> f64 {
    jido_teate_for(cfg.jido_teate_enabled, cfg.child_birth_date, current_date)
}

/// Pure helper extracted from `compute_jido_teate_jpy` so unit tests can hit
/// the production logic without building a full `Config`.
fn jido_teate_for(enabled: bool, child_birth: NaiveDate, on: NaiveDate) -> f64 {
    if !enabled { return 0.0; }
    if on.month() % 2 != 0 { return 0.0; }

    // The even-month payment covers the previous calendar month and the
    // current calendar month. Each is assessed at the rate applicable to the
    // child's age on the first day of that month.
    let prev_month_start = first_of_prev_month(on);
    let cur_month_start  = NaiveDate::from_ymd_opt(on.year(), on.month(), 1).unwrap_or(on);
    monthly_jido_rate(child_birth, prev_month_start) + monthly_jido_rate(child_birth, cur_month_start)
}

/// V7.4 — Per-month rate applied to a single month of Jido Teate accrual.
/// Returns 0 outside the [0, 18) age window.
fn monthly_jido_rate(child_birth: NaiveDate, month_start: NaiveDate) -> f64 {
    let mut age = month_start.year() - child_birth.year();
    if (month_start.month(), month_start.day()) < (child_birth.month(), child_birth.day()) {
        age -= 1;
    }
    if age < 0 || age >= 18 { return 0.0; }
    if age < 3 { 15_000.0 } else { 10_000.0 }
}

/// First-of-the-previous-month relative to `on`. Wraps from January → previous
/// year's December.
fn first_of_prev_month(on: NaiveDate) -> NaiveDate {
    if on.month() == 1 {
        NaiveDate::from_ymd_opt(on.year() - 1, 12, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(on.year(), on.month() - 1, 1).unwrap()
    }
}

/// Tier 2.5 — Education Fund drawdown. Education-tagged expenses pull from
/// `state.education_fund_jpy` first; any residual is covered by a Tier-8 sale
/// sized exactly to the remaining JPY shortfall. Standard waterfall tiers
/// (0,1,3-7) are NOT touched.
fn process_education_expense(
    state: &mut SimState,
    cfg: &Config,
    jpy_owed: f64,
    fx: f64,
    penalty: f64,
) {
    if jpy_owed <= 0.0 { return; }

    let drawn = state.education_fund_jpy.min(jpy_owed);
    state.education_fund_jpy -= drawn;
    state.stats.year_edu_fund_out_jpy += drawn;
    let residual = jpy_owed - drawn;
    if residual <= 0.0 { return; }

    // Fallback: Tier 8 sale sized to the residual JPY.
    info!("   [T2.5] Education shortfall ¥{:.0} — falling through to Tier 8 sale.", residual);
    let recovered = liquidate_for_jpy_target(state, cfg, residual, fx, penalty);
    state.stats.year_edu_fund_out_jpy += recovered;
    if recovered < residual {
        warn!("   [T2.5] Education expense underfunded by ¥{:.0} after T8 fallback.",
            residual - recovered);
    }
}

/// Tier 2.5 accumulation — skim up to `cfg.edu_savings_jpy_monthly` from the
/// available JPY surplus into the Education Fund. Returns the remaining JPY
/// surplus to flow through the normal V7.1 surplus deposit rules.
fn skim_education_savings(state: &mut SimState, cfg: &Config, jpy_surplus: f64) -> f64 {
    let target = if cfg.enable_education_savings { cfg.edu_savings_jpy_monthly } else { 0.0 };
    if target <= 0.0 || jpy_surplus <= 0.0 { return jpy_surplus; }
    let skim = target.min(jpy_surplus);
    state.education_fund_jpy += skim;
    state.stats.year_edu_fund_in_jpy += skim;
    jpy_surplus - skim
}

/// Shielded-mode Tier 8: liquidate just enough to close `gap_jpy`. Identical
/// in effect to the old inline T8 block — extracted so both regimes can share
/// the FX-spread bookkeeping. Returns the residual JPY gap that could not be
/// covered (0 on full recovery).
fn liquidate_for_jpy_gap(
    state: &mut SimState,
    cfg: &Config,
    gap_jpy: f64,
    fx: f64,
    penalty: f64,
) -> f64 {
    let needed_usd = gap_jpy / (fx * (1.0 - penalty).max(f64::EPSILON));
    state.bridge_fund_usd -= needed_usd;
    v7_liquidate_for_deficit(state, cfg);

    let mut residual_jpy = 0.0;
    if state.bridge_fund_usd >= 0.0 {
        // Fully recovered.
    } else {
        residual_jpy = state.bridge_fund_usd.abs() * fx;
        state.bridge_fund_usd = 0.0;
    }
    let recovered_usd = needed_usd - residual_jpy / (fx * (1.0 - penalty)).max(f64::EPSILON);
    if recovered_usd > 0.0 {
        let pen_jpy = recovered_usd * fx * penalty;
        state.stats.year_fx_penalty_jpy += pen_jpy;
        state.bridge_fund_usd -= pen_jpy / fx;
    }
    residual_jpy
}

/// Dynamic-mode / Tier-2.5-fallback liquidation — sells the requested JPY
/// amount (deficit + restock or education shortfall) and returns the actual
/// JPY recovered. The proceeds are passed back rather than deposited so the
/// caller can route them (gap-closure vs buffer-refill vs education-cover).
///
/// V7.4 — Pre-V7.4 this function decremented `state.bridge_fund_usd` by
/// `needed_usd` and then called `v7_liquidate_for_deficit`. Because v7 only
/// sells when the bridge is *negative*, the call silently no-op'd whenever the
/// bridge happened to have headroom — and Mode B's preemptive restock was
/// effectively dead code. V7.4 forces a real deficit by *overwriting* the
/// bridge with `-needed_usd` for the duration of the sale, then restores the
/// pre-call balance afterwards. Net effect: every JPY-target sale request now
/// actually transacts equity until either `needed_usd` is covered or the
/// taxable account is empty.
fn liquidate_for_jpy_target(
    state: &mut SimState,
    cfg: &Config,
    target_jpy: f64,
    fx: f64,
    penalty: f64,
) -> f64 {
    if target_jpy <= 0.0 { return 0.0; }
    let bridge_before = state.bridge_fund_usd;
    let needed_usd = target_jpy / (fx * (1.0 - penalty).max(f64::EPSILON));

    // Force a real deficit so v7_liquidate_for_deficit actually sells.
    state.bridge_fund_usd = -needed_usd;
    v7_liquidate_for_deficit(state, cfg);

    // After v7, bridge_fund_usd sits in [-needed_usd, ~0]. The delta from
    // -needed_usd is the gross USD value of stock sold (already net of Japan
    // capital-gains + state tax that v7 itself debits).
    let raw_recovered_usd = (state.bridge_fund_usd + needed_usd).max(0.0);
    if raw_recovered_usd > 0.0 {
        let pen_jpy = raw_recovered_usd * fx * penalty;
        state.stats.year_fx_penalty_jpy += pen_jpy;
        state.bridge_fund_usd -= pen_jpy / fx;
    }
    let net_recovered_usd = (state.bridge_fund_usd + needed_usd).max(0.0);
    // Restore the bridge to its pre-call balance — proceeds are handed back as JPY.
    state.bridge_fund_usd = bridge_before;

    net_recovered_usd * fx
}

/// V7.4 — Project gross dividend net JPY for an arbitrary forward month.
/// Generalisation of `project_next_month_dividends_jpy`. Tax-advantaged
/// accounts are skipped just like the next-month variant.
fn project_dividends_for_month_jpy(
    state: &SimState,
    target_month: u32,
    fx: f64,
    penalty: f64,
) -> f64 {
    let taxable = match state.accounts.get("Taxable") {
        Some(a) => a,
        None => return 0.0,
    };
    let mut total_jpy = 0.0_f64;
    for asset in taxable.assets.values() {
        let dist_y = asset.total_distribution_yield();
        if dist_y <= 0.0 || asset.qty() <= 0.0 || asset.price <= 0.0 { continue; }
        if !asset.dividend_months.contains(&target_month) { continue; }
        let n = asset.dividend_months.len().max(1) as f64;
        let gross = asset.qty() * asset.price * (dist_y / n);
        match asset.dividend_currency {
            crate::models::assets::DividendCurrency::Jpy => {
                total_jpy += gross * (1.0 - JAPAN_CAPITAL_GAINS_RATE);
            }
            crate::models::assets::DividendCurrency::Usd => {
                total_jpy += gross * fx * (1.0 - penalty);
            }
        }
    }
    total_jpy
}

/// V7.4 — Forward-project the minimum WC (JPY) and Bridge (USD) balances over
/// the next `lookahead_months` assuming NO sales. Income side: lumpy dividend
/// payouts only; outflow side: `target_base_jpy + monthly_non_spend_drain_jpy`
/// per month (V7.5 — Defect 1.4: adds T9 gift and education draws to projection).
///
/// This is the trigger oracle for Dynamic-mode preemptive restocking. It does
/// not mutate state — pure projection.
fn project_buffer_minimums(
    state: &SimState,
    target_base_jpy: f64,
    monthly_non_spend_drain_jpy: f64,
    fx: f64,
    penalty: f64,
    lookahead_months: u32,
) -> (f64, f64) {
    let mut proj_wc_jpy     = state.war_chest_jpy;
    let mut proj_bridge_usd = state.bridge_fund_usd;
    let mut min_wc          = proj_wc_jpy;
    let mut min_bridge_usd  = proj_bridge_usd;

    for ahead in 1..=lookahead_months {
        let cur_mo: i32 = state.date.month() as i32;
        let target_mo = (((cur_mo - 1 + ahead as i32) % 12) + 1) as u32;
        let div_jpy = project_dividends_for_month_jpy(state, target_mo, fx, penalty);
        let net_jpy = div_jpy - target_base_jpy - monthly_non_spend_drain_jpy;

        if net_jpy >= 0.0 {
            proj_wc_jpy += net_jpy;
        } else {
            let deficit = -net_jpy;
            if proj_wc_jpy >= deficit {
                proj_wc_jpy -= deficit;
            } else {
                let remainder = deficit - proj_wc_jpy.max(0.0);
                proj_wc_jpy = 0.0;
                proj_bridge_usd -= remainder / fx;
            }
        }
        if proj_wc_jpy < min_wc        { min_wc = proj_wc_jpy; }
        if proj_bridge_usd < min_bridge_usd { min_bridge_usd = proj_bridge_usd; }
    }
    (min_wc, min_bridge_usd)
}

/// Look-ahead — project the JPY-equivalent net dividend income for the next
/// calendar month, for Mode B sale-sizing. Conservative: applies the FX spread
/// penalty to USD-side projections (over-estimating the JPY drag), and skips
/// tax-free / advantaged accounts (Roth, DC, NISA/iDeCo via jurisdiction None)
/// which never flow to the cashflow waterfall.
fn project_next_month_dividends_jpy(state: &SimState, fx: f64, penalty: f64) -> f64 {
    let next_month = if state.date.month() == 12 { 1 } else { state.date.month() + 1 };
    let taxable = match state.accounts.get("Taxable") {
        Some(a) => a,
        None => return 0.0,
    };

    let mut total_jpy = 0.0_f64;
    for asset in taxable.assets.values() {
        let dist_y = asset.total_distribution_yield();
        if dist_y <= 0.0 || asset.qty() <= 0.0 || asset.price <= 0.0 { continue; }
        if !asset.dividend_months.contains(&next_month) { continue; }
        let n = asset.dividend_months.len().max(1) as f64;
        let gross = asset.qty() * asset.price * (dist_y / n);
        match asset.dividend_currency {
            crate::models::assets::DividendCurrency::Jpy => {
                total_jpy += gross * (1.0 - JAPAN_CAPITAL_GAINS_RATE);
            }
            crate::models::assets::DividendCurrency::Usd => {
                // Approximate net: gross_usd × fx × (1 − penalty), no fed-tax fold-in.
                total_jpy += gross * fx * (1.0 - penalty);
            }
        }
    }
    total_jpy
}

// ─────────────────────────────────────────────────────────────────────────────
//  V7.5 — Tier 9: Estate Planning Gift Sink
// ─────────────────────────────────────────────────────────────────────────────

/// Process Tier 9 annual gift disbursement (fires in December only).
/// Per-recipient evaluation against IRC §2503(b) flags Form 709 obligation.
fn process_tier9_gift_sink(state: &mut SimState, cfg: &Config, surplus_jpy: f64) -> f64 {
    if cfg.gift_recipient_count == 0 || cfg.annual_gift_jpy_per_recipient <= 0.0 {
        return 0.0;
    }
    let annual_total = cfg.annual_gift_jpy_per_recipient * cfg.gift_recipient_count as f64;
    let drawn = annual_total.min(surplus_jpy.max(0.0));
    state.gift_sink_jpy += drawn;
    state.stats.year_gift_sink_jpy += drawn;

    // §2503(b) per-recipient check (USD).
    let per_recipient_usd = cfg.annual_gift_jpy_per_recipient / state.current_fx;
    if per_recipient_usd > cfg.us_gift_exclusion_usd {
        state.stats.year_form_709_required = true;
    }
    drawn
}

// ─────────────────────────────────────────────────────────────────────────────
//  V7.3 — Family & Education tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod v73_tests {
    use super::*;
    use crate::engine::cashflow_engine::ExpenseBreakdown;
    use crate::engine::market_data::MarketDataService;

    #[test]
    fn v73_jido_teate_paid_only_in_even_months() {
        let child = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let odd  = NaiveDate::from_ymd_opt(2025, 5, 1).unwrap();
        let even = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        assert_eq!(jido_teate_for(true, child, odd), 0.0);
        assert_eq!(jido_teate_for(true, child, even), 30_000.0); // age 1 → 15k × 2
    }

    #[test]
    fn v73_jido_teate_rate_drops_at_age_three() {
        let child = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        // 2027-06-01: child is age 2 (turns 3 on 2027-06-15) — pays ¥15k × 2.
        // 2027-08-01: child is age 3 — pays ¥10k × 2.
        let pre  = NaiveDate::from_ymd_opt(2027, 6, 1).unwrap();
        let post = NaiveDate::from_ymd_opt(2027, 8, 1).unwrap();
        assert_eq!(jido_teate_for(true, child, pre), 30_000.0);
        assert_eq!(jido_teate_for(true, child, post), 20_000.0);
    }

    #[test]
    fn v73_jido_teate_ends_at_age_18() {
        let child = NaiveDate::from_ymd_opt(2008, 4, 1).unwrap();
        let inside  = NaiveDate::from_ymd_opt(2026, 2, 1).unwrap(); // age 17
        let outside = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(); // age 18
        assert_eq!(jido_teate_for(true, child, inside), 20_000.0);
        assert_eq!(jido_teate_for(true, child, outside), 0.0);
    }

    #[test]
    fn v73_jido_teate_disabled_flag_zero() {
        let child = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let even  = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        assert_eq!(jido_teate_for(false, child, even), 0.0);
    }

    #[test]
    fn v73_education_breakdown_field_present() {
        // ExpenseBreakdown gained an `education` field — confirm it's wired
        // into the struct literal so downstream waterfall code can read it.
        let exp = ExpenseBreakdown {
            total_desired: 0.0,
            base_desired: 100_000.0,
            base_floor:    60_000.0,
            nhi: 0.0, nenkin: 0.0, restax: 0.0,
            education: 25_000.0,
        };
        assert_eq!(exp.education, 25_000.0);
    }

    #[test]
    fn v73_lumpy_default_dividend_months_quarterly() {
        assert_eq!(MarketDataService::default_dividend_months("VTI"),  vec![3, 6, 9, 12]);
        assert_eq!(MarketDataService::default_dividend_months("SCHD"), vec![3, 6, 9, 12]);
        assert_eq!(MarketDataService::default_dividend_months("PANW"), vec![] as Vec<u32>);
        assert_eq!(MarketDataService::default_dividend_months("BND").len(), 12);
    }
}

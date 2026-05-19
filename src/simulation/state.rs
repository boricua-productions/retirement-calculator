use chrono::NaiveDate;
use std::collections::HashMap;

use crate::models::assets::Account;
use crate::models::snapshot::{AccountSnapshotRow, AnnualSnapshot, FtcCarryoverQueue, PficDriftWarning, RsuSellToCoverWarning, SolvencyWarning, TransitionReport};
use super::stats::AnnualStats;

/// V8.0 — Rolling 3-year Japan capital-loss carry-forward ledger.
/// Per 租税特別措置法 第37条の12の2 (Measures Act Art. 37-12-2), realized losses
/// can offset gains in the same year and the three subsequent calendar years.
#[derive(Debug, Default, Clone)]
pub struct JapanLossLedger {
    /// Losses realized exactly 1 year ago (still within the carry-forward window).
    pub year_minus_1: f64,
    /// Losses realized exactly 2 years ago.
    pub year_minus_2: f64,
    /// Losses realized exactly 3 years ago (last eligible year).
    pub year_minus_3: f64,
}

impl JapanLossLedger {
    /// Total carry-forward available against this year's gains (JPY).
    pub fn total(&self) -> f64 {
        self.year_minus_1 + self.year_minus_2 + self.year_minus_3
    }

    /// Advance the ledger one year: discard year_minus_3 (expired), shift older
    /// slots down, and load the current year's loss into year_minus_1.
    pub fn roll(&mut self, current_year_loss: f64) {
        self.year_minus_3 = self.year_minus_2;
        self.year_minus_2 = self.year_minus_1;
        self.year_minus_1 = current_year_loss.max(0.0);
    }
}

/// The complete mutable simulation state.
/// Passed by `&mut` reference to all handlers, eliminating the circular reference
/// anti-pattern from the Python implementation.
///
/// Mirrors `SimulationController`'s instance variables in Python's `retirement_engine.py`.
#[derive(Debug)]
pub struct SimState {
    // ── Time ───────────────────────────────────────────────────────────────────
    /// Current simulation date (the first day of the month being processed).
    pub date: NaiveDate,

    // ── FX ─────────────────────────────────────────────────────────────────────
    /// Current USD/JPY exchange rate. Drifts post-retirement if fx_drift_enabled.
    pub current_fx: f64,

    // ── Limits ──────────────────────────────────────────────────────────────────
    /// Current Roth IRA annual contribution limit. Grows by `ira_limit_growth` after 2025.
    pub ira_limit: f64,

    // ── Buffers ─────────────────────────────────────────────────────────────────
    /// V7.1 — War chest (emergency cash reserve). Always JPY-denominated.
    pub war_chest_jpy: f64,
    /// V8.5 — Current war-chest cap for THIS year. Seeded at the retirement
    /// transition from the configured target, then evolved annually per
    /// `cfg.war_chest_cap_policy`. All surplus-deposit caps read this.
    pub war_chest_target_effective_jpy: f64,
    /// V8.5 — Set true once the EmptyOnDate drain has fired (one-shot guard).
    pub war_chest_emptied: bool,
    /// V7.1 — Bridge fund / operating cash reserve. Always USD-denominated.
    pub bridge_fund_usd: f64,
    /// V7.3 — Tier 2.5 Education Fund (JPY-denominated). Accumulated from
    /// post-spend surplus at rate `cfg.edu_savings_jpy_monthly`; drained only by
    /// expense rules whose name contains "Education", which bypass the standard
    /// waterfall (T2.5 → T8 fallback).
    pub education_fund_jpy: f64,
    /// Stage 12 — Gradual buffer accumulation. JPY cash being set aside for the
    /// war chest during the pre-retirement ramp period.
    pub war_chest_accumulating_jpy: f64,
    /// Stage 12 — USD cash being set aside for the bridge fund during ramp.
    pub bridge_fund_accumulating_usd: f64,

    // ── DC Payout ────────────────────────────────────────────────────────────────
    pub dc_payout_active: bool,
    /// Months remaining in 20-year annuity payout (240 at start).
    pub dc_months_remaining: u32,

    // ── Flags ────────────────────────────────────────────────────────────────────
    pub roth_rebalance_executed: bool,
    pub bridge_exhausted_logged: bool,

    // ── FTC Carryover (IRC §904) ──────────────────────────────────────────────────
    /// Unused Foreign Tax Credit carried forward from prior years (USD).
    /// Applied to reduce US federal liability before new credits each December.
    /// Retained for snapshot reporting; equals passive + general totals.
    pub ftc_carryover_usd: f64,
    /// V7.6 — Passive-basket FTC carryover (IRC §904(c)). Dividends, cap-gains
    /// distributions, interest, PFIC §1296 MTM. Must not offset general-basket tax.
    pub ftc_carryover_passive_usd: f64,
    /// V7.6 — General-basket FTC carryover (IRC §904(c)). FERS, SS, SSDI, RSU.
    pub ftc_carryover_general_usd: f64,
    /// V8.0 — IRC §904(c) FIFO carryover queue per basket (10-year lifetime).
    /// The legacy scalar fields above are derived mirrors of this queue's totals.
    pub ftc_queue: FtcCarryoverQueue,

    // ── V8.0 — Japan 3-Year Capital-Loss Ledger ───────────────────────────────────
    /// Per 租税特別措置法 第37条の12の2: losses carry forward up to 3 years.
    pub japan_loss_ledger: JapanLossLedger,

    // ── V7.5 — Estate Planning (Gift Sink) ───────────────────────────────────────
    /// Cumulative JPY routed to the Tier 9 Gift Sink (held outside the waterfall).
    pub gift_sink_jpy: f64,

    // ── V7.5 — Ninki Keizoku (NHI continuation) ──────────────────────────────────
    /// Months remaining in a Ninki Keizoku (任意継続) Shakai Hoken window.
    /// Counts down each scheduling tick; falls back to NhiModel fallback when zero.
    pub nhi_ninki_keizoku_months_remaining: u32,

    // ── Forced Liquidation Tracking ───────────────────────────────────────────────
    /// Lifetime total of taxable portfolio sold to cover cash deficits (USD).
    pub total_forced_liquidations_usd: f64,

    // ── Recession / Recovery Trajectory ──────────────────────────────────────────
    /// True while a multi-month drawdown or its recovery phase is in progress.
    /// Surplus reinvestment is suppressed while this is set to preserve bridge liquidity.
    pub recession_active: bool,
    /// Months remaining in an active multi-month drawdown (0 = no active drawdown).
    pub recession_months_remaining: u32,
    /// Per-month shock rate for the active drawdown, pre-computed from RecessionEvent.
    pub recession_monthly_shock_rate: f64,
    /// Months remaining in an active V-shaped recovery (0 = no active recovery).
    pub recovery_months_remaining: u32,
    /// Per-month price-increase rate for the active recovery, pre-computed from RecessionEvent.
    pub recovery_monthly_boost_rate: f64,

    // ── Accounts ─────────────────────────────────────────────────────────────────
    /// "Taxable", "Roth", "DC" — the three core investment accounts.
    pub accounts: HashMap<String, Account>,

    // ── Annual Statistics ────────────────────────────────────────────────────────
    pub stats: AnnualStats,
    /// Annual FERS gross income history: year → gross_usd.
    /// Used to calculate NHI premiums based on prior-year FERS.
    pub fers_history: HashMap<i32, f64>,
    /// Annual social insurance (NHI + Nenkin) paid: year → total_jpy.
    /// Used to calculate resident tax deductions.
    pub social_insurance_history: HashMap<i32, f64>,
    /// Annual gross dividend income history: year → gross_usd.
    /// Used to include US investment income in the NHI basis when enabled.
    pub div_income_history: HashMap<i32, f64>,
    /// V7.7 — Annual gross salary history: year → gross_jpy.
    /// Captured in December of each pre-retirement year; drives N-1 resident tax.
    pub salary_history: HashMap<i32, f64>,
    /// V7.7 — Annual RSU vest value history: year → vest_jpy.
    /// Captured in December of each pre-retirement year; combined with salary
    /// for the N-1 resident tax N-1 hand-off.
    pub rsu_vest_history: HashMap<i32, f64>,

    // ── Quarterly Cashflow Tracking ───────────────────────────────────────────────
    pub qtr_inc_jpy: f64,
    pub qtr_exp_jpy: f64,
    /// V7.1 — Net USD dividend income received this month, handed from the dividend
    /// handler to the cashflow manager. Zero in non-paying months (lumpy dividends).
    pub current_month_div_net_usd: f64,
    /// V7.1 — Net JPY dividend income received this month (from JPY-denominated assets).
    /// Zero in non-paying months. Routes directly to the War Chest bucket.
    pub current_month_div_net_jpy: f64,

    // ── Outputs ───────────────────────────────────────────────────────────────────
    pub annual_summary: Vec<AnnualSnapshot>,
    pub gap_warnings: Vec<SolvencyWarning>,
    pub transition_report: Option<TransitionReport>,

    // ── V7.7.2 — RSU Sell-to-Cover Realism ───────────────────────────────────────
    /// Cumulative unpaid IRS tax liability from SELL_TO_COVER deficit events (USD).
    pub unpaid_rsu_tax_liability_usd: f64,
    /// All RSU margin-call deficit events recorded during the simulation.
    pub rsu_sell_to_cover_warnings: Vec<RsuSellToCoverWarning>,

    // ── Stage 04 — Shock ordering audit ───────────────────────────────────────────
    /// Total portfolio net worth (JPY) captured just before any shock events fire
    /// in a year with both a recession and an FX shock. `None` in all other years.
    pub shock_pre_net_worth_jpy: Option<f64>,
    /// Total portfolio net worth (JPY) captured after all shock events commit.
    pub shock_post_net_worth_jpy: Option<f64>,

    // ── Stage 05 — PFIC basis drift tracking ─────────────────────────────────────
    /// Accumulated PFIC basis drift warnings for the entire simulation run.
    pub pfic_basis_drift_warnings: Vec<PficDriftWarning>,
    /// Annual PFIC MTM JPY income history: year → total_jpy (non-tax-advantaged accounts).
    /// Archived in January of each new year; read by the Japan resident-tax scheduler.
    pub pfic_mtm_jpy_history: HashMap<i32, f64>,

    // ── Stage 06 — Real Estate ────────────────────────────────────────────────────
    /// Total HELOC drawn across all properties (USD). Accumulates with each
    /// Tier 7.5 draw; never auto-repaid in this simulation model.
    pub outstanding_heloc_usd: f64,

    // ── V8.2 — Per-account snapshots ─────────────────────────────────────────────
    /// Populated at Retirement, each per-account Rebalance, and FinalYear.
    pub account_snapshots: Vec<AccountSnapshotRow>,
}

impl SimState {
    pub fn new(
        start_date: NaiveDate,
        start_fx: f64,
        ira_limit: f64,
        accounts: HashMap<String, Account>,
    ) -> Self {
        Self {
            date: start_date,
            current_fx: start_fx,
            ira_limit,
            war_chest_jpy: 0.0,
            war_chest_target_effective_jpy: 0.0,
            war_chest_emptied: false,
            bridge_fund_usd: 0.0,
            education_fund_jpy: 0.0,
            war_chest_accumulating_jpy: 0.0,
            bridge_fund_accumulating_usd: 0.0,
            dc_payout_active: false,
            dc_months_remaining: 240,
            roth_rebalance_executed: false,
            bridge_exhausted_logged: false,
            ftc_carryover_usd: 0.0,
            ftc_carryover_passive_usd: 0.0,
            ftc_carryover_general_usd: 0.0,
            ftc_queue: FtcCarryoverQueue::default(),
            japan_loss_ledger: JapanLossLedger::default(),
            gift_sink_jpy: 0.0,
            nhi_ninki_keizoku_months_remaining: 0,
            total_forced_liquidations_usd: 0.0,
            recession_active: false,
            recession_months_remaining: 0,
            recession_monthly_shock_rate: 0.0,
            recovery_months_remaining: 0,
            recovery_monthly_boost_rate: 0.0,
            accounts,
            stats: AnnualStats::default(),
            fers_history: HashMap::new(),
            social_insurance_history: HashMap::new(),
            div_income_history: HashMap::new(),
            salary_history: HashMap::new(),
            rsu_vest_history: HashMap::new(),
            qtr_inc_jpy: 0.0,
            qtr_exp_jpy: 0.0,
            current_month_div_net_usd: 0.0,
            current_month_div_net_jpy: 0.0,
            annual_summary: Vec::new(),
            gap_warnings: Vec::new(),
            transition_report: None,
            unpaid_rsu_tax_liability_usd: 0.0,
            rsu_sell_to_cover_warnings: Vec::new(),
            shock_pre_net_worth_jpy: None,
            shock_post_net_worth_jpy: None,
            pfic_basis_drift_warnings: Vec::new(),
            pfic_mtm_jpy_history: HashMap::new(),
            outstanding_heloc_usd: 0.0,
            account_snapshots: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::JapanLossLedger;

    #[test]
    fn japan_loss_ledger_roll_and_total() {
        let mut l = JapanLossLedger::default();
        assert_eq!(l.total(), 0.0);

        l.roll(1_000_000.0);
        assert_eq!(l.year_minus_1, 1_000_000.0);
        assert_eq!(l.total(), 1_000_000.0);

        l.roll(2_000_000.0);
        assert_eq!(l.year_minus_1, 2_000_000.0);
        assert_eq!(l.year_minus_2, 1_000_000.0);
        assert_eq!(l.total(), 3_000_000.0);

        l.roll(500_000.0);
        assert_eq!(l.year_minus_1, 500_000.0);
        assert_eq!(l.year_minus_2, 2_000_000.0);
        assert_eq!(l.year_minus_3, 1_000_000.0);
        assert_eq!(l.total(), 3_500_000.0);

        // Fourth roll: year_minus_3 (the oldest 1M) is discarded.
        l.roll(0.0);
        assert_eq!(l.year_minus_1, 0.0);
        assert_eq!(l.year_minus_2, 500_000.0);
        assert_eq!(l.year_minus_3, 2_000_000.0);
        assert_eq!(l.total(), 2_500_000.0);
    }

    #[test]
    fn japan_loss_ledger_negative_input_clamped_to_zero() {
        let mut l = JapanLossLedger::default();
        l.roll(-500_000.0);
        assert_eq!(l.year_minus_1, 0.0);
        assert_eq!(l.total(), 0.0);
    }

    #[test]
    fn japan_loss_ledger_three_year_decay() {
        let mut l = JapanLossLedger::default();
        l.roll(1_000_000.0); // Y1
        l.roll(0.0);          // Y2: loss moves to year_minus_2
        l.roll(0.0);          // Y3: loss moves to year_minus_3
        assert_eq!(l.total(), 1_000_000.0, "3-year-old loss still within window");
        l.roll(0.0);          // Y4: loss falls off year_minus_3 → expired
        assert_eq!(l.total(), 0.0, "4-year-old loss must be discarded");
    }
}

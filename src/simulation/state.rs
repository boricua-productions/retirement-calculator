use chrono::NaiveDate;
use std::collections::HashMap;

use crate::models::assets::Account;
use crate::models::snapshot::{AnnualSnapshot, SolvencyWarning, TransitionReport};
use super::stats::AnnualStats;

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
    /// V7.1 — Bridge fund / operating cash reserve. Always USD-denominated.
    pub bridge_fund_usd: f64,

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
    pub ftc_carryover_usd: f64,

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
            bridge_fund_usd: 0.0,
            dc_payout_active: false,
            dc_months_remaining: 240,
            roth_rebalance_executed: false,
            bridge_exhausted_logged: false,
            ftc_carryover_usd: 0.0,
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
            qtr_inc_jpy: 0.0,
            qtr_exp_jpy: 0.0,
            current_month_div_net_usd: 0.0,
            current_month_div_net_jpy: 0.0,
            annual_summary: Vec::new(),
            gap_warnings: Vec::new(),
            transition_report: None,
        }
    }
}

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::models::config::{InvestmentLocation, TaxJurisdiction};

/// An annual financial snapshot recorded at December 31 of each simulated year.
/// Mirrors the dict appended to `annual_summary` in Python's `_record_annual_snapshot`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnualSnapshot {
    pub year: i32,
    pub usd_jpy: f64,

    // Portfolio values
    pub brokerage_usd: f64,
    pub roth_usd: f64,
    pub dc_jpy: f64,

    // Income (USD)
    pub div_gross_usd: f64,
    pub div_net_usd: f64,
    pub fers_net_usd: f64,
    pub va_net_usd: f64,
    pub rsu_vest_usd: f64,
    pub total_inc_net_usd: f64,
    pub total_inc_net_jpy: f64,

    // Expenses (JPY)
    pub base_exp_jpy: f64,
    pub nhi_obligation_jpy: f64,
    pub nenkin_jpy: f64,
    pub res_tax_jpy: f64,
    pub total_exp_jpy: f64,

    // Cash flow
    pub gap_jpy: f64,

    // Buffers
    pub bridge_fund_usd: f64,
    pub war_chest_jpy: f64,
    pub war_chest_used_jpy: f64,

    // Tax — dual-jurisdiction reporting
    /// Total US federal + state income tax charged this year (USD).
    pub us_tax_charged_usd: f64,
    /// Total Japan resident tax charged this year (JPY). Mirrors res_tax_jpy for clarity.
    pub japan_tax_charged_jpy: f64,
    /// Legacy: external US tax paid from salary (pre-retirement RSU/SALARY mode).
    pub ext_tax_paid_usd: f64,

    // Totalization pension pillars
    /// US Social Security income received this year (USD).
    pub ss_payout_usd: f64,
    /// Japanese Nenkin pension income received this year (JPY).
    pub nenkin_income_jpy: f64,
    /// Whether FEIE was applied this year (vs FTC-only) in US tax calculation.
    pub feie_applied: bool,

    // Stress-test audit columns (V5.2)
    /// True if the bridge fund went negative at any point this year.
    pub bridge_exhausted: bool,
    /// Value of taxable portfolio force-sold to cover cash deficits this year (USD).
    pub forced_liquidations_usd: f64,
    /// Unused Foreign Tax Credit carried forward into the next year (USD).
    pub ftc_carryover_usd: f64,

    // Currency stress columns (V5.3)
    /// USD cost of minimum monthly expenses at year-end FX rate (min_expense_jpy / fx).
    /// Structural floor: USD income must exceed this to avoid drawdown at floor spending.
    pub purchasing_power_usd: f64,

    // Sustainability columns (V6.0)
    /// Dividend Coverage Ratio: annual gross dividend income (converted to JPY) ÷ total expenses.
    /// > 1.0 means dividends alone cover all expenses; < 1.0 indicates a shortfall.
    pub div_coverage_ratio: f64,

    // ── V7.0 — Liquidation tax breakout ─────────────────────────────────────
    /// Japan capital-gains tax realised on V7.0 liquidations this year (¥).
    pub japan_cap_gains_tax_jpy: f64,
    /// US state capital-gains tax reserved on V7.0 liquidations this year ($).
    pub state_cap_gains_tax_usd: f64,

    // ── V7.1 — Defensive Waterfall analytics ────────────────────────────────
    /// Cumulative FX spread penalty paid this year when converting USD income to JPY (¥).
    pub fx_penalty_jpy: f64,
    /// Number of months this year where the spending target was dropped to the floor.
    pub months_at_min_target: u32,
}

/// A quarterly solvency warning recorded when income < expenses for a quarter.
/// Mirrors the dict appended to `gap_warnings` in Python.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolvencyWarning {
    pub date: String,
    pub fx_rate: f64,
    pub qtr_income_jpy: f64,
    pub qtr_expenses_jpy: f64,
    pub gap_jpy: f64,
    pub bridge_fund_left_usd: f64,
    pub absorbed_by: String,
    pub notes: String,
}

/// Details of a single buy transaction during the retirement rebalance event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuyRecord {
    pub ticker: String,
    pub qty_bought: f64,
    pub cost: f64,
}

/// Details of a single sell transaction during the retirement rebalance event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SellRecord {
    pub ticker: String,
    pub action: String,
    pub qty_sold: f64,
    pub price: f64,
    pub proceeds: f64,
}

/// The full allocation/tax breakdown from the retirement transition event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionAllocation {
    pub prorated_base_income: f64,
    pub us_tax_bill: f64,
    pub us_tax_breakdown: HashMap<String, f64>,
    pub total_st_gains: f64,
    pub total_lt_gains: f64,
    pub total_niit: f64,
    pub us_tax_pre: f64,
    pub us_tax_paid_from_portfolio: f64,
    pub wc_target: f64,
    pub wc_currency: String,
    pub wc_paid_from_portfolio_usd: f64,
    pub wc_pre: f64,
    pub bridge_total_jpy: f64,
    pub bridge_pre_general_jpy: f64,
    pub bridge_fund_currency: String,
    pub jp_tax_pre_jpy: f64,
    pub bridge_pull_usd: f64,
    pub jp_tax_bill: f64,
    pub reinvested_cash: f64,
}

/// The complete retirement transition report, generated once during the rebalance event.
/// Mirrors Python's `self.transition_report` dict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionReport {
    pub date: NaiveDate,
    pub fx_rate: f64,
    pub pre_val: f64,
    pub post_val: f64,
    pub yield_post: f64,
    pub sells: Vec<SellRecord>,
    pub buys: Vec<BuyRecord>,
    pub allocation: TransitionAllocation,
}

/// All results produced by a complete simulation run.
#[derive(Debug, Clone)]
pub struct SimResults {
    pub annual_summary: Vec<AnnualSnapshot>,
    pub gap_warnings: Vec<SolvencyWarning>,
    pub transition_report: Option<TransitionReport>,
    pub tax_jurisdiction: TaxJurisdiction,
    pub investment_location: InvestmentLocation,
    /// Prefecture of residence used for Japan resident tax rate lookup.
    pub prefecture: String,
    /// City of residence used for Japan resident tax rate lookup.
    pub city: String,
}

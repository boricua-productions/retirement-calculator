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

    // ── V7.5 — Exit Tax Monitor ──────────────────────────────────────────────
    /// True when the year-end position triggers Japan Exit Tax exposure (Art. 60-2).
    pub exit_tax_triggered: bool,
    /// Total Japan-subject financial asset value at year-end (¥).
    pub exit_tax_asset_value_jpy: f64,

    // ── V7.5 — Estate Planning ───────────────────────────────────────────────
    /// JPY diverted into the Tier 9 Gift Sink this year.
    pub year_gift_sink_jpy: f64,
    /// True if any per-recipient gift exceeded the US §2503(b) annual exclusion.
    pub year_form_709_required: bool,

    // ── V7.6 — Tax Friction & component breakdown ────────────────────────────
    /// Gross investment return this year before any taxes/expenses (USD).
    /// = price growth (pre-expense-ratio) × avg portfolio value + total distributions.
    #[serde(default)]
    pub total_gross_return_usd: f64,
    /// Net investment return after expense ratios and distribution taxes (USD).
    #[serde(default)]
    pub total_net_return_usd: f64,
    /// Tax + expense friction (gross - net) for the year (USD). Surfaced so the
    /// user can compare regimes without naming the underlying tax categories.
    #[serde(default)]
    pub tax_friction_usd: f64,
    /// Distribution breakdown (USD). Audit-only — Overview rolls these into Dividends.
    #[serde(default)] pub dist_dividend_usd: f64,
    #[serde(default)] pub dist_interest_usd: f64,
    #[serde(default)] pub dist_cap_gains_usd: f64,
    #[serde(default)] pub dist_special_usd: f64,
    #[serde(default)] pub dist_roc_usd: f64,

    // ── V7.7 — Working-year Japan income tax ────────────────────────────────
    /// Japan income tax (所得税) paid in working years (pre-retirement). Zero in
    /// all post-retirement years. Used to verify the Japan-first pipeline.
    #[serde(default)]
    pub japan_income_tax_jpy: f64,

    // ── V7.7.2 — RSU Sell-to-Cover Realism ──────────────────────────────────
    /// Cumulative unpaid IRS tax liability arising from SELL_TO_COVER deficit
    /// events that could not be fully covered by the fallback cascade (USD).
    /// Resets to 0 only if the simulation is run with realism disabled.
    #[serde(default)]
    pub unpaid_rsu_tax_liability_usd: f64,

    // ── Stage 04 — Shock Ordering audit ──────────────────────────────────────
    /// Japan CPI purchasing-power index relative to the simulation start year.
    /// 1.0 in year 0, compounded by `inflation_japan` each subsequent year.
    /// Separates price-level changes from cash-position changes.
    #[serde(default)]
    pub jpy_purchasing_power_index: f64,
    /// Total portfolio net worth in JPY immediately before any shock events
    /// fired this year. `None` in years without a combined recession+FX shock.
    #[serde(default)]
    pub pre_shock_net_worth_jpy: Option<f64>,
    /// Total portfolio net worth in JPY immediately after all shock events
    /// committed this year. `None` in years without a combined recession+FX shock.
    #[serde(default)]
    pub post_shock_net_worth_jpy: Option<f64>,

    // ── Stage 05 — PFIC MTM phantom income ───────────────────────────────────
    /// Total §1296 MTM gain for the year after §1296(d) carry-forward offset (USD).
    #[serde(default)]
    pub pfic_mtm_income_usd: f64,
    /// Total §1296 MTM gain for the year in JPY (non-NISA/iDeCo accounts only).
    #[serde(default)]
    pub pfic_mtm_income_jpy: f64,
}

/// Stage 05 — Emitted when the USD×FX vs JPY MTM basis diverges by > 1%.
/// The engine self-heals (resets JPY basis) immediately after emitting this warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PficDriftWarning {
    pub year: i32,
    pub ticker: String,
    /// Percentage drift: abs(usd_basis×fx − jpy_basis) / jpy_basis × 100.
    pub drift_pct: f64,
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

/// V7.7.2 — RSU SELL_TO_COVER margin-call event record.
/// Emitted whenever the vest price (post-recession) cannot fully fund the
/// combined US + Japan tax bill and the fallback cascade still leaves a deficit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsuSellToCoverWarning {
    pub date: String,
    pub ticker: String,
    /// Gross USD value of the vest event.
    pub vest_value_usd: f64,
    /// Combined US + Japan tax liability (USD) at the time of the vest.
    pub combined_tax_usd: f64,
    /// Shortfall between vest proceeds and combined tax (vest_value < combined_tax).
    pub deficit_usd: f64,
    /// Residual that could NOT be covered after exhausting Bridge + War Chest + T8.
    pub uncovered_usd: f64,
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
    /// V7.7.2 — RSU margin-call deficit events (non-empty only when realism on).
    pub rsu_sell_to_cover_warnings: Vec<RsuSellToCoverWarning>,
    /// Stage 02 — Effective US filing status the engine actually used.
    /// Derived from `spouse_profile` (e.g. NraMfs → "Married Filing Separately")
    /// and surfaced in the Overview tab so the user can confirm the right path ran.
    pub effective_filing_status: String,
    /// Stage 05 — PFIC basis drift events recorded during the simulation.
    /// Non-empty only when `track_pfic_basis_drift` is true and drift > 1% occurred;
    /// the engine self-heals immediately, so a non-zero count flags precision loss.
    pub pfic_basis_drift_warnings: Vec<PficDriftWarning>,
}

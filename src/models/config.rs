use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::expense::ExpenseRule;
use super::rsu::RsuAward;

fn default_one() -> u32 { 1 }
fn default_fx_spread_penalty() -> f64 { 0.005 }
fn default_true() -> bool { true }
fn default_us_gift_exclusion() -> f64 { 19_000.0 }
fn default_tlh_months() -> Vec<u32> { vec![11, 12] }
fn default_tlh_threshold() -> f64 { 500.0 }
fn default_rsu_realism() -> bool { true }
fn default_estate_jurisdiction() -> TaxProtocol { TaxProtocol::Both }
fn default_war_chest_ramp_months() -> u32 { 24 }
fn default_bridge_fund_ramp_months() -> u32 { 18 }

/// Stage 04 — Order of operations when a recession and FX shock fall in the same year.
///
/// Because the JPY purchasing-power audit trail is path-dependent, the user can choose
/// which mental model best matches their scenario:
///
/// | Variant | Equity drop | FX move | Intermediate JPY value |
/// |---------|------------|---------|------------------------|
/// | `DepreciateThenReprice` (default) | first | second | lower (conservative) |
/// | `RepriceThenDepreciate` | second | first | higher (equity looks smaller in JPY) |
/// | `Simultaneous` | snapshot | snapshot | no intermediate — path-independent |
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ShockOrdering {
    /// Equity drops first (at the old FX rate), then FX moves. Conservative: the
    /// JPY loss appears at its largest. This is the V7.x legacy behaviour.
    #[default]
    DepreciateThenReprice,
    /// FX moves first, then equity drops (at the new FX rate). The equity loss
    /// may appear smaller in JPY terms when the yen has already strengthened.
    RepriceThenDepreciate,
    /// Both shocks are computed against a snapshot of the pre-shock state and
    /// then committed together. Path-independent; recommended for comparability.
    Simultaneous,
}

impl std::fmt::Display for ShockOrdering {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShockOrdering::DepreciateThenReprice => write!(f, "Equity drop first, then FX repricing"),
            ShockOrdering::RepriceThenDepreciate => write!(f, "FX repricing first, then equity drop"),
            ShockOrdering::Simultaneous          => write!(f, "Simultaneous (snapshot both, commit together)"),
        }
    }
}

/// V7.7.2 — Controls how aggressive the SELL_TO_COVER deficit cascade is.
///
/// `Strict` (default): drains Bridge Fund → War Chest → Tier 8 liquidation,
/// then records any residual as an unpaid IRS liability.
/// `Permissive`: legacy behaviour — floors `net` at zero and silently drops the deficit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RsuSellToCoverPolicy {
    #[default]
    Strict,
    Permissive,
}

/// Stage 12 — Controls when buffer cash is raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BufferFundingTiming {
    /// Raise all cash in a single portfolio liquidation at retirement date.
    #[default]
    AtRetirement,
    /// Gradually accumulate cash from monthly income over a ramp period
    /// leading up to retirement, reducing the tax-heavy lump sale.
    GraduallyBeforeRetirement,
}

// ─── Japan NHI model ─────────────────────────────────────────────────────────

/// Rate schedule for a Japanese municipality's NHI (国民健康保険) system.
/// Sagamihara City, Kanagawa 2026 values are the built-in defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NhiCalculatedRates {
    // Medical component (医療分)
    pub medical_rate:       f64,    // income rate  (e.g. 0.0846 for 8.46%)
    pub per_capita_medical: f64,    // per insured  (e.g. ¥33,600)
    pub cap_medical:        f64,    // annual cap   (e.g. ¥650,000)
    // Elderly support component (支援分)
    pub elderly_support_rate: f64,  // income rate  (e.g. 0.0204 for 2.04%)
    pub per_capita_support:   f64,  // per insured  (e.g. ¥11,400)
    pub cap_support:          f64,  // annual cap   (e.g. ¥240,000)
    // Nursing care component (介護分, ages 40–64 only)
    pub nursing_care_rate:  f64,    // income rate  (e.g. 0.0202 for 2.02%)
    pub per_capita_nursing: f64,    // per insured  (e.g. ¥12,600)
    pub cap_nursing:        f64,    // annual cap   (e.g. ¥170,000)
    /// When true, US investment income (dividends) converted to JPY is added to
    /// the NHI income basis to capture global earnings in Japan's assessment.
    pub include_us_investment_income: bool,
}

impl NhiCalculatedRates {
    /// Sagamihara City (相模原市), Kanagawa — 2026 rate schedule.
    pub fn sagamihara_2026() -> Self {
        Self {
            medical_rate:             0.0846,
            per_capita_medical:    33_600.0,
            cap_medical:          650_000.0,
            elderly_support_rate:     0.0204,
            per_capita_support:    11_400.0,
            cap_support:          240_000.0,
            nursing_care_rate:        0.0202,
            per_capita_nursing:    12_600.0,
            cap_nursing:          170_000.0,
            include_us_investment_income: false,
        }
    }
}

/// NHI premium model: either a full municipal rate schedule or manual fixed amounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum NhiModel {
    /// Full rate-schedule calculation with 1-year income lookback.
    /// Uses actual prior-year income to compute the NHI spike (year 1) vs. ongoing premium.
    Calculated(NhiCalculatedRates),
    /// Static overrides — user provides the known annual totals.
    /// `spike_year_total_jpy` is used in the first post-retirement year;
    /// `ongoing_annual_total_jpy` is used in all subsequent years.
    ManualOverride {
        spike_year_total_jpy:     f64,
        ongoing_annual_total_jpy: f64,
    },
    /// V7.5 — Voluntary Continuation (任意継続) of employer Shakai Hoken
    /// for `duration_months` (max 24 per HIA Art. 37). Replaces NHI for that
    /// window; falls back to the `fallback` model thereafter.
    NinkiKeizoku {
        monthly_premium_jpy: f64,
        duration_months: u32,
        fallback: Box<NhiModel>,
    },
}

impl Default for NhiModel {
    fn default() -> Self {
        NhiModel::Calculated(NhiCalculatedRates::sagamihara_2026())
    }
}

/// A scheduled market recession event for stress-testing sequence-of-returns risk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecessionEvent {
    pub year: i32,
    /// Total portfolio drawdown as a fraction (e.g. 0.30 = −30%).
    pub severity: f64,
    /// Months over which the drawdown is spread (1 = legacy single-shock in January).
    #[serde(default = "default_one")]
    pub duration_months: u32,
    /// Months for a V-shaped recovery after the drawdown completes (0 = no auto-recovery).
    #[serde(default)]
    pub recovery_months: u32,
}

impl RecessionEvent {
    /// Per-month shock rate derived from total severity spread over `duration_months`.
    /// Compounded application: `portfolio *= (1.0 - monthly_rate)` each month.
    #[allow(dead_code)]  // used in Step 2 multi-month drawdown wiring
    pub fn monthly_shock_rate(&self) -> f64 {
        let months = self.duration_months.max(1) as f64;
        1.0 - (1.0 - self.severity).powf(1.0 / months)
    }
}

/// A discrete FX shock event for stress-testing currency volatility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FXShockEvent {
    pub year: i32,
    /// Target USD/JPY rate applied at the start of this year (e.g. 80.0 for ¥80/$).
    pub target_fx: f64,
    #[serde(default)]
    pub description: String,
}

/// V7.0 — Withdrawal strategy controlling the post-retirement liquidation waterfall.
///
/// The default (`TotalReturn`) preserves the V6.6 behaviour where shares may be sold
/// to cover any post-cash + post-war-chest deficit. `DividendOnly` disables share sales
/// entirely (insolvency surfaces as bridge_exhausted). `Hybrid` mirrors `TotalReturn`
/// in this engine — both pull from the highest-JPY-basis lots first to minimise
/// realised gains in the early withdrawal years (preserves portfolio longevity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WithdrawalStrategy {
    /// Live strictly off dividends + cash + war chest. Never sell shares.
    DividendOnly,
    /// Default V6.6 behaviour: full liquidation waterfall when buffers are exhausted.
    #[default]
    TotalReturn,
    /// Same waterfall as TotalReturn; reserved for future divergence.
    Hybrid,
}

impl std::fmt::Display for WithdrawalStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WithdrawalStrategy::DividendOnly => write!(f, "Dividend Only"),
            WithdrawalStrategy::TotalReturn  => write!(f, "Total Return"),
            WithdrawalStrategy::Hybrid       => write!(f, "Hybrid"),
        }
    }
}

/// V7.1 — Selects which spending waterfall algorithm runs post-retirement.
///
/// `Defensive` (default): taps buffers to maintain Base spending before cutting
/// quality-of-life to the Minimum floor.
/// `Cautious` (V7.0 legacy): cuts spending to actual income first; buffers are a
/// last resort. Use for backward-compatible scenario comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WaterfallStrategy {
    #[default]
    Defensive,
    Cautious,
}

impl std::fmt::Display for WaterfallStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaterfallStrategy::Defensive => write!(f, "Defensive (V7.1)"),
            WaterfallStrategy::Cautious  => write!(f, "Cautious (V7.0)"),
        }
    }
}

/// V7.3 — Selects buffer-management behaviour inside the Defensive waterfall.
///
/// `Shielded` (Mode A — default): Exhaust monthly inflows → drain JPY war chest →
/// drain USD bridge → liquidate equity only as a last resort. When all cash
/// buffers hit zero the target is forced to the Minimum floor (Tier 8 sizes
/// against minimum, not base) — protecting the long-term portfolio at the cost
/// of quality-of-life spending in lean months.
///
/// `Dynamic` (Mode B): Treats target buffer levels as set-points. Liquidates
/// proactively to cover the monthly deficit *plus* a "buffer restock" amount
/// that returns the bridge fund to 12 months of base spend and the war chest
/// to its target. Applies look-ahead: the sale amount is reduced by the next
/// month's expected dividends so the portfolio isn't over-sold against
/// imminent inflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WithdrawalRegime {
    #[default]
    Shielded,
    Dynamic,
}

impl std::fmt::Display for WithdrawalRegime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WithdrawalRegime::Shielded => write!(f, "Shielded (Mode A)"),
            WithdrawalRegime::Dynamic  => write!(f, "Dynamic (Mode B)"),
        }
    }
}

/// V8.5 — How the war-chest target (cap) evolves year over year, post-retirement.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WarChestCapPolicy {
    /// Hold the target fixed at its retirement-date value (default).
    #[default]
    Fixed,
    /// Grow the target by `inflation_japan` each year.
    GrowByInflation,
    /// Grow the target by `war_chest_cap_growth_pct` each year.
    GrowByPercent,
    /// Shrink the target by `war_chest_cap_growth_pct` each year (floored at 0).
    ShrinkByPercent,
    /// On `war_chest_empty_date`, empty the war chest (reinvest balance into
    /// Taxable) and hold the target at 0 thereafter.
    EmptyOnDate,
}

impl std::fmt::Display for WarChestCapPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WarChestCapPolicy::Fixed           => write!(f, "Keep the limit (fixed)"),
            WarChestCapPolicy::GrowByInflation => write!(f, "Increase by inflation each year"),
            WarChestCapPolicy::GrowByPercent   => write!(f, "Increase by a set % each year"),
            WarChestCapPolicy::ShrinkByPercent => write!(f, "Decrease by a set % each year"),
            WarChestCapPolicy::EmptyOnDate     => write!(f, "Empty on a set date"),
        }
    }
}

/// US tax mitigation strategy for the simulation run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UsTaxStrategy {
    /// Apply only the Foreign Tax Credit (default). Japan resident taxes credited
    /// against US federal liability.
    #[default]
    FtcOnly,
    /// Apply the Foreign Earned Income Exclusion first (up to the annual FEIE limit),
    /// then FTC on remaining income. The simulation runs *only* this combined path —
    /// it does not auto-compare against `FtcOnly`. Choose deliberately based on your
    /// own tax-optimization analysis.
    FeieAndFtc,
}

impl std::fmt::Display for UsTaxStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UsTaxStrategy::FtcOnly    => write!(f, "FTC Only"),
            UsTaxStrategy::FeieAndFtc => write!(f, "FEIE + FTC"),
        }
    }
}

/// VA dependent status for disability compensation table lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VaDependentStatus {
    #[default]
    VetOnly,
    WithSpouse,
    WithSpouseAndChild,
}

/// Tax treatment protocol — applied globally (on `Config`) or per income source.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaxProtocol {
    /// Apply both US federal and Japan resident tax / NHI (default).
    #[default]
    Both,
    /// US federal tax only — Japan resident tax and NHI are bypassed.
    UsOnly,
    /// Japan resident tax and NHI only — US federal / capital-gains tax is bypassed.
    JapanOnly,
    /// Income is fully tax-free (e.g., VA disability, SMC). No tax calculated.
    TaxFree,
}

impl std::fmt::Display for TaxProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TaxProtocol::Both      => write!(f, "Both (US + Japan)"),
            TaxProtocol::UsOnly    => write!(f, "US Only"),
            TaxProtocol::JapanOnly => write!(f, "Japan Only"),
            TaxProtocol::TaxFree   => write!(f, "Tax Free"),
        }
    }
}

/// Backwards-compatible type alias — keeps old code compiling during migration.
pub type TaxJurisdiction = TaxProtocol;

/// A recurring scheduled purchase rule for a specific ticker in a specific account.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccumulationRule {
    pub ticker: String,
    /// Account key: "Taxable", "Roth", "DC", etc.
    pub account: String,
    /// Monthly buy amount (USD for non-DC; JPY for DC).
    pub monthly_amount: f64,
    /// Buy every N months: 1=monthly, 3=quarterly, 12=annual.
    pub frequency_months: u32,
    /// Override growth rate for the ticker (None = use global growth_rates_annual).
    pub growth_pct_override: Option<f64>,
    pub stop_at_retirement: bool,
}

/// A single ticker position in an investment account.
///
/// V6.6: per-position rebalance trigger and recession-resilience override.
/// `rebalance_date`, when set, supersedes the global rebalance event for this position.
/// `recession_override`, when set (0.0-1.0), replaces the global drawdown severity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub ticker: String,
    pub quantity: f64,
    pub avg_cost: f64,
    #[serde(default)]
    pub rebalance_date: Option<NaiveDate>,
    #[serde(default)]
    pub recession_override: Option<f64>,
    /// V7.0 — Japan-resident cost basis (¥/share). Captures the JPY value paid at
    /// purchase, independent of today's FX. Drives the highest-basis-first
    /// liquidation order and the Japan capital-gains computation
    /// (jpy_proceeds − jpy_basis) × 20.315%. Defaults to 0.0; the runtime falls
    /// back to `avg_cost × usd_jpy_at_load` when the field is absent.
    #[serde(default)]
    pub avg_purchase_price_jpy: f64,
}

impl Position {
    pub fn cost_basis(&self) -> f64 {
        self.quantity * self.avg_cost
    }
}

impl Default for Position {
    fn default() -> Self {
        Self {
            ticker: String::new(),
            quantity: 0.0,
            avg_cost: 0.0,
            rebalance_date: None,
            recession_override: None,
            avg_purchase_price_jpy: 0.0,
        }
    }
}

/// Configuration for Military Retired Pay (distinct from FERS).
///
/// Military retired pay is taxable US + Japan per the US-Japan Tax Treaty savings
/// clause: Japan taxes it first, then the US credits Japan taxes via FTC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MilitaryRetiredConfig {
    /// Monthly military retired pay (USD).
    pub monthly_usd: f64,
    /// Tax protocol for this income source (default: Both — US + Japan savings clause).
    pub jurisdiction: TaxProtocol,
}

/// NRA spouse tax profile — controls how a Japanese-citizen spouse without a Green Card
/// affects the US tax computation.
///
/// Selecting the right profile is critical: MFJ under §6013(g) drags the NRA spouse's
/// global income into the US tax base, while MFS halves the standard deduction and
/// eliminates Roth IRA eligibility for most working professionals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpouseProfile {
    /// Both spouses are US citizens or Lawful Permanent Residents (default).
    /// Filing status and deductions follow the normal MFJ / Single path.
    #[default]
    UsPerson,
    /// Japanese-citizen spouse with no Green Card; §6013(g) election filed to treat
    /// the NRA spouse as a US resident for tax purposes.
    /// Consequence: all of the NRA spouse's global income (Japan salary, Nenkin, etc.)
    /// is added to the US return. Higher std deduction; bigger FTC pool.
    NraElectedToBeTreatedAsResident,
    /// Japanese-citizen spouse; Married Filing Separately.
    /// The NRA spouse's Japan income stays outside the US tax base.
    /// Consequence: standard deduction halves; Roth IRA phase-out drops to $0–$10k
    /// (effectively eliminating contributions for working professionals).
    NraMfs,
    /// Japanese-citizen spouse; Head of Household eligible (qualifying US-citizen child
    /// lives with the filer). HoH brackets and deductions apply.
    NraHeadOfHouseholdEligible,
}

impl std::fmt::Display for SpouseProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpouseProfile::UsPerson                        => write!(f, "US Person (default)"),
            SpouseProfile::NraElectedToBeTreatedAsResident => write!(f, "NRA — Elected MFJ (§6013(g))"),
            SpouseProfile::NraMfs                          => write!(f, "NRA — Married Filing Separately"),
            SpouseProfile::NraHeadOfHouseholdEligible      => write!(f, "NRA — Head of Household eligible"),
        }
    }
}

/// Where the primary investment activity is domiciled.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum InvestmentLocation {
    #[default]
    Us,
    Japan,
    International,
}

impl std::fmt::Display for InvestmentLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InvestmentLocation::Us            => write!(f, "US"),
            InvestmentLocation::Japan         => write!(f, "Japan"),
            InvestmentLocation::International => write!(f, "International"),
        }
    }
}

/// US federal tax rules for a given year. Mirrors Python's `TaxRules` dataclass.
/// All monetary values are in USD.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxRules {
    pub filing_status: String,
    pub us_state_code: String,
    pub us_state_rate: f64,
    pub std_deduction: f64,
    /// Threshold below which long-term capital gains are taxed at 0%.
    pub ltcg_0_limit: f64,
    /// Threshold below which long-term capital gains are taxed at 15% (above ltcg_0_limit).
    pub ltcg_15_limit: f64,
    /// MAGI threshold above which the 3.8% Net Investment Income Tax applies.
    pub niit_threshold: f64,
    pub niit_rate: f64,
    /// IRS additional standard deduction for taxpayers age ≥ 65 (per qualifying person).
    /// 2026 MFJ: $1,550/person; Single/HoH: $1,950/person.
    /// Inflates annually alongside the base standard deduction.
    pub senior_addon_per_person: f64,
    /// Ordinary income tax brackets: Vec of (upper_limit_usd, rate).
    /// The final entry has f64::INFINITY as the limit.
    pub brackets: Vec<(f64, f64)>,
}

impl Default for TaxRules {
    fn default() -> Self {
        // V8.0 — 2026 MFJ federal brackets (OBBBA indexed values)
        Self {
            filing_status: "Married Filing Jointly".into(),
            us_state_code: "None".into(),
            us_state_rate: 0.0,
            std_deduction: 32_200.0,
            ltcg_0_limit: 115_000.0,
            ltcg_15_limit: 700_000.0,
            niit_threshold: 250_000.0,
            niit_rate: 0.038,
            senior_addon_per_person: 1_550.0,
            brackets: vec![
                (23_850.0, 0.10),         // V8.0 — 2026 indexed MFJ brackets
                (96_950.0, 0.12),
                (206_700.0, 0.22),
                (394_600.0, 0.24),
                (501_050.0, 0.32),
                (752_700.0, 0.35),
                (f64::INFINITY, 0.37),
            ],
        }
    }
}

impl TaxRules {
    /// Inflate all monetary thresholds by `rate` (e.g., 0.028 for 2.8% CPI).
    /// Mirrors Python's `TaxRules.inflate()`.
    pub fn inflate(&mut self, rate: f64) {
        let factor = 1.0 + rate;
        self.std_deduction *= factor;
        self.ltcg_0_limit *= factor;
        self.ltcg_15_limit *= factor;
        self.niit_threshold *= factor;
        self.senior_addon_per_person *= factor;
        self.brackets = self.brackets.iter().map(|&(limit, rate_val)| {
            let new_limit = if limit == f64::INFINITY { f64::INFINITY } else { limit * factor };
            (new_limit, rate_val)
        }).collect();
    }
}

/// VA disability rates for a specific year.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaRates {
    pub base: f64,
    pub child_addon: f64,
}

/// A child or other dependent whose age drives VA rider eligibility and tax rules.
///
/// V6.6: `birth_date` carries full month/day precision; `birth_year` retained for
/// backward compat and is auto-derived from `birth_date` when both present.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Dependent {
    /// Calendar year of birth (e.g., 2018).
    pub birth_year: i32,
    /// Full birth date — drives age-thresholds at month resolution.
    #[serde(default)]
    pub birth_date: Option<NaiveDate>,
    /// When true, VA rider eligibility extends to age 23 instead of 18.
    pub is_college_student: bool,
}

/// Stage 07 — Relationship of an heir to the deceased.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HeirRelationship {
    Spouse,
    #[default]
    Child,
    Other,
}

impl std::fmt::Display for HeirRelationship {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeirRelationship::Spouse => write!(f, "Spouse"),
            HeirRelationship::Child  => write!(f, "Child"),
            HeirRelationship::Other  => write!(f, "Other"),
        }
    }
}

/// V8.0 — Japan visa classification for Exit Tax eligibility.
/// Per IT Act Art. 60-2, only Table 2 visa holders are subject to the
/// 5-of-10-year residency test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VisaType {
    /// Table 1 (work visas: engineer, intra-company transferee, etc.).
    /// EXEMPT from Exit Tax regardless of years of residence.
    #[default]
    Table1,
    /// Table 2 (Permanent Resident, Spouse of Japanese National, Long-Term Resident).
    /// Subject to Exit Tax once 5-of-10-year residency test is met.
    Table2,
}

/// Stage 07 — A single heir who will receive a share of the estate.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Heir {
    pub name:         String,
    pub birth_date:   Option<NaiveDate>,
    pub relationship: HeirRelationship,
}

/// Household members whose birth years govern time-aware rule evaluations:
///   - VA dependent-child rider cutoff (age 18 / 23 with college-student flag)
///   - IRS senior standard deduction add-on (age ≥ 65 per person)
///   - SSDI → SS retirement reclassification at age 65
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FamilyUnit {
    /// Primary retiree's birth year (derived from `Config.birth_date`).
    pub user_birth_year: i32,
    /// Spouse's birth year, if applicable (drives second senior deduction add-on).
    pub spouse_birth_year: Option<i32>,
    /// Children and other qualifying dependents.
    pub dependents: Vec<Dependent>,
}

/// The complete simulation configuration. Mirrors Python's `Config` dataclass.
/// All monetary fields follow the same currency convention as Python:
///   - `_jpy` suffix → Japanese Yen
///   - `_usd` suffix → US Dollars
///   - otherwise → context-dependent (see field docs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // ── Timing ──────────────────────────────────────────────────────────────────
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub retirement_date: NaiveDate,
    /// The month the major portfolio rebalance event occurs (always ≥ retirement_date).
    pub rebalance_date: NaiveDate,

    // ── Economics ───────────────────────────────────────────────────────────────
    pub usd_jpy: f64,
    pub inflation_cola: f64,
    pub inflation_japan: f64,
    pub ira_limit_growth: f64,

    pub fx_drift_enabled: bool,
    pub fx_drift_rate: f64,
    /// V6.6: cadence-based JPY drift. 0 = legacy continuous-rate mode.
    /// When > 0, every `fx_drift_cadence_months` the FX rate jumps by
    /// `fx_drift_increase_amount_jpy` (positive = yen weakens).
    #[serde(default)]
    pub fx_drift_cadence_months: u32,
    #[serde(default)]
    pub fx_drift_increase_amount_jpy: f64,
    pub recession_enabled: bool,
    pub recession_severity: f64,
    /// Scheduled recession events for sequence-of-returns stress testing.
    pub recession_events: Vec<RecessionEvent>,
    /// Scheduled FX shock events for currency volatility stress testing.
    pub fx_shock_events: Vec<FXShockEvent>,

    // ── Budgeting ───────────────────────────────────────────────────────────────
    pub base_expense_jpy: f64,
    pub min_expense_jpy: f64,
    /// Legacy static spike field — kept for JSON backward compat.
    /// Use `nhi_model` for dynamic calculation (loaded by loader, set by UI).
    pub nhi_spike_monthly_jpy: f64,
    /// NHI premium model: automatic rate schedule or manual fixed amounts.
    /// Populated by the loader or UI; not persisted directly by serde (use loader/UI JSON path).
    #[serde(skip)]
    pub nhi_model: NhiModel,

    // ── V8.1 Detailed Expense Categories ────────────────────────────────────────
    /// V8.1 — When true, the UI treats `expense_categories` as the source of truth.
    /// The engine still consumes `base_expense_jpy` / `min_expense_jpy` (which the
    /// save path keeps in sync as sums). Default false (legacy direct entry).
    #[serde(default)]
    pub expenses_detailed_mode: bool,

    /// V8.1 — Per-category breakdown for the detailed entry mode. Always persisted
    /// so toggling the mode in the UI never destroys data.
    #[serde(default)]
    pub expense_categories: Vec<crate::models::expense::ExpenseCategory>,

    /// V8.1 — Buffer added to `min_expense_jpy` in detailed mode. Fixed JPY/mo
    /// (`min_expense_buffer_jpy`) XOR percentage of essentials sum
    /// (`min_expense_buffer_pct`). Use whichever is non-zero; if both are non-zero,
    /// JPY wins (UI enforces single-choice). Default both 0.
    #[serde(default)]
    pub min_expense_buffer_jpy: f64,
    /// V8.1 — Decimal fraction of essentials sum to add as buffer (e.g., 0.10 = +10%).
    #[serde(default)]
    pub min_expense_buffer_pct: f64,

    // ── War Chest ───────────────────────────────────────────────────────────────
    #[serde(default = "default_true")]
    pub war_chest_enabled: bool,
    #[serde(default)]
    pub war_chest_funding_timing: BufferFundingTiming,
    #[serde(default = "default_war_chest_ramp_months")]
    pub war_chest_ramp_months: u32,
    /// "JPY" or "USD"
    pub war_chest_currency: String,
    pub war_chest_target_jpy: f64,
    pub war_chest_target_usd: f64,
    /// V8.5 — How the war-chest cap evolves post-retirement. Default: Fixed.
    #[serde(default)]
    pub war_chest_cap_policy: WarChestCapPolicy,
    /// V8.5 — Annual growth/shrink rate for GrowByPercent / ShrinkByPercent
    /// (decimal fraction, e.g. 0.03 = 3%). Ignored by other policies.
    #[serde(default)]
    pub war_chest_cap_growth_pct: f64,
    /// V8.5 — Date on which the war chest is emptied when policy = EmptyOnDate.
    #[serde(default)]
    pub war_chest_empty_date: Option<NaiveDate>,

    // ── Bridge Fund ─────────────────────────────────────────────────────────────
    #[serde(default = "default_true")]
    pub bridge_fund_enabled: bool,
    #[serde(default)]
    pub bridge_fund_funding_timing: BufferFundingTiming,
    #[serde(default = "default_bridge_fund_ramp_months")]
    pub bridge_fund_ramp_months: u32,
    pub bridge_months_target: u32,
    /// "JPY" or "USD"
    pub bridge_fund_currency: String,

    // ── Roth IRA ─────────────────────────────────────────────────────────────────
    pub roth_start_limit: f64,
    pub roth_contribution_made_this_year: bool,
    pub roth_contribution_so_far: f64,

    // ── Contributions ────────────────────────────────────────────────────────────
    pub dc_monthly_jpy: f64,
    pub dc_growth_rate: f64,
    pub monthly_contribution_ticker: String,
    pub va_contribution_buffer_usd: f64,

    // ── Nenkin ──────────────────────────────────────────────────────────────────
    /// Annual baseline Nenkin already embedded in base_expense_jpy.
    pub nenkin_baseline_annual_jpy: f64,

    // ── Growth rates ─────────────────────────────────────────────────────────────
    pub growth_rates_annual: HashMap<String, f64>,

    // ── VA ──────────────────────────────────────────────────────────────────────
    /// keyed by year string ("2026", "2027", …)
    pub va_disability_rates: HashMap<String, VaRates>,

    // ── FERS ─────────────────────────────────────────────────────────────────────
    pub fers_monthly_start: f64,
    pub fers_start_date: NaiveDate,
    /// Gross income (JPY) earned in the calendar year of retirement — for Japan tax calc.
    pub retirement_year_gross_income_jpy: f64,

    // ── Family dates ─────────────────────────────────────────────────────────────
    pub birth_date: NaiveDate,
    pub spouse_birth_date: NaiveDate,
    pub child_birth_date: NaiveDate,
    /// VA child addon stops on this date (child's 18th birthday).
    pub va_child_cutoff_date: Option<NaiveDate>,

    // ── DC Payout ─────────────────────────────────────────────────────────────────
    pub dc_payout_start_age: u32,
    /// "LUMP_SUM" or "ANNUITY_20YR"
    pub dc_payout_method: String,

    // ── Pre-funding ──────────────────────────────────────────────────────────────
    pub pre_funded_war_chest_jpy: f64,
    pub pre_funded_bridge_jpy: f64,
    pub pre_funded_bridge_usd: f64,
    pub pre_funded_japan_tax_jpy: f64,
    pub pre_funded_us_tax_usd: f64,

    // ── Rebalancing targets ───────────────────────────────────────────────────────
    pub target_vti_pct: f64,
    pub target_schd_pct: f64,
    pub roth_rebalance_target_vti_pct: f64,
    pub roth_rebalance_target_schd_pct: f64,

    // ── Flags ─────────────────────────────────────────────────────────────────────
    pub enable_roth_rebalance_at_59: bool,
    pub buy_schd_last_year: bool,
    /// "SALARY" or "SELL_TO_COVER"
    pub rsu_tax_handling: String,
    pub total_annual_compensation_usd: f64,

    // ── Dynamic expense rules (built at load time) ────────────────────────────────
    /// Populated by the loader, not read directly from JSON.
    #[serde(skip)]
    pub expense_rules: Vec<ExpenseRule>,

    // ── RSU awards ────────────────────────────────────────────────────────────────
    #[serde(skip)]
    pub rsu_awards: Vec<RsuAward>,

    // ── Tax rules ────────────────────────────────────────────────────────────────
    #[serde(skip)]
    pub tax_rules: TaxRules,

    // ── Military Retired Pay ──────────────────────────────────────────────────────
    /// Optional military retired pay configuration (distinct from FERS).
    #[serde(skip)]
    pub military_retired: Option<MilitaryRetiredConfig>,

    // ── Per-source tax jurisdictions ───────────────────────────────────────────────
    /// Tax protocol applied specifically to FERS pension income.
    #[serde(default)]
    pub fers_jurisdiction: TaxProtocol,
    /// Tax protocol applied specifically to Social Security income.
    #[serde(default)]
    pub ss_jurisdiction: TaxProtocol,
    /// Tax protocol applied specifically to Nenkin pension income.
    #[serde(default)]
    pub nenkin_jurisdiction: TaxProtocol,
    /// Active SMC variant label (e.g. "K (add-on)", "L"). None = no SMC.
    #[serde(default)]
    pub va_smc_variant: Option<String>,

    // ── Jurisdiction & location ───────────────────────────────────────────────────
    /// Controls which tax systems the simulation applies.
    pub tax_jurisdiction: TaxProtocol,
    /// Primary domicile of the investment portfolio.
    pub investment_location: InvestmentLocation,

    // ── US Tax Mitigation Strategy ─────────────────────────────────────────────────
    pub us_tax_strategy: UsTaxStrategy,

    // ── V7.0 — US State Tax & Withdrawal Strategy ───────────────────────────────
    /// Additive US state income-tax rate applied to USD-denominated realised gains
    /// during the V7.0 liquidation waterfall. Independent of `tax_rules.us_state_rate`
    /// (kept in sync by the loader) so the engine can dial it without touching the
    /// federal pipeline. NOT offset by the Japan FTC — pure additional drag.
    #[serde(default)]
    pub us_state_tax_rate: f64,
    /// V7.0 — Selects the post-retirement liquidation behaviour. See `WithdrawalStrategy`.
    #[serde(default)]
    pub withdrawal_strategy: WithdrawalStrategy,

    // ── V7.1 — Defensive Waterfall & Currency Segregation ────────────────────
    /// V7.1 — Selects the monthly spending waterfall algorithm. Default: Defensive.
    #[serde(default)]
    pub withdrawal_waterfall: WaterfallStrategy,
    /// V7.1 — Flat FX spread penalty applied on every USD→JPY conversion in the
    /// waterfall (Tiers 4, 5, 6, 8). Default: 0.005 (0.5%).
    #[serde(default = "default_fx_spread_penalty")]
    pub fx_spread_penalty: f64,

    // ── V7.3 — Education & Family Engine ─────────────────────────────────────
    /// V7.3 — Buffer-management regime inside the Defensive waterfall.
    /// Shielded (Mode A): preserve equity; force minimum-spend when cash zeroes.
    /// Dynamic (Mode B): proactively liquidate to restock buffers to target.
    #[serde(default)]
    pub withdrawal_regime: WithdrawalRegime,
    /// V7.3 — Monthly JPY skim from post-spend surplus into the Tier 2.5
    /// Education Fund. 0.0 disables the accumulation channel.
    #[serde(default)]
    pub edu_savings_jpy_monthly: f64,
    /// V7.3 — Tier 0.5 Jido Teate (児童手当) child allowance. When true and a
    /// dependent child is age 0-18, pay ¥15k/mo (0-3) or ¥10k/mo (3-18) on a
    /// bi-monthly cadence (even calendar months get 2 months' worth). No income
    /// cap is modeled.
    #[serde(default = "default_true")]
    pub jido_teate_enabled: bool,

    // ── VA Disability Profile ──────────────────────────────────────────────────────
    /// 0 = use legacy va_disability_rates map; 10–100 = use 2026 lookup table.
    pub va_disability_rating: u32,
    pub va_dependent_status: VaDependentStatus,
    /// Override base VA monthly benefit (USD, 2026 baseline, inflated by COLA).
    /// When Some, bypasses the rating table + SMC variant for the base VA amount.
    #[serde(default)]
    pub va_monthly_override: Option<f64>,
    /// Override SMC monthly amount (USD, 2026 baseline, inflated by COLA).
    /// When Some, bypasses the SMC variant lookup.
    #[serde(default)]
    pub smc_monthly_override: Option<f64>,

    // ── Marriage & Spouse Benefits (V6.6) ──────────────────────────────────────
    /// When true, spouse demographics and entitlements participate in calculations.
    #[serde(default)]
    pub is_married: bool,
    /// Spouse Social Security benefit (USD/month, 0 = not applicable).
    #[serde(default)]
    pub spouse_ss_monthly_usd: f64,
    /// Age at which spouse SS begins.
    #[serde(default)]
    pub spouse_ss_start_age: u32,
    #[serde(default)]
    pub spouse_ss_jurisdiction: TaxProtocol,
    /// Spouse Nenkin benefit (JPY/month, 0 = not applicable).
    #[serde(default)]
    pub spouse_nenkin_monthly_jpy: f64,
    #[serde(default)]
    pub spouse_nenkin_start_age: u32,
    #[serde(default)]
    pub spouse_nenkin_jurisdiction: TaxProtocol,

    // ── NRA Spouse Tax Profile (Stage 02) ─────────────────────────────────────────
    /// Spouse's tax residency profile. Determines effective filing status and which
    /// spouse income streams enter the US tax base.
    /// Default `UsPerson` preserves pre-Stage-02 behaviour.
    #[serde(default)]
    pub spouse_profile: SpouseProfile,
    /// Annual Japan salary earned by the NRA spouse (JPY).
    /// Only applied to the US return when `spouse_profile == NraElectedToBeTreatedAsResident`.
    #[serde(default)]
    pub spouse_japan_salary_jpy: f64,
    /// Annual Japan miscellaneous income earned by the NRA spouse (JPY).
    /// Included in the US §6013(g) pooled income alongside `spouse_japan_salary_jpy`.
    #[serde(default)]
    pub spouse_japan_misc_income_jpy: f64,

    // ── Social Security ────────────────────────────────────────────────────────────
    /// Monthly SS benefit estimate in USD (0 = not applicable).
    pub ss_monthly_usd: f64,
    /// Age at which SS payments begin (default 67).
    pub ss_start_age: u32,
    /// Monthly SSDI (Social Security Disability Insurance) benefit in USD (0 = not applicable).
    /// Taxed via the "Combined Income" rule (up to 85% taxable above $44K MFJ threshold).
    /// For Japan resident tax: routed through the public pension deduction (公的年金等控除).
    /// At age 65, classification transitions to SS retirement; dollar amount is unchanged.
    pub ssdi_monthly_usd: f64,

    // ── Family Unit (demographics for time-aware rule evaluation) ──────────────────
    /// Household composition for age-based eligibility checks.
    /// Populated by the loader; not persisted directly by serde (use JSON `dependents` key).
    #[serde(skip)]
    pub family_unit: FamilyUnit,

    // ── Nenkin pension income ──────────────────────────────────────────────────────
    /// Monthly Nenkin income estimate in JPY once pension payments begin (0 = not applicable).
    pub nenkin_income_monthly_jpy: f64,
    /// Age at which Nenkin income begins (default 65).
    pub nenkin_income_start_age: u32,

    // ── Tax Treaty flags ─────────────────────────────────────────────────────────
    /// US-Japan Tax Treaty Article 18: when true, FERS pension is excluded from
    /// the Japan resident tax (jumin-zei) income base in addition to national tax.
    /// Default false (conservative: FERS included in local tax base).
    #[serde(default)]
    pub fers_japan_local_tax_exempt: bool,

    // ── Japan regional tax location ────────────────────────────────────────────────
    /// Japanese prefecture of residence (e.g. "Kanagawa"). Used for Juminzei rate lookup.
    pub prefecture: String,
    /// City within the prefecture (e.g. "Sagamihara"). Nagoya uses 9.7%.
    pub city: String,

    // ── Active management (V6.0) ──────────────────────────────────────────────────
    /// Recurring scheduled-buy rules set by the user per ticker.
    #[serde(skip)]
    pub accumulation_rules: Vec<AccumulationRule>,
    /// Target portfolio weights per account and ticker (account → ticker → fraction).
    /// E.g. `{"Taxable": {"VTI": 0.60}, "Roth": {"VTI": 0.70}}`.
    /// Used by the periodic rebalancing engine; empty = rebalancing disabled.
    #[serde(skip)]
    pub target_allocations: HashMap<String, HashMap<String, f64>>,
    /// Whether periodic target-state rebalancing is active.
    #[serde(skip)]
    pub rebalance_enabled: bool,
    /// How often to rebalance: 1=monthly, 3=quarterly, 6=semi-annual, 12=annual.
    #[serde(skip)]
    pub rebalance_frequency_months: u32,

    // ── V7.5 — Exit Tax Monitor ───────────────────────────────────────────────────
    /// V8.0 — Visa type for Exit Tax evaluation. Defaults to Table1 (exempt).
    #[serde(default)]
    pub primary_taxpayer_visa: VisaType,
    /// Japan residency start date (used for Exit Tax 5-of-10 test per IT Act Art. 60-2).
    /// None disables the Exit Tax monitor.
    #[serde(default)]
    pub japan_residency_start_date: Option<NaiveDate>,
    /// Whether to include NISA/iDeCo asset values in the ¥100M Exit Tax threshold.
    /// Per Art. 60-2 the answer is yes; flag retained for "what if" analysis.
    #[serde(default = "default_true")]
    pub exit_tax_include_tax_advantaged: bool,

    // ── V7.5 — Tier 9: Estate Planning Gift Sink ──────────────────────────────────
    /// Annual gift amount per recipient (JPY). Typically ¥1,100,000 (暦年贈与 exclusion).
    #[serde(default)]
    pub annual_gift_jpy_per_recipient: f64,
    /// Number of gift recipients (typically 1-4 children/grandchildren).
    #[serde(default)]
    pub gift_recipient_count: u32,
    /// US §2503(b) annual gift exclusion per donor-recipient pair (2026 = $19,000).
    #[serde(default = "default_us_gift_exclusion")]
    pub us_gift_exclusion_usd: f64,

    // ── V7.5 — Tax-Loss Harvesting (IRC §1091) ────────────────────────────────────
    /// When true, the TLH pre-waterfall handler fires in `tlh_active_months`.
    #[serde(default)]
    pub tlh_enabled: bool,
    /// Calendar months in which TLH is active (default: November + December).
    #[serde(default = "default_tlh_months")]
    pub tlh_active_months: Vec<u32>,
    /// Minimum USD loss required to harvest a lot (transaction cost threshold).
    #[serde(default = "default_tlh_threshold")]
    pub tlh_min_loss_usd: f64,

    // ── V7.7 — Master Toggle Switches ────────────────────────────────────────────
    /// When false, the Tier 2.5 Education Fund accumulation channel is disabled.
    /// All surplus that would have gone to the education fund instead stays in the
    /// waterfall. Default true (matches V7.3 behaviour).
    #[serde(default = "default_true")]
    pub enable_education_savings: bool,
    /// When false, the Tier 9 Gift Sink December drain is disabled.
    /// Gifts are not modeled; the JPY stays in the war chest. Default true.
    #[serde(default = "default_true")]
    pub enable_gift_sink: bool,

    // ── V7.7.2 — RSU Sell-to-Cover Realism Layer ─────────────────────────────
    /// Master on/off switch for the RSU margin-call realism layer.
    /// When true and `rsu_tax_handling = "SELL_TO_COVER"`, the engine models
    /// the case where a recession pushes the vest price below the combined
    /// US + Japan tax bill. Default true — existing scenario files become
    /// "realistic" automatically (see CHANGELOG V7.7.2).
    #[serde(default = "default_rsu_realism")]
    pub rsu_sell_to_cover_realism: bool,
    /// Selects how aggressively the deficit cascade operates.
    /// `Strict` drains buffers and logs unpaid liability; `Permissive` is legacy.
    #[serde(default)]
    pub rsu_sell_to_cover_policy: RsuSellToCoverPolicy,

    // ── Stage 03 — Monthly Dependent Precision ────────────────────────────────
    /// When true (default), VA add-ons, NHI per-capita charges, and Jido Teate
    /// are computed at month resolution using exact birth dates. When false,
    /// falls back to annual-bucket approximations (legacy behaviour).
    #[serde(default = "default_true")]
    pub monthly_dependent_precision: bool,

    // ── Stage 04 — Shock Application Order ───────────────────────────────────
    /// When a recession and FX shock fall in the same year, determines which is
    /// applied first. See `ShockOrdering` for the impact on the JPY net-worth
    /// audit trail. Default: `DepreciateThenReprice` (conservative).
    #[serde(default)]
    pub shock_ordering: ShockOrdering,

    // ── Stage 09 — Cryptocurrency / Web3 Asset Handling ──────────────────────
    /// When true, assets marked as Crypto are taxed under Japan's miscellaneous
    /// income regime (up to 55%) instead of the standard 20.315% cap-gains rate.
    /// The US side continues to use LTCG/STCG treatment. Default true.
    #[serde(default = "default_true")]
    pub crypto_tax_enabled: bool,

    // ── Stage 05 — PFIC Basis Drift Monitor ──────────────────────────────────
    /// When true (default), the engine cross-checks USD vs JPY MTM basis each year
    /// and emits a PficDriftWarning + self-heals if the two reference values diverge
    /// by more than 1%. Disable to measure the cumulative drift effect on terminal
    /// portfolio value (proving the toggle does something in tests).
    #[serde(default = "default_true")]
    pub track_pfic_basis_drift: bool,

    // ── Stage 06 — Real Estate Module ────────────────────────────────────────
    /// List of real-estate holdings (primary, rental, inherited, vacation).
    /// When empty the engine behaves identically to the pre-Stage-06 baseline;
    /// no real-estate flows enter cashflow or tax.
    #[serde(default)]
    pub real_estate: Vec<crate::models::real_estate::RealEstateHolding>,
    /// Master toggle for the Tier 7.5 HELOC-draw step in the defensive waterfall.
    /// Fires only when true AND at least one holding has an active `HelocLine`.
    #[serde(default)]
    pub enable_heloc_tier: bool,

    // ── Stage 07 — Estate Planning ───────────────────────────────────────────
    /// When true, the engine computes Japan Sōzoku-zei and US Estate Tax at the
    /// end of the simulation horizon (or at `death_date`) and attaches an
    /// `EstateSummary` to the final snapshot.
    #[serde(default)]
    pub enable_estate_planning: bool,
    /// Optional user-specified death date.  When `None` the engine uses `end_date`.
    #[serde(default)]
    pub death_date: Option<NaiveDate>,
    /// Optional spouse death date (currently informational; used to pre-apply the
    /// spousal 1/2 deduction on the first-to-die simulation).
    #[serde(default)]
    pub spouse_death_date: Option<NaiveDate>,
    /// List of heirs who will receive the estate.
    #[serde(default)]
    pub heirs: Vec<Heir>,
    /// Tax protocol used in the estate projection.  Typically `Both` for a US
    /// citizen long-term resident of Japan.
    #[serde(default = "default_estate_jurisdiction")]
    pub estate_planning_jurisdiction: TaxProtocol,
    /// When true and estate_planning is on, show the annual gifting optimiser
    /// output and write the suggested amount into `annual_gift_jpy_per_recipient`.
    #[serde(default)]
    pub enable_gifting_optimiser: bool,

    // ── V8.0 — Active-Phase Resident Tax Modeling ────────────────────────────
    /// V8.0 — When true, model active-phase resident tax as a 12-month Tokubetsu
    /// Choushuu (Special Collection) deduction. When false (default), assume
    /// employer-withheld and net (legacy V7 behaviour).
    #[serde(default)]
    pub model_active_phase_resident_tax: bool,

    // ── Stage 08 — Correlated Monte Carlo ────────────────────────────────────
    /// When true, use correlated asset paths (multivariate normal) instead of
    /// independent draws. Models the historical "safe haven yen" effect.
    #[serde(default)]
    pub mc_use_correlated_paths: bool,
    /// Correlation matrix: map of asset-class pairs to correlation coefficients.
    /// Example: { "US Equity": { "USD/JPY": -0.40, "US Bond": -0.10 } }
    #[serde(default)]
    pub mc_correlation_matrix: HashMap<String, HashMap<String, f64>>,

    // ── Stage 10 — Long-Term Care Insurance (介護保険 / Kaigo Hoken) ─────────
    /// When true (default), model the age-65+ Kaigo Hoken premium as a separate
    /// expense line. Ages 40-64 premium is bundled into NHI (already modeled).
    /// Disable to revert to legacy behavior (no separate charge after age 65).
    #[serde(default = "default_true")]
    pub kaigo_hoken_enabled: bool,
    /// Custom bracket schedule for age-65+ Kaigo Hoken premium calculation.
    /// When None, uses the prefecture-default schedule (Sagamihara 2026).
    #[serde(default)]
    pub kaigo_hoken_brackets: Option<crate::engine::tax::kaigo_hoken::KaigoHokenBrackets>,
    /// Care need scenario for optional out-of-pocket cost projection.
    /// None = premium only; Low/Medium/High add projected care draws.
    #[serde(default)]
    pub kaigo_care_scenario: crate::engine::tax::kaigo_hoken::CareScenario,

    // ── V8.4 — Conservative Waterfall Alt-Mode ──────────────────────────────
    /// V8.4 — When true, skip belt-tightening and liquidate stock first if the
    /// Taxable portfolio can sustain minimum spending through the end of the
    /// simulation (coarse projection: no growth/inflation adjustment).
    /// Default false (conservative: belt-tighten before liquidating).
    #[serde(default)]
    pub prefer_liquidation_over_belt_tightening: bool,
}

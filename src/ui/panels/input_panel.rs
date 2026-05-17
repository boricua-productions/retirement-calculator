use std::collections::HashSet;
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDate};
use egui::{Color32, Frame, RichText, Ui};
use serde_json::Value;

use crate::engine::tax::japan_regions::{ALL_PREFECTURES, cities_for_prefecture, city_rate_annotation};
use crate::engine::tax::us_tax::{state_tax_rate, state_display_name, ALL_STATE_CODES, ALL_FILING_STATUSES};
use crate::engine::va_benefits::{lookup_va_monthly_2026, lookup_smc_monthly_2026, ALL_VA_RATINGS, ALL_SMC_VARIANTS};
use crate::models::config::{HeirRelationship, NhiCalculatedRates, ShockOrdering, SpouseProfile, TaxJurisdiction, TaxProtocol, UsTaxStrategy, VaDependentStatus};

const SAVE_STATUS_ID: &str = "input_panel_save_status";
const SAVE_STATUS_TTL: Duration = Duration::from_secs(5);

const ALL_ACCOUNT_TYPES: &[&str] = &[
    "Taxable Brokerage",
    "IRA (Traditional)",
    "Roth IRA",
    "401(k)",
    "DC Plan",
    "NISA",
    "iDeCo",
];

#[derive(Clone)]
struct SaveStatus {
    message: String,
    is_success: bool,
    when: Instant,
}

// ─── Public UI data structures ────────────────────────────────────────────────

/// A single position within an investment account.
#[derive(Clone)]
pub struct PositionRow {
    pub ticker:         String,
    pub units:          String,   // share quantity
    pub unit_value:     String,   // current price (→ market_prices_usd)
    pub cost_basis:     String,   // avg cost per share (→ holdings.avg_cost); optional
    pub growth_pct:     String,   // expected annual growth % e.g. "7.0"; optional
    pub volatility_pct: String,   // annual std-dev % for Marco Polo mode, e.g. "18.0"
    // ── Active management (V6.0) ────────────────────────────────────────────────
    pub accum_amount:         String, // monthly buy amount (USD); empty = no rule
    pub accum_frequency:      String, // "Monthly" | "Quarterly" | "Annual"
    pub stop_at_retirement:   bool,   // stop accumulation on retirement date
    pub drip_enabled:         bool,   // reinvest dividends (true by default)
    pub drip_reinvest_ticker: String, // empty = self; "CASH" = cash; else redirect ticker
    pub target_alloc_pct:     String, // target portfolio weight %; empty = no target
    pub mgmt_expanded:        bool,   // UI-only: expand the management sub-panel
    /// V6.6: per-position rebalance date (YYYY-MM-DD, empty = use account/global).
    pub rebalance_date:       String,
    // ── V7.6 — Asset classification & detailed return profile ──────────────────
    /// "Stock" | "ETF" | "MutualFund" | "Other".
    pub asset_class:          String,
    // ── Stage 05 — PFIC regime ─────────────────────────────────────────────────
    /// "NotPfic" | "Mtm" | "Qef" | "ExcessDistribution".
    pub pfic_regime:          String,
    /// When true, the component-wise return profile drives the engine instead of
    /// the single legacy `growth_pct` + flat dividend yield.
    pub use_detailed_profile: bool,
    /// UI-only: expand the return-profile sub-panel.
    pub profile_expanded:     bool,
    /// Annual price-only capital appreciation %, e.g. "5.2". Stocks / ETFs / Other.
    pub cap_growth_pct:        String,
    /// Annual fund-NAV growth %. Mutual Fund / Other.
    pub nav_growth_pct:        String,
    pub dividend_yield_pct:    String,
    pub interest_yield_pct:    String,
    pub cap_gains_dist_pct:    String,
    pub special_dist_pct:      String,
    pub roc_pct:               String,
    pub expense_ratio_pct:     String,
}

impl Default for PositionRow {
    fn default() -> Self {
        Self {
            ticker:               String::new(),
            units:                String::new(),
            unit_value:           String::new(),
            cost_basis:           String::new(),
            growth_pct:           String::new(),
            volatility_pct:       "18.0".into(),
            accum_amount:         String::new(),
            accum_frequency:      "Monthly".into(),
            stop_at_retirement:   true,
            drip_enabled:         true,
            drip_reinvest_ticker: String::new(),
            target_alloc_pct:     String::new(),
            mgmt_expanded:        false,
            rebalance_date:       String::new(),
            asset_class:          "Stock".into(),
            pfic_regime:          "NotPfic".into(),
            use_detailed_profile: false,
            profile_expanded:     false,
            cap_growth_pct:        String::new(),
            nav_growth_pct:        String::new(),
            dividend_yield_pct:    String::new(),
            interest_yield_pct:    String::new(),
            cap_gains_dist_pct:    String::new(),
            special_dist_pct:      String::new(),
            roc_pct:               String::new(),
            expense_ratio_pct:     String::new(),
        }
    }
}

/// A single RSU award tranche.
#[derive(Clone)]
pub struct RsuRow {
    pub ticker:                   String,
    pub grant_date:               String,  // YYYY-MM-DD
    pub units_awarded:            String,  // total shares at grant
    pub months_to_finish_vesting: String,  // integer months e.g. "48"
    pub specific_vesting_months:  String,  // comma-separated e.g. "2,5,8,11"
    pub delayed_initial_vest:     bool,
    pub cliff_vest_months:        String,  // active only when delayed_initial_vest
    // ── V7.7 — Per-ticker pricing & return profile ──────────────────────────
    pub unit_value:           String,   // starter price USD/share; empty = fallback
    pub growth_pct:           String,   // annual growth %; empty = fallback
    pub use_detailed_profile: bool,
    pub profile_expanded:     bool,     // UI-only: expand the return profile sub-panel
    pub cap_growth_pct:       String,   // annual price-only appreciation %
    pub dividend_yield_pct:   String,   // annual dividend yield %
}

impl Default for RsuRow {
    fn default() -> Self {
        Self {
            ticker:                   String::new(),
            grant_date:               String::new(),
            units_awarded:            String::new(),
            months_to_finish_vesting: "48".into(),
            specific_vesting_months:  "2,5,8,11".into(),
            delayed_initial_vest:     false,
            cliff_vest_months:        "0".into(),
            unit_value:               String::new(),
            growth_pct:               String::new(),
            use_detailed_profile:     false,
            profile_expanded:         false,
            cap_growth_pct:           String::new(),
            dividend_yield_pct:       String::new(),
        }
    }
}

/// A single fund within a DC Plan account.
#[derive(Clone)]
pub struct DcFundRow {
    pub fund_name:         String,
    pub ticker:            String,  // optional Yahoo symbol (e.g. 0331418A.T) for ✨ auto-fetch
    pub units:             String,  // units held (口)
    pub price_per_10k:     String,  // NAV in ¥ per 10,000 units
    pub contrib_alloc_pct: String,  // % of total monthly contribution routed here
    pub growth_pct:        String,  // expected annual CAGR %
    pub stop_at_retirement: bool,
}

impl Default for DcFundRow {
    fn default() -> Self {
        Self {
            fund_name:         String::new(),
            ticker:            String::new(),
            units:             String::new(),
            price_per_10k:     String::new(),
            contrib_alloc_pct: "100".into(),
            growth_pct:        "8.0".into(),
            stop_at_retirement: true,
        }
    }
}

/// A dependent child entry for the Demographics section.
///
/// V6.6: birthdate now carries full month/day precision (YYYY-MM-DD).
#[derive(Clone, Default)]
pub struct DependentEntry {
    /// Full birth date as text, e.g. "2018-09-18".
    pub birth_date: String,
    /// When true, VA rider eligibility extends to age 23 instead of age 18.
    pub is_college_student: bool,
}

/// A single investment account with its own type, tax jurisdiction, and positions.
#[derive(Clone)]
pub struct InvestmentAccountRow {
    pub account_type:     String,
    pub tax_jurisdiction: TaxProtocol,
    pub positions:        Vec<PositionRow>,
    // ── DC Plan inline fields ──────────────────────────────────────────────────
    pub dc_monthly_jpy:    String,
    pub dc_funds:          Vec<DcFundRow>,  // per-fund allocation list
    pub dc_growth_rate:    String,          // fallback CAGR when fund row has no growth_pct
    pub dc_use_market_avg: bool,
    pub dc_volatility:     String,          // annual std-dev % for Monte Carlo
}

impl Default for InvestmentAccountRow {
    fn default() -> Self {
        Self {
            account_type:      "Taxable Brokerage".into(),
            tax_jurisdiction:  TaxProtocol::Both,
            positions:         Vec::new(),
            dc_monthly_jpy:    "45000".into(),
            dc_funds:          vec![DcFundRow::default()],
            dc_growth_rate:    "8.0".into(),
            dc_use_market_avg: false,
            dc_volatility:     "15.0".into(),
        }
    }
}

// ─── Public state ─────────────────────────────────────────────────────────────

/// Editable UI state for the Input Configuration panel.
pub struct InputPanelState {
    // ── Timing ──────────────────────────────────────────────────────────────
    pub start_date:       String,
    pub end_date:         String,
    pub retirement_date:  String,
    pub rebalance_date:   String,
    // ── Economics ────────────────────────────────────────────────────────────
    pub usd_jpy_rate:     String,
    pub inflation_us:     String,
    pub inflation_japan:  String,
    // ── Expenses (JPY/month) ──────────────────────────────────────────────────
    pub base_expense_jpy: String,
    pub min_expense_jpy:  String,
    pub nhi_first_year_monthly_jpy: String,
    // ── Tax ──────────────────────────────────────────────────────────────────
    pub tax_jurisdiction: TaxJurisdiction,
    pub filing_status:    String,
    pub us_state_code:    String,
    // ── US Tax Mitigation Strategy ────────────────────────────────────────────
    pub us_tax_strategy:  UsTaxStrategy,
    // ── RSU tax handling ─────────────────────────────────────────────────────
    pub rsu_tax_handling: String,
    /// V7.7.2 — When true, model RSU tax-liability margin calls (realism on).
    pub rsu_sell_to_cover_realism: bool,
    // ── Stage 03: Monthly Dependent Precision ─────────────────────────────────
    /// When true (default), VA add-ons, NHI per-capita, and Jido Teate are
    /// computed at month resolution using exact birth dates.
    pub monthly_dependent_precision: bool,
    // ── Family demographics (V6.6) ───────────────────────────────────────────
    pub user_birth_date:   String,
    pub is_married:        bool,
    pub spouse_birth_date: String,
    // ── NRA Spouse Tax Profile (Stage 02) ────────────────────────────────────
    pub spouse_profile:               SpouseProfile,
    pub spouse_japan_salary_jpy:      String,
    pub spouse_japan_misc_income_jpy: String,
    // ── Spouse SS (V6.6) ─────────────────────────────────────────────────────
    pub spouse_ss_enabled:      bool,
    pub spouse_ss_monthly_usd:  String,
    pub spouse_ss_start_age:    String,
    pub spouse_ss_jurisdiction: TaxProtocol,
    // ── Spouse Nenkin (V6.6) ────────────────────────────────────────────────
    pub spouse_nenkin_enabled:      bool,
    pub spouse_nenkin_monthly_jpy:  String,
    pub spouse_nenkin_start_age:    String,
    pub spouse_nenkin_jurisdiction: TaxProtocol,
    // ── FX Drift cadence (V6.6) ──────────────────────────────────────────────
    pub fx_drift_cadence_months:    String,
    pub fx_drift_increase_amount_jpy: String,
    // ── FERS ─────────────────────────────────────────────────────────────────
    pub fers_monthly_usd: String,
    pub fers_start_age:   String,
    // ── VA Disability Profile ─────────────────────────────────────────────────
    pub va_disability_rating: String,
    pub va_dependent_status:  VaDependentStatus,
    // ── Social Security ───────────────────────────────────────────────────────
    pub ss_monthly_usd: String,
    pub ss_start_age:   String,
    // ── SSDI ─────────────────────────────────────────────────────────────────
    pub ssdi_monthly_usd: String,
    // ── Family Demographics ───────────────────────────────────────────────────
    pub dependents: Vec<DependentEntry>,
    // ── Nenkin pension income ─────────────────────────────────────────────────
    pub nenkin_income_monthly_jpy: String,
    pub nenkin_income_start_age:   String,
    // ── Buffers ───────────────────────────────────────────────────────────────
    pub war_chest_enabled:         bool,
    pub war_chest_target_jpy:      String,
    pub pre_funded_war_chest_jpy:  String,
    pub bridge_fund_enabled:       bool,
    pub bridge_months:             String,
    pub pre_funded_bridge_usd:     String,
    // ── Market simulation ────────────────────────────────────────────────────
    pub fx_drift_enabled:   bool,
    pub fx_drift_rate:      String,
    pub recession_enabled:  bool,
    pub recession_severity: String,
    pub recession_years:    String,
    // ── Stage 04 — Shock Application Order ────────────────────────────────────
    /// Controls which shock is applied first when both recession and FX shock
    /// fall in the same calendar year.
    pub shock_ordering: ShockOrdering,
    // ── Marco Polo (Monte Carlo) mode ──────────────────────────────────────────
    pub marco_polo_enabled: bool,
    // ── Active Management / Rebalancing (V6.0) ────────────────────────────────
    pub rebalance_enabled:   bool,
    pub rebalance_frequency: String, // "Monthly" | "Quarterly" | "Semi-Annual" | "Annual"
    // ── Japan regional location ───────────────────────────────────────────────
    pub prefecture: String,
    pub city:       String,
    // ── Per-source tax jurisdictions ──────────────────────────────────────────
    pub fers_jurisdiction:            TaxProtocol,
    pub fers_japan_local_tax_exempt:  bool,
    pub ss_jurisdiction:              TaxProtocol,
    pub nenkin_jurisdiction:   TaxProtocol,
    // ── Military Retired Pay ──────────────────────────────────────────────────
    pub military_monthly_usd:  String,
    pub military_jurisdiction: TaxProtocol,
    // ── VA SMC ────────────────────────────────────────────────────────────────
    pub va_smc_variant: String,
    // ── Investment Accounts (dynamic list) ────────────────────────────────────
    pub accounts: Vec<InvestmentAccountRow>,
    // ── RSU Awards (multi-tranche) ────────────────────────────────────────────
    pub rsu_awards: Vec<RsuRow>,
    // ── NHI Settings ─────────────────────────────────────────────────────────
    /// true = Automatic (Calculated rate schedule); false = Manual (fixed amounts)
    pub nhi_calculated_mode:    bool,
    // Automatic mode fields (rates as percent strings, e.g. "8.46")
    pub nhi_medical_rate:       String,
    pub nhi_support_rate:       String,
    pub nhi_nursing_rate:       String,
    pub nhi_per_capita_medical: String,
    pub nhi_per_capita_support: String,
    pub nhi_per_capita_nursing: String,
    pub nhi_cap_medical:        String,
    pub nhi_cap_support:        String,
    pub nhi_cap_nursing:        String,
    pub nhi_include_us_income:  bool,
    // Manual mode fields (annual totals in JPY)
    pub nhi_spike_total_jpy:    String,
    pub nhi_ongoing_total_jpy:  String,
    // ── Stage 10 — Long-Term Care Insurance (Kaigo Hoken) ─────────────────────
    pub kaigo_hoken_enabled: bool,
    pub kaigo_care_scenario: String,  // "none" | "low" | "medium" | "high"
    // ── Entitlement overrides ─────────────────────────────────────────────────
    pub va_override_enabled:  bool,
    pub va_override_monthly:  String,
    pub smc_override_enabled: bool,
    pub smc_override_monthly: String,
    // ── Family Financial Planning (V7.3 / V7.5 — optional) ──────────────────
    /// When true, the Tier 2.5 Education Fund accumulation channel is active.
    pub education_fund_enabled: bool,
    pub edu_savings_jpy_monthly: String,
    /// When true, the Tier 9 Estate Planning Gift Sink is active.
    pub gift_sink_enabled: bool,
    pub annual_gift_jpy_per_recipient: String,
    pub gift_recipient_count: String,
    pub us_gift_exclusion_usd: String,
    // ── Stage 05 — PFIC Basis Drift Monitor ──────────────────────────────────
    pub track_pfic_basis_drift: bool,
    // ── Stage 07 — Estate Planning ───────────────────────────────────────────
    pub enable_estate_planning: bool,
    pub death_date: String,
    pub spouse_death_date: String,
    pub estate_heirs: Vec<HeirEntry>,
    pub enable_gifting_optimiser: bool,
    // ── Source ───────────────────────────────────────────────────────────────
    pub source_json: Option<Value>,
    pub source_path: Option<String>,
    // ── Signals back to app.rs ───────────────────────────────────────────────
    pub reload_path: Option<String>,
}

/// Stage 07 — A single heir row in the input panel.
#[derive(Debug, Clone, Default)]
pub struct HeirEntry {
    pub name:         String,
    pub birth_date:   String,
    pub relationship: HeirRelationship,
}

impl Default for InputPanelState {
    /// Blank-slate scenario: all monetary fields zeroed, one empty Taxable Brokerage account.
    /// `source_json` is primed with the minimal template so Save works immediately.
    fn default() -> Self {
        let blank_json = serde_json::json!({
            "simulation_settings": {},
            "holdings": {},
            "rsu_awards": [],
            "market_prices_usd": {},
            "growth_rates_annual": {}
        });
        // V6.6: defaults — Start = Today, End = Today + 50 years.
        let today = Local::now().date_naive();
        let end_default = NaiveDate::from_ymd_opt(today.year() + 50, today.month(), today.day())
            .unwrap_or(today);
        Self {
            start_date:          today.format("%Y-%m-%d").to_string(),
            end_date:            end_default.format("%Y-%m-%d").to_string(),
            retirement_date:     String::new(),
            rebalance_date:      String::new(),
            usd_jpy_rate:        "0".into(),
            inflation_us:        "0.028".into(),
            inflation_japan:     "0.028".into(),
            base_expense_jpy:    "0".into(),
            min_expense_jpy:     "0".into(),
            nhi_first_year_monthly_jpy: "0".into(),
            tax_jurisdiction:    TaxJurisdiction::Both,
            filing_status:       "Married Filing Jointly".into(),
            us_state_code:       "None".into(),
            us_tax_strategy:     UsTaxStrategy::FtcOnly,
            rsu_tax_handling:    "SALARY".into(),
            rsu_sell_to_cover_realism: true,
            monthly_dependent_precision: true,
            shock_ordering: ShockOrdering::DepreciateThenReprice,
            user_birth_date:        String::new(),
            is_married:             false,
            spouse_birth_date:      String::new(),
            spouse_profile:               SpouseProfile::UsPerson,
            spouse_japan_salary_jpy:      "0".into(),
            spouse_japan_misc_income_jpy: "0".into(),
            spouse_ss_enabled:      false,
            spouse_ss_monthly_usd:  "0".into(),
            spouse_ss_start_age:    "67".into(),
            spouse_ss_jurisdiction: TaxProtocol::Both,
            spouse_nenkin_enabled:      false,
            spouse_nenkin_monthly_jpy:  "0".into(),
            spouse_nenkin_start_age:    "65".into(),
            spouse_nenkin_jurisdiction: TaxProtocol::Both,
            fx_drift_cadence_months:    "0".into(),
            fx_drift_increase_amount_jpy: "0".into(),
            fers_monthly_usd:    "0".into(),
            fers_start_age:      "62".into(),
            va_disability_rating: "0".into(),
            va_dependent_status:  VaDependentStatus::VetOnly,
            ss_monthly_usd:       "0".into(),
            ss_start_age:         "67".into(),
            ssdi_monthly_usd:     "0".into(),
            dependents:           vec![],
            nenkin_income_monthly_jpy: "0".into(),
            nenkin_income_start_age:   "65".into(),
            war_chest_enabled:       true,
            war_chest_target_jpy:    "0".into(),
            pre_funded_war_chest_jpy: "0".into(),
            bridge_fund_enabled:     true,
            bridge_months:           "0".into(),
            pre_funded_bridge_usd:   "0".into(),
            fx_drift_enabled:   false,
            fx_drift_rate:      "0.02".into(),
            recession_enabled:  false,
            recession_severity: "0.20".into(),
            recession_years:    String::new(),
            marco_polo_enabled:  false,
            rebalance_enabled:   false,
            rebalance_frequency: "Annual".into(),
            prefecture:          String::new(),
            city:               String::new(),
            fers_jurisdiction:            TaxProtocol::Both,
            fers_japan_local_tax_exempt:  false,
            ss_jurisdiction:              TaxProtocol::Both,
            nenkin_jurisdiction:  TaxProtocol::Both,
            military_monthly_usd: "0".into(),
            military_jurisdiction: TaxProtocol::Both,
            va_smc_variant:     String::new(),
            accounts:           vec![InvestmentAccountRow::default()],
            rsu_awards:         Vec::new(),
            nhi_calculated_mode:    true,
            nhi_medical_rate:       "8.46".into(),
            nhi_support_rate:       "2.04".into(),
            nhi_nursing_rate:       "2.02".into(),
            nhi_per_capita_medical: "33600".into(),
            nhi_per_capita_support: "11400".into(),
            nhi_per_capita_nursing: "12600".into(),
            nhi_cap_medical:        "650000".into(),
            nhi_cap_support:        "240000".into(),
            nhi_cap_nursing:        "170000".into(),
            nhi_include_us_income:  false,
            nhi_spike_total_jpy:    "0".into(),
            nhi_ongoing_total_jpy:  "0".into(),
            kaigo_hoken_enabled: true,
            kaigo_care_scenario: "none".into(),
            va_override_enabled:  false,
            va_override_monthly:  String::new(),
            smc_override_enabled: false,
            smc_override_monthly: String::new(),
            education_fund_enabled:        false,
            edu_savings_jpy_monthly:       "0".into(),
            gift_sink_enabled:             false,
            annual_gift_jpy_per_recipient: "1100000".into(),
            gift_recipient_count:          "0".into(),
            us_gift_exclusion_usd:         "19000".into(),
            track_pfic_basis_drift:        true,
            enable_estate_planning:        false,
            death_date:                    String::new(),
            spouse_death_date:             String::new(),
            estate_heirs:                  vec![],
            enable_gifting_optimiser:      false,
            source_json:  Some(blank_json),
            source_path:  None,
            reload_path:  None,
        }
    }
}

impl InputPanelState {
    /// Build state from a parsed scenario JSON value.
    pub fn from_json(json: &Value, path: &str) -> Self {
        let sets = &json["simulation_settings"];

        let str_val = |key: &str, default: &str| -> String {
            sets[key].as_str().unwrap_or(default).to_string()
        };
        let num_str = |key: &str, default: &str| -> String {
            match &sets[key] {
                Value::Number(n) => {
                    if let Some(i) = n.as_i64() { i.to_string() }
                    else if let Some(f) = n.as_f64() { format!("{}", f) }
                    else { default.to_string() }
                }
                Value::String(s) => s.clone(),
                _ => default.to_string(),
            }
        };
        let bool_val = |key: &str, default: bool| -> bool {
            sets[key].as_bool().unwrap_or(default)
        };

        let tax_jurisdiction = match sets["tax_jurisdiction"].as_str().unwrap_or("both") {
            "us_only" | "US_ONLY" | "UsOnly"          => TaxJurisdiction::UsOnly,
            "japan_only" | "JAPAN_ONLY" | "JapanOnly" => TaxJurisdiction::JapanOnly,
            _                                          => TaxJurisdiction::Both,
        };

        let recession_years = if let Value::Array(arr) = &sets["simulated_recessions"] {
            arr.iter().filter_map(|item| {
                let year = item["year"].as_i64().or_else(|| item.as_i64())?;
                let sev  = item["severity"].as_f64().unwrap_or(0.20);
                Some(format!("{}:{:.2}", year, sev))
            }).collect::<Vec<_>>().join(", ")
        } else {
            String::new()
        };

        let us_tax_strategy = match sets["us_tax_strategy"].as_str().unwrap_or("ftc_only") {
            "feie_and_ftc" | "FeieAndFtc" | "FEIE+FTC" => UsTaxStrategy::FeieAndFtc,
            _ => UsTaxStrategy::FtcOnly,
        };

        let va_dependent_status = match sets["va_dependent_status"].as_str().unwrap_or("vet_only") {
            "with_spouse"           | "WithSpouse"         => VaDependentStatus::WithSpouse,
            "with_spouse_and_child" | "WithSpouseAndChild" => VaDependentStatus::WithSpouseAndChild,
            _ => VaDependentStatus::VetOnly,
        };

        let parse_protocol = |key: &str| -> TaxProtocol {
            match sets[key].as_str().unwrap_or("both") {
                "us_only"    | "UsOnly"    => TaxProtocol::UsOnly,
                "japan_only" | "JapanOnly" => TaxProtocol::JapanOnly,
                "tax_free"   | "TaxFree"   => TaxProtocol::TaxFree,
                _                          => TaxProtocol::Both,
            }
        };

        let military_monthly_usd = match &sets["military_retired"]["monthly_usd"] {
            Value::Number(n) => n.as_f64().map(|f| format!("{}", f)).unwrap_or_else(|| "0".into()),
            _ => "0".into(),
        };
        let military_jurisdiction = match sets["military_retired"]["jurisdiction"].as_str().unwrap_or("both") {
            "us_only"    | "UsOnly"    => TaxProtocol::UsOnly,
            "japan_only" | "JapanOnly" => TaxProtocol::JapanOnly,
            "tax_free"   | "TaxFree"   => TaxProtocol::TaxFree,
            _                          => TaxProtocol::Both,
        };

        // ── Entitlement overrides ─────────────────────────────────────────────
        let va_override_monthly = match &sets["va_monthly_override"] {
            Value::Number(n) => n.as_f64()
                .filter(|&f| f >= 0.0)
                .map(|f| format!("{:.2}", f))
                .unwrap_or_default(),
            _ => String::new(),
        };
        let va_override_enabled = !va_override_monthly.is_empty();
        let smc_override_monthly = match &sets["smc_monthly_override"] {
            Value::Number(n) => n.as_f64()
                .filter(|&f| f >= 0.0)
                .map(|f| format!("{:.2}", f))
                .unwrap_or_default(),
            _ => String::new(),
        };
        let smc_override_enabled = !smc_override_monthly.is_empty();

        // ── Load investment accounts from holdings sections ────────────────────
        let market_prices = &json["market_prices_usd"];
        let growth_rates  = &json["growth_rates_annual"];

        let volatility_pcts = &json["volatility_pcts"];

        let load_positions = |holdings_obj: &serde_json::Map<String, Value>| -> Vec<PositionRow> {
            let mut rows = Vec::new();
            for (ticker, info) in holdings_obj {
                if ticker.starts_with("//") || ticker.starts_with('_') { continue; }
                if !info.is_object() { continue; }
                let units = match &info["qty"] {
                    Value::Number(n) => n.as_f64().map(|f| format!("{}", f)).unwrap_or_default(),
                    _ => String::new(),
                };
                if units.is_empty() { continue; }
                let cost_basis = match &info["avg_cost"] {
                    Value::Number(n) => n.as_f64().map(|f| format!("{}", f)).unwrap_or_default(),
                    _ => String::new(),
                };
                let unit_value = match &market_prices[ticker.as_str()] {
                    Value::Number(n) => n.as_f64().map(|f| format!("{}", f)).unwrap_or_default(),
                    _ => String::new(),
                };
                let growth_pct = match &growth_rates[ticker.as_str()] {
                    Value::Number(n) => n.as_f64().map(|f| format!("{:.1}", f * 100.0)).unwrap_or_default(),
                    _ => String::new(),
                };
                let volatility_pct = match &volatility_pcts[ticker.as_str()] {
                    Value::Number(n) => n.as_f64()
                        .map(|f| format!("{:.1}", f * 100.0))
                        .unwrap_or_else(|| "18.0".into()),
                    _ => "18.0".into(),
                };
                // Also load DRIP settings from per-ticker info (V6.0)
                let drip_enabled = info["drip_enabled"].as_bool().unwrap_or(true);
                let drip_reinvest_ticker = info["dividend_reinvest_target"]
                    .as_str().unwrap_or("").to_string();
                // V7.6 — Asset classification + detailed return profile (optional).
                let asset_class = match info["asset_class"].as_str().unwrap_or("stock") {
                    "etf" | "ETF" | "Etf"                  => "ETF",
                    "mutual_fund" | "MutualFund" | "mutual" => "MutualFund",
                    "other" | "Other"                       => "Other",
                    _                                       => "Stock",
                }.to_string();
                let pfic_regime = match info["pfic_regime"].as_str().unwrap_or("not_pfic") {
                    "mtm" | "Mtm"                                   => "Mtm",
                    "qef" | "Qef"                                   => "Qef",
                    "excess_distribution" | "ExcessDistribution"    => "ExcessDistribution",
                    _                                               => "NotPfic",
                }.to_string();
                let profile = &info["return_profile"];
                let use_detailed_profile = profile.is_object();
                let pct_from_frac = |k: &str| -> String {
                    profile[k].as_f64()
                        .map(|f| format!("{:.3}", f * 100.0))
                        .unwrap_or_default()
                };
                rows.push(PositionRow {
                    ticker: ticker.clone(), units, unit_value, cost_basis, growth_pct, volatility_pct,
                    drip_enabled, drip_reinvest_ticker,
                    asset_class, pfic_regime,
                    use_detailed_profile,
                    cap_growth_pct:     pct_from_frac("cap_growth"),
                    nav_growth_pct:     pct_from_frac("nav_growth"),
                    dividend_yield_pct: pct_from_frac("dividend_yield"),
                    interest_yield_pct: pct_from_frac("interest_yield"),
                    cap_gains_dist_pct: pct_from_frac("cap_gains_dist"),
                    special_dist_pct:   pct_from_frac("special_dist"),
                    roc_pct:            pct_from_frac("roc"),
                    expense_ratio_pct:  pct_from_frac("expense_ratio"),
                    ..Default::default()
                });
            }
            rows
        };

        // Ordered map: JSON holdings key → display account type
        let key_to_type: &[(&str, &str)] = &[
            ("taxable",  "Taxable Brokerage"),
            ("ira",      "IRA (Traditional)"),
            ("roth_ira", "Roth IRA"),
            ("k401",     "401(k)"),
            ("nisa",     "NISA"),
            ("ideco",    "iDeCo"),
        ];

        let parse_protocol_val = |v: &Value| -> TaxProtocol {
            match v.as_str().unwrap_or("both") {
                "us_only"    | "UsOnly"    => TaxProtocol::UsOnly,
                "japan_only" | "JapanOnly" => TaxProtocol::JapanOnly,
                "tax_free"   | "TaxFree"   => TaxProtocol::TaxFree,
                _                          => TaxProtocol::Both,
            }
        };

        let mut accounts: Vec<InvestmentAccountRow> = Vec::new();

        // ── DC Plan growth rate (read once, used below) ───────────────────────
        let dc_growth_rate_str: String = {
            let from_settings = sets["dc_growth_rate"].as_f64();
            let from_holdings = json["holdings"]["japan_dc"]["growth_rate"].as_f64();
            let rate = from_settings.or(from_holdings).unwrap_or(0.08);
            format!("{:.1}", rate * 100.0)
        };
        let dc_use_market_avg: bool = dc_growth_rate_str.trim().parse::<f64>()
            .map(|f| (f - 10.0).abs() < 0.1)
            .unwrap_or(false);

        for &(key, account_type) in key_to_type {
            if let Value::Object(h) = &json["holdings"][key] {
                if h.is_empty() { continue; }
                let positions = load_positions(h);
                if positions.is_empty() { continue; }
                let jur_key = format!("{}_tax_jurisdiction", key);
                let account_tax = parse_protocol_val(&sets[jur_key.as_str()]);
                accounts.push(InvestmentAccountRow {
                    account_type:      account_type.into(),
                    tax_jurisdiction:  account_tax,
                    positions,
                    dc_monthly_jpy:    "0".into(),
                    dc_funds:          Vec::new(),
                    dc_growth_rate:    "8.0".into(),
                    dc_use_market_avg: false,
                    dc_volatility:     "15.0".into(),
                });
            }
        }

        // DC Plan is handled separately (different JSON schema)
        let dc_monthly = num_str("japan_dc_monthly_contribution_jpy", "0");
        let dc_val: f64 = dc_monthly.trim().parse().unwrap_or(0.0);
        if dc_val > 0.0 {
            // Parse per-fund rows; fall back to legacy single-fund fields.
            let dc_funds: Vec<DcFundRow> = if let Value::Array(arr) = &sets["dc_funds"] {
                arr.iter().map(|f| DcFundRow {
                    fund_name: f["fund_name"].as_str().unwrap_or("").to_string(),
                    ticker:    f["ticker"].as_str().unwrap_or("").to_string(),
                    units:     f["units"].as_f64()
                        .map(|v| if v == v.floor() { format!("{:.0}", v) } else { format!("{}", v) })
                        .unwrap_or_default(),
                    price_per_10k: f["price_per_10k_jpy"].as_f64()
                        .map(|v| format!("{:.0}", v))
                        .unwrap_or_default(),
                    contrib_alloc_pct: f["contrib_alloc_pct"].as_f64()
                        .map(|v| format!("{:.0}", v))
                        .unwrap_or_else(|| "100".to_string()),
                    growth_pct: f["growth_rate_pct"].as_f64()
                        .map(|v| format!("{:.1}", v))
                        .unwrap_or_else(|| dc_growth_rate_str.clone()),
                    stop_at_retirement: f["stop_at_retirement"].as_bool().unwrap_or(true),
                }).collect()
            } else {
                // Backward compat: single fund from legacy dc_fund_name / dc_contrib_pct.
                let name = str_val("dc_fund_name", "");
                vec![DcFundRow {
                    fund_name: name,
                    ticker: String::new(),
                    units: String::new(),
                    price_per_10k: String::new(),
                    contrib_alloc_pct: str_val("dc_contrib_pct", "100"),
                    growth_pct: dc_growth_rate_str.clone(),
                    stop_at_retirement: true,
                }]
            };
            accounts.push(InvestmentAccountRow {
                account_type:      "DC Plan".into(),
                tax_jurisdiction:  TaxProtocol::JapanOnly,
                positions:         Vec::new(),
                dc_monthly_jpy:    dc_monthly,
                dc_funds,
                dc_growth_rate:    dc_growth_rate_str,
                dc_use_market_avg,
                dc_volatility:     "15.0".into(),
            });
        }

        // Fallback: legacy account_type + holdings.taxable
        if accounts.is_empty() {
            let legacy_type = sets["account_type"].as_str()
                .unwrap_or("Taxable Brokerage").to_string();
            let legacy_positions = if let Value::Object(h) = &json["holdings"]["taxable"] {
                load_positions(h)
            } else {
                Vec::new()
            };
            accounts.push(InvestmentAccountRow {
                account_type:      legacy_type,
                tax_jurisdiction:  TaxProtocol::Both,
                positions:         legacy_positions,
                dc_monthly_jpy:    "0".into(),
                dc_funds:          Vec::new(),
                dc_growth_rate:    "8.0".into(),
                dc_use_market_avg: false,
                dc_volatility:     "15.0".into(),
            });
        }

        // ── Inject active management fields into positions (V6.0) ────────────
        {
            // Build per-ticker target allocation lookup (fraction → percent string)
            let mut target_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            if let Value::Object(allocs) = &sets["target_allocations"] {
                for (ticker, v) in allocs {
                    if let Some(f) = v.as_f64() {
                        target_map.insert(ticker.clone(), format!("{:.1}", f * 100.0));
                    }
                }
            }

            // Build per-(ticker,account_sim_key) accumulation rule lookup
            #[derive(Default)]
            struct AccumEntry {
                amount: String, freq: String, stop: bool,
                drip: bool, drip_redirect: String,
            }
            let mut accum_map: std::collections::HashMap<(String, String), AccumEntry> = std::collections::HashMap::new();
            if let Value::Array(rules) = &sets["accumulation_rules"] {
                for rule in rules {
                    let ticker  = rule["ticker"].as_str().unwrap_or("").to_string();
                    let account = rule["account"].as_str().unwrap_or("Taxable").to_string();
                    if ticker.is_empty() { continue; }
                    let freq_str = match rule["frequency_months"].as_u64().unwrap_or(1) as u32 {
                        3  => "Quarterly",
                        12 => "Annual",
                        _  => "Monthly",
                    }.to_string();
                    accum_map.insert((ticker, account), AccumEntry {
                        amount: rule["monthly_amount"].as_f64().map(|f| format!("{}", f)).unwrap_or_default(),
                        freq: freq_str,
                        stop: rule["stop_at_retirement"].as_bool().unwrap_or(true),
                        drip: rule["drip_enabled"].as_bool().unwrap_or(true),
                        drip_redirect: rule["drip_reinvest_ticker"].as_str().unwrap_or("").to_string(),
                    });
                }
            }

            for account in &mut accounts {
                let sim_key = account_sim_key(&account.account_type).to_string();
                for pos in &mut account.positions {
                    if let Some(t) = target_map.get(&pos.ticker) {
                        pos.target_alloc_pct = t.clone();
                    }
                    if let Some(entry) = accum_map.get(&(pos.ticker.clone(), sim_key.clone())) {
                        pos.accum_amount       = entry.amount.clone();
                        pos.accum_frequency    = entry.freq.clone();
                        pos.stop_at_retirement = entry.stop;
                        pos.drip_enabled       = entry.drip;
                        pos.drip_reinvest_ticker = entry.drip_redirect.clone();
                    }
                }
            }
        }

        // ── Load RSU awards (all tranches) ────────────────────────────────────
        let rsu_awards: Vec<RsuRow> = if let Value::Array(arr) = &json["rsu_awards"] {
            arr.iter().filter_map(|rsu| {
                let ticker = rsu["ticker"].as_str().unwrap_or("").to_string();
                if ticker.is_empty() { return None; }
                let grant_date = rsu["grant_date"].as_str().unwrap_or("").to_string();
                let units_awarded = rsu["total_shares"].as_f64()
                    .map(|f| if f == f.floor() { format!("{:.0}", f) } else { format!("{}", f) })
                    .unwrap_or_default();
                // months_to_finish_vesting: prefer explicit field, fall back to vesting_years*12.
                let months_to_finish_vesting = rsu["vesting_months_total"].as_u64()
                    .unwrap_or_else(|| {
                        let years = rsu["vesting_years"].as_u64().unwrap_or(4);
                        years * 12
                    })
                    .to_string();
                // specific_vesting_months: prefer explicit array, else derive from cadence.
                let specific_vesting_months = if let Value::Array(vm) = &rsu["vesting_months"] {
                    vm.iter().filter_map(|v| v.as_u64()).map(|m| m.to_string())
                        .collect::<Vec<_>>().join(",")
                } else {
                    match rsu["vesting_cadence"].as_str().unwrap_or("quarterly") {
                        "monthly"  | "Monthly"  => "1,2,3,4,5,6,7,8,9,10,11,12".to_string(),
                        "annually" | "Annually" => "1".to_string(),
                        _                       => "2,5,8,11".to_string(),
                    }
                };
                let cliff_months = rsu["cliff_vest_months"].as_u64().unwrap_or(0);
                let rp = &rsu["return_profile"];
                let rp_pct = |k: &str| -> String {
                    rp[k].as_f64().map(|f| format!("{:.3}", f * 100.0)).unwrap_or_default()
                };
                let use_detailed_profile = rp.is_object();
                Some(RsuRow {
                    ticker,
                    grant_date,
                    units_awarded,
                    months_to_finish_vesting,
                    specific_vesting_months,
                    delayed_initial_vest: cliff_months > 0,
                    cliff_vest_months: cliff_months.to_string(),
                    unit_value: rsu["unit_value"].as_f64()
                        .map(|f| format!("{:.2}", f)).unwrap_or_default(),
                    growth_pct: rsu["growth_rate"].as_f64()
                        .map(|f| format!("{:.1}", f * 100.0)).unwrap_or_default(),
                    use_detailed_profile,
                    profile_expanded: false,
                    cap_growth_pct:     rp_pct("cap_growth"),
                    dividend_yield_pct: rp_pct("dividend_yield"),
                })
            }).collect()
        } else {
            Vec::new()
        };

        // ── NHI Settings ─────────────────────────────────────────────────────
        let def_nhi = NhiCalculatedRates::sagamihara_2026();
        let nhi_mode = sets["nhi_model"]["mode"].as_str().unwrap_or("calculated");
        let nhi_calculated_mode = nhi_mode != "manual_override";

        let nhi_get_rate = |key: &str, default: f64| -> String {
            sets["nhi_model"][key].as_f64().map(|f| format!("{:.4}", f * 100.0))
                .unwrap_or_else(|| format!("{:.2}", default * 100.0))
        };
        let nhi_get_jpy = |key: &str, default: f64| -> String {
            sets["nhi_model"][key].as_f64().map(|f| format!("{:.0}", f))
                .unwrap_or_else(|| format!("{:.0}", default))
        };

        let nhi_medical_rate       = nhi_get_rate("medical_rate",        def_nhi.medical_rate);
        let nhi_support_rate       = nhi_get_rate("elderly_support_rate", def_nhi.elderly_support_rate);
        let nhi_nursing_rate       = nhi_get_rate("nursing_care_rate",    def_nhi.nursing_care_rate);
        let nhi_per_capita_medical = nhi_get_jpy("per_capita_medical",    def_nhi.per_capita_medical);
        let nhi_per_capita_support = nhi_get_jpy("per_capita_support",    def_nhi.per_capita_support);
        let nhi_per_capita_nursing = nhi_get_jpy("per_capita_nursing",    def_nhi.per_capita_nursing);
        let nhi_cap_medical        = nhi_get_jpy("cap_medical",           def_nhi.cap_medical);
        let nhi_cap_support        = nhi_get_jpy("cap_support",           def_nhi.cap_support);
        let nhi_cap_nursing        = nhi_get_jpy("cap_nursing",           def_nhi.cap_nursing);
        let nhi_include_us_income  = sets["nhi_model"]["include_us_investment_income"].as_bool().unwrap_or(false);
        let nhi_spike_total_jpy    = sets["nhi_model"]["spike_year_total_jpy"].as_f64()
            .map(|f| format!("{:.0}", f)).unwrap_or_else(|| "0".into());
        let nhi_ongoing_total_jpy  = sets["nhi_model"]["ongoing_annual_total_jpy"].as_f64()
            .map(|f| format!("{:.0}", f)).unwrap_or_else(|| "0".into());

        Self {
            start_date:      str_val("start_date",      ""),
            end_date:        str_val("end_date",         ""),
            retirement_date: str_val("retirement_date",  ""),
            rebalance_date:  str_val("rebalance_date",   ""),
            usd_jpy_rate:    num_str("usd_jpy_rate",     "0"),
            inflation_us:    num_str("inflation_us_cpi", "0.028"),
            inflation_japan: num_str("inflation_japan_cpi", "0.028"),
            base_expense_jpy: num_str("base_monthly_expenses_jpy", "0"),
            min_expense_jpy:  num_str("min_monthly_expenses_jpy",  "0"),
            nhi_first_year_monthly_jpy: num_str("nhi_spike_monthly_jpy", "0"),
            tax_jurisdiction,
            filing_status: str_val("us_filing_status", "Married Filing Jointly"),
            us_state_code: str_val("us_state_code", "None"),
            us_tax_strategy,
            rsu_tax_handling: str_val("rsu_tax_handling", "SALARY"),
            rsu_sell_to_cover_realism: sets["rsu_sell_to_cover_realism"].as_bool().unwrap_or(true),
            monthly_dependent_precision: bool_val("monthly_dependent_precision", true),
            user_birth_date:   str_val("birth_date",        ""),
            is_married:        sets["is_married"].as_bool()
                                  .unwrap_or_else(|| sets["spouse_birth_date"].is_string()),
            spouse_birth_date: str_val("spouse_birth_date", ""),
            spouse_profile: match sets["spouse_profile"].as_str().unwrap_or("us_person") {
                "nra_elected_to_be_treated_as_resident" | "nra_elected_mfj" =>
                    SpouseProfile::NraElectedToBeTreatedAsResident,
                "nra_mfs"  => SpouseProfile::NraMfs,
                "nra_head_of_household_eligible" | "nra_hoh" =>
                    SpouseProfile::NraHeadOfHouseholdEligible,
                _ => SpouseProfile::UsPerson,
            },
            spouse_japan_salary_jpy:      num_str("spouse_japan_salary_jpy",      "0"),
            spouse_japan_misc_income_jpy: num_str("spouse_japan_misc_income_jpy", "0"),
            spouse_ss_enabled:      sets["spouse_ss_monthly_usd"].as_f64().unwrap_or(0.0) > 0.0,
            spouse_ss_monthly_usd:  num_str("spouse_ss_monthly_usd", "0"),
            spouse_ss_start_age:    num_str("spouse_ss_start_age",   "67"),
            spouse_ss_jurisdiction: parse_protocol("spouse_ss_jurisdiction"),
            spouse_nenkin_enabled:      sets["spouse_nenkin_monthly_jpy"].as_f64().unwrap_or(0.0) > 0.0,
            spouse_nenkin_monthly_jpy:  num_str("spouse_nenkin_monthly_jpy", "0"),
            spouse_nenkin_start_age:    num_str("spouse_nenkin_start_age",   "65"),
            spouse_nenkin_jurisdiction: parse_protocol("spouse_nenkin_jurisdiction"),
            fx_drift_cadence_months:      num_str("fx_drift_cadence_months",      "0"),
            fx_drift_increase_amount_jpy: num_str("fx_drift_increase_amount_jpy", "0"),
            fers_monthly_usd: num_str("fers_monthly_payment_usd", "0"),
            fers_start_age: if sets["fers_start_age"].is_number() {
                num_str("fers_start_age", "62")
            } else { "62".into() },
            va_disability_rating: num_str("va_disability_rating", "0"),
            va_dependent_status,
            ss_monthly_usd:            num_str("ss_monthly_usd",            "0"),
            ss_start_age:              num_str("ss_start_age",               "67"),
            ssdi_monthly_usd:          num_str("ssdi_monthly_usd",           "0"),
            dependents: if let Value::Array(arr) = &sets["dependents"] {
                arr.iter().filter_map(|d| {
                    // V6.6: prefer full birth_date, fall back to birth_year (legacy).
                    let date = d["birth_date"].as_str()
                        .map(|s| s.to_string())
                        .or_else(|| d["birth_year"].as_i64()
                            .map(|y| format!("{}-01-01", y)))?;
                    let college = d["is_college_student"].as_bool().unwrap_or(false);
                    Some(DependentEntry { birth_date: date, is_college_student: college })
                }).collect()
            } else {
                vec![]
            },
            nenkin_income_monthly_jpy: num_str("nenkin_income_monthly_jpy", "0"),
            nenkin_income_start_age:   num_str("nenkin_income_start_age",   "65"),
            war_chest_enabled:        bool_val("war_chest_enabled",         true),
            war_chest_target_jpy:     num_str("war_chest_target_jpy",       "0"),
            pre_funded_war_chest_jpy: num_str("pre_funded_war_chest_jpy",   "0"),
            bridge_fund_enabled:      bool_val("bridge_fund_enabled",       true),
            bridge_months:            num_str("bridge_fund_months_target",  "0"),
            pre_funded_bridge_usd:    num_str("pre_funded_bridge_usd",      "0"),
            fx_drift_enabled:   bool_val("simulate_yen_strengthening",      false),
            fx_drift_rate:      num_str("fx_drift_rate_annual",             "0.02"),
            recession_enabled:  bool_val("simulate_recession_at_retirement", false),
            recession_severity: num_str("recession_severity_pct",           "0.20"),
            recession_years,
            shock_ordering: match sets["shock_ordering"].as_str().unwrap_or("depreciate_then_reprice") {
                "reprice_then_depreciate" => ShockOrdering::RepriceThenDepreciate,
                "simultaneous"            => ShockOrdering::Simultaneous,
                _                         => ShockOrdering::DepreciateThenReprice,
            },
            marco_polo_enabled:  false, // session-only, not persisted in JSON
            rebalance_enabled:   bool_val("rebalance_enabled", false),
            rebalance_frequency: {
                let freq = sets["rebalance_frequency_months"].as_u64().unwrap_or(12) as u32;
                match freq {
                    1  => "Monthly",
                    3  => "Quarterly",
                    6  => "Semi-Annual",
                    _  => "Annual",
                }.to_string()
            },
            prefecture: str_val("prefecture", ""),
            city:       str_val("city",       ""),
            fers_jurisdiction:            parse_protocol("fers_jurisdiction"),
            fers_japan_local_tax_exempt:  bool_val("fers_japan_local_tax_exempt", false),
            ss_jurisdiction:              parse_protocol("ss_jurisdiction"),
            nenkin_jurisdiction:  parse_protocol("nenkin_jurisdiction"),
            military_monthly_usd,
            military_jurisdiction,
            va_smc_variant: str_val("va_smc_variant", ""),
            accounts,
            rsu_awards,
            nhi_calculated_mode,
            nhi_medical_rate,
            nhi_support_rate,
            nhi_nursing_rate,
            nhi_per_capita_medical,
            nhi_per_capita_support,
            nhi_per_capita_nursing,
            nhi_cap_medical,
            nhi_cap_support,
            nhi_cap_nursing,
            nhi_include_us_income,
            nhi_spike_total_jpy,
            nhi_ongoing_total_jpy,
            kaigo_hoken_enabled: bool_val("kaigo_hoken_enabled", true),
            kaigo_care_scenario: sets["kaigo_care_scenario"].as_str().unwrap_or("none").to_string(),
            va_override_enabled,
            va_override_monthly,
            smc_override_enabled,
            smc_override_monthly,
            education_fund_enabled: sets["edu_savings_jpy_monthly"].as_f64().unwrap_or(0.0) > 0.0,
            edu_savings_jpy_monthly: num_str("edu_savings_jpy_monthly", "0"),
            gift_sink_enabled: sets["gift_recipient_count"].as_u64().unwrap_or(0) > 0
                            && sets["annual_gift_jpy_per_recipient"].as_f64().unwrap_or(0.0) > 0.0,
            annual_gift_jpy_per_recipient: num_str("annual_gift_jpy_per_recipient", "1100000"),
            gift_recipient_count:          num_str("gift_recipient_count",          "0"),
            us_gift_exclusion_usd:         num_str("us_gift_exclusion_usd",         "19000"),
            track_pfic_basis_drift: sets["track_pfic_basis_drift"].as_bool().unwrap_or(true),
            enable_estate_planning: sets["enable_estate_planning"].as_bool().unwrap_or(false),
            death_date:             str_val("death_date", ""),
            spouse_death_date:      str_val("spouse_death_date", ""),
            estate_heirs: if let Value::Array(arr) = &sets["heirs"] {
                arr.iter().filter_map(|item| {
                    if !item.is_object() { return None; }
                    let rel = match item["relationship"].as_str().unwrap_or("child") {
                        "spouse" | "Spouse" => HeirRelationship::Spouse,
                        "other"  | "Other"  => HeirRelationship::Other,
                        _                   => HeirRelationship::Child,
                    };
                    Some(HeirEntry {
                        name:       item["name"].as_str().unwrap_or("").to_string(),
                        birth_date: item["birth_date"].as_str().unwrap_or("").to_string(),
                        relationship: rel,
                    })
                }).collect()
            } else { vec![] },
            enable_gifting_optimiser: sets["enable_gifting_optimiser"].as_bool().unwrap_or(false),
            source_json: Some(json.clone()),
            source_path: Some(path.to_string()),
            reload_path: None,
        }
    }

    /// Returns a set of field names that fail validation.
    pub fn validation_errors(&self) -> HashSet<&'static str> {
        let mut bad: HashSet<&'static str> = HashSet::new();

        let bad_date = |s: &str| s.is_empty() || chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_err();
        let bad_f64  = |s: &str| s.is_empty() || s.parse::<f64>().is_err();
        let bad_u32  = |s: &str| s.is_empty() || s.parse::<u32>().is_err();
        let bad_pos  = |s: &str| s.is_empty() || s.parse::<f64>().map(|v| v <= 0.0).unwrap_or(true);
        let is_na    = |s: &str| { let t = s.trim().to_ascii_lowercase(); t == "n/a" || t == "na" || t == "disabled" || t == "none" };
        let bad_optional_f64 = |s: &str| { if is_na(s) { return false; } s.is_empty() || s.trim().parse::<f64>().is_err() };
        let opt_value = |s: &str| -> f64 { if is_na(s) { 0.0 } else { s.trim().parse::<f64>().unwrap_or(0.0) } };

        if bad_date(&self.start_date)      { bad.insert("start_date"); }
        if bad_date(&self.end_date)        { bad.insert("end_date"); }
        if bad_date(&self.retirement_date) { bad.insert("retirement_date"); }
        if bad_date(&self.rebalance_date)  { bad.insert("rebalance_date"); }

        if bad_f64(&self.inflation_us)    { bad.insert("inflation_us"); }
        if bad_f64(&self.inflation_japan) { bad.insert("inflation_japan"); }

        if bad_pos(&self.base_expense_jpy) { bad.insert("base_expense_jpy"); }
        if bad_pos(&self.min_expense_jpy)  { bad.insert("min_expense_jpy"); }
        if bad_f64(&self.nhi_first_year_monthly_jpy) { bad.insert("nhi_first_year_monthly_jpy"); }

        if bad_optional_f64(&self.fers_monthly_usd) { bad.insert("fers_monthly_usd"); }
        if opt_value(&self.fers_monthly_usd) > 0.0 {
            if bad_u32(&self.fers_start_age) { bad.insert("fers_start_age"); }
        }

        if let Ok(r) = self.va_disability_rating.parse::<u32>() {
            if r != 0 && r % 10 != 0 { bad.insert("va_disability_rating"); }
            if r > 100               { bad.insert("va_disability_rating"); }
        } else {
            bad.insert("va_disability_rating");
        }

        if bad_optional_f64(&self.ss_monthly_usd) { bad.insert("ss_monthly_usd"); }
        if opt_value(&self.ss_monthly_usd) > 0.0 {
            if bad_u32(&self.ss_start_age) { bad.insert("ss_start_age"); }
        }

        if bad_optional_f64(&self.ssdi_monthly_usd) { bad.insert("ssdi_monthly_usd"); }

        if bad_optional_f64(&self.nenkin_income_monthly_jpy) { bad.insert("nenkin_income_monthly_jpy"); }
        if opt_value(&self.nenkin_income_monthly_jpy) > 0.0 {
            if bad_u32(&self.nenkin_income_start_age) { bad.insert("nenkin_income_start_age"); }
        }

        if bad_u32(&self.bridge_months)        { bad.insert("bridge_months"); }
        if bad_f64(&self.war_chest_target_jpy) { bad.insert("war_chest_target_jpy"); }

        bad
    }

    /// Merge the current field values back into source_json for saving.
    fn build_save_json(&self) -> Option<Value> {
        let mut json = self.source_json.clone().unwrap_or_else(|| serde_json::json!({
            "simulation_settings": {},
            "holdings": {},
            "rsu_awards": [],
            "market_prices_usd": {},
            "growth_rates_annual": {}
        }));

        // Ensure required top-level sections exist
        for key in &["simulation_settings", "holdings", "market_prices_usd", "growth_rates_annual"] {
            if json.get(key).is_none() {
                json[key] = Value::Object(serde_json::Map::new());
            }
        }

        // ── Patch simulation_settings ─────────────────────────────────────────
        {
            let settings = json.get_mut("simulation_settings")?.as_object_mut()?;

            macro_rules! set_str { ($k:expr, $v:expr) => {
                settings.insert($k.into(), Value::String($v.clone()));
            };}
            macro_rules! set_f64 { ($k:expr, $v:expr) => {
                if let Ok(f) = $v.parse::<f64>() {
                    if let Some(n) = serde_json::Number::from_f64(f) {
                        settings.insert($k.into(), Value::Number(n));
                    }
                }
            };}
            macro_rules! set_f64_or_na { ($k:expr, $v:expr) => {
                let t = $v.trim().to_ascii_lowercase();
                let normalized = if t == "n/a" || t == "na" || t == "disabled" || t == "none" { "0" } else { $v.trim() };
                if let Ok(f) = normalized.parse::<f64>() {
                    if let Some(n) = serde_json::Number::from_f64(f) {
                        settings.insert($k.into(), Value::Number(n));
                    }
                }
            };}
            macro_rules! set_u64 { ($k:expr, $v:expr) => {
                if let Ok(i) = $v.parse::<u64>() {
                    settings.insert($k.into(), Value::Number(i.into()));
                }
            };}
            macro_rules! set_bool { ($k:expr, $v:expr) => {
                settings.insert($k.into(), Value::Bool($v));
            };}

            set_str!("start_date",      self.start_date);
            set_str!("end_date",        self.end_date);
            set_str!("retirement_date", self.retirement_date);
            set_str!("rebalance_date",  self.rebalance_date);
            set_f64!("usd_jpy_rate",         self.usd_jpy_rate);
            set_f64!("inflation_us_cpi",     self.inflation_us);
            set_f64!("inflation_japan_cpi",  self.inflation_japan);
            set_f64!("base_monthly_expenses_jpy", self.base_expense_jpy);
            set_f64!("min_monthly_expenses_jpy",  self.min_expense_jpy);
            set_f64!("nhi_spike_monthly_jpy", self.nhi_first_year_monthly_jpy);
            set_str!("rsu_tax_handling", self.rsu_tax_handling);
            set_bool!("rsu_sell_to_cover_realism", self.rsu_sell_to_cover_realism);
            set_bool!("track_pfic_basis_drift", self.track_pfic_basis_drift);
            set_bool!("monthly_dependent_precision", self.monthly_dependent_precision);
            set_f64_or_na!("fers_monthly_payment_usd", self.fers_monthly_usd);
            set_u64!("fers_start_age",   self.fers_start_age);
            set_bool!("war_chest_enabled",        self.war_chest_enabled);
            set_f64!("war_chest_target_jpy",      self.war_chest_target_jpy);
            set_f64!("pre_funded_war_chest_jpy",  self.pre_funded_war_chest_jpy);
            set_bool!("bridge_fund_enabled",      self.bridge_fund_enabled);
            set_f64!("bridge_fund_months_target", self.bridge_months);
            set_f64!("pre_funded_bridge_usd",     self.pre_funded_bridge_usd);
            set_bool!("simulate_yen_strengthening",       self.fx_drift_enabled);
            set_f64!("fx_drift_rate_annual",              self.fx_drift_rate);
            set_bool!("simulate_recession_at_retirement", self.recession_enabled);
            set_f64!("recession_severity_pct",            self.recession_severity);
            settings.insert("shock_ordering".into(), Value::String(
                match self.shock_ordering {
                    ShockOrdering::DepreciateThenReprice => "depreciate_then_reprice",
                    ShockOrdering::RepriceThenDepreciate => "reprice_then_depreciate",
                    ShockOrdering::Simultaneous          => "simultaneous",
                }.into()
            ));
            set_str!("us_filing_status", self.filing_status);
            set_str!("us_state_code",    self.us_state_code);

            settings.insert("us_tax_strategy".into(), Value::String(
                match self.us_tax_strategy { UsTaxStrategy::FtcOnly => "ftc_only", UsTaxStrategy::FeieAndFtc => "feie_and_ftc" }.into()
            ));

            set_u64!("va_disability_rating", self.va_disability_rating);
            settings.insert("va_dependent_status".into(), Value::String(
                match self.va_dependent_status {
                    VaDependentStatus::VetOnly            => "vet_only",
                    VaDependentStatus::WithSpouse         => "with_spouse",
                    VaDependentStatus::WithSpouseAndChild => "with_spouse_and_child",
                }.into()
            ));

            set_f64_or_na!("ss_monthly_usd", self.ss_monthly_usd);
            set_u64!("ss_start_age",         self.ss_start_age);
            set_f64_or_na!("ssdi_monthly_usd", self.ssdi_monthly_usd);
            // Serialize dependents array (V6.6: full birth_date + derived birth_year)
            {
                let deps: Vec<Value> = self.dependents.iter().filter_map(|d| {
                    let date = d.birth_date.trim();
                    if date.is_empty() { return None; }
                    let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
                    let mut obj = serde_json::Map::new();
                    obj.insert("birth_date".into(), Value::String(date.to_string()));
                    obj.insert("birth_year".into(), Value::Number((parsed.year() as i64).into()));
                    obj.insert("is_college_student".into(), Value::Bool(d.is_college_student));
                    Some(Value::Object(obj))
                }).collect();
                settings.insert("dependents".into(), Value::Array(deps));
            }
            // V6.6: family demographics (no jurisdiction enums here — see post-proto_str block)
            settings.insert("birth_date".into(), Value::String(self.user_birth_date.clone()));
            settings.insert("is_married".into(), Value::Bool(self.is_married));
            if self.is_married && !self.spouse_birth_date.is_empty() {
                settings.insert("spouse_birth_date".into(), Value::String(self.spouse_birth_date.clone()));
            } else {
                settings.remove("spouse_birth_date");
            }
            // Stage 02: persist spouse profile and Japan income fields.
            if self.is_married {
                settings.insert("spouse_profile".into(), Value::String(
                    match self.spouse_profile {
                        SpouseProfile::UsPerson =>
                            "us_person".into(),
                        SpouseProfile::NraElectedToBeTreatedAsResident =>
                            "nra_elected_to_be_treated_as_resident".into(),
                        SpouseProfile::NraMfs =>
                            "nra_mfs".into(),
                        SpouseProfile::NraHeadOfHouseholdEligible =>
                            "nra_head_of_household_eligible".into(),
                    }
                ));
                if self.spouse_profile == SpouseProfile::NraElectedToBeTreatedAsResident {
                    set_f64!("spouse_japan_salary_jpy",      self.spouse_japan_salary_jpy);
                    set_f64!("spouse_japan_misc_income_jpy", self.spouse_japan_misc_income_jpy);
                } else {
                    settings.remove("spouse_japan_salary_jpy");
                    settings.remove("spouse_japan_misc_income_jpy");
                }
            } else {
                settings.remove("spouse_profile");
                settings.remove("spouse_japan_salary_jpy");
                settings.remove("spouse_japan_misc_income_jpy");
            }
            if self.is_married && self.spouse_ss_enabled {
                set_f64_or_na!("spouse_ss_monthly_usd", self.spouse_ss_monthly_usd);
                set_u64!("spouse_ss_start_age",         self.spouse_ss_start_age);
            } else {
                settings.remove("spouse_ss_monthly_usd");
                settings.remove("spouse_ss_start_age");
                settings.remove("spouse_ss_jurisdiction");
            }
            if self.is_married && self.spouse_nenkin_enabled {
                set_f64_or_na!("spouse_nenkin_monthly_jpy", self.spouse_nenkin_monthly_jpy);
                set_u64!("spouse_nenkin_start_age",         self.spouse_nenkin_start_age);
            } else {
                settings.remove("spouse_nenkin_monthly_jpy");
                settings.remove("spouse_nenkin_start_age");
                settings.remove("spouse_nenkin_jurisdiction");
            }
            // V6.6: FX drift cadence / amount
            set_u64!("fx_drift_cadence_months",        self.fx_drift_cadence_months);
            set_f64!("fx_drift_increase_amount_jpy",   self.fx_drift_increase_amount_jpy);
            set_f64_or_na!("nenkin_income_monthly_jpy", self.nenkin_income_monthly_jpy);
            set_u64!("nenkin_income_start_age",         self.nenkin_income_start_age);

            settings.insert("prefecture".into(), Value::String(self.prefecture.clone()));
            settings.insert("city".into(),       Value::String(self.city.clone()));

            let state_rate = state_tax_rate(&self.us_state_code);
            if let Some(n) = serde_json::Number::from_f64(state_rate) {
                settings.insert("us_state_tax_rate".into(), Value::Number(n));
            }

            let proto_str = |p: TaxProtocol| -> &'static str {
                match p {
                    TaxProtocol::Both      => "both",
                    TaxProtocol::UsOnly    => "us_only",
                    TaxProtocol::JapanOnly => "japan_only",
                    TaxProtocol::TaxFree   => "tax_free",
                }
            };
            settings.insert("fers_jurisdiction".into(),              Value::String(proto_str(self.fers_jurisdiction).into()));
            settings.insert("fers_japan_local_tax_exempt".into(),    Value::Bool(self.fers_japan_local_tax_exempt));
            settings.insert("ss_jurisdiction".into(),                Value::String(proto_str(self.ss_jurisdiction).into()));
            settings.insert("nenkin_jurisdiction".into(), Value::String(proto_str(self.nenkin_jurisdiction).into()));
            // V6.6: spouse jurisdictions — only emitted when the entitlement is on.
            if self.is_married && self.spouse_ss_enabled {
                settings.insert("spouse_ss_jurisdiction".into(),
                    Value::String(proto_str(self.spouse_ss_jurisdiction).into()));
            }
            if self.is_married && self.spouse_nenkin_enabled {
                settings.insert("spouse_nenkin_jurisdiction".into(),
                    Value::String(proto_str(self.spouse_nenkin_jurisdiction).into()));
            }
            settings.insert("va_smc_variant".into(),      Value::String(self.va_smc_variant.clone()));

            let tj = match self.tax_jurisdiction {
                TaxJurisdiction::Both      => "both",
                TaxJurisdiction::UsOnly    => "us_only",
                TaxJurisdiction::JapanOnly => "japan_only",
                TaxJurisdiction::TaxFree   => "both",
            };
            settings.insert("tax_jurisdiction".into(), Value::String(tj.into()));

            // Military Retired Pay
            let mil_monthly: f64 = self.military_monthly_usd.trim().parse().unwrap_or(0.0);
            let mut mil = serde_json::Map::new();
            if let Some(n) = serde_json::Number::from_f64(mil_monthly) {
                mil.insert("monthly_usd".into(), Value::Number(n));
            }
            mil.insert("jurisdiction".into(), Value::String(proto_str(self.military_jurisdiction).into()));
            settings.insert("military_retired".into(), Value::Object(mil));

            // Entitlement overrides
            if self.va_override_enabled {
                if let Ok(v) = self.va_override_monthly.trim().parse::<f64>() {
                    if let Some(n) = serde_json::Number::from_f64(v) {
                        settings.insert("va_monthly_override".into(), Value::Number(n));
                    }
                }
            } else {
                settings.remove("va_monthly_override");
            }
            if self.smc_override_enabled {
                if let Ok(v) = self.smc_override_monthly.trim().parse::<f64>() {
                    if let Some(n) = serde_json::Number::from_f64(v) {
                        settings.insert("smc_monthly_override".into(), Value::Number(n));
                    }
                }
            } else {
                settings.remove("smc_monthly_override");
            }

            // ── Family Financial Planning (V7.3 Education / V7.5 Gift Sink) ──
            // Toggled OFF → write 0 so the simulator's optional channels stay dormant.
            if self.education_fund_enabled {
                set_f64!("edu_savings_jpy_monthly", self.edu_savings_jpy_monthly);
            } else {
                settings.insert("edu_savings_jpy_monthly".into(),
                                Value::Number(serde_json::Number::from(0)));
            }
            if self.gift_sink_enabled {
                set_f64!("annual_gift_jpy_per_recipient", self.annual_gift_jpy_per_recipient);
                set_u64!("gift_recipient_count",          self.gift_recipient_count);
                set_f64!("us_gift_exclusion_usd",         self.us_gift_exclusion_usd);
            } else {
                settings.insert("annual_gift_jpy_per_recipient".into(),
                                Value::Number(serde_json::Number::from(0)));
                settings.insert("gift_recipient_count".into(),
                                Value::Number(serde_json::Number::from(0)));
            }

            // ── Stage 07: Estate Planning ─────────────────────────────────────
            set_bool!("enable_estate_planning", self.enable_estate_planning);
            set_bool!("enable_gifting_optimiser", self.enable_gifting_optimiser);
            if self.enable_estate_planning {
                if !self.death_date.is_empty() {
                    settings.insert("death_date".into(), Value::String(self.death_date.clone()));
                } else {
                    settings.remove("death_date");
                }
                if !self.spouse_death_date.is_empty() {
                    settings.insert("spouse_death_date".into(), Value::String(self.spouse_death_date.clone()));
                } else {
                    settings.remove("spouse_death_date");
                }
                let heirs_arr: Vec<Value> = self.estate_heirs.iter().map(|h| {
                    let mut obj = serde_json::Map::new();
                    obj.insert("name".into(), Value::String(h.name.clone()));
                    if !h.birth_date.is_empty() {
                        obj.insert("birth_date".into(), Value::String(h.birth_date.clone()));
                    }
                    obj.insert("relationship".into(), Value::String(
                        match h.relationship {
                            HeirRelationship::Spouse => "spouse",
                            HeirRelationship::Child  => "child",
                            HeirRelationship::Other  => "other",
                        }.into()
                    ));
                    Value::Object(obj)
                }).collect();
                settings.insert("heirs".into(), Value::Array(heirs_arr));
            } else {
                settings.remove("death_date");
                settings.remove("spouse_death_date");
                settings.remove("heirs");
            }

            // Recession events
            let recessions: Vec<Value> = self.recession_years.split(',')
                .filter_map(|entry| {
                    let entry = entry.trim();
                    if entry.is_empty() { return None; }
                    let parts: Vec<&str> = entry.split(':').collect();
                    let year = parts.first()?.trim().parse::<i64>().ok()?;
                    let sev  = parts.get(1).and_then(|s| s.trim().parse::<f64>().ok()).unwrap_or(0.20);
                    let mut obj = serde_json::Map::new();
                    obj.insert("year".into(), Value::Number(year.into()));
                    if let Some(n) = serde_json::Number::from_f64(sev) {
                        obj.insert("severity".into(), Value::Number(n));
                    }
                    Some(Value::Object(obj))
                })
                .collect();
            settings.insert("simulated_recessions".into(), Value::Array(recessions));

            // DC Plan settings from the DC account (if present)
            if let Some(dc) = self.accounts.iter().find(|a| a.account_type == "DC Plan") {
                if let Ok(f) = dc.dc_monthly_jpy.trim().parse::<f64>() {
                    if let Some(n) = serde_json::Number::from_f64(f) {
                        settings.insert("japan_dc_monthly_contribution_jpy".into(), Value::Number(n));
                    }
                }
                let growth_rate: f64 = if dc.dc_use_market_avg {
                    0.10
                } else {
                    dc.dc_growth_rate.trim().parse::<f64>().unwrap_or(8.0) / 100.0
                };
                if let Some(n) = serde_json::Number::from_f64(growth_rate) {
                    settings.insert("dc_growth_rate".into(), Value::Number(n));
                }
                if !dc.dc_funds.is_empty() {
                    let funds_arr: Vec<Value> = dc.dc_funds.iter().map(|f| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("fund_name".into(), Value::String(f.fund_name.clone()));
                        if !f.ticker.trim().is_empty() {
                            obj.insert("ticker".into(), Value::String(f.ticker.trim().to_string()));
                        }
                        if let Ok(u) = f.units.trim().parse::<f64>() {
                            if let Some(n) = serde_json::Number::from_f64(u) {
                                obj.insert("units".into(), Value::Number(n));
                            }
                        }
                        if let Ok(p) = f.price_per_10k.trim().parse::<f64>() {
                            if let Some(n) = serde_json::Number::from_f64(p) {
                                obj.insert("price_per_10k_jpy".into(), Value::Number(n));
                            }
                        }
                        if let Ok(a) = f.contrib_alloc_pct.trim().parse::<f64>() {
                            if let Some(n) = serde_json::Number::from_f64(a) {
                                obj.insert("contrib_alloc_pct".into(), Value::Number(n));
                            }
                        }
                        if let Ok(g) = f.growth_pct.trim().parse::<f64>() {
                            if let Some(n) = serde_json::Number::from_f64(g) {
                                obj.insert("growth_rate_pct".into(), Value::Number(n));
                            }
                        }
                        obj.insert("stop_at_retirement".into(), Value::Bool(f.stop_at_retirement));
                        Value::Object(obj)
                    }).collect();
                    settings.insert("dc_funds".into(), Value::Array(funds_arr));
                }
            }

            // NHI model
            {
                let mut nhi = serde_json::Map::new();
                if self.nhi_calculated_mode {
                    nhi.insert("mode".into(), Value::String("calculated".into()));
                    let pct_to_f64 = |s: &str| s.trim().parse::<f64>().ok().map(|f| f / 100.0);
                    let jpy_to_f64 = |s: &str| s.trim().parse::<f64>().ok();
                    if let Some(v) = pct_to_f64(&self.nhi_medical_rate) {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("medical_rate".into(), Value::Number(n)); }
                    }
                    if let Some(v) = pct_to_f64(&self.nhi_support_rate) {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("elderly_support_rate".into(), Value::Number(n)); }
                    }
                    if let Some(v) = pct_to_f64(&self.nhi_nursing_rate) {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("nursing_care_rate".into(), Value::Number(n)); }
                    }
                    if let Some(v) = jpy_to_f64(&self.nhi_per_capita_medical) {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("per_capita_medical".into(), Value::Number(n)); }
                    }
                    if let Some(v) = jpy_to_f64(&self.nhi_per_capita_support) {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("per_capita_support".into(), Value::Number(n)); }
                    }
                    if let Some(v) = jpy_to_f64(&self.nhi_per_capita_nursing) {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("per_capita_nursing".into(), Value::Number(n)); }
                    }
                    if let Some(v) = jpy_to_f64(&self.nhi_cap_medical) {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("cap_medical".into(), Value::Number(n)); }
                    }
                    if let Some(v) = jpy_to_f64(&self.nhi_cap_support) {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("cap_support".into(), Value::Number(n)); }
                    }
                    if let Some(v) = jpy_to_f64(&self.nhi_cap_nursing) {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("cap_nursing".into(), Value::Number(n)); }
                    }
                    nhi.insert("include_us_investment_income".into(), Value::Bool(self.nhi_include_us_income));
                } else {
                    nhi.insert("mode".into(), Value::String("manual_override".into()));
                    if let Ok(v) = self.nhi_spike_total_jpy.trim().parse::<f64>() {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("spike_year_total_jpy".into(), Value::Number(n)); }
                    }
                    if let Ok(v) = self.nhi_ongoing_total_jpy.trim().parse::<f64>() {
                        if let Some(n) = serde_json::Number::from_f64(v) { nhi.insert("ongoing_annual_total_jpy".into(), Value::Number(n)); }
                    }
                }
                settings.insert("nhi_model".into(), Value::Object(nhi));
            }

            // Primary account type (backward compat)
            let primary_type = self.accounts.first().map(|a| a.account_type.as_str()).unwrap_or("Taxable Brokerage");
            settings.insert("account_type".into(), Value::String(primary_type.into()));

            // Legacy investment_location derived from accounts
            let il = if self.accounts.iter().any(|a| matches!(a.account_type.as_str(), "NISA" | "iDeCo" | "DC Plan")) {
                "japan"
            } else {
                "us"
            };
            settings.insert("investment_location".into(), Value::String(il.into()));

            // Per-account tax jurisdiction
            for account in &self.accounts {
                let key = account_json_key(&account.account_type);
                let jk  = format!("{}_tax_jurisdiction", key);
                settings.insert(jk, Value::String(proto_str(account.tax_jurisdiction).into()));
            }
        }

        // ── Patch holdings from each account's positions ───────────────────────
        {
            let mut market_prices_out:  Vec<(String, f64)> = Vec::new();
            let mut growth_rates_out:   Vec<(String, f64)> = Vec::new();
            let mut volatility_pcts_out: Vec<(String, f64)> = Vec::new();

            for account in &self.accounts {
                if account.account_type == "DC Plan" { continue; } // DC uses a different schema

                let key = account_json_key(&account.account_type);
                let mut holdings_map = serde_json::Map::new();

                for row in &account.positions {
                    if row.ticker.is_empty() { continue; }
                    let units: f64 = row.units.trim().parse().unwrap_or(0.0);
                    if units <= 0.0 { continue; }

                    let mut pos = serde_json::Map::new();
                    if let Some(n) = serde_json::Number::from_f64(units) {
                        pos.insert("qty".into(), Value::Number(n));
                    }
                    if let Ok(cb) = row.cost_basis.trim().parse::<f64>() {
                        if cb > 0.0 {
                            if let Some(n) = serde_json::Number::from_f64(cb) {
                                pos.insert("avg_cost".into(), Value::Number(n));
                            }
                        }
                    }
                    pos.insert("drip_enabled".into(), Value::Bool(row.drip_enabled));
                    if !row.drip_reinvest_ticker.is_empty() {
                        pos.insert("dividend_reinvest_target".into(),
                            Value::String(row.drip_reinvest_ticker.clone()));
                    }

                    // ── V7.6 — Asset class + (optional) detailed return profile.
                    // DC Plan uses a separate ¥/万口 fund table and keeps the legacy
                    // flat-growth model. Every other account (incl. iDeCo and 401(k))
                    // can carry per-position asset class and detailed return profile.
                    let class_is_container = account.account_type == "DC Plan";
                    if !class_is_container {
                        let class_snake = match row.asset_class.as_str() {
                            "ETF"        => "etf",
                            "MutualFund" => "mutual_fund",
                            "Other"      => "other",
                            _            => "stock",
                        };
                        pos.insert("asset_class".into(), Value::String(class_snake.into()));

                        // Stage 05 — persist PFIC regime (omit key when not a PFIC).
                        if row.pfic_regime != "NotPfic" {
                            let regime_snake = match row.pfic_regime.as_str() {
                                "Mtm"               => "mtm",
                                "Qef"               => "qef",
                                "ExcessDistribution" => "excess_distribution",
                                _                   => "not_pfic",
                            };
                            pos.insert("pfic_regime".into(), Value::String(regime_snake.into()));
                        }

                        if row.use_detailed_profile {
                            let mut prof = serde_json::Map::new();
                            let put = |m: &mut serde_json::Map<String, Value>, k: &str, s: &str| {
                                if let Ok(v) = s.trim().parse::<f64>() {
                                    if let Some(n) = serde_json::Number::from_f64(v / 100.0) {
                                        m.insert(k.into(), Value::Number(n));
                                    }
                                }
                            };
                            put(&mut prof, "cap_growth",     &row.cap_growth_pct);
                            put(&mut prof, "nav_growth",     &row.nav_growth_pct);
                            put(&mut prof, "dividend_yield", &row.dividend_yield_pct);
                            put(&mut prof, "interest_yield", &row.interest_yield_pct);
                            put(&mut prof, "cap_gains_dist", &row.cap_gains_dist_pct);
                            put(&mut prof, "special_dist",   &row.special_dist_pct);
                            put(&mut prof, "roc",            &row.roc_pct);
                            put(&mut prof, "expense_ratio",  &row.expense_ratio_pct);
                            pos.insert("return_profile".into(), Value::Object(prof));
                        }
                    }
                    holdings_map.insert(row.ticker.clone(), Value::Object(pos));

                    if let Ok(uv) = row.unit_value.trim().parse::<f64>() {
                        if uv > 0.0 { market_prices_out.push((row.ticker.clone(), uv)); }
                    }
                    if let Ok(gp) = row.growth_pct.trim().parse::<f64>() {
                        if gp > 0.0 { growth_rates_out.push((row.ticker.clone(), gp / 100.0)); }
                    }
                    if let Ok(vp) = row.volatility_pct.trim().parse::<f64>() {
                        if vp > 0.0 { volatility_pcts_out.push((row.ticker.clone(), vp / 100.0)); }
                    }
                }

                if let Some(h) = json.get_mut("holdings").and_then(|h| h.as_object_mut()) {
                    h.insert(key.to_string(), Value::Object(holdings_map));
                }
            }

            if let Some(mp) = json.get_mut("market_prices_usd").and_then(|h| h.as_object_mut()) {
                for (ticker, price) in market_prices_out {
                    if let Some(n) = serde_json::Number::from_f64(price) {
                        mp.insert(ticker, Value::Number(n));
                    }
                }
            }
            if let Some(gr) = json.get_mut("growth_rates_annual").and_then(|h| h.as_object_mut()) {
                for (ticker, rate) in growth_rates_out {
                    if let Some(n) = serde_json::Number::from_f64(rate) {
                        gr.insert(ticker, Value::Number(n));
                    }
                }
            }
            // Persist per-ticker volatility assumptions for Marco Polo mode.
            if !volatility_pcts_out.is_empty() {
                let mut vmap = serde_json::Map::new();
                for (ticker, vol) in volatility_pcts_out {
                    if let Some(n) = serde_json::Number::from_f64(vol) {
                        vmap.insert(ticker, Value::Number(n));
                    }
                }
                json["volatility_pcts"] = Value::Object(vmap);
            }
        }

        // ── Write active management settings (V6.0) ───────────────────────────
        {
            let settings = json.get_mut("simulation_settings")?.as_object_mut()?;
            settings.insert("rebalance_enabled".into(), Value::Bool(self.rebalance_enabled));
            let freq_months: u64 = match self.rebalance_frequency.as_str() {
                "Monthly"    => 1,
                "Quarterly"  => 3,
                "Semi-Annual"=> 6,
                _            => 12,
            };
            settings.insert("rebalance_frequency_months".into(), Value::Number(freq_months.into()));

            // target_allocations: collect from all positions, normalise to fraction
            let mut alloc_map = serde_json::Map::new();
            let mut accum_arr: Vec<Value> = Vec::new();

            for account in &self.accounts {
                if account.account_type == "DC Plan" { continue; }
                let sim_key = account_sim_key(&account.account_type);
                for pos in &account.positions {
                    if pos.ticker.is_empty() { continue; }
                    // target allocation
                    if let Ok(pct) = pos.target_alloc_pct.trim().parse::<f64>() {
                        if pct > 0.0 {
                            if let Some(n) = serde_json::Number::from_f64(pct / 100.0) {
                                alloc_map.insert(pos.ticker.clone(), Value::Number(n));
                            }
                        }
                    }
                    // accumulation rule
                    if let Ok(amt) = pos.accum_amount.trim().parse::<f64>() {
                        if amt > 0.0 {
                            let freq_mo: u64 = match pos.accum_frequency.as_str() {
                                "Quarterly" => 3,
                                "Annual"    => 12,
                                _           => 1,
                            };
                            let mut obj = serde_json::Map::new();
                            obj.insert("ticker".into(),   Value::String(pos.ticker.clone()));
                            obj.insert("account".into(),  Value::String(sim_key.to_string()));
                            if let Some(n) = serde_json::Number::from_f64(amt) {
                                obj.insert("monthly_amount".into(), Value::Number(n));
                            }
                            obj.insert("frequency_months".into(), Value::Number(freq_mo.into()));
                            obj.insert("stop_at_retirement".into(), Value::Bool(pos.stop_at_retirement));
                            obj.insert("drip_enabled".into(), Value::Bool(pos.drip_enabled));
                            if !pos.drip_reinvest_ticker.is_empty() {
                                obj.insert("drip_reinvest_ticker".into(),
                                    Value::String(pos.drip_reinvest_ticker.clone()));
                            }
                            accum_arr.push(Value::Object(obj));
                        }
                    }
                }
            }
            if !alloc_map.is_empty() {
                settings.insert("target_allocations".into(), Value::Object(alloc_map));
            }
            settings.insert("accumulation_rules".into(), Value::Array(accum_arr));
        }

        // ── Write RSU awards (full replacement) ───────────────────────────────
        {
            let rsu_arr: Vec<Value> = self.rsu_awards.iter()
                .filter(|r| !r.ticker.is_empty())
                .filter_map(|r| {
                    let units: f64 = r.units_awarded.trim().parse().ok()?;
                    if units <= 0.0 { return None; }
                    let mut obj = serde_json::Map::new();
                    obj.insert("ticker".into(), Value::String(r.ticker.clone()));
                    if !r.grant_date.is_empty() {
                        obj.insert("grant_date".into(), Value::String(r.grant_date.clone()));
                    }
                    if let Some(n) = serde_json::Number::from_f64(units) {
                        obj.insert("total_shares".into(), Value::Number(n));
                    }
                    // months_to_finish_vesting → vesting_years (rounded up) + explicit field.
                    if let Ok(months) = r.months_to_finish_vesting.trim().parse::<u64>() {
                        let years = (months + 11) / 12;
                        if let Some(n) = serde_json::Number::from_f64(years as f64) {
                            obj.insert("vesting_years".into(), Value::Number(n));
                        }
                        if let Some(n) = serde_json::Number::from_f64(months as f64) {
                            obj.insert("vesting_months_total".into(), Value::Number(n));
                        }
                    }
                    // specific_vesting_months → validated month list.
                    let month_vals: Vec<Value> = r.specific_vesting_months.split(',')
                        .filter_map(|s| s.trim().parse::<u64>().ok())
                        .filter(|&m| m >= 1 && m <= 12)
                        .map(|m| Value::Number(m.into()))
                        .collect();
                    if !month_vals.is_empty() {
                        obj.insert("vesting_months".into(), Value::Array(month_vals));
                    }
                    // Cliff vesting.
                    if r.delayed_initial_vest {
                        if let Ok(c) = r.cliff_vest_months.trim().parse::<u64>() {
                            obj.insert("cliff_vest_months".into(), Value::Number(c.into()));
                        }
                    }
                    // V7.7 — Per-ticker pricing.
                    if let Ok(uv) = r.unit_value.trim().parse::<f64>() {
                        if uv > 0.0 {
                            if let Some(n) = serde_json::Number::from_f64(uv) {
                                obj.insert("unit_value".into(), Value::Number(n));
                            }
                        }
                    }
                    if let Ok(gp) = r.growth_pct.trim().parse::<f64>() {
                        if gp.is_finite() {
                            if let Some(n) = serde_json::Number::from_f64(gp / 100.0) {
                                obj.insert("growth_rate".into(), Value::Number(n));
                            }
                        }
                    }
                    // V7.7 — Detailed return profile (cap_growth + dividend_yield only).
                    if r.use_detailed_profile {
                        let mut prof = serde_json::Map::new();
                        let put = |m: &mut serde_json::Map<String, Value>, k: &str, s: &str| {
                            if let Ok(v) = s.trim().parse::<f64>() {
                                if let Some(n) = serde_json::Number::from_f64(v / 100.0) {
                                    m.insert(k.into(), Value::Number(n));
                                }
                            }
                        };
                        put(&mut prof, "cap_growth",     &r.cap_growth_pct);
                        put(&mut prof, "dividend_yield", &r.dividend_yield_pct);
                        if !prof.is_empty() {
                            obj.insert("return_profile".into(), Value::Object(prof));
                        }
                    }
                    Some(Value::Object(obj))
                })
                .collect();
            json["rsu_awards"] = Value::Array(rsu_arr);
        }

        Some(json)
    }

    pub fn state_tax_rate(&self) -> f64 {
        state_tax_rate(&self.us_state_code)
    }
}

// ─── Public render function ───────────────────────────────────────────────────

pub fn show(ui: &mut Ui, state: &mut InputPanelState) {
    ui.heading("Input Configuration");
    ui.add_space(4.0);

    // ── V6.6 dividend-focus header ───────────────────────────────────────────
    Frame::none()
        .fill(Color32::from_rgba_unmultiplied(40, 90, 60, 60))
        .stroke(egui::Stroke::new(1.0, Color32::from_rgb(80, 160, 100)))
        .inner_margin(egui::Margin::same(8.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(
                    "This retirement calculator is heavily focused on living off the dividends \
                     in a portfolio before selling stock — the engine prioritizes dividend \
                     coverage of expenses, then bridge cash, then war chest, and only liquidates \
                     equity as a last resort."
                )
                .color(Color32::from_rgb(180, 230, 200))
                .strong(),
            );
        });
    ui.add_space(6.0);

    // Source file path / blank-slate banner
    if let Some(path) = &state.source_path {
        ui.label(RichText::new(format!("📄 {}", path)).small().color(Color32::GRAY));
    } else {
        ui.label(RichText::new("✏️ New Scenario (unsaved) — fill in the fields below, then Save.")
            .color(Color32::from_rgb(180, 180, 100)));
    }

    ui.add_space(8.0);

    let status_id = egui::Id::new(SAVE_STATUS_ID);
    let errors    = state.validation_errors();
    let error_count = errors.len();
    let has_source  = state.source_json.is_some();

    let mut should_reset = false;

    ui.horizontal(|ui| {
        if ui.add_enabled(has_source, egui::Button::new("💾 Save Configuration")).clicked() {
            let result = save_configuration(state);
            let msg = match result {
                Some(Ok(path)) => {
                    let display = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.to_string_lossy().to_string());
                    state.reload_path = Some(path.to_string_lossy().to_string());
                    SaveStatus { message: format!("✅ Saved → {}", display), is_success: true, when: Instant::now() }
                }
                Some(Err(e)) => SaveStatus { message: format!("❌ Save failed: {}", e), is_success: false, when: Instant::now() },
                None         => return,
            };
            ui.ctx().data_mut(|d| d.insert_temp(status_id, msg));
        }

        ui.add_space(8.0);

        if ui.button("📂 Reload Scenario").clicked() {
            if let Some(path) = rfd::FileDialog::new()
                .set_title("Open Scenario")
                .add_filter("JSON Scenario", &["json"])
                .pick_file()
            {
                state.reload_path = Some(path.to_string_lossy().to_string());
            }
        }

        ui.add_space(8.0);

        if ui.button("🗑 New Scenario").on_hover_text("Reset all fields to a blank slate").clicked() {
            should_reset = true;
        }
    });

    if should_reset {
        *state = InputPanelState::default();
        return;
    }

    if let Some(status) = ui.ctx().data(|d| d.get_temp::<SaveStatus>(status_id)) {
        if status.when.elapsed() < SAVE_STATUS_TTL {
            let color = if status.is_success { Color32::GREEN } else { Color32::RED };
            ui.label(RichText::new(&status.message).color(color));
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
    }

    if !has_source { return; }

    if error_count > 0 {
        ui.add_space(4.0);
        ui.label(RichText::new(format!(
            "⛔ {} field(s) require correction before simulation can run (highlighted in red).",
            error_count
        )).color(Color32::from_rgb(255, 100, 100)).strong());
    }

    ui.add_space(4.0);
    ui.label(RichText::new(
        "Edit any field, then 'Save Configuration' to export a new JSON file. \
         Reload it via 'Reload Scenario', then click ▶ Run Simulation.",
    ).small().color(Color32::GRAY));
    ui.add_space(6.0);
    ui.separator();

    egui::ScrollArea::vertical()
        .id_salt("input_panel_scroll")
        .show(ui, |ui| {

        // ── Timing ───────────────────────────────────────────────────────────────
        section(ui, "Timing");
        grid(ui, "g_timing", |ui| {
            vfield_tt(ui, "Start Date",      &mut state.start_date,      "YYYY-MM-DD", !errors.contains("start_date"),
                "First simulated month. Defaults to today; back-date if you want pre-retirement years in the report.");
            vfield_tt(ui, "End Date",        &mut state.end_date,        "YYYY-MM-DD", !errors.contains("end_date"),
                "Last simulated month. Defaults to today + 50 years; long horizons surface FX drift and NHI cost compounding.");
            vfield_tt(ui, "Retirement Date", &mut state.retirement_date, "YYYY-MM-DD", !errors.contains("retirement_date"),
                "First month with no salary. Drives FERS/SS bridge math and Japan resident-tax spike year.");
            vfield_tt(ui, "Rebalance Date",  &mut state.rebalance_date,  "YYYY-MM-DD", !errors.contains("rebalance_date"),
                "Major portfolio rebalance event. Per-position overrides (V6.6) supersede this.");
        });

        // ── Economics ────────────────────────────────────────────────────────────
        section(ui, "Economics");
        grid(ui, "g_econ", |ui| {
            vfield_tt(ui, "USD/JPY Rate",          &mut state.usd_jpy_rate,   "0 = live fetch (primary JPY bridge)", true,
                "Spot USD→JPY rate. 0 triggers a live fetch on simulation start. All cross-currency conversions key off this.");
            vfield_tt(ui, "US Inflation (CPI)",    &mut state.inflation_us,   "e.g. 0.028", !errors.contains("inflation_us"),
                "Annual US CPI used for COLA and tax-bracket inflation. 0.028 ≈ 30-year mean.");
            vfield_tt(ui, "Japan Inflation (CPI)", &mut state.inflation_japan,"e.g. 0.028", !errors.contains("inflation_japan"),
                "Annual Japan CPI. Inflates base/min expenses, NHI per-capita amounts, and Nenkin pension floor.");
        });
        // V6.6: FX Drift now lives directly under USD/JPY for adjacency.
        ui.add_space(4.0);
        ui.checkbox(&mut state.fx_drift_enabled, "FX Drift (¥ trajectory after retirement)")
            .on_hover_text("When on, the FX rate evolves post-retirement. Use either the legacy continuous rate or the V6.6 cadence-based JPY step.");
        if state.fx_drift_enabled {
            grid(ui, "g_fx", |ui| {
                vfield_tt(ui, "FX Drift Rate (annual, legacy)", &mut state.fx_drift_rate, "e.g. 0.02", true,
                    "Continuous annual yen-strengthening rate. Ignored when Cadence Months > 0.");
                vfield_tt(ui, "Cadence Months",                  &mut state.fx_drift_cadence_months, "0 = disabled, e.g. 6", true,
                    "V6.6: every N months after retirement, FX jumps by Increase Amount JPY. 0 = use legacy rate above.");
                vfield_tt(ui, "Increase Amount (JPY)",           &mut state.fx_drift_increase_amount_jpy, "e.g. 5.0", true,
                    "Signed JPY step per cadence (positive = yen weakens, negative = yen strengthens).");
            });
        }

        // ── Expenses ─────────────────────────────────────────────────────────────
        section(ui, "Monthly Expenses (JPY)");
        grid(ui, "g_exp", |ui| {
            vfield_tt(ui, "Base Monthly",               &mut state.base_expense_jpy,          "e.g. 1000000", !errors.contains("base_expense_jpy"),
                "Base household burn rate before NHI / resident-tax. Inflated by Japan CPI each year.");
            vfield_tt(ui, "Minimum Monthly",            &mut state.min_expense_jpy,           "e.g. 600000",  !errors.contains("min_expense_jpy"),
                "Floor expenses if dividends/bridge run low. Drives forced-liquidation thresholds.");
            vfield_tt(ui, "NHI Spike Monthly (yr 1-2)", &mut state.nhi_first_year_monthly_jpy,"e.g. 73333",   !errors.contains("nhi_first_year_monthly_jpy"),
                "国民健康保険スパイク: first-post-retirement-year NHI premium based on prior-year salary. Manual override; the calculated NHI engine handles ongoing years.");
        });

        // ── NHI Settings ─────────────────────────────────────────────────────────
        section(ui, "NHI Settings (National Health Insurance)");
        ui.label(RichText::new(
            "Japan's NHI is assessed each June based on prior-year income, producing a \
             spike in the first post-retirement year. Use Automatic mode for a \
             municipality-specific calculation or Manual mode to enter known totals."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            if ui.radio(state.nhi_calculated_mode,  "Automatic (Municipal Rates)").clicked() {
                state.nhi_calculated_mode = true;
            }
            if ui.radio(!state.nhi_calculated_mode, "Manual (Fixed Amounts)").clicked() {
                state.nhi_calculated_mode = false;
            }
        });
        ui.add_space(6.0);

        if state.nhi_calculated_mode {
            if ui.small_button("Load Sagamihara 2026 Defaults").clicked() {
                let d = NhiCalculatedRates::sagamihara_2026();
                state.nhi_medical_rate       = format!("{:.2}", d.medical_rate       * 100.0);
                state.nhi_support_rate       = format!("{:.2}", d.elderly_support_rate * 100.0);
                state.nhi_nursing_rate       = format!("{:.2}", d.nursing_care_rate   * 100.0);
                state.nhi_per_capita_medical = format!("{:.0}", d.per_capita_medical);
                state.nhi_per_capita_support = format!("{:.0}", d.per_capita_support);
                state.nhi_per_capita_nursing = format!("{:.0}", d.per_capita_nursing);
                state.nhi_cap_medical        = format!("{:.0}", d.cap_medical);
                state.nhi_cap_support        = format!("{:.0}", d.cap_support);
                state.nhi_cap_nursing        = format!("{:.0}", d.cap_nursing);
            }
            ui.add_space(4.0);

            egui::Grid::new("g_nhi_auto")
                .num_columns(2)
                .spacing([24.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Medical rate (%):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_medical_rate)
                        .hint_text("e.g. 8.46").desired_width(100.0));
                    ui.end_row();

                    ui.label(RichText::new("Support rate (%):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_support_rate)
                        .hint_text("e.g. 2.04").desired_width(100.0));
                    ui.end_row();

                    ui.label(RichText::new("Nursing care rate (%) [40–64]:").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_nursing_rate)
                        .hint_text("e.g. 2.02").desired_width(100.0));
                    ui.end_row();

                    ui.label(RichText::new("Per-capita medical (JPY/person):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_per_capita_medical)
                        .hint_text("e.g. 33600").desired_width(120.0));
                    ui.end_row();

                    ui.label(RichText::new("Per-capita support (JPY/person):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_per_capita_support)
                        .hint_text("e.g. 11400").desired_width(120.0));
                    ui.end_row();

                    ui.label(RichText::new("Per-capita nursing (JPY/person):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_per_capita_nursing)
                        .hint_text("e.g. 12600").desired_width(120.0));
                    ui.end_row();

                    ui.label(RichText::new("Medical annual cap (JPY):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_cap_medical)
                        .hint_text("e.g. 650000").desired_width(120.0));
                    ui.end_row();

                    ui.label(RichText::new("Support annual cap (JPY):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_cap_support)
                        .hint_text("e.g. 240000").desired_width(120.0));
                    ui.end_row();

                    ui.label(RichText::new("Nursing annual cap (JPY):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_cap_nursing)
                        .hint_text("e.g. 170000").desired_width(120.0));
                    ui.end_row();
                });

            ui.add_space(4.0);
            ui.checkbox(&mut state.nhi_include_us_income,
                "Include US Investment Income in NHI Base");
            if state.nhi_include_us_income {
                ui.label(RichText::new(
                    "When checked, prior-year US dividends (converted to JPY at current FX) \
                     are added to the NHI income basis. Accurate for global-income residents."
                ).small().color(Color32::from_rgb(255, 200, 50)));
            }
        } else {
            egui::Grid::new("g_nhi_manual")
                .num_columns(2)
                .spacing([24.0, 5.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Spike Year Annual Total (JPY):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_spike_total_jpy)
                        .hint_text("e.g. 880000").desired_width(140.0));
                    ui.end_row();

                    ui.label(RichText::new("Ongoing Annual Total (JPY):").strong());
                    ui.add(egui::TextEdit::singleline(&mut state.nhi_ongoing_total_jpy)
                        .hint_text("e.g. 540000").desired_width(140.0));
                    ui.end_row();
                });
            ui.label(RichText::new(
                "Spike year = first post-retirement year (prior-year employment income basis). \
                 Ongoing = all subsequent years."
            ).small().color(Color32::GRAY));
        }
        ui.add_space(8.0);

        // ── Stage 10 — Long-Term Care Insurance (Kaigo Hoken) ────────────────────
        section(ui, "Long-Term Care Insurance (介護保険)");
        ui.label(RichText::new(
            "Japan mandates Long-Term Care Insurance for all residents ≥ 40. From age 40-64 \
             it's bundled into NHI (already modeled above). From age 65+ it becomes a separate \
             municipal premium tied to your pension income — typically ¥30k-¥150k/year. \
             The model automatically handles the smooth transition at age 65."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);

        ui.checkbox(&mut state.kaigo_hoken_enabled, "Model Long-Term Care Insurance (Recommended for retirees ≥ 40)");

        if state.kaigo_hoken_enabled {
            ui.label(RichText::new(
                "When enabled, the age-65+ municipal premium is charged as a separate expense line. \
                 Disable to revert to legacy behavior (no charge after 65)."
            ).small().color(Color32::from_rgb(255, 200, 50)));
            ui.add_space(6.0);

            ui.label(RichText::new("Care Need Scenario:").strong());
            ui.horizontal(|ui| {
                ui.radio_value(&mut state.kaigo_care_scenario, "none".to_string(),
                    "None (Premium only)");
                ui.radio_value(&mut state.kaigo_care_scenario, "low".to_string(),
                    "Low (~¥20k/mo from age 75)");
            });
            ui.horizontal(|ui| {
                ui.radio_value(&mut state.kaigo_care_scenario, "medium".to_string(),
                    "Medium (~¥40k/mo from age 75)");
                ui.radio_value(&mut state.kaigo_care_scenario, "high".to_string(),
                    "High (~¥80k/mo from age 80)");
            });

            match state.kaigo_care_scenario.as_str() {
                "low" => {
                    ui.label(RichText::new(
                        "Low scenario: light intermittent home help starting at age 75."
                    ).small().italics().color(Color32::GRAY));
                }
                "medium" => {
                    ui.label(RichText::new(
                        "Medium scenario: regular home visits + occasional facility stays from age 75."
                    ).small().italics().color(Color32::GRAY));
                }
                "high" => {
                    ui.label(RichText::new(
                        "High scenario: intensive care assumption from age 80. \
                         Useful for stress-testing, not a likely outcome."
                    ).small().italics().color(Color32::from_rgb(255, 150, 100)));
                }
                _ => {}
            }
        } else {
            ui.label(RichText::new(
                "Disabled: ages 65+ will not be charged the separate Kaigo Hoken premium. \
                 Your retirement projection may understate costs by ¥30k-¥150k/year."
            ).small().color(Color32::from_rgb(255, 100, 100)));
        }
        ui.add_space(8.0);

        // ── US Tax Mitigation Strategy ───────────────────────────────────────────
        section(ui, "US Tax Mitigation Strategy");
        ui.label(RichText::new("Strategy").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut state.us_tax_strategy, UsTaxStrategy::FtcOnly,
                "FTC Only — Japan taxes credited against US liability");
            ui.radio_value(&mut state.us_tax_strategy, UsTaxStrategy::FeieAndFtc,
                "FEIE + FTC — Exclude earned income, then FTC on remainder");
        });
        ui.label(RichText::new(match state.us_tax_strategy {
            UsTaxStrategy::FtcOnly    => "Standard: Japan-First FTC credits resident tax against US federal liability.",
            UsTaxStrategy::FeieAndFtc => "Simulation computes both paths and uses the lower-tax result. 2026 FEIE limit: $126,500.",
        }).small().color(Color32::GRAY));
        ui.add_space(4.0);

        // Stage 05 — PFIC basis drift monitor toggle
        ui.horizontal(|ui| {
            ui.checkbox(&mut state.track_pfic_basis_drift, "☑ Track PFIC Basis Drift (Recommended)")
                .on_hover_text(
                    "When enabled, the engine cross-checks each PFIC asset's JPY basis each year \
                     against USD basis × current FX rate. If drift exceeds 1%, it self-heals the \
                     JPY basis and logs a warning in the audit report. Disable only for \
                     apples-to-apples comparisons where you want a frozen FX basis."
                );
        });
        ui.add_space(8.0);

        // ── US Tax Filing Status ─────────────────────────────────────────────────
        section(ui, "US Tax Filing Status & State Residency");
        ui.label(RichText::new("Filing Status").strong());
        ui.horizontal_wrapped(|ui| {
            for &status in ALL_FILING_STATUSES {
                let selected = state.filing_status == status;
                if ui.radio(selected, status).clicked() {
                    state.filing_status = status.into();
                }
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("US State Residency").strong());
        egui::ComboBox::from_id_salt("state_combo")
            .selected_text(format!("{} — {}", state.us_state_code, state_display_name(&state.us_state_code)))
            .show_ui(ui, |ui| {
                for &code in ALL_STATE_CODES {
                    let label = format!("{} — {}", code, state_display_name(code));
                    let selected = state.us_state_code == code;
                    if ui.selectable_label(selected, &label).clicked() {
                        state.us_state_code = code.into();
                    }
                }
            });
        ui.add_space(4.0);
        ui.label(RichText::new(format!("State Tax Rate: {:.1}%  (auto-derived, read-only)", state.state_tax_rate() * 100.0))
            .color(Color32::GRAY).small())
            .on_hover_text(
                "US State Tax — Calculates additional US liability that may not be \
                 fully offset by foreign credits.\n\n\
                 Japan resident tax credits US FEDERAL via the Foreign Tax Credit \
                 (IRC §901), but does NOT credit STATE tax. State tax therefore acts \
                 as a permanent additional drag on US-domiciled realised gains, \
                 regardless of how much Japan tax you have paid. The V7.0 \
                 liquidation engine grosses up share sales by this rate so the \
                 shortfall is still covered after the year-end state-tax true-up."
            );
        ui.add_space(8.0);

        // ── Japan Location ───────────────────────────────────────────────────────
        section(ui, "Japan Location");
        ui.label(RichText::new(
            "Selects the resident tax (住民税) rate. Standard: 10% + ¥6,000/yr. Nagoya City (Aichi): 9.7%."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);
        ui.label(RichText::new("Prefecture").strong());
        egui::ComboBox::from_id_salt("prefecture_combo")
            .selected_text(if state.prefecture.is_empty() { "— select —" } else { &state.prefecture })
            .show_ui(ui, |ui| {
                for &pref in ALL_PREFECTURES {
                    let selected = state.prefecture == pref;
                    if ui.selectable_label(selected, pref).clicked() && !selected {
                        state.prefecture = pref.to_string();
                        state.city = "Other (Standard Rate)".to_string();
                    }
                }
            });
        ui.add_space(4.0);
        ui.label(RichText::new("City").strong());
        let cities = cities_for_prefecture(&state.prefecture);
        egui::ComboBox::from_id_salt("city_combo")
            .selected_text(if state.city.is_empty() { "— select —" } else { &state.city })
            .show_ui(ui, |ui| {
                for &city in cities {
                    let label = if let Some(note) = city_rate_annotation(city) {
                        format!("{} ({})", city, note)
                    } else {
                        city.to_string()
                    };
                    let selected = state.city == city;
                    if ui.selectable_label(selected, &label).clicked() {
                        state.city = city.to_string();
                    }
                }
            });
        if let Some(note) = city_rate_annotation(&state.city) {
            ui.label(RichText::new(format!("⚠ Special rate: {}", note))
                .color(Color32::from_rgb(255, 200, 50)).small());
        }
        ui.add_space(8.0);

        // ── Tax Jurisdiction & Investment Accounts ───────────────────────────────
        section(ui, "Tax Jurisdiction & Investment Accounts");

        ui.label(RichText::new("Global Tax Jurisdiction").strong())
            .on_hover_text(
                "Global Tax Jurisdiction — Sets your primary tax residency. \
                 Essential for applying local rates (like Japan's 20.315%) and \
                 determining Foreign Tax Credit (FTC) eligibility for US citizens.\n\n\
                 • Both (US + Japan): Standard expat path. Japan taxes computed \
                   first; their JPY value credits US federal liability via the FTC.\n\
                 • US Only: Bypasses Japan resident tax + NHI entirely.\n\
                 • Japan Only: Bypasses US federal + capital-gains + FERS \
                   withholding entirely.\n\n\
                 Per-source overrides (FERS, SS, Nenkin, military) live below and \
                 take precedence over this global setting for that one stream."
            );
        ui.horizontal(|ui| {
            ui.radio_value(&mut state.tax_jurisdiction, TaxJurisdiction::Both,      "Both (US + Japan)");
            ui.radio_value(&mut state.tax_jurisdiction, TaxJurisdiction::UsOnly,    "US Only");
            ui.radio_value(&mut state.tax_jurisdiction, TaxJurisdiction::JapanOnly, "Japan Only");
        });
        ui.label(RichText::new(match state.tax_jurisdiction {
            TaxJurisdiction::Both      => "US federal + Japan resident tax. Japan taxes credited against US via FTC.",
            TaxJurisdiction::UsOnly    => "⚠ Japan resident tax and NHI calculations are bypassed.",
            TaxJurisdiction::JapanOnly => "⚠ US federal, capital-gains, and FERS withholding are bypassed.",
            TaxJurisdiction::TaxFree   => "⚠ Tax-free mode is per-income-source only; global field defaults to Both.",
        }).small().color(Color32::GRAY));
        ui.add_space(8.0);

        ui.label(RichText::new("Investment Accounts").strong());
        ui.label(RichText::new(
            "Each account has its own type and tax jurisdiction. \
             Click ✨ next to any ticker to auto-fill Price and Capital Appreciation from Yahoo Finance. \
             Cost Basis and Capital Appreciation % are optional."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);

        // ── Marco Polo (Monte Carlo) toggle ──────────────────────────────────────
        ui.horizontal(|ui| {
            ui.checkbox(&mut state.marco_polo_enabled, "🎲 Marco Polo Mode (Monte Carlo)");
            if state.marco_polo_enabled {
                ui.label(
                    RichText::new("1,000 iterations · GBM · P10/P50/P90 in Compare tab")
                        .small()
                        .color(Color32::from_rgb(255, 200, 80)),
                );
            }
        });
        if state.marco_polo_enabled {
            ui.label(
                RichText::new(
                    "Volatility % replaces Capital Appreciation % below. \
                     Capital Appreciation values are read from the simulation engine; \
                     Volatility drives stochastic path dispersion."
                )
                .small()
                .color(Color32::from_rgb(180, 180, 100)),
            );
        }
        ui.add_space(4.0);

        // ── Target-State Rebalancing toggle ──────────────────────────────────────
        ui.horizontal(|ui| {
            ui.checkbox(&mut state.rebalance_enabled, "⚖ Target-State Rebalancing");
            if state.rebalance_enabled {
                ui.add_space(8.0);
                ui.label(RichText::new("Frequency:").small());
                if ui.radio(state.rebalance_frequency == "Monthly",    "Monthly").clicked()    { state.rebalance_frequency = "Monthly".into(); }
                if ui.radio(state.rebalance_frequency == "Quarterly",  "Quarterly").clicked()  { state.rebalance_frequency = "Quarterly".into(); }
                if ui.radio(state.rebalance_frequency == "Semi-Annual","Semi-Annual").clicked() { state.rebalance_frequency = "Semi-Annual".into(); }
                if ui.radio(state.rebalance_frequency == "Annual",     "Annual").clicked()      { state.rebalance_frequency = "Annual".into(); }
            }
        });
        if state.rebalance_enabled {
            ui.label(RichText::new(
                "Set Target Alloc % per position below (⚙ Management panel). Engine sells overweight \
                 positions and buys underweight ones each period (15% LTCG on taxable sells)."
            ).small().color(Color32::from_rgb(100, 200, 220)));
        }
        ui.add_space(6.0);

        let num_accounts = state.accounts.len();
        let mut remove_account:   Option<usize>          = None;
        let mut add_position_to:  Option<usize>          = None;
        let mut remove_position:  Option<(usize, usize)> = None;
        let mut auto_fill:        Option<(usize, usize)> = None;
        let mut toggle_mgmt:      Option<(usize, usize)> = None;
        let mut toggle_profile:   Option<(usize, usize)> = None;
        let mut auto_fill_profile: Option<(usize, usize)> = None;
        let mut dc_auto_fill:     Option<(usize, usize)> = None;

        for acct_idx in 0..num_accounts {
            if acct_idx > 0 {
                ui.add_space(12.0);
            }
            let card_color = Color32::from_rgba_unmultiplied(255, 255, 255, 6);
            Frame::none()
                .fill(card_color)
                .stroke(egui::Stroke::new(1.0, Color32::from_rgb(75, 75, 75)))
                .inner_margin(egui::Margin::same(8.0))
                .outer_margin(egui::Margin { top: 2.0, bottom: 2.0, left: 0.0, right: 0.0 })
                .show(ui, |ui| {
                    // ── Account header ─────────────────────────────────────────
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Account {}:", acct_idx + 1)).strong());
                        egui::ComboBox::from_id_salt(egui::Id::new("at").with(acct_idx))
                            .selected_text(state.accounts[acct_idx].account_type.as_str())
                            .show_ui(ui, |ui| {
                                for &at in ALL_ACCOUNT_TYPES {
                                    let sel = state.accounts[acct_idx].account_type == at;
                                    if ui.selectable_label(sel, at).clicked() {
                                        state.accounts[acct_idx].account_type = at.to_string();
                                    }
                                }
                            });

                        ui.label(RichText::new("Jurisdiction:").color(Color32::GRAY).small())
                            .on_hover_text(
                                "Tax Jurisdiction (Source) — Defines which country has the \
                                 primary taxing right for this specific income source under \
                                 the US-Japan Tax Treaty.\n\n\
                                 Overrides the Global Tax Jurisdiction for this account only. \
                                 Examples: a Roth IRA's gains are typically US Only, an iDeCo \
                                 / Japan DC account is Japan Only, a US taxable brokerage \
                                 held by a Japan resident is Both. Tax Free is reserved for \
                                 income explicitly exempt under the treaty (e.g., VA \
                                 disability, SMC)."
                            );
                        let cur_jur = state.accounts[acct_idx].tax_jurisdiction;
                        let jur_label = match cur_jur {
                            TaxProtocol::Both      => "Both",
                            TaxProtocol::UsOnly    => "US Only",
                            TaxProtocol::JapanOnly => "Japan Only",
                            TaxProtocol::TaxFree   => "Tax Free",
                        };
                        egui::ComboBox::from_id_salt(egui::Id::new("aj").with(acct_idx))
                            .selected_text(jur_label)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(cur_jur == TaxProtocol::Both,      "Both").clicked()      { state.accounts[acct_idx].tax_jurisdiction = TaxProtocol::Both; }
                                if ui.selectable_label(cur_jur == TaxProtocol::UsOnly,    "US Only").clicked()   { state.accounts[acct_idx].tax_jurisdiction = TaxProtocol::UsOnly; }
                                if ui.selectable_label(cur_jur == TaxProtocol::JapanOnly, "Japan Only").clicked(){ state.accounts[acct_idx].tax_jurisdiction = TaxProtocol::JapanOnly; }
                                if ui.selectable_label(cur_jur == TaxProtocol::TaxFree,   "Tax Free").clicked()  { state.accounts[acct_idx].tax_jurisdiction = TaxProtocol::TaxFree; }
                            });

                        if num_accounts > 1 {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button("✕ Remove").clicked() {
                                    remove_account = Some(acct_idx);
                                }
                            });
                        }
                    });

                    let acct_type = state.accounts[acct_idx].account_type.clone();

                    if acct_type == "DC Plan" {
                        // ── DC Plan inline fields ──────────────────────────────
                        ui.add_space(6.0);
                        ui.label(RichText::new("DC Plan Configuration").strong().size(12.0));
                        ui.label(RichText::new(
                            "⚓ Monthly Contribution is JPY-denominated. It stays fixed in Yen \
                             regardless of USD/JPY rate changes — only its USD equivalent shifts."
                        ).small().color(Color32::from_rgb(100, 200, 150)));
                        ui.add_space(4.0);
                        egui::Grid::new(egui::Id::new("g_dc_top").with(acct_idx))
                            .num_columns(2)
                            .spacing([24.0, 4.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("Monthly Contribution (JPY):").strong());
                                ui.add(egui::TextEdit::singleline(&mut state.accounts[acct_idx].dc_monthly_jpy)
                                    .hint_text("e.g. 45000").desired_width(160.0));
                                ui.end_row();
                            });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut state.accounts[acct_idx].dc_use_market_avg,
                                "Use Market Average (10%)");
                            if !state.accounts[acct_idx].dc_use_market_avg {
                                ui.add_space(8.0);
                                ui.label(RichText::new("Fallback Total Return %:").strong())
                                    .on_hover_text(
                                        "Assumed annual total return for DC funds with no per-fund rate set. \
                                         Applied as a single flat CAGR (capital appreciation + reinvested distributions \
                                         combined). DC accounts are tax-deferred, so dividends always compound here \
                                         regardless of the DRIP setting elsewhere."
                                    );
                                ui.add(egui::TextEdit::singleline(&mut state.accounts[acct_idx].dc_growth_rate)
                                    .hint_text("e.g. 8.0").desired_width(60.0));
                            }
                        });
                        if state.marco_polo_enabled {
                            ui.add_space(4.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Marco Polo Volatility %:").strong()
                                    .color(Color32::from_rgb(255, 200, 80)));
                                ui.add(egui::TextEdit::singleline(&mut state.accounts[acct_idx].dc_volatility)
                                    .hint_text("15.0").desired_width(60.0));
                                ui.label(RichText::new("(annual std dev for stochastic paths)")
                                    .small().color(Color32::GRAY));
                            });
                        }
                        // ── Per-fund table (¥/万口 pricing) ──────────────────────
                        ui.add_space(6.0);
                        ui.label(RichText::new("Fund Allocation (¥/万口 pricing — enter a Yahoo symbol like 0331418A.T to enable ✨ auto-fetch)").strong().small());
                        {
                            let alloc_sum: f64 = state.accounts[acct_idx].dc_funds.iter()
                                .filter_map(|f| f.contrib_alloc_pct.trim().parse::<f64>().ok())
                                .sum();
                            if (alloc_sum - 100.0).abs() > 0.5 && !state.accounts[acct_idx].dc_funds.is_empty() {
                                ui.label(RichText::new(format!("⚠ Alloc total: {:.0}% (must equal 100%)", alloc_sum))
                                    .small().color(Color32::from_rgb(255, 180, 60)));
                            }
                        }
                        egui::Grid::new(egui::Id::new("g_dc_funds").with(acct_idx))
                            .num_columns(9)
                            .striped(true)
                            .spacing([8.0, 4.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("Fund Name").strong().small());
                                ui.label(RichText::new("Ticker").strong().small())
                                    .on_hover_text(
                                        "Optional Yahoo Finance symbol for this fund. Japanese DC mutual \
                                         funds usually use the form `XXXXXXXX.T` (e.g. 0331418A.T for the \
                                         eMAXIS Slim 全世界株式). When set, ✨ fills Price (¥/万口) and \
                                         Total Return % from Yahoo's chart API."
                                    );
                                ui.label(RichText::new("✨").strong().small())
                                    .on_hover_text("Auto-fill Price (¥/万口) and Total Return % from Yahoo (10-year price CAGR).");
                                ui.label(RichText::new("Units (口)").strong().small());
                                ui.label(RichText::new("Price (¥/万口)").strong().small());
                                ui.label(RichText::new("Alloc %").strong().small());
                                ui.label(RichText::new("Total Return %").strong().small())
                                    .on_hover_text(
                                        "Assumed annual total return for this fund (capital appreciation + \
                                         reinvested distributions combined). DC accounts are tax-deferred \
                                         and always reinvest internally, so this single CAGR drives all growth."
                                    );
                                ui.label(RichText::new("Stop@Retire").strong().small());
                                ui.label(RichText::new("Remove").strong().small());
                                ui.end_row();
                                let nf = state.accounts[acct_idx].dc_funds.len();
                                let mut remove_fund: Option<usize> = None;
                                for fi in 0..nf {
                                    let fund = &mut state.accounts[acct_idx].dc_funds[fi];
                                    ui.add(egui::TextEdit::singleline(&mut fund.fund_name)
                                        .hint_text("e.g. Domestic Equity").desired_width(140.0)
                                        .id(egui::Id::new("fn").with(acct_idx).with(fi)));
                                    ui.add(egui::TextEdit::singleline(&mut fund.ticker)
                                        .hint_text("e.g. 0331418A.T").desired_width(100.0)
                                        .id(egui::Id::new("ft").with(acct_idx).with(fi)));
                                    if ui.small_button("✨")
                                        .on_hover_text("Auto-fill Price (¥/万口) & Total Return % from Yahoo Finance.")
                                        .clicked()
                                    {
                                        dc_auto_fill = Some((acct_idx, fi));
                                    }
                                    ui.add(egui::TextEdit::singleline(&mut fund.units)
                                        .hint_text("e.g. 15000").desired_width(80.0)
                                        .id(egui::Id::new("fu").with(acct_idx).with(fi)));
                                    ui.add(egui::TextEdit::singleline(&mut fund.price_per_10k)
                                        .hint_text("e.g. 21340").desired_width(80.0)
                                        .id(egui::Id::new("fp").with(acct_idx).with(fi)));
                                    ui.add(egui::TextEdit::singleline(&mut fund.contrib_alloc_pct)
                                        .hint_text("e.g. 70").desired_width(50.0)
                                        .id(egui::Id::new("fa").with(acct_idx).with(fi)));
                                    ui.add(egui::TextEdit::singleline(&mut fund.growth_pct)
                                        .hint_text("e.g. 8.0").desired_width(50.0)
                                        .id(egui::Id::new("fg").with(acct_idx).with(fi)));
                                    ui.checkbox(&mut state.accounts[acct_idx].dc_funds[fi].stop_at_retirement, "");
                                    if ui.small_button("✕").clicked() { remove_fund = Some(fi); }
                                    ui.end_row();
                                }
                                if let Some(idx) = remove_fund {
                                    state.accounts[acct_idx].dc_funds.remove(idx);
                                }
                            });
                        if ui.small_button("+ Add Fund").clicked() {
                            state.accounts[acct_idx].dc_funds.push(DcFundRow::default());
                        }
                    }
                    if acct_type != "DC Plan" {
                        // ── Positions table ────────────────────────────────────
                        ui.add_space(4.0);
                        let pos_count = state.accounts[acct_idx].positions.len();

                        let marco_polo = state.marco_polo_enabled;

                        if pos_count == 0 {
                            ui.label(RichText::new("No positions yet.").small().color(Color32::GRAY));
                        } else {
                            // Single unified 8-column grid — all rows share one ID scope so the
                            // auto-counter never resets between rows. Each TextEdit also carries an
                            // explicit id_salt for additional row-level uniqueness.
                            egui::Grid::new(egui::Id::new("input_positions_grid").with(acct_idx))
                                .num_columns(10)
                                .striped(true)
                                .spacing([12.0, 8.0])
                                .show(ui, |ui| {
                                    // ── Header row ──────────────────────────────────────
                                    ui.label(RichText::new("Ticker").strong().small());
                                    ui.label(RichText::new("Units").strong().small());
                                    ui.label(RichText::new("✨ Auto-Fetch").strong().small());
                                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                        ui.label(RichText::new("Price USD").strong().small());
                                    });
                                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new("Cost Basis").strong().small());
                                            ui.label(RichText::new(" (opt.)").small().color(Color32::from_rgb(120, 120, 120)));
                                        });
                                    });
                                    ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                                        let growth_label = if marco_polo { "Volatility %" } else { "Capital Appreciation %" };
                                        let growth_color = if marco_polo { Color32::from_rgb(255, 200, 80) } else { Color32::WHITE };
                                        ui.horizontal(|ui| {
                                            ui.label(RichText::new(growth_label).strong().small().color(growth_color))
                                                .on_hover_text(
                                                    "Annual price-only change in market value (capital appreciation; \
                                                     a negative value represents capital depreciation). \
                                                     Does NOT include dividend payments, interest income, or \
                                                     capital-gains distributions — those are tracked separately \
                                                     under 📊 Detail.\n\n\
                                                     DRIP does not change this number. When DRIP is on, dividend \
                                                     payments buy additional shares, and those new shares then \
                                                     appreciate at this same rate (compounding the total return). \
                                                     When DRIP is off, dividends are paid out as cash instead.\n\n\
                                                     Auto-fetch (✨) populates this from the ticker's 10-year \
                                                     split-adjusted price CAGR (dividends NOT reinvested)."
                                                );
                                            if !marco_polo {
                                                ui.label(RichText::new(" (opt.)").small().color(Color32::from_rgb(120, 120, 120)));
                                            }
                                        });
                                    });
                                    ui.label(RichText::new("Asset Class").strong().small())
                                        .on_hover_text("Drives which return components are taxable and how distributions are routed (qualified dividends, ROC basis-reduction, etc.).");
                                    ui.label(RichText::new("Return Profile").strong().small())
                                        .on_hover_text("Toggle a component-level total-return breakdown (capital appreciation, dividend payments, interest income, capital-gains distributions, return of capital, expense ratio).");
                                    ui.label(RichText::new("⚙ Management").strong().small());
                                    ui.label(RichText::new("☐ Select/Delete").strong().small());
                                    ui.end_row();

                                    // ── Data rows ───────────────────────────────────────
                                    for pos_idx in 0..pos_count {
                                        let pos = &mut state.accounts[acct_idx].positions[pos_idx];

                                        // V6.6: doubled desired_width for Ticker/Units/Price.
                                        ui.add(egui::TextEdit::singleline(&mut pos.ticker)
                                            .id_salt(("ticker", acct_idx, pos_idx))
                                            .hint_text("e.g. VTI").desired_width(108.0));

                                        ui.add(egui::TextEdit::singleline(&mut pos.units)
                                            .id_salt(("units", acct_idx, pos_idx))
                                            .hint_text("e.g. 250")
                                            .desired_width(140.0));

                                        if ui.small_button("✨")
                                            .on_hover_text("Auto-fill Price & Capital Appreciation % from Yahoo Finance (10-year price CAGR, dividends NOT reinvested).")
                                            .clicked()
                                        {
                                            auto_fill = Some((acct_idx, pos_idx));
                                        }

                                        ui.add(egui::TextEdit::singleline(&mut pos.unit_value)
                                            .id_salt(("price", acct_idx, pos_idx))
                                            .hint_text("current $")
                                            .desired_width(164.0));

                                        ui.add(egui::TextEdit::singleline(&mut pos.cost_basis)
                                            .id_salt(("cost_basis", acct_idx, pos_idx))
                                            .hint_text("optional").desired_width(74.0));

                                        if marco_polo {
                                            ui.add(egui::TextEdit::singleline(&mut pos.volatility_pct)
                                                .id_salt(("volatility", acct_idx, pos_idx))
                                                .hint_text("18.0").desired_width(60.0));
                                        } else {
                                            ui.add(egui::TextEdit::singleline(&mut pos.growth_pct)
                                                .id_salt(("growth", acct_idx, pos_idx))
                                                .hint_text("opt. %").desired_width(60.0))
                                                .on_hover_text(
                                                    "Annual capital appreciation % (price-only). Negative = capital depreciation. \
                                                     Excludes dividends and other distributions. Blank = fall back to the global default."
                                                );
                                        }

                                        // ── V7.6 — Asset class dropdown + detailed-profile toggle ──
                                        egui::ComboBox::from_id_salt(("ac", acct_idx, pos_idx))
                                            .selected_text(match pos.asset_class.as_str() {
                                                "ETF"        => "ETF",
                                                "MutualFund" => "Mutual Fund",
                                                "Other"      => "Other",
                                                _            => "Stock",
                                            })
                                            .width(110.0)
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(&mut pos.asset_class, "Stock".into(),       "Stock");
                                                ui.selectable_value(&mut pos.asset_class, "ETF".into(),         "ETF");
                                                ui.selectable_value(&mut pos.asset_class, "MutualFund".into(),  "Mutual Fund");
                                                ui.selectable_value(&mut pos.asset_class, "Other".into(),       "Other");
                                            });

                                        // ── Stage 05 — PFIC regime dropdown ──────────────────────
                                        egui::ComboBox::from_id_salt(("pfic", acct_idx, pos_idx))
                                            .selected_text(match pos.pfic_regime.as_str() {
                                                "Mtm"                => "§1296 MTM",
                                                "Qef"                => "§1295 QEF",
                                                "ExcessDistribution" => "§1291 XD",
                                                _                    => "Not PFIC",
                                            })
                                            .width(90.0)
                                            .show_ui(ui, |ui| {
                                                ui.selectable_value(&mut pos.pfic_regime, "NotPfic".into(),            "Not a PFIC");
                                                ui.selectable_value(&mut pos.pfic_regime, "Mtm".into(),                "§1296 MTM");
                                                ui.selectable_value(&mut pos.pfic_regime, "Qef".into(),                "§1295 QEF");
                                                ui.selectable_value(&mut pos.pfic_regime, "ExcessDistribution".into(), "§1291 Excess Dist.");
                                            })
                                            .response
                                            .on_hover_text(
                                                "PFIC Regime (US persons holding non-US mutual funds)\n\n\
                                                 If you hold a Japanese mutual fund and you're a US citizen, \
                                                 you owe US tax on the year-end paper gain even if you didn't sell anything.\n\n\
                                                 Not a PFIC — standard equity treatment.\n\
                                                 §1296 MTM — Mark-to-Market: annual phantom income taxed as ordinary (recommended).\n\
                                                 §1295 QEF — Qualified Electing Fund: requires annual QEF statement from fund.\n\
                                                 §1291 Excess Dist. — Default punitive regime with penalty interest."
                                            );

                                        let prof_label = if pos.use_detailed_profile {
                                            if pos.profile_expanded { "📊 Detail ▾" } else { "📊 Detail" }
                                        } else {
                                            "Simple"
                                        };
                                        if ui.small_button(prof_label)
                                            .on_hover_text("Toggle a per-component total-return breakdown (capital appreciation, dividend payments, interest income, capital-gains distributions, return of capital, expense ratio). Available components depend on asset class.")
                                            .clicked()
                                        {
                                            toggle_profile = Some((acct_idx, pos_idx));
                                        }

                                        let mgmt_label = if pos.mgmt_expanded { "⚙▾" } else { "⚙" };
                                        if ui.small_button(mgmt_label)
                                            .on_hover_text("Toggle management options: Accumulation, DRIP, Target Allocation")
                                            .clicked()
                                        {
                                            toggle_mgmt = Some((acct_idx, pos_idx));
                                        }

                                        if ui.small_button("✕").on_hover_text("Remove this position").clicked() {
                                            remove_position = Some((acct_idx, pos_idx));
                                        }

                                        ui.end_row();
                                    }
                                });

                            // ── Management sub-panels (rendered after unified grid) ─────
                            for pos_idx in 0..pos_count {
                                if state.accounts[acct_idx].positions[pos_idx].mgmt_expanded {
                                    let pos = &mut state.accounts[acct_idx].positions[pos_idx];
                                    Frame::none()
                                        .fill(Color32::from_rgba_unmultiplied(40, 80, 120, 40))
                                        .inner_margin(egui::Margin::same(6.0))
                                        .show(ui, |ui| {
                                            ui.label(RichText::new(
                                                format!("⚙ {} Management", pos.ticker)
                                            ).small().strong());
                                            ui.horizontal_wrapped(|ui| {
                                                ui.checkbox(&mut pos.drip_enabled, "DRIP");
                                                if pos.drip_enabled {
                                                    ui.label(RichText::new("→").small().color(Color32::GRAY));
                                                    ui.add(egui::TextEdit::singleline(&mut pos.drip_reinvest_ticker)
                                                        .hint_text("blank=self, CASH, or ticker")
                                                        .desired_width(120.0))
                                                        .on_hover_text("Leave blank to reinvest in same ticker. Enter CASH to route dividends to cash. Enter another ticker to redirect.");
                                                }
                                                ui.add_space(12.0);
                                                ui.label(RichText::new("Target Alloc %:").small().strong());
                                                ui.add(egui::TextEdit::singleline(&mut pos.target_alloc_pct)
                                                    .hint_text("e.g. 60").desired_width(56.0))
                                                    .on_hover_text("Target portfolio weight %. Used by rebalancing engine.");
                                            });
                                            ui.add_space(2.0);
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label(RichText::new("Rebalance Date:").small().strong());
                                                ui.add(egui::TextEdit::singleline(&mut pos.rebalance_date)
                                                    .id_salt(("rb_date", acct_idx, pos_idx))
                                                    .hint_text("YYYY-MM-DD (V6.6, blank = global)")
                                                    .desired_width(160.0))
                                                    .on_hover_text("V6.6: per-position rebalance trigger; supersedes the global Rebalance Date when set.");
                                            });
                                            ui.add_space(2.0);
                                            ui.horizontal_wrapped(|ui| {
                                                ui.label(RichText::new("Accum $/mo:").small().strong());
                                                ui.add(egui::TextEdit::singleline(&mut pos.accum_amount)
                                                    .hint_text("USD/mo").desired_width(72.0))
                                                    .on_hover_text("Monthly scheduled buy amount (USD). 0 or blank = no rule.");
                                                ui.label(RichText::new("Freq:").small());
                                                if ui.radio(pos.accum_frequency == "Monthly",   "Monthly").clicked()   { pos.accum_frequency = "Monthly".into(); }
                                                if ui.radio(pos.accum_frequency == "Quarterly", "Quarterly").clicked() { pos.accum_frequency = "Quarterly".into(); }
                                                if ui.radio(pos.accum_frequency == "Annual",    "Annual").clicked()    { pos.accum_frequency = "Annual".into(); }
                                                ui.checkbox(&mut pos.stop_at_retirement, "Stop at Retirement");
                                            });
                                        });
                                }
                            }

                            // ── V7.6 — Detailed Return Profile sub-panels ────────────
                            {
                                for pos_idx in 0..pos_count {
                                    if !state.accounts[acct_idx].positions[pos_idx].profile_expanded {
                                        continue;
                                    }
                                    let pos = &mut state.accounts[acct_idx].positions[pos_idx];
                                    Frame::none()
                                        .fill(Color32::from_rgba_unmultiplied(80, 50, 120, 40))
                                        .inner_margin(egui::Margin::same(6.0))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(RichText::new(
                                                    format!("📊 {} Return Profile  (class: {})", pos.ticker, pos.asset_class)
                                                ).small().strong());
                                                ui.add_space(10.0);
                                                ui.checkbox(&mut pos.use_detailed_profile,
                                                    "Use detailed return profile")
                                                    .on_hover_text("When on, the engine drives total return from the components below (capital appreciation + dividend payments + interest + cap-gains distributions + …) instead of the flat Capital Appreciation % column.");
                                                ui.add_space(10.0);
                                                if ui.small_button("✨ Auto-Fetch")
                                                    .on_hover_text(
                                                        "Fetch the asset-class-appropriate components from Yahoo Finance:\n\
                                                         • Stock: Capital Appreciation + Dividend Payments\n\
                                                         • ETF: Capital Appreciation + Dividend Payments + Capital-Gains Distributions + Expense Ratio\n\
                                                         • Mutual Fund: NAV Appreciation + Dividend Payments + Capital-Gains Distributions + Expense Ratio\n\
                                                         • Other: all of the above\n\
                                                         Interest Income, Special Distributions, and Return of Capital are not exposed \
                                                         by Yahoo and remain under manual control."
                                                    )
                                                    .clicked()
                                                {
                                                    auto_fill_profile = Some((acct_idx, pos_idx));
                                                }
                                            });
                                            ui.label(RichText::new(
                                                "All values are annual percentages. Components not shown for the chosen \
                                                 asset class default to 0. Switch class to 'Other' to expose every field."
                                            ).small().color(Color32::GRAY));

                                            // Which fields to show, per asset class.
                                            let show_cap_growth     = matches!(pos.asset_class.as_str(), "Stock" | "ETF" | "Other");
                                            let show_nav_growth     = matches!(pos.asset_class.as_str(), "MutualFund" | "Other");
                                            let show_dividend       = true;
                                            let show_interest       = matches!(pos.asset_class.as_str(), "MutualFund" | "Other");
                                            let show_cap_gains_dist = matches!(pos.asset_class.as_str(), "ETF" | "MutualFund" | "Other");
                                            let show_special        = matches!(pos.asset_class.as_str(), "MutualFund" | "Other");
                                            let show_roc            = matches!(pos.asset_class.as_str(), "MutualFund" | "Other");
                                            let show_expense        = matches!(pos.asset_class.as_str(), "ETF" | "MutualFund" | "Other");

                                            let enabled = pos.use_detailed_profile;
                                            ui.add_enabled_ui(enabled, |ui| {
                                                egui::Grid::new(egui::Id::new("g_profile").with(acct_idx).with(pos_idx))
                                                    .num_columns(4)
                                                    .spacing([16.0, 4.0])
                                                    .show(ui, |ui| {
                                                        let field = |ui: &mut Ui, label: &str, value: &mut String, hint: &str, tip: &str| {
                                                            ui.label(RichText::new(label).small().strong())
                                                                .on_hover_text(tip);
                                                            ui.add(egui::TextEdit::singleline(value)
                                                                .hint_text(hint).desired_width(64.0));
                                                        };
                                                        let mut cells = 0usize;
                                                        let wrap = |ui: &mut Ui, cells: &mut usize| {
                                                            *cells += 1;
                                                            if *cells % 2 == 0 { ui.end_row(); }
                                                        };
                                                        if show_cap_growth {
                                                            field(ui, "Capital Appreciation %:", &mut pos.cap_growth_pct, "e.g. 5.2",
                                                                "Annual change in the market price of the investment (price-only). A negative value represents capital depreciation. Excludes all distributions.");
                                                            wrap(ui, &mut cells);
                                                        }
                                                        if show_nav_growth {
                                                            field(ui, "NAV Appreciation %:", &mut pos.nav_growth_pct, "e.g. 4.8",
                                                                "Annual change in fund net asset value, post-distribution. Distinct from market price for ETFs that trade at a premium/discount to NAV.");
                                                            wrap(ui, &mut cells);
                                                        }
                                                        if show_dividend {
                                                            field(ui, "Dividend Payments %:", &mut pos.dividend_yield_pct, "e.g. 1.8",
                                                                "Cash distributions paid to shareholders from company earnings (annual yield, qualified or ordinary). DRIP on → reinvested into more shares; DRIP off → paid out as cash.");
                                                            wrap(ui, &mut cells);
                                                        }
                                                        if show_interest {
                                                            field(ui, "Interest Income %:", &mut pos.interest_yield_pct, "e.g. 0.4",
                                                                "Regular interest payments received from bonds, money-market holdings, or cash deposits. Taxed as ordinary income (US) / 利子所得 (JP).");
                                                            wrap(ui, &mut cells);
                                                        }
                                                        if show_cap_gains_dist {
                                                            field(ui, "Capital Gains Distributions %:", &mut pos.cap_gains_dist_pct, "e.g. 0.6",
                                                                "Payouts made to mutual-fund or ETF investors from the fund's own asset sales (year-end pass-through). LTCG basket unless PFIC §1296 MTM applies.");
                                                            wrap(ui, &mut cells);
                                                        }
                                                        if show_special {
                                                            field(ui, "Special Distributions %:", &mut pos.special_dist_pct, "e.g. 0.0",
                                                                "Non-recurring payouts (e.g. one-off year-end specials).");
                                                            wrap(ui, &mut cells);
                                                        }
                                                        if show_roc {
                                                            field(ui, "Return of Capital %:", &mut pos.roc_pct, "e.g. 0.0",
                                                                "Distribution treated as a return of the investor's own principal — non-taxable in the year received; reduces USD and JPY cost basis pro rata.");
                                                            wrap(ui, &mut cells);
                                                        }
                                                        if show_expense {
                                                            field(ui, "Expense Ratio %:", &mut pos.expense_ratio_pct, "e.g. 0.03",
                                                                "Annual fund management fee. Deducted from capital appreciation before each month's price update.");
                                                            wrap(ui, &mut cells);
                                                        }
                                                        if cells % 2 == 1 { ui.end_row(); }
                                                    });
                                            });

                                            // ── Computed Total Return readout ────────
                                            let f = |s: &str| s.trim().parse::<f64>().unwrap_or(0.0);
                                            let price_growth = if show_nav_growth {
                                                f(&pos.nav_growth_pct)
                                            } else {
                                                f(&pos.cap_growth_pct)
                                            } - f(&pos.expense_ratio_pct);
                                            let distributions = f(&pos.dividend_yield_pct)
                                                + f(&pos.interest_yield_pct)
                                                + f(&pos.cap_gains_dist_pct)
                                                + f(&pos.special_dist_pct);
                                            let total_return = price_growth + distributions + f(&pos.roc_pct);
                                            ui.add_space(2.0);
                                            ui.label(RichText::new(format!(
                                                "Total Return ≈ {:.2}%  (capital appreciation {:+.2}%  +  distributions {:.2}%  +  return of capital {:.2}%)",
                                                total_return, price_growth, distributions, f(&pos.roc_pct)
                                            )).small().color(Color32::from_rgb(140, 200, 240)));
                                        });
                                }
                            }

                            // Cost-basis / est-value summary
                            let (mut total_basis, mut total_value) = (0.0_f64, 0.0_f64);
                            for pos in &state.accounts[acct_idx].positions {
                                let qty: f64 = pos.units.trim().parse().unwrap_or(0.0);
                                let cb:  f64 = pos.cost_basis.trim().parse().unwrap_or(0.0);
                                let uv:  f64 = pos.unit_value.trim().parse()
                                    .unwrap_or_else(|_| crate::engine::market_data::MarketDataService::fallback_price(&pos.ticker));
                                total_basis += qty * cb;
                                total_value += qty * uv;
                            }
                            let gain = total_value - total_basis;
                            if total_basis > 0.0 || total_value > 0.0 {
                                ui.add_space(2.0);
                                let gc = if gain >= 0.0 { Color32::from_rgb(100, 220, 100) } else { Color32::from_rgb(220, 100, 100) };
                                ui.label(RichText::new(format!(
                                    "Basis: ${:.0}  |  Est. Value: ${:.0}  |  P/L: {:+.0}",
                                    total_basis, total_value, gain
                                )).small().color(gc));
                            }
                        }

                        if ui.small_button("+ Add Position").clicked() {
                            add_position_to = Some(acct_idx);
                        }
                    }
                }); // Frame
        } // for acct_idx

        // Apply deferred mutations
        if let Some((ai, pi)) = toggle_mgmt {
            if ai < state.accounts.len() && pi < state.accounts[ai].positions.len() {
                let expanded = &mut state.accounts[ai].positions[pi].mgmt_expanded;
                *expanded = !*expanded;
            }
        }
        if let Some((ai, pi)) = toggle_profile {
            if ai < state.accounts.len() && pi < state.accounts[ai].positions.len() {
                let pos = &mut state.accounts[ai].positions[pi];
                // First click: expand and turn on detailed profile.
                // Click while expanded: just collapse (leave the toggle as-is).
                if pos.profile_expanded {
                    pos.profile_expanded = false;
                } else {
                    pos.profile_expanded = true;
                    pos.use_detailed_profile = true;
                }
            }
        }
        if let Some((ai, pi)) = remove_position {
            if ai < state.accounts.len() { state.accounts[ai].positions.remove(pi); }
        }
        if let Some(ai) = add_position_to {
            if ai < state.accounts.len() { state.accounts[ai].positions.push(PositionRow::default()); }
        }
        if let Some(idx) = remove_account {
            state.accounts.remove(idx);
        }
        if let Some((ai, pi)) = auto_fill {
            if ai < state.accounts.len() && pi < state.accounts[ai].positions.len() {
                let ticker = state.accounts[ai].positions[pi].ticker.clone();
                if !ticker.is_empty() {
                    let price = crate::engine::market_data::MarketDataService::fetch_current_price(&ticker);
                    let cagr  = crate::engine::market_data::MarketDataService::fetch_10y_cagr(&ticker);
                    let pos = &mut state.accounts[ai].positions[pi];
                    pos.unit_value = format!("{:.2}", price);
                    pos.growth_pct = format!("{:.1}", cagr * 100.0);
                }
            }
        }
        if let Some((ai, fi)) = dc_auto_fill {
            if ai < state.accounts.len() && fi < state.accounts[ai].dc_funds.len() {
                let ticker = state.accounts[ai].dc_funds[fi].ticker.trim().to_string();
                if ticker.is_empty() {
                    log::warn!("[DC Auto-Fetch] fund #{}: no ticker set — enter a Yahoo symbol (e.g. 0331418A.T) first.", fi + 1);
                } else {
                    // Yahoo's v8/chart endpoint returns prices in the security's native currency.
                    // For Japanese mutual-fund tickers (`.T`), that means JPY per 10,000 units (基準価額),
                    // which lines up directly with the ¥/万口 column — no conversion needed.
                    let price = crate::engine::market_data::MarketDataService::fetch_current_price(&ticker);
                    let cagr  = crate::engine::market_data::MarketDataService::fetch_10y_cagr(&ticker);
                    let fund = &mut state.accounts[ai].dc_funds[fi];
                    fund.price_per_10k = format!("{:.0}", price);
                    fund.growth_pct    = format!("{:.1}", cagr * 100.0);
                    log::info!("[DC Auto-Fetch] {}: price ¥{:.0}/万口, CAGR {:.2}%", ticker, price, cagr * 100.0);
                }
            }
        }
        if let Some((ai, pi)) = auto_fill_profile {
            if ai < state.accounts.len() && pi < state.accounts[ai].positions.len() {
                let ticker    = state.accounts[ai].positions[pi].ticker.clone();
                let class_str = state.accounts[ai].positions[pi].asset_class.clone();
                if !ticker.is_empty() {
                    let (show_cap, show_nav, show_cg, show_er) = match class_str.as_str() {
                        "Stock"      => (true,  false, false, false),
                        "ETF"        => (true,  false, true,  true ),
                        "MutualFund" => (false, true,  true,  true ),
                        "Other"      => (true,  true,  true,  true ),
                        _            => (true,  false, false, false),
                    };
                    let profile = crate::engine::market_data::MarketDataService::fetch_detailed_profile(&ticker, show_er);
                    let pos = &mut state.accounts[ai].positions[pi];
                    pos.dividend_yield_pct = format!("{:.3}", profile.dividend_yield * 100.0);
                    if show_cap { pos.cap_growth_pct     = format!("{:.3}", profile.cap_growth     * 100.0); }
                    if show_nav { pos.nav_growth_pct     = format!("{:.3}", profile.nav_growth     * 100.0); }
                    if show_cg  { pos.cap_gains_dist_pct = format!("{:.3}", profile.cap_gains_dist * 100.0); }
                    if show_er {
                        use crate::engine::market_data::ExpenseRatioSource;
                        match profile.expense_ratio_source {
                            ExpenseRatioSource::Unavailable => {
                                log::warn!(
                                    "[ExpenseRatio] {}: no live source and no fallback — kept your existing value '{}'.",
                                    ticker, pos.expense_ratio_pct,
                                );
                            }
                            _ => {
                                pos.expense_ratio_pct = format!("{:.3}", profile.expense_ratio * 100.0);
                                log::info!(
                                    "[ExpenseRatio] {}: applied {:.3}% ({})",
                                    ticker, profile.expense_ratio * 100.0,
                                    profile.expense_ratio_source.label(),
                                );
                            }
                        }
                    }
                    if matches!(class_str.as_str(), "MutualFund" | "Other") {
                        log::warn!(
                            "[MarketData] {}: Yahoo does not expose interest_yield, special_dist, \
                             or roc — those fields were left as-is. Edit manually if needed.",
                            ticker
                        );
                    }
                    pos.use_detailed_profile = true;
                }
            }
        }

        if ui.button("+ Add Account").clicked() {
            state.accounts.push(InvestmentAccountRow::default());
        }
        ui.add_space(8.0);

        // ── Family Demographics ──────────────────────────────────────────────────
        section(ui, "Family Demographics");
        ui.label(RichText::new(
            "User and (optional) spouse birthdates drive senior deduction add-ons, FERS/SS/Nenkin start ages, \
             and Spouse SS / Spouse Nenkin eligibility (V6.6). Dependent children carry full birthdates.").small().color(Color32::GRAY));
        ui.add_space(4.0);

        grid(ui, "g_family", |ui| {
            vfield_tt(ui, "User Birthday",  &mut state.user_birth_date,   "YYYY-MM-DD", true,
                "Primary retiree's full birth date. Drives age-based eligibility (FERS, SS, NHI nursing 40-64, IRS senior add-on at 65).");
        });

        ui.add_space(4.0);
        ui.checkbox(&mut state.is_married, "Married — include spouse demographics & entitlements")
            .on_hover_text("V6.6: enables spouse birthday, Spouse SS, Spouse Nenkin, and a second IRS senior add-on at 65.");
        if state.is_married {
            grid(ui, "g_spouse", |ui| {
                vfield_tt(ui, "Spouse Birthday", &mut state.spouse_birth_date, "YYYY-MM-DD", true,
                    "Spouse's full birth date. Triggers second senior std-deduction add-on at 65 and gates Spouse SS / Nenkin start ages.");
            });

            // ── Stage 02: Spouse Tax Profile ─────────────────────────────────────
            ui.add_space(4.0);
            ui.label(RichText::new("Spouse Tax Profile").strong())
                .on_hover_text(
                    "Stage 02: NRA Spouse Profiles\n\n\
                     If your spouse is a Japanese citizen without a Green Card (a Non-Resident Alien \
                     or NRA under IRS rules), selecting the right profile can change your US tax bill \
                     by tens of thousands of dollars.\n\n\
                     US Person: both spouses are US citizens or Lawful Permanent Residents.\n\
                     NRA Elected MFJ (§6013(g)): pools the NRA spouse's global Japan income onto the \
                     US return — higher std deduction and larger FTC pool, but Japan salary / Nenkin \
                     become US-taxable.\n\
                     NRA Married Filing Separately: keeps the spouse's Japan income out of the US \
                     return. Lower std deduction; Roth IRA contributions are effectively disallowed \
                     (MAGI phase-out $0–$10k).\n\
                     NRA Head of Household: available when a qualifying US-citizen child lives with \
                     you. Uses HoH brackets and deduction."
                );
            let profile_label = format!("{}", state.spouse_profile);
            egui::ComboBox::from_id_salt("spouse_profile_combo")
                .selected_text(&profile_label)
                .show_ui(ui, |ui| {
                    let profiles = [
                        (SpouseProfile::UsPerson,
                         "US Person (default) — both spouses are US citizens or LPRs.",
                         ""),
                        (SpouseProfile::NraElectedToBeTreatedAsResident,
                         "NRA — Elected MFJ (§6013(g)) — Japanese-citizen spouse; global incomes pooled.",
                         "Higher std deduction and FTC pool. Japan salary & Nenkin become US-taxable."),
                        (SpouseProfile::NraMfs,
                         "NRA — Married Filing Separately — Japan income out of US return.",
                         "Lower std deduction; Roth IRA disallowed (MAGI phase-out $0–$10k)."),
                        (SpouseProfile::NraHeadOfHouseholdEligible,
                         "NRA — Head of Household eligible (qualifying US-citizen child present).",
                         "HoH brackets and deduction; spouse's Japan income excluded from US return."),
                    ];
                    for (profile, label, note) in &profiles {
                        let selected = state.spouse_profile == *profile;
                        let resp = ui.selectable_label(selected, *label);
                        let clicked = resp.clicked();
                        if !note.is_empty() {
                            resp.on_hover_text(*note);
                        }
                        if clicked {
                            state.spouse_profile = *profile;
                        }
                    }
                });

            // Why this matters expander
            egui::CollapsingHeader::new("Why this matters")
                .id_salt("nra_spouse_why")
                .show(ui, |ui| {
                    ui.label(RichText::new(
                        "IRS §6013(g) Election (NRA Elected MFJ):\n\
                         Filing jointly with an NRA spouse is allowed but triggers a \
                         one-time election that brings all of the NRA spouse's worldwide income \
                         into the US return — including Japan salary, Nenkin, and rental income. \
                         This increases the FTC pool (more Japan tax to credit) but can also push \
                         you into a higher bracket if the NRA spouse earns significant yen income.\n\n\
                         Married Filing Separately (NRA-MFS):\n\
                         Keeps the NRA spouse's Japan income completely outside the US return. \
                         However, the standard deduction is halved ($14,600 vs $35,000 for MFJ) \
                         and the Roth IRA contribution phase-out drops from $236k–$246k to \
                         $0–$10k, effectively eliminating Roth contributions for any working \
                         professional."
                    ).small().color(Color32::GRAY));
                });

            // Spouse Japan income fields (only for §6013(g) path)
            if state.spouse_profile == SpouseProfile::NraElectedToBeTreatedAsResident {
                ui.add_space(4.0);
                ui.label(RichText::new(
                    "Enter the NRA spouse's Japan income below. \
                     These amounts are added to the US return as ordinary income and the \
                     Japan resident tax paid on them is credited via the FTC."
                ).small().color(Color32::from_rgb(255, 220, 100)));
                grid(ui, "g_spouse_japan", |ui| {
                    vfield_tt(ui, "Spouse Japan Salary (JPY/yr)",
                        &mut state.spouse_japan_salary_jpy, "e.g. 8000000", true,
                        "Annual Japan employment income earned by the NRA spouse (yen). \
                         Converted to USD at the simulation FX rate and added to gross ordinary income.");
                    vfield_tt(ui, "Spouse Japan Misc Income (JPY/yr)",
                        &mut state.spouse_japan_misc_income_jpy, "e.g. 0", false,
                        "Other Japan-source income (freelance, rental, etc.) earned by the NRA spouse. \
                         Pooled with spouse salary on the US §6013(g) return.");
                });
            }
            ui.add_space(4.0);
        }
        ui.add_space(6.0);

        ui.label(RichText::new("Dependent Children").strong());
        let mut remove_dep: Option<usize> = None;
        for (dep_idx, dep) in state.dependents.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Birthday:").small());
                ui.add(egui::TextEdit::singleline(&mut dep.birth_date)
                    .id_salt(("dep_date", dep_idx))
                    .hint_text("YYYY-MM-DD")
                    .desired_width(110.0))
                    .on_hover_text("Full birthdate (V6.6). Drives VA child rider cutoff at the exact 18th/23rd birthday.");
                ui.checkbox(&mut dep.is_college_student, RichText::new("College Student (VA eligible to age 23)").small());
                if ui.small_button("✕").on_hover_text("Remove this dependent").clicked() {
                    remove_dep = Some(dep_idx);
                }
            });
        }
        if let Some(idx) = remove_dep {
            state.dependents.remove(idx);
        }
        if ui.small_button("+ Add Dependent").clicked() {
            state.dependents.push(DependentEntry::default());
        }
        ui.add_space(6.0);

        // ── Stage 03: Monthly Dependent Precision toggle ─────────────────────────
        ui.checkbox(&mut state.monthly_dependent_precision,
            RichText::new("Monthly dependent-eligibility precision (Recommended)").strong())
            .on_hover_text(
                "Stage 03: Recalculates VA dependent add-ons, NHI per-capita charges, and \
                 Jido Teate every month using exact birthdays.\n\n\
                 Disable to fall back to the legacy annual-bucket approximation, which can \
                 mis-fund the bridge fund by 5–7 months in a transition year."
            );
        // Show upcoming drop-off read-out when any dependent is within 2 years of turning 18.
        for dep in &state.dependents {
            if let Ok(birth) = chrono::NaiveDate::parse_from_str(dep.birth_date.trim(), "%Y-%m-%d") {
                let cutoff_yr = birth.year() + 18;
                if let Some(cutoff) = chrono::NaiveDate::from_ymd_opt(cutoff_yr, birth.month(), birth.day()) {
                    ui.label(RichText::new(format!(
                        "Upcoming dependent drop-off: Child turns 18 on {} \
                         (loses VA child add-on, NHI per-capita head, Jido Teate).",
                        cutoff.format("%Y-%m-%d"),
                    )).small().color(egui::Color32::from_rgb(255, 200, 80)));
                }
            }
        }
        ui.add_space(8.0);

        // ── VA Disability Profile ────────────────────────────────────────────────
        section(ui, "VA Disability Profile");
        ui.label(RichText::new("(Using Official 2026 VA Rates)").weak().small());
        ui.label(RichText::new(
            "VA disability compensation is tax-free (US federal, state, and Japan resident \
             tax per US-Japan Tax Treaty Art. 19).").small().color(Color32::GRAY));
        ui.add_space(4.0);

        ui.label(RichText::new("Disability Rating").strong());
        let rating_ok = !errors.contains("va_disability_rating");
        let va_selected_text = if state.va_disability_rating == "0" {
            "0% — No VA Disability ($0.00)".to_string()
        } else {
            format!("{}%", state.va_disability_rating)
        };
        egui::ComboBox::from_id_salt("va_rating_combo")
            .selected_text(va_selected_text)
            .show_ui(ui, |ui| {
                for &rating in ALL_VA_RATINGS {
                    let label = if rating == 0 { "0% — No VA Disability ($0.00)".to_string() } else { format!("{}%", rating) };
                    let selected = state.va_disability_rating == rating.to_string();
                    if ui.selectable_label(selected, &label).clicked() {
                        state.va_disability_rating = rating.to_string();
                    }
                }
            });
        if !rating_ok {
            ui.label(RichText::new("⚠ Invalid rating").color(Color32::RED).small());
        }
        ui.add_space(4.0);

        ui.label(RichText::new("Dependent Status").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut state.va_dependent_status, VaDependentStatus::VetOnly,            "Vet Only");
            ui.radio_value(&mut state.va_dependent_status, VaDependentStatus::WithSpouse,         "Married");
            ui.radio_value(&mut state.va_dependent_status, VaDependentStatus::WithSpouseAndChild, "Married + Child");
        });
        ui.add_space(4.0);

        if let Ok(r) = state.va_disability_rating.parse::<u32>() {
            if r > 0 && !state.va_override_enabled {
                let monthly = lookup_va_monthly_2026(r, state.va_dependent_status);
                ui.label(RichText::new(format!(
                    "2026 Base Monthly Benefit: ${:.2}/mo  (inflated by COLA each year)", monthly
                )).color(Color32::from_rgb(100, 220, 100)).strong());
            }
        }
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            ui.checkbox(&mut state.va_override_enabled, "Override VA Monthly Amount");
            if state.va_override_enabled {
                ui.label(RichText::new("→").color(Color32::GRAY));
                ui.add(egui::TextEdit::singleline(&mut state.va_override_monthly)
                    .hint_text("e.g. 4267.28").desired_width(120.0));
                ui.label(RichText::new("USD/mo (2026 base)").small().color(Color32::GRAY));
            }
        });
        if state.va_override_enabled {
            ui.label(RichText::new("Override bypasses the rating table. Value is treated as 2026 base and inflated by COLA.")
                .small().color(Color32::from_rgb(255, 200, 50)));
        }
        ui.add_space(4.0);

        ui.label(RichText::new("Special Monthly Compensation (SMC)").strong());
        ui.label(RichText::new(
            "SMC-K adds to the base rate. All other variants replace it. All SMC is tax-free."
        ).small().color(Color32::GRAY));
        let smc_text = if state.va_smc_variant.is_empty() { "None — No SMC".to_string() } else { state.va_smc_variant.clone() };
        egui::ComboBox::from_id_salt("smc_combo")
            .selected_text(smc_text)
            .show_ui(ui, |ui| {
                if ui.selectable_label(state.va_smc_variant.is_empty(), "None — No SMC").clicked() {
                    state.va_smc_variant = "".into();
                }
                for &(label, variant) in ALL_SMC_VARIANTS {
                    let rate    = lookup_smc_monthly_2026(variant);
                    let display = format!("{} — ${:.2}/mo", label, rate);
                    let selected = state.va_smc_variant == label;
                    if ui.selectable_label(selected, &display).clicked() {
                        state.va_smc_variant = label.into();
                    }
                }
            });
        if !state.va_smc_variant.is_empty() && !state.smc_override_enabled {
            if let Some(entry) = ALL_SMC_VARIANTS.iter().find(|e| e.0 == state.va_smc_variant.as_str()) {
                let rate = lookup_smc_monthly_2026(entry.1);
                ui.label(RichText::new(format!(
                    "SMC-{}: ${:.2}/mo (2026 base, inflated by COLA each year)", state.va_smc_variant, rate
                )).color(Color32::from_rgb(100, 220, 100)).strong());
            }
        }
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut state.smc_override_enabled, "Override SMC Monthly Amount");
            if state.smc_override_enabled {
                ui.label(RichText::new("→").color(Color32::GRAY));
                ui.add(egui::TextEdit::singleline(&mut state.smc_override_monthly)
                    .hint_text("e.g. 4543.29").desired_width(120.0));
                ui.label(RichText::new("USD/mo (2026 base)").small().color(Color32::GRAY));
            }
        });
        if state.smc_override_enabled {
            ui.label(RichText::new("SMC override is treated as additive (K-style). Bypasses variant lookup.")
                .small().color(Color32::from_rgb(255, 200, 50)));
        }
        ui.add_space(8.0);

        // ── FERS Pension ─────────────────────────────────────────────────────────
        section(ui, "FERS Pension");
        grid(ui, "g_fers", |ui| {
            vfield(ui, "FERS Monthly (USD)",      &mut state.fers_monthly_usd, "e.g. 794.55 or 0 / N/A", !errors.contains("fers_monthly_usd"));
            vfield(ui, "FERS Expected Start Age", &mut state.fers_start_age,   "e.g. 62",                 !errors.contains("fers_start_age"));
        });
        ui.label(RichText::new("Enter 0 or N/A to disable FERS. Start age required only when FERS > 0.")
            .small().color(Color32::GRAY));
        ui.add_space(4.0);
        ui.label(RichText::new("Tax Jurisdiction").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut state.fers_jurisdiction, TaxProtocol::Both,      "Both");
            ui.radio_value(&mut state.fers_jurisdiction, TaxProtocol::UsOnly,    "US Only");
            ui.radio_value(&mut state.fers_jurisdiction, TaxProtocol::JapanOnly, "Japan Only");
            ui.radio_value(&mut state.fers_jurisdiction, TaxProtocol::TaxFree,   "Tax Free");
        });
        ui.checkbox(&mut state.fers_japan_local_tax_exempt,
            "Treaty Art. 18 — Exempt from Japan resident tax (jumin-zei)")
            .on_hover_text(
                "Article 18 of the US-Japan Tax Treaty exempts US government \
                 pensions (FERS) from Japan's local resident tax (住民税). \
                 Check this box to exclude FERS from the Japan resident-tax base. \
                 National income tax treatment is governed separately by the \
                 Tax Jurisdiction setting above."
            );
        ui.add_space(8.0);

        // ── Military Retired Pay ─────────────────────────────────────────────────
        section(ui, "Military Retired Pay");
        ui.label(RichText::new(
            "Military retired pay is taxable under the US-Japan Tax Treaty savings clause. Set to 0 to disable."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);
        grid(ui, "g_mil", |ui| {
            vfield(ui, "Military Monthly (USD)", &mut state.military_monthly_usd, "e.g. 2500 or 0", true);
        });
        ui.label(RichText::new("Tax Jurisdiction").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut state.military_jurisdiction, TaxProtocol::Both,      "Both");
            ui.radio_value(&mut state.military_jurisdiction, TaxProtocol::UsOnly,    "US Only");
            ui.radio_value(&mut state.military_jurisdiction, TaxProtocol::JapanOnly, "Japan Only");
            ui.radio_value(&mut state.military_jurisdiction, TaxProtocol::TaxFree,   "Tax Free");
        });
        ui.add_space(8.0);

        // ── Social Security ──────────────────────────────────────────────────────
        section(ui, "US Social Security (Totalization Pillar)");
        ui.label(RichText::new(
            "SS is subject to the US Savings Clause — taxable in the US and credited in Japan. Leave at $0 if not applicable."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);
        grid(ui, "g_ss", |ui| {
            vfield(ui, "SS Monthly Estimate (USD)", &mut state.ss_monthly_usd, "e.g. 1500 or 0 / N/A", !errors.contains("ss_monthly_usd"));
            vfield(ui, "SS Start Age",              &mut state.ss_start_age,   "default 67",            !errors.contains("ss_start_age"));
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Tax Jurisdiction").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut state.ss_jurisdiction, TaxProtocol::Both,      "Both");
            ui.radio_value(&mut state.ss_jurisdiction, TaxProtocol::UsOnly,    "US Only");
            ui.radio_value(&mut state.ss_jurisdiction, TaxProtocol::JapanOnly, "Japan Only");
            ui.radio_value(&mut state.ss_jurisdiction, TaxProtocol::TaxFree,   "Tax Free");
        });

        // ── Spouse SS (V6.6) ─────────────────────────────────────────────────
        if state.is_married {
            ui.add_space(6.0);
            ui.checkbox(&mut state.spouse_ss_enabled, "Spouse Social Security eligible")
                .on_hover_text("Adds spouse SS to monthly stats once spouse age >= start age.");
            if state.spouse_ss_enabled {
                grid(ui, "g_sp_ss", |ui| {
                    vfield_tt(ui, "Spouse SS Monthly (USD)", &mut state.spouse_ss_monthly_usd, "e.g. 1200", true,
                        "Spouse's monthly SS benefit estimate (USD).");
                    vfield_tt(ui, "Spouse SS Start Age",     &mut state.spouse_ss_start_age,   "default 67", true,
                        "Age (in spouse years) at which Spouse SS payments begin.");
                });
                ui.label(RichText::new("Spouse SS Tax Jurisdiction").strong());
                ui.horizontal(|ui| {
                    ui.radio_value(&mut state.spouse_ss_jurisdiction, TaxProtocol::Both,      "Both");
                    ui.radio_value(&mut state.spouse_ss_jurisdiction, TaxProtocol::UsOnly,    "US Only");
                    ui.radio_value(&mut state.spouse_ss_jurisdiction, TaxProtocol::JapanOnly, "Japan Only");
                    ui.radio_value(&mut state.spouse_ss_jurisdiction, TaxProtocol::TaxFree,   "Tax Free");
                });
            }
        }
        ui.add_space(8.0);

        // ── SSDI ─────────────────────────────────────────────────────────────────
        section(ui, "SSDI — Social Security Disability Insurance");
        ui.label(RichText::new(
            "SSDI is taxed via the IRS Combined Income rule (up to 85% taxable above $44K MFJ). \
             For Japan resident tax, it is routed through the public pension deduction (公的年金等控除). \
             At age 65 it reclassifies as SS retirement — amount is unchanged. Leave at $0 if not applicable."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);
        grid(ui, "g_ssdi", |ui| {
            vfield(ui, "SSDI Monthly (USD)", &mut state.ssdi_monthly_usd, "e.g. 1200 or 0 / N/A", !errors.contains("ssdi_monthly_usd"));
        });
        ui.add_space(8.0);

        // ── Nenkin ───────────────────────────────────────────────────────────────
        section(ui, "Japanese Nenkin Income (Totalization Pillar)");
        ui.label(RichText::new(
            "Nenkin pension income (separate from contribution expenses). Leave at ¥0 if not applicable."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);
        grid(ui, "g_nenkin_inc", |ui| {
            vfield(ui, "Nenkin Monthly Income (JPY)", &mut state.nenkin_income_monthly_jpy, "e.g. 100000 or 0 / N/A", !errors.contains("nenkin_income_monthly_jpy"));
            vfield(ui, "Nenkin Start Age",            &mut state.nenkin_income_start_age,   "default 65",             !errors.contains("nenkin_income_start_age"));
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Tax Jurisdiction").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut state.nenkin_jurisdiction, TaxProtocol::Both,      "Both");
            ui.radio_value(&mut state.nenkin_jurisdiction, TaxProtocol::UsOnly,    "US Only");
            ui.radio_value(&mut state.nenkin_jurisdiction, TaxProtocol::JapanOnly, "Japan Only");
            ui.radio_value(&mut state.nenkin_jurisdiction, TaxProtocol::TaxFree,   "Tax Free");
        });

        // ── Spouse Nenkin (V6.6) ─────────────────────────────────────────────
        if state.is_married {
            ui.add_space(6.0);
            ui.checkbox(&mut state.spouse_nenkin_enabled, "Spouse Nenkin eligible")
                .on_hover_text("Adds spouse Nenkin to monthly stats once spouse age >= start age.");
            if state.spouse_nenkin_enabled {
                grid(ui, "g_sp_nenkin", |ui| {
                    vfield_tt(ui, "Spouse Nenkin Monthly (JPY)", &mut state.spouse_nenkin_monthly_jpy, "e.g. 65000", true,
                        "Spouse's monthly Nenkin pension estimate (JPY).");
                    vfield_tt(ui, "Spouse Nenkin Start Age",     &mut state.spouse_nenkin_start_age,   "default 65", true,
                        "Age (in spouse years) at which Spouse Nenkin payments begin.");
                });
                ui.label(RichText::new("Spouse Nenkin Tax Jurisdiction").strong());
                ui.horizontal(|ui| {
                    ui.radio_value(&mut state.spouse_nenkin_jurisdiction, TaxProtocol::Both,      "Both");
                    ui.radio_value(&mut state.spouse_nenkin_jurisdiction, TaxProtocol::UsOnly,    "US Only");
                    ui.radio_value(&mut state.spouse_nenkin_jurisdiction, TaxProtocol::JapanOnly, "Japan Only");
                    ui.radio_value(&mut state.spouse_nenkin_jurisdiction, TaxProtocol::TaxFree,   "Tax Free");
                });
            }
        }
        ui.add_space(8.0);

        // ── Financial Buffers ────────────────────────────────────────────────────
        section(ui, "Financial Buffers");
        ui.label(RichText::new(
            "Cash buffers protect your portfolio from forced liquidation during market downturns. \
             Choose which buffers to fund at retirement based on your situation.")
            .small().color(Color32::GRAY));
        ui.add_space(4.0);

        // ── War Chest ────────────────────────────────────────────────────────
        ui.checkbox(&mut state.war_chest_enabled,
            "Enable War Chest (JPY Emergency Reserve)")
            .on_hover_text(
                "A JPY cash reserve for tax bills, NHI true-ups, and equity-shock recovery. \
                 Sits at Tier 3 in the withdrawal waterfall — tapped only when JPY income \
                 and dividends can't cover monthly expenses.\n\n\
                 Disable if you already hold sufficient JPY cash outside this model, or if \
                 you live in a non-JPY economy.");
        if state.war_chest_enabled {
            grid(ui, "g_wc", |ui| {
                vfield_tt(ui, "Target Amount (JPY)", &mut state.war_chest_target_jpy,
                    "e.g. 7000000", !errors.contains("war_chest_target_jpy"),
                    "Total JPY you want held in the war chest at retirement.");
                vfield_tt(ui, "Already Set Aside (JPY)", &mut state.pre_funded_war_chest_jpy,
                    "e.g. 3000000", !errors.contains("pre_funded_war_chest_jpy"),
                    "JPY you have already earmarked for the war chest (savings account, envelope, etc.). \
                     The model will only liquidate portfolio shares to cover the gap: Target minus this amount.");
            });
        }
        ui.add_space(6.0);

        // ── Bridge Fund ──────────────────────────────────────────────────────
        ui.checkbox(&mut state.bridge_fund_enabled,
            "Enable Bridge Fund (USD Operating Liquidity)")
            .on_hover_text(
                "A USD cash buffer covering several months of expenses. Sits at Tier 6 in the \
                 withdrawal waterfall — drawn after JPY sources are exhausted, before \
                 belt-tightening or stock liquidation.\n\n\
                 The target is calculated as: (monthly expense shortfall) x (months you specify). \
                 Disable if your guaranteed income (VA, FERS, pension) covers your expenses from day one.");
        if state.bridge_fund_enabled {
            grid(ui, "g_bridge", |ui| {
                vfield_tt(ui, "Months of Expenses", &mut state.bridge_months,
                    "e.g. 12", !errors.contains("bridge_months"),
                    "How many months of expense shortfall to hold in USD cash. The model calculates \
                     the dollar target based on your expenses minus guaranteed income.");
                vfield_tt(ui, "Already Set Aside (USD)", &mut state.pre_funded_bridge_usd,
                    "e.g. 25000", !errors.contains("pre_funded_bridge_usd"),
                    "USD you have already earmarked for the bridge fund. The model will only \
                     liquidate shares to cover the gap.");
            });
        }
        ui.add_space(8.0);

        // ── Family Financial Planning (V7.3 Education / V7.5 Gift Sink) ──────────
        section(ui, "Family Financial Planning (Optional)");
        ui.label(RichText::new(
            "Education funding and inter-generational gifting are optional. Leave the \
             toggles off if you don't plan to fund schooling or pass money to heirs.")
            .small().color(Color32::GRAY));
        ui.add_space(4.0);

        ui.checkbox(&mut state.education_fund_enabled,
            "Fund Education (Tier 2.5 — dedicated JPY bucket for school costs)")
            .on_hover_text("Skims JPY surplus into a separate fund that pays Education-tagged expenses BEFORE the main waterfall.");
        if state.education_fund_enabled {
            grid(ui, "g_edu", |ui| {
                vfield_tt(ui, "Monthly Skim (JPY)",
                    &mut state.edu_savings_jpy_monthly,
                    "e.g. 50000",
                    true,
                    "Monthly JPY skim from post-spend surplus into the Education Fund. \
                     Skim is opportunistic — only taken when surplus exists.");
            });
        }
        ui.add_space(6.0);

        ui.checkbox(&mut state.gift_sink_enabled,
            "Annual Gift to Heirs (Tier 9 — 暦年贈与 estate-planning sink)")
            .on_hover_text("Diverts a fixed annual amount per recipient out of the spendable pool in December. Modeled against the JP 暦年贈与 exclusion and flagged against the US §2503(b) limit.");
        if state.gift_sink_enabled {
            grid(ui, "g_gift", |ui| {
                vfield_tt(ui, "JPY per Recipient / Year",
                    &mut state.annual_gift_jpy_per_recipient,
                    "e.g. 1100000",
                    true,
                    "Annual JPY gift per recipient. ¥1,100,000 is the Japan 暦年贈与 exclusion.");
                vfield_tt(ui, "Number of Recipients",
                    &mut state.gift_recipient_count,
                    "e.g. 2",
                    true,
                    "Count of distinct recipients (children, grandchildren, etc.).");
                vfield_tt(ui, "US §2503(b) Exclusion (USD)",
                    &mut state.us_gift_exclusion_usd,
                    "19000",
                    true,
                    "US annual gift-tax exclusion per donor-recipient pair (2026 = $19,000). \
                     Per-recipient gifts above this trigger a US reporting flag.");
            });
        }
        ui.add_space(8.0);

        // ── Estate Planning (Stage 07) ────────────────────────────────────────────
        section(ui, "Estate Planning");
        ui.label(RichText::new(
            "Japan taxes inheritance heavily — up to 55%. If you're a long-term resident, \
             your global assets are in scope. Use this to project the bill your heirs will \
             face and to plan annual gifting under the ¥1.1M / $19k exclusion."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);
        ui.checkbox(
            &mut state.enable_estate_planning,
            "Project Estate / Inheritance Tax",
        ).on_hover_text(
            "At the end of the simulation horizon (or at a user-set death date), compute \
             the Japan Sōzoku-zei and US Estate Tax liability on your remaining global assets, \
             apply the US-Japan treaty credit, and show the net wealth that transfers to heirs."
        );
        if state.enable_estate_planning {
            ui.add_space(4.0);
            grid(ui, "g_estate_dates", |ui| {
                vfield_tt(ui, "Death Date (optional)",
                    &mut state.death_date,
                    "YYYY-MM-DD or leave blank",
                    false,
                    "Optional date for the estate projection. Leave blank to use the simulation end date.");
                vfield_tt(ui, "Spouse Death Date (optional)",
                    &mut state.spouse_death_date,
                    "YYYY-MM-DD or leave blank",
                    false,
                    "Informational — used to apply the spousal ½ deduction (配偶者の税額軽減).");
            });
            ui.add_space(4.0);
            ui.label(RichText::new("Heirs").strong());
            ui.label(RichText::new(
                "Add each heir below. The spousal ½ deduction is automatically applied when \
                 a Spouse heir is listed."
            ).small().color(Color32::GRAY));
            ui.add_space(2.0);

            let mut to_remove: Option<usize> = None;
            for (idx, heir) in state.estate_heirs.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut heir.name)
                        .hint_text("Name")
                        .desired_width(110.0));
                    ui.add(egui::TextEdit::singleline(&mut heir.birth_date)
                        .hint_text("Birth date")
                        .desired_width(100.0));
                    egui::ComboBox::from_id_salt(format!("heir_rel_{idx}"))
                        .selected_text(heir.relationship.to_string())
                        .width(90.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut heir.relationship, HeirRelationship::Spouse, "Spouse");
                            ui.selectable_value(&mut heir.relationship, HeirRelationship::Child,  "Child");
                            ui.selectable_value(&mut heir.relationship, HeirRelationship::Other,  "Other");
                        });
                    if ui.small_button("✕").clicked() {
                        to_remove = Some(idx);
                    }
                });
            }
            if let Some(i) = to_remove { state.estate_heirs.remove(i); }
            if ui.small_button("+ Add heir").clicked() {
                state.estate_heirs.push(HeirEntry::default());
            }

            ui.add_space(4.0);
            ui.checkbox(
                &mut state.enable_gifting_optimiser,
                "Use Lifetime Gifting Optimiser (Rough guidance — not legal advice)",
            ).on_hover_text(
                "Suggests how much to pre-gift each year using the ¥1.1M Japan 暦年贈与 and \
                 $19k US §2503(b) annual exclusions, and estimates the resulting reduction \
                 in Sōzoku-zei. Requires the Gift Sink (below) to be configured."
            );
        }
        ui.add_space(8.0);

        // ── Market Simulation ────────────────────────────────────────────────────
        // V6.6: FX Drift moved into the Economics section (under USD/JPY).
        section(ui, "Market Simulation");
        ui.checkbox(&mut state.recession_enabled, "Simulate Recession at Retirement");
        if state.recession_enabled {
            grid(ui, "g_rec", |ui| {
                vfield(ui, "Recession Severity", &mut state.recession_severity, "e.g. 0.20", true);
            });
        }
        ui.add_space(4.0);
        grid(ui, "g_rec_years", |ui| {
            vfield(ui, "Dynamic Recession Events", &mut state.recession_years, r#"e.g. "2027:0.20, 2035:0.15""#, true);
        });
        ui.label(RichText::new("Format: YEAR:SEVERITY pairs, comma-separated.").small().color(Color32::GRAY));
        ui.add_space(8.0);

        // ── Shock Application Order (Stage 04) ───────────────────────────────────
        ui.label(RichText::new("Shock Application Order").strong());
        ui.label(RichText::new(
            "How to apply a recession and FX shock that fall in the same calendar year."
        ).small().color(Color32::GRAY));
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if ui.radio(
                state.shock_ordering == ShockOrdering::DepreciateThenReprice,
                "Equity drop first, then FX repricing (default, conservative)",
            ).on_hover_text(
                "Conservative — equity drops first at the old FX rate. \
                 JPY purchasing-power loss is shown at its largest."
            ).clicked() {
                state.shock_ordering = ShockOrdering::DepreciateThenReprice;
            }
        });
        ui.horizontal(|ui| {
            if ui.radio(
                state.shock_ordering == ShockOrdering::RepriceThenDepreciate,
                "FX repricing first, then equity drop",
            ).on_hover_text(
                "Optimistic — FX moves first; equity loss is denominated in \
                 the new FX, which may look smaller in JPY terms."
            ).clicked() {
                state.shock_ordering = ShockOrdering::RepriceThenDepreciate;
            }
        });
        ui.horizontal(|ui| {
            if ui.radio(
                state.shock_ordering == ShockOrdering::Simultaneous,
                "Simultaneous (snapshot both, commit together)",
            ).on_hover_text(
                "Path-independent — both shocks are computed against a \
                 pre-shock snapshot and committed together. Recommended for \
                 stress-test comparability."
            ).clicked() {
                state.shock_ordering = ShockOrdering::Simultaneous;
            }
        });
        ui.add_space(4.0);
        egui::CollapsingHeader::new("ℹ What does ordering do?")
            .id_salt("shock_ordering_expander")
            .show(ui, |ui| {
                ui.label(RichText::new(
                    "Example: $100k in VTI, FX 145 → 80, recession −35%\n\
                     \n\
                     Option A (Equity first): $100k → $65k (at ¥145) = ¥9,425,000 → FX → ¥5,200,000\n\
                     Option B (FX first):    $100k → ¥8,000,000 (at ¥80) → −35% → ¥5,200,000\n\
                     Option C (Simultaneous): ¥14,500,000 → ¥5,200,000 (no intermediate)\n\
                     \n\
                     All three end at the same ¥5.2M — but the intermediate audit value \
                     (shown in the Annual Table tooltip for shock years) differs. Option A \
                     makes the combined loss look largest; Option B shows a smaller apparent \
                     equity loss because it is already denominated in the stronger yen."
                ).small().color(Color32::from_rgb(200, 200, 200)));
            });
        ui.add_space(8.0);

        // ── RSU Settings ─────────────────────────────────────────────────────────
        section(ui, "RSU Settings");
        ui.label(RichText::new("RSU Tax Handling").strong());
        ui.horizontal(|ui| {
            let is_salary = state.rsu_tax_handling == "SALARY";
            if ui.radio(is_salary,  "SALARY — withheld from paycheck").clicked() {
                state.rsu_tax_handling = "SALARY".into();
            }
            if ui.radio(!is_salary, "SELL_TO_COVER — sell shares at vest").clicked() {
                state.rsu_tax_handling = "SELL_TO_COVER".into();
            }
        });
        if state.rsu_tax_handling == "SELL_TO_COVER" {
            ui.add_space(4.0);
            let tooltip = "When a scheduled recession drops the share price below the combined \
                US + Japan tax owed at vest, your broker can't collect the tax from the vest \
                alone (margin call). The simulator will sell more shares, drain the Bridge Fund \
                and War Chest, and flag an unpaid IRS liability instead of silently zeroing the \
                bill. Disable to keep the legacy \"best case\" behaviour.";
            ui.horizontal(|ui| {
                ui.checkbox(&mut state.rsu_sell_to_cover_realism,
                    "☑ Model RSU Tax-Liability Margin Calls (Recommended)")
                    .on_hover_text(tooltip);
            });
            ui.add_space(4.0);
        }
        ui.add_space(8.0);

        ui.label(RichText::new("RSU Awards").strong());
        ui.label(RichText::new(
            "One row per grant tranche. Vest value is calculated from current ticker price × \
             projected CAGR — no grant price required."
        ).small().color(Color32::GRAY));
        ui.add_space(4.0);

        if !state.rsu_awards.is_empty() {
            let mut remove_rsu:           Option<usize> = None;
            let mut rsu_auto_fill_simple: Option<usize> = None;
            let mut rsu_toggle_profile:   Option<usize> = None;
            let num_rsu = state.rsu_awards.len();
            egui::Grid::new("g_rsu_awards")
                .num_columns(12)
                .striped(true)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Ticker").strong().small());
                    ui.label(RichText::new("Grant Date").strong().small());
                    ui.label(RichText::new("Units").strong().small());
                    ui.label(RichText::new("Months Total").strong().small());
                    ui.label(RichText::new("Vest Months").strong().small())
                        .on_hover_text("Comma-separated months (1–12) for vesting events, e.g. 2,5,8,11 for quarterly");
                    ui.label(RichText::new("Cliff (M)").strong().small())
                        .on_hover_text("Cliff period in months. Shares accumulate and vest in a lump on the first event after the cliff ends.");
                    ui.label(RichText::new("Delayed Init").strong().small())
                        .on_hover_text("Delayed initial vest: first vest event is shifted to grant month + 12 months instead of the first matching calendar month.");
                    ui.label(RichText::new("✨").strong().small())
                        .on_hover_text("Auto-fill Price & Capital Appreciation % from Yahoo Finance (10-year price CAGR, dividends NOT reinvested).");
                    ui.label(RichText::new("Price USD").strong().small())
                        .on_hover_text("Starter price for the underlying. Used only if this ticker is NOT held in brokerage. Once vested, the engine drives the price forward.");
                    ui.label(RichText::new("Capital Appreciation %").strong().small())
                        .on_hover_text(
                            "Annual price-only change in market value (negative = capital depreciation). \
                             Excludes dividend payments and other distributions. \
                             DRIP does not affect this number — it only decides whether dividends buy more shares. \
                             Falls back to the global default if blank."
                        );
                    ui.label(RichText::new("Return Profile").strong().small())
                        .on_hover_text("Toggle a Capital Appreciation + Dividend Payments component breakdown for this RSU's underlying stock.");
                    ui.label(""); // remove button
                    ui.end_row();

                    for rsu_idx in 0..num_rsu {
                        ui.add(egui::TextEdit::singleline(&mut state.rsu_awards[rsu_idx].ticker)
                            .hint_text("e.g. PANW").desired_width(60.0)
                            .id(egui::Id::new("rtk").with(rsu_idx)));
                        ui.add(egui::TextEdit::singleline(&mut state.rsu_awards[rsu_idx].grant_date)
                            .hint_text("YYYY-MM-DD").desired_width(100.0)
                            .id(egui::Id::new("rgd").with(rsu_idx)));
                        ui.add(egui::TextEdit::singleline(&mut state.rsu_awards[rsu_idx].units_awarded)
                            .hint_text("e.g. 150").desired_width(60.0)
                            .id(egui::Id::new("rua").with(rsu_idx)));
                        ui.add(egui::TextEdit::singleline(&mut state.rsu_awards[rsu_idx].months_to_finish_vesting)
                            .hint_text("e.g. 48").desired_width(60.0)
                            .id(egui::Id::new("rmf").with(rsu_idx)));
                        ui.add(egui::TextEdit::singleline(&mut state.rsu_awards[rsu_idx].specific_vesting_months)
                            .hint_text("e.g. 2,5,8,11").desired_width(100.0)
                            .id(egui::Id::new("rsm").with(rsu_idx)));
                        ui.add(egui::TextEdit::singleline(&mut state.rsu_awards[rsu_idx].cliff_vest_months)
                            .hint_text("0").desired_width(44.0)
                            .id(egui::Id::new("rcv").with(rsu_idx)));
                        ui.checkbox(&mut state.rsu_awards[rsu_idx].delayed_initial_vest, "");
                        if ui.small_button("✨")
                            .on_hover_text("Auto-fill Price & Capital Appreciation % from Yahoo Finance (10-year price CAGR, dividends NOT reinvested).")
                            .clicked()
                        {
                            rsu_auto_fill_simple = Some(rsu_idx);
                        }
                        ui.add(egui::TextEdit::singleline(&mut state.rsu_awards[rsu_idx].unit_value)
                            .hint_text("current $").desired_width(74.0)
                            .id(egui::Id::new("ruv").with(rsu_idx)));
                        ui.add(egui::TextEdit::singleline(&mut state.rsu_awards[rsu_idx].growth_pct)
                            .hint_text("opt. %").desired_width(60.0)
                            .id(egui::Id::new("rgp").with(rsu_idx)))
                            .on_hover_text(
                                "Annual capital appreciation % (price-only). Negative = capital depreciation. \
                                 Excludes dividend payments and other distributions."
                            );
                        let prof_label = if state.rsu_awards[rsu_idx].use_detailed_profile {
                            if state.rsu_awards[rsu_idx].profile_expanded { "📊 Detail ▾" } else { "📊 Detail" }
                        } else {
                            "Simple"
                        };
                        if ui.small_button(prof_label)
                            .on_hover_text("Toggle Capital Appreciation + Dividend Payments component breakdown.")
                            .clicked()
                        {
                            rsu_toggle_profile = Some(rsu_idx);
                        }
                        if ui.small_button("✕").clicked() {
                            remove_rsu = Some(rsu_idx);
                        }
                        ui.end_row();
                    }
                });
            if let Some(idx) = remove_rsu {
                state.rsu_awards.remove(idx);
            }

            // V7.7 — Per-RSU Detail (return profile) sub-panels.
            let mut rsu_auto_fill_profile: Option<usize> = None;
            for rsu_idx in 0..num_rsu {
                if !state.rsu_awards[rsu_idx].profile_expanded { continue; }
                let row = &mut state.rsu_awards[rsu_idx];
                Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(80, 50, 120, 40))
                    .inner_margin(egui::Margin::same(6.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!(
                                "📊 {} Return Profile  (RSU — single stock)",
                                if row.ticker.is_empty() { "[unset]" } else { row.ticker.as_str() }
                            )).small().strong());
                            ui.add_space(10.0);
                            ui.checkbox(&mut row.use_detailed_profile, "Use detailed return profile")
                                .on_hover_text("When on, the engine seeds the post-vest Asset's capital appreciation and dividend payments from the component fields below.");
                            ui.add_space(10.0);
                            if ui.small_button("✨ Auto-Fetch")
                                .on_hover_text(
                                    "Fetch Capital Appreciation (10y price CAGR, dividends NOT reinvested) \
                                     and Dividend Payments (TTM yield) from Yahoo Finance. Single-stock RSUs \
                                     do not have expense ratios or capital-gains distributions."
                                )
                                .clicked()
                            {
                                rsu_auto_fill_profile = Some(rsu_idx);
                            }
                        });
                        ui.label(RichText::new(
                            "Annual percentages. NAV appreciation, interest income, capital-gains \
                             distributions, expense ratio, return of capital, and special distributions \
                             do not apply to single-stock awards."
                        ).small().color(Color32::GRAY));
                        let enabled = row.use_detailed_profile;
                        ui.add_enabled_ui(enabled, |ui| {
                            egui::Grid::new(egui::Id::new("g_rsu_profile").with(rsu_idx))
                                .num_columns(4)
                                .spacing([16.0, 4.0])
                                .show(ui, |ui| {
                                    ui.label(RichText::new("Capital Appreciation %:").small().strong())
                                        .on_hover_text("Annual change in the market price of the stock (price-only). A negative value represents capital depreciation. Excludes dividend payments.");
                                    ui.add(egui::TextEdit::singleline(&mut row.cap_growth_pct)
                                        .hint_text("e.g. 12.0").desired_width(64.0)
                                        .id(egui::Id::new("rcg").with(rsu_idx)));
                                    ui.label(RichText::new("Dividend Payments %:").small().strong())
                                        .on_hover_text("Cash distributions paid to shareholders from company earnings (annual yield). DRIP on → reinvested into more shares; DRIP off → paid out as cash.");
                                    ui.add(egui::TextEdit::singleline(&mut row.dividend_yield_pct)
                                        .hint_text("e.g. 1.2").desired_width(64.0)
                                        .id(egui::Id::new("rdy").with(rsu_idx)));
                                    ui.end_row();
                                });
                        });
                        let f = |s: &str| s.trim().parse::<f64>().unwrap_or(0.0);
                        let cg  = f(&row.cap_growth_pct);
                        let div = f(&row.dividend_yield_pct);
                        ui.add_space(2.0);
                        ui.label(RichText::new(format!(
                            "Total Return ≈ {:.2}%  (capital appreciation {:+.2}%  +  dividend payments {:.2}%)",
                            cg + div, cg, div
                        )).small().color(Color32::from_rgb(140, 200, 240)));
                    });
            }

            // Deferred mutations for RSU rows.
            if let Some(idx) = rsu_auto_fill_simple {
                if idx < state.rsu_awards.len() {
                    let ticker = state.rsu_awards[idx].ticker.clone();
                    if !ticker.is_empty() {
                        let price = crate::engine::market_data::MarketDataService::fetch_current_price(&ticker);
                        let cagr  = crate::engine::market_data::MarketDataService::fetch_10y_cagr(&ticker);
                        let row = &mut state.rsu_awards[idx];
                        row.unit_value = format!("{:.2}", price);
                        row.growth_pct = format!("{:.1}", cagr * 100.0);
                    }
                }
            }
            if let Some(idx) = rsu_toggle_profile {
                if idx < state.rsu_awards.len() {
                    let row = &mut state.rsu_awards[idx];
                    if row.profile_expanded {
                        row.profile_expanded = false;
                    } else {
                        row.profile_expanded = true;
                        row.use_detailed_profile = true;
                    }
                }
            }
            if let Some(idx) = rsu_auto_fill_profile {
                if idx < state.rsu_awards.len() {
                    let ticker = state.rsu_awards[idx].ticker.clone();
                    if !ticker.is_empty() {
                        // RSUs are single stocks — no expense ratio. Skip the fundProfile call.
                        let profile = crate::engine::market_data::MarketDataService::fetch_detailed_profile(&ticker, false);
                        let row = &mut state.rsu_awards[idx];
                        row.cap_growth_pct     = format!("{:.3}", profile.cap_growth     * 100.0);
                        row.dividend_yield_pct = format!("{:.3}", profile.dividend_yield * 100.0);
                        row.use_detailed_profile = true;
                    }
                }
            }
        } else {
            ui.label(RichText::new("No RSU awards. Click '+ Add RSU Award' to add a tranche.")
                .small().color(Color32::GRAY));
        }

        if ui.small_button("+ Add RSU Award").clicked() {
            state.rsu_awards.push(RsuRow::default());
        }
        ui.add_space(16.0);
    }); // ScrollArea
}

// ─── Save helper ─────────────────────────────────────────────────────────────

fn save_configuration(state: &InputPanelState) -> Option<Result<std::path::PathBuf, std::io::Error>> {
    let json = state.build_save_json()?;
    let path = rfd::FileDialog::new()
        .set_title("Save Configuration As")
        .set_file_name("scenario_edited.json")
        .add_filter("JSON Scenario", &["json"])
        .save_file()?;

    let text = match serde_json::to_string_pretty(&json) {
        Ok(s)  => s,
        Err(e) => return Some(Err(std::io::Error::new(std::io::ErrorKind::Other, e))),
    };

    Some(std::fs::write(&path, text).map(|_| path))
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Maps an account type display string to its JSON holdings key.
fn account_json_key(account_type: &str) -> &'static str {
    match account_type {
        "Taxable Brokerage" => "taxable",
        "IRA (Traditional)" => "ira",
        "Roth IRA"          => "roth_ira",
        "401(k)"            => "k401",
        "DC Plan"           => "japan_dc",
        "NISA"              => "nisa",
        "iDeCo"             => "ideco",
        _                   => "taxable",
    }
}

/// Maps an account type display string to the simulation engine account key
/// (used in AccumulationRule.account and state.accounts HashMap keys).
fn account_sim_key(account_type: &str) -> &'static str {
    match account_type {
        "Taxable Brokerage" => "Taxable",
        "IRA (Traditional)" => "IRA",
        "Roth IRA"          => "Roth",
        "401(k)"            => "k401",
        "DC Plan"           => "DC",
        "NISA"              => "NISA",
        "iDeCo"             => "iDeCo",
        _                   => "Taxable",
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title).strong().size(13.0));
    ui.separator();
    ui.add_space(2.0);
}

fn grid(ui: &mut Ui, id: &str, add_rows: impl FnOnce(&mut Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([24.0, 5.0])
        .show(ui, add_rows);
}

fn vfield(ui: &mut Ui, label: &str, value: &mut String, hint: &str, valid: bool) {
    ui.label(RichText::new(format!("{}:", label)).strong());
    if valid {
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(220.0));
    } else {
        Frame::none()
            .fill(Color32::from_rgba_unmultiplied(180, 30, 30, 60))
            .inner_margin(egui::Margin::same(2.0))
            .stroke(egui::Stroke::new(1.5, Color32::from_rgb(220, 60, 60)))
            .show(ui, |ui| {
                ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(216.0));
            });
    }
    ui.end_row();
}

/// V6.6: vfield variant with an inline tooltip.
fn vfield_tt(ui: &mut Ui, label: &str, value: &mut String, hint: &str, valid: bool, tooltip: &str) {
    ui.label(RichText::new(format!("{}:", label)).strong()).on_hover_text(tooltip);
    let resp = if valid {
        ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(220.0))
    } else {
        let mut inner = None;
        Frame::none()
            .fill(Color32::from_rgba_unmultiplied(180, 30, 30, 60))
            .inner_margin(egui::Margin::same(2.0))
            .stroke(egui::Stroke::new(1.5, Color32::from_rgb(220, 60, 60)))
            .show(ui, |ui| {
                inner = Some(ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(216.0)));
            });
        inner.unwrap()
    };
    resp.on_hover_text(tooltip);
    ui.end_row();
}

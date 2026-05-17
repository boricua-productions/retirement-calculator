use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Geographic jurisdiction for tax routing and depreciation schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PropertyLocation {
    #[default]
    Japan,
    Us,
    International,
}

/// Property use classification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PropertyType {
    #[default]
    Primary,
    Rental,
    Inherited,
    Vacation,
}

/// Building structure type — determines depreciation useful life.
/// Japan: wood 22 yr, RC 47 yr, steel 34 yr.
/// US:    residential 27.5 yr, commercial 39 yr.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StructureType {
    Wood,
    #[default]
    ReinforcedConcrete,
    Steel,
    Other,
}

/// Currency denomination of the mortgage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MortgageCurrency {
    #[default]
    Jpy,
    Usd,
}

/// Fixed-rate amortizing mortgage terms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MortgageTerms {
    /// Original loan amount in the mortgage's currency.
    pub original_principal: f64,
    /// Annual interest rate (e.g. 0.01 = 1%).
    pub annual_rate: f64,
    /// Total amortization term in months (e.g. 360 = 30 years).
    pub term_months: u32,
    /// Loan origination date — used to compute elapsed months.
    pub start_date: NaiveDate,
    #[serde(default)]
    pub currency: MortgageCurrency,
}

/// A HELOC (Home Equity Line of Credit) attached to a property.
/// The HELOC fires as Tier 7.5 in the defensive waterfall — only when
/// `cfg.enable_heloc_tier` is true AND this line's `enabled` flag is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HelocLine {
    /// Maximum credit limit in USD (convert JPY FMV at current FX if needed).
    pub credit_line_usd: f64,
    /// Annual interest rate on outstanding balance (informational; not yet
    /// modeled as a monthly ongoing cost in this stage).
    pub draw_rate: f64,
    /// LTV ceiling: HELOC freezes when combined LTV exceeds this (e.g. 0.80).
    pub ltv_cap: f64,
    /// User has explicitly enabled this HELOC for waterfall use.
    pub enabled: bool,
}

/// Reverse mortgage — one-shot user election, not auto-fired.
/// Available for users ≥ 62 (US) or ≥ 60 (Japan リバースモーゲージ).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReverseMortgageTerms {
    /// Gross maximum proceeds in the property's native currency.
    pub max_proceeds_local: f64,
    /// True once the user has elected to draw.
    pub elected: bool,
}

/// Rental income and operating-cost profile for income-generating properties.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RentalProfile {
    /// Gross monthly rent in JPY (Japan properties).
    #[serde(default)]
    pub monthly_rent_jpy: f64,
    /// Gross monthly rent in USD (US / international properties).
    #[serde(default)]
    pub monthly_rent_usd: f64,
    /// Annual vacancy fraction (e.g. 0.05 = 5%).
    pub vacancy_pct: f64,
    /// Annual property insurance in JPY.
    #[serde(default)]
    pub annual_insurance_jpy: f64,
    /// Annual property insurance in USD.
    #[serde(default)]
    pub annual_insurance_usd: f64,
    /// Annual maintenance / repairs as a fraction of FMV (e.g. 0.01 = 1%).
    pub annual_repairs_pct_fmv: f64,
}

/// A single real-estate holding — primary residence, rental, inherited, or vacation.
///
/// When `cfg.real_estate` is empty this struct is never instantiated, and the
/// engine behaves identically to the pre-Stage-06 baseline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealEstateHolding {
    pub name: String,
    #[serde(default)]
    pub location: PropertyLocation,
    #[serde(default)]
    pub property_type: PropertyType,
    /// Building material — drives depreciation schedule for both Japan and US tax.
    #[serde(default)]
    pub structure_type: StructureType,
    /// Original acquisition date (used for depreciation start year).
    pub purchase_date: Option<NaiveDate>,
    /// Purchase price in JPY; 0 if not applicable.
    #[serde(default)]
    pub purchase_price_jpy: f64,
    /// Purchase price in USD; 0 if not applicable.
    #[serde(default)]
    pub purchase_price_usd: f64,
    /// Current fair market value in JPY.
    #[serde(default)]
    pub current_fmv_jpy: f64,
    /// Current fair market value in USD.
    #[serde(default)]
    pub current_fmv_usd: f64,
    /// Annual Kotei Shisanzei + Toshikeikaku-zei (typically ~1.7 % of assessed value).
    #[serde(default)]
    pub annual_property_tax_jpy: f64,
    /// Annual US property tax (state-specific).
    #[serde(default)]
    pub annual_property_tax_usd: f64,
    /// Optional first mortgage.
    pub mortgage: Option<MortgageTerms>,
    /// Optional HELOC line.  Enabled only when `cfg.enable_heloc_tier = true`.
    pub heloc: Option<HelocLine>,
    /// Optional reverse mortgage terms.
    pub reverse_mortgage: Option<ReverseMortgageTerms>,
    /// Optional rental operating profile.
    pub rental: Option<RentalProfile>,
}

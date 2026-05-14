use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// How frequently shares vest within each vesting year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VestingCadence {
    /// Vests 4× per year (Feb/May/Aug/Nov by default). Default behaviour.
    #[default]
    Quarterly,
    /// Vests 12× per year (1st of every month).
    Monthly,
    /// Vests once per year on the anniversary month of the grant or start date.
    Annually,
}

impl VestingCadence {
    /// Returns the canonical list of vesting months for this cadence.
    /// Used when `vesting_months` is not explicitly provided in the JSON.
    pub fn default_months(&self) -> Vec<u32> {
        match self {
            VestingCadence::Quarterly => vec![2, 5, 8, 11],
            VestingCadence::Monthly   => (1..=12).collect(),
            VestingCadence::Annually  => vec![1],
        }
    }
}

impl std::fmt::Display for VestingCadence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VestingCadence::Quarterly => write!(f, "Quarterly"),
            VestingCadence::Monthly   => write!(f, "Monthly"),
            VestingCadence::Annually  => write!(f, "Annually"),
        }
    }
}

/// Represents a single grant of Restricted Stock Units (RSUs).
/// Mirrors Python's `RSUAward` dataclass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsuAward {
    /// The date the award was granted (used as the clock reference for vesting years).
    pub grant_date: NaiveDate,
    /// Optional explicit vesting start date. When set, vesting events are computed
    /// relative to this date rather than `grant_date`.
    #[serde(default)]
    pub vesting_start_date: Option<NaiveDate>,
    pub ticker: String,
    pub total_shares: f64,
    pub vesting_years: u32,
    /// Total vesting duration in months. When set, takes precedence over `vesting_years`.
    /// Allows sub-year-granularity schedules (e.g. 36 months instead of 3 years).
    #[serde(default)]
    pub vesting_months_total: Option<u32>,
    /// The specific calendar months (1–12) within each vesting year when shares vest.
    /// If empty and `vesting_cadence` is set, the cadence's default months are used.
    #[serde(default)]
    pub vesting_months: Vec<u32>,
    /// Vesting cadence: Monthly, Quarterly, or Annually.
    /// Only used when `vesting_months` is empty.
    #[serde(default)]
    pub vesting_cadence: VestingCadence,
    /// Months from clock_origin to the first actual vest date. 0 = no cliff.
    /// Shares that would have vested during the cliff period accumulate and are
    /// delivered all at once on the first event on or after the cliff end date.
    #[serde(default)]
    pub cliff_vest_months: u32,
    /// V7.7 — Optional starter price (USD/share). Used on first vest if no
    /// brokerage Taxable asset for this ticker already exists.
    #[serde(default)]
    pub unit_value: Option<f64>,
    /// V7.7 — Optional annual growth rate (fraction, e.g. 0.10 = 10%).
    /// Seeds the Asset on first vest; thereafter the engine owns price growth.
    #[serde(default)]
    pub growth_rate: Option<f64>,
    /// V7.7 — Optional per-component return profile (cap_growth + dividend_yield
    /// in practice). Attached to the Taxable Asset on first vest if no profile
    /// is already set (brokerage holding wins).
    #[serde(default)]
    pub return_profile: Option<crate::models::assets::DetailedReturnProfile>,
    /// V7.7 — When true, unvested shares are forfeited at retirement (handled by
    /// the RSU engine's retirement_date cutoff) and the Taxable account's
    /// `rebalance_strategy` fires immediately post-transition to migrate the
    /// proceeds into target allocations.
    #[serde(default)]
    pub migrate_on_retirement: bool,
}

impl RsuAward {
    /// Returns the effective vesting months: explicit list if provided, otherwise
    /// derived from the vesting cadence.
    pub fn effective_vesting_months(&self) -> Vec<u32> {
        if !self.vesting_months.is_empty() {
            self.vesting_months.clone()
        } else {
            self.vesting_cadence.default_months()
        }
    }

    /// Returns the effective clock reference date for vesting year calculations.
    pub fn effective_start_date(&self) -> NaiveDate {
        self.vesting_start_date.unwrap_or(self.grant_date)
    }
}

/// Represents a single vesting event: a specific date, share count, and ticker.
/// Mirrors Python's `VestingEvent` dataclass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VestingEvent {
    pub date: NaiveDate,
    pub shares: f64,
    pub ticker: String,
}

/// Summary of vested vs unvested shares for a single ticker.
#[derive(Debug, Clone, Default)]
pub struct VestStatus {
    pub vested: f64,
    pub unvested: f64,
}

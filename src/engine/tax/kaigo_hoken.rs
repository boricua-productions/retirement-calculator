use serde::{Deserialize, Serialize};

/// Long-Term Care Insurance (介護保険 / Kaigo Hoken) calculation engine.
///
/// Japan mandates Long-Term Care Insurance for all residents starting at age 40:
/// - **Ages 40–64 (第2号被保険者)**: premium is bundled into NHI (already modeled
///   in `nhi.rs` via the nursing_care_rate component).
/// - **Ages 65+ (第1号被保険者)**: premium is separate from NHI and computed against
///   pension income brackets set by the municipality (typically 9 tiers).
///
/// This module handles the age-65+ premium calculation and optional out-of-pocket
/// care cost projections.

/// Stage 10 — Care need scenario for optional out-of-pocket cost projection.
///
/// Drives the projected late-life care expenses beyond the insurance premium.
/// - `None`: premium only; no additional care costs projected.
/// - `Low`: light intermittent home help (~¥20k/month avg from age 75).
/// - `Medium`: regular home visits + occasional facility stays (~¥40k/month from age 75).
/// - `High`: intensive care assumption (~¥80k/month from age 80) — stress-test aid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CareScenario {
    #[default]
    None,
    Low,
    Medium,
    High,
}

impl std::fmt::Display for CareScenario {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CareScenario::None   => write!(f, "None (premium only)"),
            CareScenario::Low    => write!(f, "Low (~¥20k/month from age 75)"),
            CareScenario::Medium => write!(f, "Medium (~¥40k/month from age 75)"),
            CareScenario::High   => write!(f, "High (~¥80k/month from age 80)"),
        }
    }
}

/// Income bracket schedule for age-65+ Kaigo Hoken premium calculation.
///
/// Most municipalities use a 9-tier income-based schedule. The default values are
/// from Sagamihara City (相模原市), Kanagawa Prefecture, 2026 rate schedule.
///
/// **Source**: Sagamihara City FY2026 Long-Term Care Insurance Premium Notice
/// (相模原市 令和8年度 介護保険料のお知らせ)
/// <https://www.city.sagamihara.kanagawa.jp/kurashi/kenko/1026531/1007427.html>
///
/// The brackets are defined as `(upper_income_limit_jpy, annual_premium_jpy)`.
/// The final bracket has `f64::INFINITY` as the upper limit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KaigoHokenBrackets {
    /// 9-tier bracket schedule: `(upper_income_jpy, annual_premium_jpy)`.
    /// Income is annual pension income (公的年金等収入額).
    /// The final bracket should have `f64::INFINITY` as the limit.
    pub brackets: Vec<(f64, f64)>,
}

impl KaigoHokenBrackets {
    /// Sagamihara City (相模原市), Kanagawa — 2026 rate schedule.
    ///
    /// **Source**: Sagamihara City FY2026 Long-Term Care Insurance Premium Notice
    /// <https://www.city.sagamihara.kanagawa.jp/kurashi/kenko/1026531/1007427.html>
    ///
    /// 9-tier schedule (approximate — actual brackets may vary by household composition):
    /// - Tier 1: municipal tax non-payer (生活保護受給者等) → base rate ¥30,000/year
    /// - Tier 2-3: low income (年金収入 ≤ ¥1,200,000) → ¥45,000-¥60,000/year
    /// - Tier 4-6: middle income (¥1,200,000-¥3,000,000) → ¥75,000-¥95,000/year
    /// - Tier 7-8: upper-middle (¥3,000,000-¥5,000,000) → ¥110,000-¥130,000/year
    /// - Tier 9: high income (> ¥5,000,000) → ¥150,000+/year
    ///
    /// The brackets below are illustrative defaults matching typical Sagamihara values.
    /// Users should verify against their actual municipal notice.
    pub fn sagamihara_2026() -> Self {
        Self {
            brackets: vec![
                (800_000.0,    30_000.0),  // Tier 1: very low income
                (1_200_000.0,  45_000.0),  // Tier 2
                (1_800_000.0,  60_000.0),  // Tier 3
                (2_400_000.0,  75_000.0),  // Tier 4
                (3_000_000.0,  85_000.0),  // Tier 5
                (3_600_000.0,  95_000.0),  // Tier 6
                (4_200_000.0, 110_000.0),  // Tier 7
                (5_000_000.0, 130_000.0),  // Tier 8
                (f64::INFINITY, 150_000.0), // Tier 9: high income cap
            ],
        }
    }

    /// Nagoya City (名古屋市), Aichi — 2026 rate schedule (approximate).
    ///
    /// Nagoya typically has slightly lower premiums than Sagamihara in the mid-tiers.
    /// Values below are illustrative; consult Nagoya City's official notice for exact rates.
    pub fn nagoya_2026() -> Self {
        Self {
            brackets: vec![
                (800_000.0,    28_000.0),
                (1_200_000.0,  42_000.0),
                (1_800_000.0,  56_000.0),
                (2_400_000.0,  70_000.0),
                (3_000_000.0,  82_000.0),
                (3_600_000.0,  92_000.0),
                (4_200_000.0, 105_000.0),
                (5_000_000.0, 125_000.0),
                (f64::INFINITY, 145_000.0),
            ],
        }
    }
}

impl Default for KaigoHokenBrackets {
    fn default() -> Self {
        Self::sagamihara_2026()
    }
}

/// Compute the annual Kaigo Hoken premium for ages 65+.
///
/// # Arguments
/// - `annual_pension_jpy`: Total annual pension income (公的年金等収入額).
///   This includes Nenkin, FERS (if pensioned), Social Security, etc. converted to JPY.
/// - `brackets`: The municipality's bracket schedule.
///
/// # Returns
/// Annual premium in JPY.
///
/// # Example
/// ```
/// use retirement_calculator::engine::tax::kaigo_hoken::{calculate_age_65_plus_premium_annual, KaigoHokenBrackets};
///
/// let brackets = KaigoHokenBrackets::sagamihara_2026();
/// // Retiree with ¥2M annual pension income falls into Tier 4 → ¥75,000/year
/// let premium = calculate_age_65_plus_premium_annual(2_000_000.0, &brackets);
/// assert_eq!(premium, 75_000.0);
/// ```
pub fn calculate_age_65_plus_premium_annual(
    annual_pension_jpy: f64,
    brackets: &KaigoHokenBrackets,
) -> f64 {
    // Find the first bracket where income <= upper_limit.
    for &(upper_limit, premium) in &brackets.brackets {
        if annual_pension_jpy <= upper_limit {
            return premium;
        }
    }
    // Fallback: if no bracket matched, return the last bracket's premium.
    // This shouldn't happen if the final bracket has f64::INFINITY.
    brackets.brackets.last().map(|&(_, p)| p).unwrap_or(0.0)
}

/// Project the monthly out-of-pocket care cost based on age and care scenario.
///
/// This is an **optional** projection for users who want a high-realism long-tail
/// scenario. The insurance premium is mandatory; actual care draws are probabilistic.
///
/// # Arguments
/// - `age`: Current age of the retiree.
/// - `scenario`: Care need scenario (None / Low / Medium / High).
///
/// # Returns
/// Monthly out-of-pocket cost in JPY (in addition to the insurance premium).
///
/// # Care Scenario Assumptions
/// - `None`: ¥0/month (premium only).
/// - `Low`: ¥20,000/month starting at age 75 (light intermittent home help).
/// - `Medium`: ¥40,000/month starting at age 75 (regular home visits + occasional facility).
/// - `High`: ¥80,000/month starting at age 80 (intensive care — stress-test aid).
///
/// These are illustrative average out-of-pocket costs after insurance coverage.
/// Actual costs depend on care level (要介護1-5), facility type, and municipality.
pub fn projected_out_of_pocket_care(age: i32, scenario: CareScenario) -> f64 {
    match scenario {
        CareScenario::None => 0.0,
        CareScenario::Low => {
            if age >= 75 { 20_000.0 } else { 0.0 }
        }
        CareScenario::Medium => {
            if age >= 75 { 40_000.0 } else { 0.0 }
        }
        CareScenario::High => {
            if age >= 80 { 80_000.0 } else { 0.0 }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sagamihara_brackets_tier_1() {
        let brackets = KaigoHokenBrackets::sagamihara_2026();
        // Income ¥600,000 → Tier 1 → ¥30,000/year
        let premium = calculate_age_65_plus_premium_annual(600_000.0, &brackets);
        assert_eq!(premium, 30_000.0);
    }

    #[test]
    fn test_sagamihara_brackets_tier_4() {
        let brackets = KaigoHokenBrackets::sagamihara_2026();
        // Income ¥2,000,000 → Tier 4 → ¥75,000/year
        let premium = calculate_age_65_plus_premium_annual(2_000_000.0, &brackets);
        assert_eq!(premium, 75_000.0);
    }

    #[test]
    fn test_sagamihara_brackets_tier_9() {
        let brackets = KaigoHokenBrackets::sagamihara_2026();
        // Income ¥6,000,000 → Tier 9 → ¥150,000/year
        let premium = calculate_age_65_plus_premium_annual(6_000_000.0, &brackets);
        assert_eq!(premium, 150_000.0);
    }

    #[test]
    fn test_nagoya_brackets_lower_than_sagamihara_mid_tier() {
        let sag = KaigoHokenBrackets::sagamihara_2026();
        let nag = KaigoHokenBrackets::nagoya_2026();
        // Income ¥2,000,000 → Sagamihara ¥75k, Nagoya ¥70k
        let sag_premium = calculate_age_65_plus_premium_annual(2_000_000.0, &sag);
        let nag_premium = calculate_age_65_plus_premium_annual(2_000_000.0, &nag);
        assert!(nag_premium < sag_premium, "Nagoya should be lower in mid-tiers");
    }

    #[test]
    fn test_care_scenario_none_is_zero() {
        assert_eq!(projected_out_of_pocket_care(70, CareScenario::None), 0.0);
        assert_eq!(projected_out_of_pocket_care(85, CareScenario::None), 0.0);
    }

    #[test]
    fn test_care_scenario_low_starts_at_75() {
        assert_eq!(projected_out_of_pocket_care(74, CareScenario::Low), 0.0);
        assert_eq!(projected_out_of_pocket_care(75, CareScenario::Low), 20_000.0);
        assert_eq!(projected_out_of_pocket_care(80, CareScenario::Low), 20_000.0);
    }

    #[test]
    fn test_care_scenario_medium_starts_at_75() {
        assert_eq!(projected_out_of_pocket_care(74, CareScenario::Medium), 0.0);
        assert_eq!(projected_out_of_pocket_care(75, CareScenario::Medium), 40_000.0);
    }

    #[test]
    fn test_care_scenario_high_starts_at_80() {
        assert_eq!(projected_out_of_pocket_care(79, CareScenario::High), 0.0);
        assert_eq!(projected_out_of_pocket_care(80, CareScenario::High), 80_000.0);
        assert_eq!(projected_out_of_pocket_care(85, CareScenario::High), 80_000.0);
    }
}

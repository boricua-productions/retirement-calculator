use crate::models::config::TaxRules;
use std::collections::HashMap;

/// 2026 Foreign Earned Income Exclusion annual limit (USD).
pub const FEIE_LIMIT_2026: f64 = 126_500.0;

/// US federal tax calculation result.
#[derive(Debug, Default, Clone)]
pub struct TaxLiability {
    /// Total federal + state tax owed (STCG + LTCG brackets + NIIT + state).
    pub total_tax: f64,
    /// Detailed breakdown keyed by component name.
    pub breakdown: HashMap<String, f64>,
    /// US state tax component (for FTC credit separation).
    pub state_tax: f64,
    /// Foreign Tax Credit applied (Japan resident taxes credited against US liability).
    pub ftc_applied: f64,
    /// Amount of earned income excluded via FEIE (0 if FEIE was not applied).
    pub feie_exclusion: f64,
    /// True when the FEIE+FTC path produced lower tax than FTC-only.
    pub feie_applied: bool,
}

/// V7.5 — Defect 1.2: IRC §904 FTC basket separation.
///
/// PFIC MTM income (§1296) is passive basket income under §904(d)(1)(B).
/// FERS, SS, SSDI, RSU vest value are general basket income.
/// Separating baskets prevents PFIC-generated FTC from spuriously absorbing
/// Japan tax credit that legally belongs to a different basket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FtcBasket {
    /// Dividends, capital gains, PFIC MTM income.
    Passive,
    /// FERS pension, Social Security, SSDI, RSU vest value.
    General,
}

/// Compute the per-basket §904 FTC limit and return (passive_limit, general_limit).
///
/// `ftc_limit_per_basket = US_tax_before_ftc × (basket_foreign_income / total_taxable_income)`
pub fn compute_ftc_basket_limits(
    federal_before_ftc: f64,
    passive_foreign_income: f64,
    general_foreign_income: f64,
) -> (f64, f64) {
    let total = passive_foreign_income + general_foreign_income;
    if total <= 0.0 {
        return (0.0, 0.0);
    }
    let passive_limit = federal_before_ftc * (passive_foreign_income / total);
    let general_limit = federal_before_ftc * (general_foreign_income / total);
    (passive_limit, general_limit)
}

/// Calculates US federal tax on investment income (capital gains + NIIT).
/// Mirrors Python's `TaxEngine` class in `tax_engine.py`.
///
/// NOTE: Ordinary income is used as a "floor" for bracket placement — gains are
/// stacked on top. This accurately models the situation of a US expat whose ordinary
/// income is excluded via FEIE but whose capital gains are still taxed.
pub struct TaxEngine {
    pub rules: TaxRules,
}

impl TaxEngine {
    pub fn new(rules: TaxRules) -> Self {
        Self { rules }
    }

    /// Calculate the US tax liability on investment income with optional FTC.
    ///
    /// # Arguments
    /// * `_year` — simulation year (unused, kept for API parity)
    /// * `gross_ord` — gross ordinary income (FERS, RSU vest value) as bracket floor
    /// * `gross_st_cap` — short-term capital gains (taxed at ordinary rates)
    /// * `gross_lt_cap` — long-term capital gains / qualified dividends
    /// * `japan_tax_paid_usd` — Japan resident tax already paid (in USD) for FTC credit
    pub fn calculate_liability(
        &self,
        _year: i32,
        gross_ord: f64,
        gross_st_cap: f64,
        gross_lt_cap: f64,
    ) -> TaxLiability {
        self.calculate_liability_with_ftc(_year, gross_ord, gross_st_cap, gross_lt_cap, 0.0)
    }

    /// Full calculation with Foreign Tax Credit support.
    pub fn calculate_liability_with_ftc(
        &self,
        _year: i32,
        gross_ord: f64,
        gross_st_cap: f64,
        gross_lt_cap: f64,
        japan_tax_paid_usd: f64,
    ) -> TaxLiability {
        let mut breakdown: HashMap<String, f64> = HashMap::from([
            ("gains_at_0_pct".into(), 0.0),
            ("gains_at_15_pct".into(), 0.0),
            ("gains_at_20_pct".into(), 0.0),
            ("niit_on_gains".into(), 0.0),
            ("state_tax".into(), 0.0),
            ("ftc_applied".into(), 0.0),
        ]);

        // ── 0. Ordinary income tax (FERS, SS, Military Retired Pay) ─────────────
        // Apply standard deduction first; stacked-bracket method on taxable remainder.
        let ord_taxable = (gross_ord - self.rules.std_deduction).max(0.0);
        let mut federal_tax = self.calc_ordinary_tax_on_stacked(0.0, ord_taxable);

        // Post-deduction ordinary income is the bracket floor for capital gains stacking.
        let mut floor = ord_taxable;

        // ── 1. Short-term capital gains — taxed at ordinary rates ──
        if gross_st_cap > 0.0 {
            let stcg_tax = self.calc_ordinary_tax_on_stacked(floor, gross_st_cap);
            federal_tax += stcg_tax;
            floor += gross_st_cap;
        }

        // ── 2. Long-term capital gains — 0% / 15% / 20% brackets ──
        let space_0 = (self.rules.ltcg_0_limit - floor).max(0.0);
        let amt_0 = gross_lt_cap.min(space_0);
        breakdown.insert("gains_at_0_pct".into(), amt_0);
        let rem_cap = gross_lt_cap - amt_0;

        if rem_cap > 0.0 {
            let curr_lvl = floor + amt_0;
            let space_15 = (self.rules.ltcg_15_limit - curr_lvl).max(0.0);
            let amt_15 = rem_cap.min(space_15);
            let amt_20 = (rem_cap - amt_15).max(0.0);

            breakdown.insert("gains_at_15_pct".into(), amt_15);
            breakdown.insert("gains_at_20_pct".into(), amt_20);
            federal_tax += amt_15 * 0.15 + amt_20 * 0.20;
        }

        // ── 3. Net Investment Income Tax (NIIT) ──
        let total_gains = gross_st_cap + gross_lt_cap;
        let magi = gross_ord + total_gains;
        if magi > self.rules.niit_threshold {
            let excess_magi = magi - self.rules.niit_threshold;
            let subj_niit = total_gains.min(excess_magi);
            let niit_tax = subj_niit * self.rules.niit_rate;
            federal_tax += niit_tax;
            breakdown.insert("niit_on_gains".into(), niit_tax);
        }

        let federal_before_ftc = federal_tax.max(0.0);

        // ── 4. Foreign Tax Credit (Japan-First FTC) ──
        // Credit Japan resident taxes paid against US federal liability.
        // FTC cannot exceed the US federal tax liability.
        let ftc_applied = japan_tax_paid_usd.min(federal_before_ftc);
        let federal_after_ftc = (federal_before_ftc - ftc_applied).max(0.0);
        breakdown.insert("ftc_applied".into(), ftc_applied);

        // ── 5. State income tax ──
        let state_tax = if self.rules.us_state_rate > 0.0 {
            // State tax applies to total investment income (gains + FERS floor)
            let state_taxable = (gross_ord + total_gains - self.rules.std_deduction).max(0.0);
            let s = state_taxable * self.rules.us_state_rate;
            breakdown.insert("state_tax".into(), s);
            s
        } else {
            0.0
        };

        TaxLiability {
            total_tax: (federal_after_ftc + state_tax).max(0.0),
            breakdown,
            state_tax,
            ftc_applied,
            feie_exclusion: 0.0,
            feie_applied: false,
        }
    }

    /// V7.6 — §904 basket-aware FTC. Splits Japan tax into a passive bucket
    /// (dividends, interest, cap-gains distributions, cap gains, PFIC §1296 MTM)
    /// and a general bucket (FERS, SS, SSDI, RSU vest), capping each at its
    /// own §904 limit so passive credit cannot bleed into the general basket
    /// and vice versa.
    ///
    /// `gross_ord_general` — FERS / SS / SSDI / RSU vest (general basket).
    /// `gross_ord_passive` — PFIC §1296 MTM + interest + special distributions.
    /// `gross_st_cap`      — short-term capital gains (passive).
    /// `gross_lt_cap`      — long-term capital gains + dividends (passive).
    /// `japan_tax_passive_usd` / `japan_tax_general_usd` — pre-split Japan tax.
    pub fn calculate_liability_with_basket_ftc(
        &self,
        year: i32,
        gross_ord_general: f64,
        gross_ord_passive: f64,
        gross_st_cap: f64,
        gross_lt_cap: f64,
        japan_tax_passive_usd: f64,
        japan_tax_general_usd: f64,
    ) -> TaxLiability {
        let total_ord = gross_ord_general + gross_ord_passive;

        // Step 1 — compute federal tax pre-FTC on the lumped income.
        let pre_ftc = self.calculate_liability(year, total_ord, gross_st_cap, gross_lt_cap);
        // Federal portion before state and before any FTC. Equal to total_tax - state_tax.
        let federal_before_ftc = (pre_ftc.total_tax - pre_ftc.state_tax).max(0.0);

        // Step 2 — per-basket §904 limits.
        let passive_income = gross_ord_passive + gross_st_cap + gross_lt_cap;
        let general_income = gross_ord_general;
        let (passive_lim, general_lim) =
            compute_ftc_basket_limits(federal_before_ftc, passive_income, general_income);

        // Step 3 — cap each basket's credit at its own §904 limit.
        let passive_ftc = japan_tax_passive_usd.min(passive_lim);
        let general_ftc = japan_tax_general_usd.min(general_lim);
        let total_ftc   = passive_ftc + general_ftc;

        let federal_after_ftc = (federal_before_ftc - total_ftc).max(0.0);

        let mut breakdown = pre_ftc.breakdown.clone();
        breakdown.insert("ftc_passive".into(), passive_ftc);
        breakdown.insert("ftc_general".into(), general_ftc);
        breakdown.insert("ftc_applied".into(), total_ftc);

        TaxLiability {
            total_tax: federal_after_ftc + pre_ftc.state_tax,
            breakdown,
            state_tax: pre_ftc.state_tax,
            ftc_applied: total_ftc,
            feie_exclusion: 0.0,
            feie_applied: false,
        }
    }

    /// Calculate using the FEIE → Federal Tax → FTC pipeline.
    ///
    /// Pipeline (IRS Form 2555 / IRC §904 compliant):
    ///   1. Subtract FEIE exclusion from **earned** ordinary income only (up to annual limit).
    ///      Pension income (FERS, SS, SSDI) is NOT FEIE-eligible — pass it as `gross_unearned`.
    ///   2. Calculate federal tax on the post-exclusion total (earned remainder + unearned + gains).
    ///   3. Apply Japan FTC, proportioned by `(total_japan_taxable − feie_exclusion) /
    ///      total_japan_taxable` so credits are disallowed only on the excluded share
    ///      (anti-double-dip, IRC §911(d)(6)).
    ///
    /// # Arguments
    /// * `gross_earned`   — FEIE-eligible earned income: salary, RSU vest value.
    /// * `gross_unearned` — Taxable but NOT FEIE-eligible: FERS pension, Social Security, SSDI.
    pub fn calculate_liability_with_feie_ftc(
        &self,
        year: i32,
        gross_earned: f64,
        gross_unearned: f64,
        gross_st_cap: f64,
        gross_lt_cap: f64,
        japan_tax_paid_usd: f64,
    ) -> TaxLiability {
        // Step 1 — FEIE: exclude up to annual limit from earned income only.
        let feie_exclusion = gross_earned.min(FEIE_LIMIT_2026);
        let earned_after_feie = gross_earned - feie_exclusion;
        let total_ord_after_feie = earned_after_feie + gross_unearned;

        // Step 2 — FTC apportionment (IRC §904 proportioning).
        // Denominator = total Japan-taxable income (earned + unearned + gains).
        // Only the excluded share is disallowed; gains/pension remain fully creditable.
        let total_japan_taxable = gross_earned + gross_unearned + gross_st_cap + gross_lt_cap;
        let ftc_ratio = if total_japan_taxable > 0.0 {
            (total_japan_taxable - feie_exclusion) / total_japan_taxable
        } else {
            1.0
        };
        let ftc_for_path = japan_tax_paid_usd * ftc_ratio;

        // Step 3 — full tax on post-exclusion ordinary + gains with FTC.
        let mut result = self.calculate_liability_with_ftc(
            year, total_ord_after_feie, gross_st_cap, gross_lt_cap, ftc_for_path,
        );
        result.feie_exclusion = feie_exclusion;
        result.feie_applied = feie_exclusion > 0.0;
        result
    }

    /// Calculate tax on `stcg` income stacked on top of an existing `floor` using ordinary
    /// income brackets.
    ///
    /// Invariant: `self.rules.brackets` MUST end with a `(f64::INFINITY, top_rate)`
    /// sentinel so income above the highest finite limit still gets taxed. If a
    /// caller-supplied `TaxRules` violates that contract, we synthesize a fallback
    /// using the last finite bracket's rate to avoid silent under-taxation.
    fn calc_ordinary_tax_on_stacked(&self, floor: f64, stcg: f64) -> f64 {
        let mut tax = 0.0;
        let mut current_income = floor;
        let mut remaining = stcg;
        let mut prev_limit = 0.0_f64;

        for &(limit, rate) in &self.rules.brackets {
            if remaining <= 0.0 {
                break;
            }
            if current_income >= limit {
                prev_limit = limit;
                continue;
            }
            let bracket_span = limit - current_income.max(prev_limit);
            let taxable = remaining.min(bracket_span);
            tax += taxable * rate;
            remaining -= taxable;
            current_income += taxable;
            prev_limit = limit;
        }

        // Sentinel-missing fallback: if any income remains after walking every
        // bracket, the table lacks the required `INFINITY` cap. Tax the residual
        // at the top finite rate so we don't under-collect.
        if remaining > 0.0 {
            if let Some(&(_, top_rate)) = self.rules.brackets.last() {
                tax += remaining * top_rate;
            }
        }

        tax
    }
}

// ─── SSDI Combined Income Tax Rule ────────────────────────────────────────────

/// Computes the taxable portion of annual SSDI income under the IRS Combined Income rule.
///
/// Formula (MFJ thresholds — identical for SSDI and SS retirement benefits):
///   provisional_income = AGI_before_SSDI + 0.5 × annual_ssdi
///   • PI ≤ $32K  → 0% of SSDI is taxable
///   • $32K < PI ≤ $44K → min(50% × (PI − $32K), 50% × annual_ssdi)
///   • PI > $44K  → min(85% × annual_ssdi, $6,000 + 85% × (PI − $44K))
///
/// Caller passes `provisional_income` (already includes the 0.5× SSDI term).
pub fn ssdi_combined_income_taxable_portion(provisional_income: f64, annual_ssdi: f64) -> f64 {
    if annual_ssdi <= 0.0 {
        return 0.0;
    }
    if provisional_income <= 32_000.0 {
        0.0
    } else if provisional_income <= 44_000.0 {
        ((provisional_income - 32_000.0) * 0.5).min(annual_ssdi * 0.5)
    } else {
        let tier1 = 6_000.0_f64; // 0.5 × min($12K spread, SSDI) = 0.5 × $12K = $6K (cap)
        let tier2 = (provisional_income - 44_000.0) * 0.85;
        (tier1 + tier2).min(annual_ssdi * 0.85)
    }
}

// ─── Filing-Status Tax Rules ───────────────────────────────────────────────────

impl TaxRules {
    /// Build 2024 IRS parameters for the given filing status.
    /// Brackets inflate annually via `TaxRules::inflate()`.
    pub fn for_filing_status(status: &str) -> TaxRules {
        match status {
            "Single" | "Married Filing Separately" => TaxRules {
                filing_status: status.into(),
                std_deduction: 14_600.0,
                ltcg_0_limit:  47_025.0,
                ltcg_15_limit: 518_900.0,
                niit_threshold: 200_000.0,
                senior_addon_per_person: 1_950.0,
                brackets: vec![
                    (11_600.0, 0.10),
                    (47_150.0, 0.12),
                    (100_525.0, 0.22),
                    (191_950.0, 0.24),
                    (243_725.0, 0.32),
                    (609_350.0, 0.35),
                    (f64::INFINITY, 0.37),
                ],
                ..TaxRules::default()
            },
            "Head of Household" => TaxRules {
                filing_status: status.into(),
                std_deduction: 21_900.0,
                ltcg_0_limit:  63_000.0,
                ltcg_15_limit: 551_350.0,
                niit_threshold: 200_000.0,
                senior_addon_per_person: 1_950.0,
                brackets: vec![
                    (16_550.0, 0.10),
                    (63_100.0, 0.12),
                    (100_500.0, 0.22),
                    (191_950.0, 0.24),
                    (243_700.0, 0.32),
                    (609_350.0, 0.35),
                    (f64::INFINITY, 0.37),
                ],
                ..TaxRules::default()
            },
            // "Married Filing Jointly" and anything else → MFJ (the original default)
            _ => TaxRules::default(),
        }
    }
}

// ─── US State Income Tax Rates ────────────────────────────────────────────────

/// Returns the state income tax rate (flat or representative marginal rate) for
/// a US state given its two-letter postal code.
///
/// Rates are approximate 2024 values. Zero-income-tax states return 0.0.
/// Graduated-rate states return the rate applicable to moderate retirement income
/// (~$50 000–$100 000 AGI).
pub fn state_tax_rate(code: &str) -> f64 {
    match code {
        // No income tax
        "AK" | "FL" | "NV" | "NH" | "SD" | "TN" | "TX" | "WA" | "WY" => 0.0,
        // Flat-rate states
        "CO" => 0.044,
        "IL" => 0.0495,
        "IN" => 0.0305,
        "KY" => 0.040,
        "MA" => 0.050,
        "MI" => 0.0425,
        "NC" => 0.045,
        "PA" => 0.0307,
        "UT" => 0.0465,
        // Graduated states — moderate income rate
        "AL" => 0.050,
        "AR" => 0.047,
        "AZ" => 0.025,
        "CA" => 0.093,
        "CT" => 0.050,
        "DC" => 0.085,
        "DE" => 0.066,
        "GA" => 0.055,
        "HI" => 0.079,
        "IA" => 0.060,
        "ID" => 0.058,
        "KS" => 0.057,
        "LA" => 0.042,
        "MD" => 0.055,
        "ME" => 0.075,
        "MN" => 0.0685,
        "MO" => 0.054,
        "MS" => 0.047,
        "MT" => 0.069,
        "NE" => 0.0664,
        "NJ" => 0.0637,
        "NM" => 0.059,
        "NY" => 0.0685,
        "OH" => 0.040,
        "OK" => 0.050,
        "OR" => 0.099,
        "RI" => 0.0599,
        "SC" => 0.065,
        "VA" => 0.0575,
        "VT" => 0.0875,
        "WI" => 0.0765,
        "WV" => 0.065,
        _ => 0.0,
    }
}

/// Returns the display name of a US state given its postal code.
pub fn state_display_name(code: &str) -> &'static str {
    match code {
        "AL" => "Alabama", "AK" => "Alaska", "AZ" => "Arizona",
        "AR" => "Arkansas", "CA" => "California", "CO" => "Colorado",
        "CT" => "Connecticut", "DC" => "DC", "DE" => "Delaware",
        "FL" => "Florida", "GA" => "Georgia", "HI" => "Hawaii",
        "ID" => "Idaho", "IL" => "Illinois", "IN" => "Indiana",
        "IA" => "Iowa", "KS" => "Kansas", "KY" => "Kentucky",
        "LA" => "Louisiana", "ME" => "Maine", "MD" => "Maryland",
        "MA" => "Massachusetts", "MI" => "Michigan", "MN" => "Minnesota",
        "MS" => "Mississippi", "MO" => "Missouri", "MT" => "Montana",
        "NE" => "Nebraska", "NV" => "Nevada", "NH" => "New Hampshire",
        "NJ" => "New Jersey", "NM" => "New Mexico", "NY" => "New York",
        "NC" => "North Carolina", "ND" => "North Dakota", "OH" => "Ohio",
        "OK" => "Oklahoma", "OR" => "Oregon", "PA" => "Pennsylvania",
        "RI" => "Rhode Island", "SC" => "South Carolina", "SD" => "South Dakota",
        "TN" => "Tennessee", "TX" => "Texas", "UT" => "Utah",
        "VT" => "Vermont", "VA" => "Virginia", "WA" => "Washington",
        "WV" => "West Virginia", "WI" => "Wisconsin", "WY" => "Wyoming",
        _ => "None",
    }
}

/// All US state postal codes in alphabetical order (including DC, excluding territories).
pub const ALL_STATE_CODES: &[&str] = &[
    "None",
    "AL", "AK", "AZ", "AR", "CA", "CO", "CT", "DC", "DE",
    "FL", "GA", "HI", "ID", "IL", "IN", "IA", "KS", "KY",
    "LA", "ME", "MD", "MA", "MI", "MN", "MS", "MO", "MT",
    "NE", "NV", "NH", "NJ", "NM", "NY", "NC", "ND", "OH",
    "OK", "OR", "PA", "RI", "SC", "SD", "TN", "TX", "UT",
    "VT", "VA", "WA", "WV", "WI", "WY",
];

pub const ALL_FILING_STATUSES: &[&str] = &[
    "Married Filing Jointly",
    "Single",
    "Married Filing Separately",
    "Head of Household",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> TaxEngine {
        TaxEngine::new(TaxRules::default())
    }

    #[test]
    fn test_ltcg_all_in_0pct_bracket() {
        let e = engine();
        // gross_ord=50_000, std_deduction=35_000 → ord_taxable=15_000 → floor=15_000
        // ltcg_0_limit=115_000, space_0=100_000, gains at 0%=20_000 → 0 LTCG tax
        // ordinary tax on 15_000: all in 10% bracket → 1_500
        let result = e.calculate_liability(2024, 50_000.0, 0.0, 20_000.0);
        assert_eq!(result.breakdown["gains_at_0_pct"] as i64, 20_000);
        assert_eq!(result.breakdown["gains_at_15_pct"] as i64, 0);
        assert!((result.total_tax - 1_500.0).abs() < 1.0, "total={}", result.total_tax);
    }

    #[test]
    fn test_ltcg_spans_0_and_15_brackets() {
        let e = engine();
        // gross_ord=90_000, std_deduction=35_000 → ord_taxable=55_000 → floor=55_000
        // ltcg_0_limit=115_000, space_0=60_000, gains_0=60_000, remaining=40_000 at 15%
        let result = e.calculate_liability(2024, 90_000.0, 0.0, 100_000.0);
        assert!((result.breakdown["gains_at_0_pct"] - 60_000.0).abs() < 0.01);
        assert!((result.breakdown["gains_at_15_pct"] - 40_000.0).abs() < 0.01);
        // ordinary tax on 55_000: 23_200@10%=2_320, 31_800@12%=3_816 → 6_136
        // LTCG tax: 40_000@15% = 6_000; total = 12_136
        assert!((result.total_tax - 12_136.0).abs() < 5.0, "total={}", result.total_tax);
    }

    #[test]
    fn test_niit_applies_when_magi_exceeds_threshold() {
        let e = engine();
        let result = e.calculate_liability(2024, 200_000.0, 0.0, 100_000.0);
        let niit = result.breakdown["niit_on_gains"];
        assert!((niit - 1_900.0).abs() < 0.01, "niit={}", niit);
    }

    #[test]
    fn test_tax_bracket_inflation() {
        let mut rules = TaxRules::default();
        let original_limit = rules.ltcg_0_limit;
        rules.inflate(0.028);
        let expected = original_limit * 1.028;
        assert!((rules.ltcg_0_limit - expected).abs() < 0.01);
        let inf_bracket = rules.brackets.last().unwrap();
        assert_eq!(inf_bracket.0, f64::INFINITY);
    }

    #[test]
    fn test_zero_gains_produces_zero_tax() {
        let e = engine();
        // Zero income of any kind → zero tax.
        let result = e.calculate_liability(2024, 0.0, 0.0, 0.0);
        assert_eq!(result.total_tax, 0.0);
    }

    #[test]
    fn test_ordinary_income_taxed_at_brackets() {
        let e = engine();
        // gross_ord=50_000, no gains → ord_taxable=15_000 → $1_500 ordinary tax
        let result = e.calculate_liability(2024, 50_000.0, 0.0, 0.0);
        assert!(result.total_tax > 1_000.0, "ordinary income should be taxed; got {}", result.total_tax);
        assert_eq!(result.breakdown["gains_at_0_pct"] as i64, 0);
    }

    #[test]
    fn test_ftc_reduces_federal_tax() {
        let e = engine();
        // Generate some tax liability first
        let baseline = e.calculate_liability(2024, 90_000.0, 0.0, 100_000.0);
        let with_ftc = e.calculate_liability_with_ftc(2024, 90_000.0, 0.0, 100_000.0, 5_000.0);
        // FTC should reduce total tax by up to $5,000
        assert!(with_ftc.total_tax < baseline.total_tax);
        assert!((with_ftc.ftc_applied - 5_000.0).abs() < 0.01);
    }

    #[test]
    fn test_single_filing_has_lower_std_deduction() {
        let single = TaxRules::for_filing_status("Single");
        let mfj = TaxRules::for_filing_status("Married Filing Jointly");
        assert!(single.std_deduction < mfj.std_deduction);
    }

    /// NRA-MFS: MFS and Single share the same std_deduction (both ~$14,600 in 2024).
    /// MFJ ($35,000) is more than double MFS ($14,600), confirming the halving effect.
    #[test]
    fn test_mfs_std_deduction_less_than_mfj() {
        let mfs = TaxRules::for_filing_status("Married Filing Separately");
        let mfj = TaxRules::for_filing_status("Married Filing Jointly");
        assert!(mfs.std_deduction < mfj.std_deduction,
            "MFS std_deduction ({}) should be less than MFJ ({})",
            mfs.std_deduction, mfj.std_deduction);
        assert_eq!(mfs.filing_status, "Married Filing Separately");
    }

    /// NRA-MFS: LTCG-0 threshold under MFS is much lower than MFJ.
    #[test]
    fn test_mfs_ltcg_threshold_lower_than_mfj() {
        let mfs = TaxRules::for_filing_status("Married Filing Separately");
        let mfj = TaxRules::for_filing_status("Married Filing Jointly");
        assert!(mfs.ltcg_0_limit < mfj.ltcg_0_limit,
            "MFS ltcg_0_limit ({}) should be less than MFJ ({})",
            mfs.ltcg_0_limit, mfj.ltcg_0_limit);
    }

    #[test]
    fn test_no_income_tax_states() {
        for code in &["FL", "TX", "WA", "NV"] {
            assert_eq!(state_tax_rate(code), 0.0, "{} should be 0%", code);
        }
    }

    #[test]
    fn test_state_tax_applied() {
        let mut rules = TaxRules::default();
        rules.us_state_rate = state_tax_rate("MD"); // ~5.5%
        rules.us_state_code = "MD".into();
        let e = TaxEngine::new(rules);
        let result = e.calculate_liability(2024, 100_000.0, 0.0, 50_000.0);
        assert!(result.state_tax > 0.0);
    }

    // ── SSDI Combined Income Rule Tests ──────────────────────────────────────

    #[test]
    fn test_ssdi_zero_income() {
        // No SSDI → always 0 taxable
        assert_eq!(ssdi_combined_income_taxable_portion(50_000.0, 0.0), 0.0);
    }

    #[test]
    fn test_ssdi_below_32k_threshold() {
        // PI ≤ $32K → 0% taxable regardless of SSDI amount
        let taxable = ssdi_combined_income_taxable_portion(30_000.0, 18_000.0);
        assert_eq!(taxable, 0.0);
    }

    #[test]
    fn test_ssdi_partial_50pct_bracket() {
        // PI = $38K (between $32K and $44K), annual SSDI = $24K
        // taxable = min(0.5*(38K-32K), 0.5*24K) = min(3_000, 12_000) = 3_000
        let taxable = ssdi_combined_income_taxable_portion(38_000.0, 24_000.0);
        assert!((taxable - 3_000.0).abs() < 0.01, "taxable={}", taxable);
    }

    #[test]
    fn test_ssdi_fully_85pct_taxable() {
        // PI = $60K (above $44K), annual SSDI = $24K
        // tier1 = 6_000, tier2 = (60K - 44K)*0.85 = 13_600 → sum = 19_600
        // cap = 0.85*24K = 20_400 → taxable = 19_600
        let taxable = ssdi_combined_income_taxable_portion(60_000.0, 24_000.0);
        assert!((taxable - 19_600.0).abs() < 0.01, "taxable={}", taxable);
    }

    #[test]
    fn test_ssdi_capped_at_85pct() {
        // Very high PI: tier1+tier2 would exceed 85% cap
        // PI = $200K, SSDI = $12K → 85% cap = $10_200
        // tier1=6_000, tier2=(200K-44K)*0.85=132_600 → 138_600 > 10_200 → capped at 10_200
        let taxable = ssdi_combined_income_taxable_portion(200_000.0, 12_000.0);
        assert!((taxable - 10_200.0).abs() < 0.01, "taxable={}", taxable);
    }
}

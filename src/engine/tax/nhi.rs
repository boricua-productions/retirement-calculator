use crate::engine::tax::japan_tax::JapanTaxEngine;
use crate::models::config::{NhiCalculatedRates, NhiModel};

/// Calculates annual NHI (National Health Insurance) premiums.
///
/// Dispatches between a full municipal rate-schedule calculation (`Calculated` mode)
/// and static manual overrides (`ManualOverride` mode).
pub struct NhiEngine;

impl NhiEngine {
    /// Compute the annual NHI premium for one household.
    ///
    /// - `is_spike_year`: true for the first post-retirement year. Japan's NHI is
    ///   assessed in June based on the prior calendar year's income, so the first
    ///   post-retirement assessment reflects peak employment income — the "spike".
    /// - `prev_year_*` are the gross income figures for the PRIOR calendar year.
    ///   The caller is responsible for providing these (1-year lookback contract).
    /// - `prev_year_investment_income_jpy`: only applied when
    ///   `rates.include_us_investment_income` is true.
    pub fn compute_annual(
        model: &NhiModel,
        prev_year_gross_salary_jpy: f64,
        prev_year_gross_pension_jpy: f64,
        prev_year_investment_income_jpy: f64,
        // Stage 03: fractional household size; 1.0 = primary only; 1.333 = retiree
        // plus a dependent covered for 4/12 months.
        num_insured: f64,
        age: i32,
        is_spike_year: bool,
    ) -> f64 {
        match model {
            NhiModel::Calculated(rates) => Self::compute_from_rates(
                rates,
                prev_year_gross_salary_jpy,
                prev_year_gross_pension_jpy,
                prev_year_investment_income_jpy,
                num_insured,
                age,
            ),
            NhiModel::ManualOverride { spike_year_total_jpy, ongoing_annual_total_jpy } => {
                if is_spike_year { *spike_year_total_jpy } else { *ongoing_annual_total_jpy }
            }
            // V7.5 — 任意継続 Shakai Hoken continuation (HIA Art. 37).
            // Duration tracking lives in SimState; this function is stateless.
            // The caller (schedule_annual_nhi) is responsible for switching to fallback
            // once nhi_ninki_keizoku_months_remaining reaches zero.
            NhiModel::NinkiKeizoku { monthly_premium_jpy, duration_months: _, fallback } => {
                if is_spike_year {
                    monthly_premium_jpy * 12.0
                } else {
                    Self::compute_annual(
                        fallback,
                        prev_year_gross_salary_jpy,
                        prev_year_gross_pension_jpy,
                        prev_year_investment_income_jpy,
                        num_insured,
                        age,
                        is_spike_year,
                    )
                }
            }
        }
    }

    /// Core rate-schedule calculation for any municipality.
    ///
    /// # NHI income basis
    /// `max(0, (net_salary + net_pension [+ investment_income]) − ¥430,000_basic_deduction)`
    ///
    /// Employment and pension deductions follow NTA tables (same as resident tax).
    ///
    /// # Components
    /// Each component is computed as `min(income_basis × rate + per_capita × n, cap)`:
    /// - **Medical** (医療分) — applies to all ages
    /// - **Support** (支援分) — applies to all ages
    /// - **Nursing care** (介護分) — ages 40–64 only
    pub fn compute_from_rates(
        rates: &NhiCalculatedRates,
        prev_year_gross_salary_jpy: f64,
        prev_year_gross_pension_jpy: f64,
        prev_year_investment_income_jpy: f64,
        num_insured: f64,
        age: i32,
    ) -> f64 {
        let net_salary = (prev_year_gross_salary_jpy
            - JapanTaxEngine::employment_deduction(prev_year_gross_salary_jpy))
            .max(0.0);
        let net_pension = (prev_year_gross_pension_jpy
            - JapanTaxEngine::pension_deduction(prev_year_gross_pension_jpy, age))
            .max(0.0);

        let investment_income = if rates.include_us_investment_income {
            prev_year_investment_income_jpy.max(0.0)
        } else {
            0.0
        };

        const NHI_BASIC_DEDUCTION: f64 = 430_000.0;
        let income_basis =
            (net_salary + net_pension + investment_income - NHI_BASIC_DEDUCTION).max(0.0);

        let n = num_insured;

        // Medical component (医療分)
        let medical = (income_basis * rates.medical_rate + rates.per_capita_medical * n)
            .min(rates.cap_medical);

        // Elderly support component (支援分)
        let support =
            (income_basis * rates.elderly_support_rate + rates.per_capita_support * n)
                .min(rates.cap_support);

        // Nursing care component (介護分): ages 40–64 only
        let nursing = if (40..=64).contains(&age) {
            (income_basis * rates.nursing_care_rate + rates.per_capita_nursing * n)
                .min(rates.cap_nursing)
        } else {
            0.0
        };

        medical + support + nursing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::config::NhiCalculatedRates;

    fn sagamihara() -> NhiModel {
        NhiModel::Calculated(NhiCalculatedRates::sagamihara_2026())
    }

    /// Transition-year spike: prior year was employment income (¥8M salary).
    /// Verifies the NhiEngine produces the same result as the existing cashflow
    /// engine implementation (767,484 — component breakdown is identical).
    #[test]
    fn test_nhi_engine_transition_year_spike() {
        // salary ¥8M → emp_ded ¥1,900,000 → net ¥6,100,000
        // income_basis = ¥5,670,000
        // medical:  5,670,000 × 8.46% + 33,600  = 513,282
        // support:  5,670,000 × 2.04% + 11,400  = 127,068
        // nursing:  5,670,000 × 2.02% + 12,600  = 127,134
        // total:    767,484
        let nhi = NhiEngine::compute_annual(&sagamihara(), 8_000_000.0, 0.0, 0.0, 1.0, 50, true);
        assert!((nhi - 767_484.0).abs() < 1.0, "nhi={nhi:.0} expected≈767,484");
    }

    /// Post-retirement normalized: low pension income produces only the per-capita fee.
    #[test]
    fn test_nhi_engine_post_retirement_normalized() {
        // pension ¥1.2M, age 65 → pension_ded = ¥1.1M → net = ¥100,000
        // income_basis = (100,000 − 430,000).max(0) = 0
        // medical = ¥33,600; support = ¥11,400; nursing = 0 (age 65)
        let nhi = NhiEngine::compute_annual(&sagamihara(), 0.0, 1_200_000.0, 0.0, 1.0, 65, false);
        assert!((nhi - 45_000.0).abs() < 1.0, "nhi={nhi:.0} expected=45,000");
    }

    /// Spike exceeds normalized by more than 4×.
    #[test]
    fn test_nhi_engine_spike_exceeds_normalized() {
        let spike      = NhiEngine::compute_annual(&sagamihara(), 8_000_000.0, 0.0, 0.0, 1.0, 50, true);
        let normalized = NhiEngine::compute_annual(&sagamihara(), 0.0, 1_200_000.0, 0.0, 1.0, 50, false);
        assert!(spike > normalized * 4.0,
            "spike ¥{spike:.0} should be >4× normalized ¥{normalized:.0}");
    }

    /// Nursing care adds the 介護分 per-capita for ages 40–64.
    #[test]
    fn test_nhi_engine_nursing_care_applied_age_50() {
        // zero income → income_basis = 0; only per-capita portions.
        // medical = ¥33,600; support = ¥11,400; nursing = ¥12,600 → total ¥57,600
        let nhi = NhiEngine::compute_annual(&sagamihara(), 0.0, 0.0, 0.0, 1.0, 50, false);
        assert!((nhi - 57_600.0).abs() < 1.0, "nhi={nhi:.0} expected=57,600");
    }

    /// Nursing care is excluded for age 65+.
    #[test]
    fn test_nhi_engine_no_nursing_care_age_65() {
        let nhi = NhiEngine::compute_annual(&sagamihara(), 0.0, 0.0, 0.0, 1.0, 65, false);
        assert!((nhi - 45_000.0).abs() < 1.0, "nhi={nhi:.0} expected=45,000");
    }

    /// All three components hit their annual caps at very high income.
    #[test]
    fn test_nhi_engine_caps_enforced() {
        // salary ¥20M → emp_ded ¥1,950,000 → net ¥18,050,000
        // income_basis = ¥17,620,000
        // medical raw: 17,620,000×8.46%+33,600 = 1,524,252 → cap 650,000
        // support raw: 17,620,000×2.04%+11,400 = 370,848   → cap 240,000
        // nursing raw: 17,620,000×2.02%+12,600 = 368,524   → cap 170,000
        // total: 1,060,000
        let nhi = NhiEngine::compute_annual(&sagamihara(), 20_000_000.0, 0.0, 0.0, 1.0, 50, false);
        assert!((nhi - 1_060_000.0).abs() < 1.0, "nhi={nhi:.0} expected=1,060,000");
    }

    /// US investment income is included in the NHI base when the flag is set.
    #[test]
    fn test_nhi_engine_investment_income_included() {
        let mut rates = NhiCalculatedRates::sagamihara_2026();
        rates.include_us_investment_income = true;
        let model = NhiModel::Calculated(rates);

        // Zero salary/pension, ¥1M investment income.
        // income_basis = max(0, 1,000,000 − 430,000) = 570,000
        // medical:  570,000 × 8.46% + 33,600 = 81,822
        // support:  570,000 × 2.04% + 11,400 = 23,028
        // nursing:  570,000 × 2.02% + 12,600 = 24,114  (age 50)
        // total:    128,964
        let nhi = NhiEngine::compute_annual(&model, 0.0, 0.0, 1_000_000.0, 1.0, 50, false);
        let expected = 81_822.0 + 23_028.0 + 24_114.0;
        assert!((nhi - expected).abs() < 1.0, "nhi={nhi:.0} expected≈{expected:.0}");
    }

    /// US investment income is excluded when the flag is false.
    #[test]
    fn test_nhi_engine_investment_income_excluded_by_default() {
        // same scenario as above but default (include_us_investment_income = false)
        let nhi = NhiEngine::compute_annual(&sagamihara(), 0.0, 0.0, 1_000_000.0, 1.0, 50, false);
        // income_basis = 0 (no salary/pension after deduction), so only per-capita
        assert!((nhi - 57_600.0).abs() < 1.0, "nhi={nhi:.0} expected=57,600 (per-capita only)");
    }

    /// Stage 03 — per-capita prorates with fractional num_insured.
    /// A dependent covered for 4 of 12 months → num_insured = 1 + 4/12 ≈ 1.333.
    /// At zero income, only per-capita components are non-zero:
    ///   medical: 33,600 × 1.333 = 44,800
    ///   support: 11,400 × 1.333 = 15,200
    ///   nursing: 12,600 × 1.333 = 16,800  (age 50)
    ///   total:   76,800
    #[test]
    fn test_nhi_engine_fractional_insured_prorates_per_capita() {
        let n = 1.0 + 4.0 / 12.0; // 1.333…
        let nhi = NhiEngine::compute_annual(&sagamihara(), 0.0, 0.0, 0.0, n, 50, false);
        // per-capita for 1 person (age 50): 33,600 + 11,400 + 12,600 = 57,600
        let one_person = 57_600.0_f64;
        let expected = one_person * n;
        assert!((nhi - expected).abs() < 1.0,
            "nhi={nhi:.1} expected≈{expected:.1} (n={n:.4})");
        assert!(nhi > one_person,
            "fractional insured must exceed single-person per-capita");
    }

    /// ManualOverride returns spike amount in spike year, ongoing otherwise.
    #[test]
    fn test_nhi_engine_manual_override_dispatch() {
        let model = NhiModel::ManualOverride {
            spike_year_total_jpy:     880_000.0,
            ongoing_annual_total_jpy: 540_000.0,
        };
        let spike   = NhiEngine::compute_annual(&model, 0.0, 0.0, 0.0, 1.0, 55, true);
        let ongoing = NhiEngine::compute_annual(&model, 0.0, 0.0, 0.0, 1.0, 55, false);
        assert!((spike   - 880_000.0).abs() < 0.01);
        assert!((ongoing - 540_000.0).abs() < 0.01);
    }
}

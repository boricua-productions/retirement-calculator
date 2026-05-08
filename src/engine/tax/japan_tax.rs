
/// Detailed result from the resident tax transition estimate.
#[derive(Debug, Clone, Default)]
pub struct ResidentTaxTransitionResult {
    #[allow(dead_code)]
    pub taxable_income: f64,
    pub tax_bill: f64,
}

/// Calculates Japanese tax liabilities:
/// - Employment and pension income deductions (NTA tables)
/// - Resident tax (住民税) — 10% + flat fee
/// - NHI premium estimation based on FERS income (Sagamihara City 3-tier discount)
///
/// Mirrors Python's `JapanTaxEngine` class in `tax_engine.py`.
pub struct JapanTaxEngine;

impl JapanTaxEngine {
    /// Calculates the employment income deduction (給与所得控除) from gross salary.
    /// Mirrors Python's `get_employment_deduction()`.
    pub fn employment_deduction(gross_salary: f64) -> f64 {
        if gross_salary <= 550_000.0 {
            gross_salary
        } else if gross_salary < 1_625_000.0 {
            550_000.0
        } else if gross_salary < 1_800_000.0 {
            gross_salary * 0.40 - 100_000.0
        } else if gross_salary < 3_600_000.0 {
            gross_salary * 0.30 + 80_000.0
        } else if gross_salary < 6_600_000.0 {
            gross_salary * 0.20 + 440_000.0
        } else if gross_salary < 8_500_000.0 {
            gross_salary * 0.10 + 1_100_000.0
        } else {
            1_950_000.0
        }
    }

    /// Calculates the public pension income deduction (公的年金等控除).
    /// The thresholds differ by age (under 65 vs 65+).
    /// Mirrors Python's `get_pension_deduction()`.
    pub fn pension_deduction(gross_pension: f64, age: i32) -> f64 {
        let (first_tier_max, second_tier_limit, second_tier_deduction) = if age < 65 {
            (600_000.0, 1_300_000.0, 600_000.0)
        } else {
            (1_100_000.0, 3_300_000.0, 1_100_000.0)
        };

        if gross_pension < first_tier_max {
            gross_pension
        } else if gross_pension < second_tier_limit {
            second_tier_deduction
        } else if gross_pension < 4_100_000.0 {
            gross_pension * 0.25 + 275_000.0
        } else if gross_pension < 7_700_000.0 {
            gross_pension * 0.15 + 685_000.0
        } else {
            gross_pension * 0.05 + 1_455_000.0
        }
    }

    /// Calculates the annual Japanese resident tax (住民税).
    ///
    /// Formula:
    ///   net_salary  = max(0, salary - employment_deduction)
    ///   net_pension = max(0, pension - pension_deduction)
    ///   basis       = net_salary + net_pension
    ///   taxable     = max(0, basis - basic_deduction - spouse_deduction - social_insurance_paid)
    ///               (rounded down to nearest ¥1,000)
    ///   tax         = taxable × income_rate + per_capita_jpy
    ///
    /// `income_rate` and `per_capita_jpy` come from the regional lookup table
    /// (`japan_regions::lookup_resident_tax_rates`). Standard: 10% + ¥6,000.
    pub fn calculate_resident_tax(
        gross_salary: f64,
        gross_pension: f64,
        social_insurance_paid: f64,
        age: i32,
        num_dependents: u32,
        income_rate: f64,
        per_capita_jpy: f64,
    ) -> f64 {
        let net_salary = (gross_salary - Self::employment_deduction(gross_salary)).max(0.0);
        let net_pension = (gross_pension - Self::pension_deduction(gross_pension, age)).max(0.0);
        let total_income_basis = net_salary + net_pension;

        let basic_deduction = 430_000.0;

        let spouse_deduction = if total_income_basis <= 9_000_000.0 {
            330_000.0 * num_dependents as f64
        } else if total_income_basis <= 9_500_000.0 {
            220_000.0 * num_dependents as f64
        } else if total_income_basis <= 10_000_000.0 {
            110_000.0 * num_dependents as f64
        } else {
            0.0
        };

        let total_deductions = social_insurance_paid + basic_deduction + spouse_deduction;
        let taxable_raw = (total_income_basis - total_deductions).max(0.0);
        // Round down to the nearest ¥1,000 (matching Python's `int(x/1000)*1000`).
        let taxable = (taxable_raw / 1_000.0).floor() * 1_000.0;

        if taxable <= 0.0 {
            return per_capita_jpy;
        }

        taxable * income_rate + per_capita_jpy
    }

    /// Estimates resident tax for the year immediately following retirement.
    /// Uses the same basic deduction (¥430,000) and standard per-capita flat fee
    /// (¥6,000) as `calculate_resident_tax`, so the bridge-fund sizing matches
    /// the live tax engine.
    pub fn estimate_resident_tax_transition(gross_income_jpy: f64) -> ResidentTaxTransitionResult {
        let ded = Self::employment_deduction(gross_income_jpy);
        let taxable = (gross_income_jpy - ded - 430_000.0).max(0.0);
        let tax_bill = taxable * 0.10 + 6_000.0;
        ResidentTaxTransitionResult { taxable_income: taxable, tax_bill }
    }

    /// Estimates annual NHI (National Health Insurance) premiums based on FERS income.
    /// Uses the Sagamihara City 3-step discount tier system.
    ///
    /// The discount tiers reduce the flat fee portion:
    ///   30% (low income) / 50% (middle) / 80% (base rate)
    ///
    /// Mirrors Python's `estimate_nhi_from_fers()`.
    pub fn estimate_nhi_from_fers(fers_gross_jpy: f64) -> f64 {
        if fers_gross_jpy <= 0.0 {
            return 0.0;
        }

        // Step 1: Derive NHI income basis from FERS gross using pension deduction tiers.
        let c1 = fers_gross_jpy;
        let nhi_income_basis = if c1 < 1_300_000.0 {
            (c1 - 600_000.0).max(0.0)
        } else if c1 < 4_100_000.0 {
            (c1 * 0.75 - 275_000.0).max(0.0)
        } else {
            (c1 * 0.85 - 685_000.0).max(0.0)
        };

        // Step 2: Determine the discount multiplier based on income basis.
        let multiplier = if nhi_income_basis <= 430_000.0 {
            0.3
        } else if nhi_income_basis <= 1_345_000.0 {
            0.5
        } else if nhi_income_basis <= 2_110_000.0 {
            0.8
        } else {
            1.0
        };

        // Step 3: Compute total premium.
        let flat_fee_total = 133_500.0 * multiplier;
        let income_levy = (nhi_income_basis - 430_000.0).max(0.0) * 0.1142;

        flat_fee_total + income_levy
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::tax::japan_regions::{STANDARD_INCOME_RATE, STANDARD_PER_CAPITA_JPY};

    #[test]
    fn test_employment_deduction_tier_1_below_floor() {
        // Below 550,000 → full deduction
        assert!((JapanTaxEngine::employment_deduction(400_000.0) - 400_000.0).abs() < 0.01);
    }

    #[test]
    fn test_employment_deduction_tier_2_flat() {
        // 550,001 to 1,624,999 → flat 550,000
        assert!((JapanTaxEngine::employment_deduction(1_000_000.0) - 550_000.0).abs() < 0.01);
    }

    #[test]
    fn test_employment_deduction_tier_3() {
        // 1,800,000 → 1,800,000 * 0.40 - 100,000 = 620,000
        assert!((JapanTaxEngine::employment_deduction(1_800_000.0) - 620_000.0).abs() < 0.01);
    }

    #[test]
    fn test_employment_deduction_max_cap() {
        // >= 8,500,000 → capped at 1,950,000
        assert!((JapanTaxEngine::employment_deduction(10_000_000.0) - 1_950_000.0).abs() < 0.01);
    }

    #[test]
    fn test_pension_deduction_under_65_below_first_tier() {
        // age=60, pension=500,000 < 600,000 → full deduction
        assert!((JapanTaxEngine::pension_deduction(500_000.0, 60) - 500_000.0).abs() < 0.01);
    }

    #[test]
    fn test_pension_deduction_age_65_threshold() {
        // At age 65, first_tier_max=1,100,000
        // pension=1,050,000 < 1,100,000 → full deduction = 1,050,000
        assert!((JapanTaxEngine::pension_deduction(1_050_000.0, 65) - 1_050_000.0).abs() < 0.01);
        // pension=1,100,001 → second_tier_deduction = 1,100,000
        assert!((JapanTaxEngine::pension_deduction(1_200_000.0, 65) - 1_100_000.0).abs() < 0.01);
    }

    #[test]
    fn test_resident_tax_zero_income_returns_flat_fee() {
        let tax = JapanTaxEngine::calculate_resident_tax(
            0.0, 0.0, 0.0, 50, 1, STANDARD_INCOME_RATE, STANDARD_PER_CAPITA_JPY,
        );
        assert!((tax - STANDARD_PER_CAPITA_JPY).abs() < 0.01);
    }

    #[test]
    fn test_resident_tax_full_calculation() {
        // salary=5,000,000 JPY, no pension, no social insurance, age=50, 1 dependent
        let ded = JapanTaxEngine::employment_deduction(5_000_000.0); // 5M*0.20+440,000=1,440,000
        let net = 5_000_000.0 - ded; // 3,560,000
        let basis = net; // no pension
        let deductions = 430_000.0 + 330_000.0; // basic + spouse (1 dep, income <= 9M)
        let taxable_raw = basis - deductions; // 2,800,000
        let taxable = (taxable_raw / 1_000.0).floor() * 1_000.0; // 2,800,000 (already exact)
        let expected_tax = taxable * STANDARD_INCOME_RATE + STANDARD_PER_CAPITA_JPY; // 280,000 + 6,000 = 286,000
        let tax = JapanTaxEngine::calculate_resident_tax(
            5_000_000.0, 0.0, 0.0, 50, 1, STANDARD_INCOME_RATE, STANDARD_PER_CAPITA_JPY,
        );
        assert!((tax - expected_tax).abs() < 1.0, "tax={} expected={}", tax, expected_tax);
    }

    #[test]
    fn test_nhi_estimate_low_income_tier() {
        // Very low FERS → nhi_income_basis <= 430,000 → multiplier=0.3
        let nhi = JapanTaxEngine::estimate_nhi_from_fers(800_000.0);
        // nhi_income_basis = 800,000 - 600,000 = 200,000 → multiplier=0.3
        let expected = 133_500.0 * 0.3; // 40,050 (no income levy since basis < 430,000)
        assert!((nhi - expected).abs() < 1.0, "nhi={} expected={}", nhi, expected);
    }

    #[test]
    fn test_nhi_estimate_zero_fers() {
        assert_eq!(JapanTaxEngine::estimate_nhi_from_fers(0.0), 0.0);
    }
}

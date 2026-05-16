use crate::models::config::VaDependentStatus;

/// Official 2026 VA disability compensation monthly rates (USD).
/// Source: https://www.va.gov/disability/compensation-rates/veteran-rates/
/// Effective date: December 1, 2025.
///
/// `with_spouse_and_child` uses the under-18 dependent child add-on, NOT the
/// over-18 schoolchild add-on (those are a separate VA category not modeled
/// here). 10% and 20% ratings do not receive dependent additions.
///
/// VA disability compensation is tax-free for US federal, US state, and
/// Japan resident tax per the US-Japan Tax Treaty Article 19.
struct Va2026Rate {
    vet_only: f64,
    with_spouse: f64,
    with_spouse_and_child: f64,
}

/// Returns the 2026 official monthly VA compensation amount (USD) for the given
/// disability rating (10–100 in steps of 10) and dependent status.
///
/// Returns 0.0 for an unrecognised rating.
pub fn lookup_va_monthly_2026(rating: u32, status: VaDependentStatus) -> f64 {
    let rate = match rating {
        10  => Va2026Rate { vet_only: 180.42,   with_spouse: 180.42,   with_spouse_and_child: 180.42   },
        20  => Va2026Rate { vet_only: 356.66,   with_spouse: 356.66,   with_spouse_and_child: 356.66   },
        30  => Va2026Rate { vet_only: 552.47,   with_spouse: 617.47,   with_spouse_and_child: 666.47   },
        40  => Va2026Rate { vet_only: 795.84,   with_spouse: 882.84,   with_spouse_and_child: 947.84   },
        50  => Va2026Rate { vet_only: 1_132.90, with_spouse: 1_241.90, with_spouse_and_child: 1_322.90 },
        60  => Va2026Rate { vet_only: 1_435.02, with_spouse: 1_566.02, with_spouse_and_child: 1_663.02 },
        70  => Va2026Rate { vet_only: 1_808.45, with_spouse: 1_961.45, with_spouse_and_child: 2_074.45 },
        80  => Va2026Rate { vet_only: 2_102.15, with_spouse: 2_277.15, with_spouse_and_child: 2_406.15 },
        90  => Va2026Rate { vet_only: 2_362.30, with_spouse: 2_559.30, with_spouse_and_child: 2_704.30 },
        100 => Va2026Rate { vet_only: 3_938.58, with_spouse: 4_158.17, with_spouse_and_child: 4_318.99 },
        _   => Va2026Rate { vet_only: 0.0,      with_spouse: 0.0,      with_spouse_and_child: 0.0      },
    };
    match status {
        VaDependentStatus::VetOnly            => rate.vet_only,
        VaDependentStatus::WithSpouse         => rate.with_spouse,
        VaDependentStatus::WithSpouseAndChild => rate.with_spouse_and_child,
    }
}

/// All valid VA disability rating values (10% to 100% in steps of 10).
pub const ALL_VA_RATINGS: &[u32] = &[0, 10, 20, 30, 40, 50, 60, 70, 80, 90, 100];

// ─── Special Monthly Compensation (SMC) — 2026 Official Rates ─────────────────

/// SMC benefit level variants. Official 2026 published rates, effective Dec 1, 2025.
/// Source: https://www.va.gov/disability/compensation-rates/special-monthly-compensation-rates/
/// SMC is always tax-free (same treaty treatment as base VA disability compensation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmcVariant {
    /// SMC-K: loss of use of a creative organ, foot, hand, or eye (added to base rating).
    K,
    /// SMC-L: loss of use of both feet, one hand and one foot, vision, etc.
    L,
    L5,
    M,
    M5,
    N,
    N5,
    /// SMC-O/P: highest non-aid-and-attendance level.
    OP,
    /// SMC-R1: aid-and-attendance by a licensed healthcare professional.
    R1,
    /// SMC-R2: highest SMC level; requires the most intensive care.
    R2,
    /// SMC-S: housebound (total + additional disability).
    Housebound,
}

/// Returns the 2026 official SMC monthly rate (USD) for the given variant.
/// For SMC-K, add the returned amount ON TOP of the base VA disability rate.
/// All other variants replace the base rate.
pub fn lookup_smc_monthly_2026(variant: SmcVariant) -> f64 {
    match variant {
        SmcVariant::K          =>   139.87,
        SmcVariant::L          => 4_900.83,
        SmcVariant::L5         => 5_154.39,
        SmcVariant::M          => 5_408.55,
        SmcVariant::M5         => 5_780.00,
        SmcVariant::N          => 6_152.64,
        SmcVariant::N5         => 6_514.00,
        SmcVariant::OP         => 6_876.52,
        SmcVariant::R1         => 9_826.88,
        SmcVariant::R2         => 11_271.67,
        SmcVariant::Housebound => 4_408.53,
    }
}

/// All SMC variants in severity order for UI display.
pub const ALL_SMC_VARIANTS: &[(&str, SmcVariant)] = &[
    ("K (add-on)",    SmcVariant::K),
    ("L",             SmcVariant::L),
    ("L½",            SmcVariant::L5),
    ("M",             SmcVariant::M),
    ("M½",            SmcVariant::M5),
    ("N",             SmcVariant::N),
    ("N½",            SmcVariant::N5),
    ("O/P",           SmcVariant::OP),
    ("R.1",           SmcVariant::R1),
    ("R.2",           SmcVariant::R2),
    ("Housebound (S)", SmcVariant::Housebound),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_100pct_with_spouse_and_child() {
        let rate = lookup_va_monthly_2026(100, VaDependentStatus::WithSpouseAndChild);
        assert!((rate - 4_318.99).abs() < 0.01, "rate={}", rate);
    }

    #[test]
    fn test_100pct_vet_only() {
        let rate = lookup_va_monthly_2026(100, VaDependentStatus::VetOnly);
        assert!((rate - 3_938.58).abs() < 0.01);
    }

    #[test]
    fn test_unknown_rating_returns_zero() {
        let rate = lookup_va_monthly_2026(0, VaDependentStatus::VetOnly);
        assert_eq!(rate, 0.0);
    }

    #[test]
    fn test_50pct_with_spouse() {
        let rate = lookup_va_monthly_2026(50, VaDependentStatus::WithSpouse);
        assert!((rate - 1_241.90).abs() < 0.01);
    }

    #[test]
    fn test_70pct_all_dependent_columns() {
        // Pins all three columns at a common rating to catch future regressions
        // in both the COLA bug (with_spouse) and the schoolchild-vs-child bug
        // (with_spouse_and_child). Source: VA published rates, eff. 2025-12-01.
        assert!((lookup_va_monthly_2026(70, VaDependentStatus::VetOnly)            - 1_808.45).abs() < 0.01);
        assert!((lookup_va_monthly_2026(70, VaDependentStatus::WithSpouse)         - 1_961.45).abs() < 0.01);
        assert!((lookup_va_monthly_2026(70, VaDependentStatus::WithSpouseAndChild) - 2_074.45).abs() < 0.01);
    }

    #[test]
    fn test_smc_corrected_variants() {
        // Pins variants that were previously wrong (linear-interpolation bug).
        assert!((lookup_smc_monthly_2026(SmcVariant::M5) - 5_780.00).abs() < 0.01);
        assert!((lookup_smc_monthly_2026(SmcVariant::N)  - 6_152.64).abs() < 0.01);
        assert!((lookup_smc_monthly_2026(SmcVariant::N5) - 6_514.00).abs() < 0.01);
        // SMC-K is an additive add-on, not a replacement.
        assert!((lookup_smc_monthly_2026(SmcVariant::K)  -   139.87).abs() < 0.01);
    }
}

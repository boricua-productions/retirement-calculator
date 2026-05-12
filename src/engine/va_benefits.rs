use crate::models::config::VaDependentStatus;

/// Official 2026 VA disability compensation monthly rates (USD).
/// Source: VA 2026 official rate table (effective December 1, 2025).
/// Dependent additions at 30–90% are 2.8% COLA-adjusted from 2025 published values.
/// Dependent additions at 100% are official published amounts ($219.59 spouse, $109.11 child).
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
        30  => Va2026Rate { vet_only: 552.47,   with_spouse: 618.01,   with_spouse_and_child: 659.13   },
        40  => Va2026Rate { vet_only: 795.84,   with_spouse: 877.57,   with_spouse_and_child: 928.46   },
        50  => Va2026Rate { vet_only: 1_132.90, with_spouse: 1_233.13, with_spouse_and_child: 1_292.75 },
        60  => Va2026Rate { vet_only: 1_435.02, with_spouse: 1_552.73, with_spouse_and_child: 1_620.58 },
        70  => Va2026Rate { vet_only: 1_808.45, with_spouse: 1_943.63, with_spouse_and_child: 2_019.70 },
        80  => Va2026Rate { vet_only: 2_102.15, with_spouse: 2_254.80, with_spouse_and_child: 2_339.10 },
        90  => Va2026Rate { vet_only: 2_362.30, with_spouse: 2_532.43, with_spouse_and_child: 2_624.95 },
        100 => Va2026Rate { vet_only: 3_938.58, with_spouse: 4_158.17, with_spouse_and_child: 4_267.28 },
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

// ─── Special Monthly Compensation (SMC) — 2026 Estimated Rates ────────────────

/// SMC benefit level variants (2026 estimated; inflated from 2025 published rates).
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

/// Returns the 2026 estimated SMC monthly rate (USD) for the given variant.
/// For SMC-K, add the returned amount ON TOP of the base VA disability rate.
/// All other variants replace the base rate.
pub fn lookup_smc_monthly_2026(variant: SmcVariant) -> f64 {
    match variant {
        SmcVariant::K          =>   139.87,
        SmcVariant::L          => 4_900.83,
        SmcVariant::L5         => 5_154.39,
        SmcVariant::M          => 5_408.55,
        SmcVariant::M5         => 5_662.63,
        SmcVariant::N          => 5_916.71,
        SmcVariant::N5         => 6_170.79,
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
        assert!((rate - 4_267.28).abs() < 0.01, "rate={}", rate);
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
        assert!((rate - 1_233.13).abs() < 0.01);
    }
}

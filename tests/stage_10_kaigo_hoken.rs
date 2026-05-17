/// Stage 10 — Long-Term Care Insurance (介護保険 / Kaigo Hoken) Integration Tests
///
/// Tests the Kaigo Hoken bracket calculation and care scenario logic.
/// Integration tests with full simulation are deferred to manual testing due to
/// the complexity of the simulation controller initialization.

use retirement_calculator::engine::tax::kaigo_hoken::{
    calculate_age_65_plus_premium_annual, projected_out_of_pocket_care,
    CareScenario, KaigoHokenBrackets,
};

/// Test 1: Bracket calculation for Sagamihara — typical retiree with ¥1.44M annual Nenkin.
#[test]
fn test_sagamihara_bracket_typical_retiree() {
    let brackets = KaigoHokenBrackets::sagamihara_2026();
    // Retiree with ¥1.44M annual Nenkin (¥120k/month) → Tier 3 (1.2M-1.8M range) → ¥60,000/year
    let annual_pension_jpy = 1_440_000.0;
    let premium = calculate_age_65_plus_premium_annual(annual_pension_jpy, &brackets);
    assert_eq!(premium, 60_000.0, "¥1.44M pension should match Tier 3: ¥60k");
}

/// Test 2: Nagoya brackets are lower than Sagamihara in mid-tiers.
#[test]
fn test_nagoya_vs_sagamihara_mid_tier() {
    let sag = KaigoHokenBrackets::sagamihara_2026();
    let nag = KaigoHokenBrackets::nagoya_2026();

    let annual_pension_jpy = 2_000_000.0;  // ¥2M pension
    let sag_premium = calculate_age_65_plus_premium_annual(annual_pension_jpy, &sag);
    let nag_premium = calculate_age_65_plus_premium_annual(annual_pension_jpy, &nag);

    // Sagamihara ¥75k, Nagoya ¥70k
    assert_eq!(sag_premium, 75_000.0);
    assert_eq!(nag_premium, 70_000.0);
    assert!(nag_premium < sag_premium, "Nagoya should be cheaper in mid-tier");
}

/// Test 3: Care scenario None returns zero out-of-pocket costs.
#[test]
fn test_care_scenario_none_zero_cost() {
    assert_eq!(projected_out_of_pocket_care(70, CareScenario::None), 0.0);
    assert_eq!(projected_out_of_pocket_care(80, CareScenario::None), 0.0);
    assert_eq!(projected_out_of_pocket_care(90, CareScenario::None), 0.0);
}

/// Test 4: Care scenario Low starts at age 75 with ¥20k/month.
#[test]
fn test_care_scenario_low_age_75_threshold() {
    assert_eq!(projected_out_of_pocket_care(74, CareScenario::Low), 0.0);
    assert_eq!(projected_out_of_pocket_care(75, CareScenario::Low), 20_000.0);
    assert_eq!(projected_out_of_pocket_care(80, CareScenario::Low), 20_000.0);
}

/// Test 5: Care scenario High starts at age 80 with ¥80k/month.
#[test]
fn test_care_scenario_high_age_80_threshold() {
    assert_eq!(projected_out_of_pocket_care(79, CareScenario::High), 0.0);
    assert_eq!(projected_out_of_pocket_care(80, CareScenario::High), 80_000.0);
    assert_eq!(projected_out_of_pocket_care(85, CareScenario::High), 80_000.0);
}

/// Test 6: High-income retiree hits Tier 9 ceiling.
#[test]
fn test_high_income_tier_9_ceiling() {
    let brackets = KaigoHokenBrackets::sagamihara_2026();
    // Retiree with ¥6M annual pension → Tier 9 → ¥150,000/year
    let annual_pension_jpy = 6_000_000.0;
    let premium = calculate_age_65_plus_premium_annual(annual_pension_jpy, &brackets);
    assert_eq!(premium, 150_000.0, "¥6M pension should hit Tier 9 ceiling: ¥150k");
}

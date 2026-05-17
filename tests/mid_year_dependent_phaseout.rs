//! Stage 03 — Mid-Year Dependent Phase-Out Tests
//!
//! Acceptance checklist (from instructions/03_edge_case_midyear_dependent_phaseout.md):
//!
//!   (A) VA add-on uses WithSpouseAndChild for Jan–Apr 2024; WithSpouse for May–Dec 2024.
//!   (B) Jido Teate April 2024 payment includes age-17 rate; June 2024 payment is zero.
//!   (C) NHI per-capita prorates correctly when a dependent turns 18 mid-year.
//!   (D) next_12_months_income_jpy starting Dec 2023 > starting Dec 2024 (cliff drop captured).
//!
//! Scenario: child born April 15, 2006, turns 18 on April 15, 2024.
//! Simulation ticks at the 1st of each month. The cutoff comparison `current_date > cutoff`
//! means May 1, 2024 > April 15, 2024 → first ineligible month is May.

use std::collections::HashMap;

use chrono::NaiveDate;

use retirement_calculator::engine::cashflow_engine::CashFlowEngine;
use retirement_calculator::engine::tax::nhi::NhiEngine;
use retirement_calculator::engine::va_benefits::lookup_va_monthly_2026;
use retirement_calculator::handlers::cashflow_manager::{jido_teate_monthly_jpy, next_12_months_income_jpy};
use retirement_calculator::models::config::{
    Config, Dependent, FamilyUnit, NhiCalculatedRates, NhiModel, SpouseProfile,
    TaxProtocol, TaxRules, UsTaxStrategy, VaDependentStatus, WaterfallStrategy,
    WithdrawalRegime, WithdrawalStrategy,
};

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn child_birth() -> NaiveDate { iso(2006, 4, 15) }
fn child_cutoff() -> NaiveDate { iso(2024, 4, 15) }

/// Minimal Config wired for the mid-year dependent scenario.
/// VA rating 70% + WithSpouseAndChild; Jido Teate enabled for the child.
fn scenario_cfg() -> Config {
    Config {
        start_date:      iso(2024, 1, 1),
        end_date:        iso(2025, 12, 31),
        retirement_date: iso(2024, 1, 1),
        rebalance_date:  iso(2024, 2, 1),
        usd_jpy:         150.0,
        inflation_cola:  0.0,
        inflation_japan: 0.0,
        ira_limit_growth: 0.0,
        fx_drift_enabled: false,
        fx_drift_rate:    0.0,
        fx_drift_cadence_months:   0,
        fx_drift_increase_amount_jpy: 0.0,
        recession_enabled:  false,
        recession_severity: 0.0,
        recession_events:   vec![],
        fx_shock_events:    vec![],
        base_expense_jpy:   0.0,
        min_expense_jpy:    0.0,
        nhi_spike_monthly_jpy: 0.0,
        nhi_model: NhiModel::default(),
        war_chest_currency:    "JPY".into(),
        war_chest_target_jpy:  0.0,
        war_chest_target_usd:  0.0,
        bridge_months_target:  12,
        bridge_fund_currency:  "USD".into(),
        roth_start_limit:      7_000.0,
        roth_contribution_made_this_year: false,
        roth_contribution_so_far: 0.0,
        dc_monthly_jpy:   0.0,
        dc_growth_rate:   0.0,
        monthly_contribution_ticker: "VTI".into(),
        va_contribution_buffer_usd:  0.0,
        nenkin_baseline_annual_jpy:  0.0,
        growth_rates_annual:         HashMap::new(),
        va_disability_rates:         HashMap::new(),
        fers_monthly_start:  0.0,
        fers_start_date:     iso(2099, 1, 1),
        retirement_year_gross_income_jpy: 0.0,
        birth_date:       iso(1975, 6, 1),
        spouse_birth_date: iso(1977, 6, 1),
        child_birth_date:  child_birth(),
        va_child_cutoff_date: Some(child_cutoff()),
        dc_payout_start_age:  99,
        dc_payout_method:     "LUMP_SUM".into(),
        pre_funded_war_chest_jpy: 0.0,
        pre_funded_bridge_jpy:   0.0,
        pre_funded_bridge_usd:   0.0,
        pre_funded_japan_tax_jpy: 0.0,
        pre_funded_us_tax_usd:    0.0,
        target_vti_pct:  0.5,
        target_schd_pct: 0.5,
        roth_rebalance_target_vti_pct:  0.5,
        roth_rebalance_target_schd_pct: 0.5,
        enable_roth_rebalance_at_59: false,
        buy_schd_last_year: false,
        rsu_tax_handling:   "SALARY".into(),
        total_annual_compensation_usd: 0.0,
        expense_rules: vec![],
        rsu_awards:    vec![],
        tax_rules:     TaxRules::default(),
        tax_jurisdiction: TaxProtocol::Both,
        investment_location: retirement_calculator::models::config::InvestmentLocation::Us,
        us_tax_strategy:   UsTaxStrategy::FtcOnly,
        va_disability_rating: 70,
        va_dependent_status:  VaDependentStatus::WithSpouseAndChild,
        ss_monthly_usd:    0.0,
        ss_start_age:      99,
        ssdi_monthly_usd:  0.0,
        is_married:        true,
        spouse_ss_monthly_usd: 0.0,
        spouse_ss_start_age:   99,
        spouse_ss_jurisdiction: TaxProtocol::Both,
        spouse_nenkin_monthly_jpy:  0.0,
        spouse_nenkin_start_age:    99,
        spouse_nenkin_jurisdiction: TaxProtocol::Both,
        family_unit: FamilyUnit {
            user_birth_year:   1975,
            spouse_birth_year: Some(1977),
            dependents: vec![
                Dependent {
                    birth_year: 2006,
                    birth_date: Some(child_birth()),
                    is_college_student: false,
                }
            ],
        },
        nenkin_income_monthly_jpy: 0.0,
        nenkin_income_start_age:   99,
        prefecture: "Kanagawa".into(),
        city:       "Sagamihara".into(),
        military_retired:  None,
        fers_jurisdiction: TaxProtocol::Both,
        fers_japan_local_tax_exempt: false,
        ss_jurisdiction:    TaxProtocol::Both,
        nenkin_jurisdiction: TaxProtocol::Both,
        va_smc_variant:     None,
        va_monthly_override: None,
        smc_monthly_override: None,
        accumulation_rules: vec![],
        target_allocations: HashMap::new(),
        rebalance_enabled: false,
        rebalance_frequency_months: 12,
        us_state_tax_rate: 0.0,
        withdrawal_strategy: WithdrawalStrategy::TotalReturn,
        withdrawal_waterfall: WaterfallStrategy::Defensive,
        fx_spread_penalty: 0.005,
        withdrawal_regime: WithdrawalRegime::Shielded,
        edu_savings_jpy_monthly: 0.0,
        jido_teate_enabled: true,
        japan_residency_start_date: None,
        exit_tax_include_tax_advantaged: true,
        annual_gift_jpy_per_recipient: 0.0,
        gift_recipient_count: 0,
        us_gift_exclusion_usd: 19_000.0,
        tlh_enabled: false,
        tlh_active_months: vec![11, 12],
        tlh_min_loss_usd: 500.0,
        enable_education_savings: false,
        enable_gift_sink: false,
        rsu_sell_to_cover_realism: false,
        rsu_sell_to_cover_policy: retirement_calculator::models::config::RsuSellToCoverPolicy::Strict,
        spouse_profile: SpouseProfile::UsPerson,
        spouse_japan_salary_jpy: 0.0,
        spouse_japan_misc_income_jpy: 0.0,
        monthly_dependent_precision: true,
        shock_ordering: retirement_calculator::models::config::ShockOrdering::DepreciateThenReprice,
    }
}

// ─── Test (A): VA add-on switches at exact 18th birthday ─────────────────────

/// Jan–Apr 2024: child under 18 on the 1st of each month → WithSpouseAndChild.
/// May–Dec 2024: child is 18+ on the 1st → WithSpouse.
#[test]
fn test_va_transitions_at_18th_birthday() {
    let cfg = scenario_cfg();
    let cf_engine = CashFlowEngine::new(cfg);

    let expected_child  = lookup_va_monthly_2026(70, VaDependentStatus::WithSpouseAndChild);
    let expected_spouse = lookup_va_monthly_2026(70, VaDependentStatus::WithSpouse);

    for month in [1u32, 2, 3, 4] {
        let date = iso(2024, month, 1);
        let income = cf_engine.get_incomes_usd(date);
        assert!(
            (income.va_usd - expected_child).abs() < 0.01,
            "Month {} 2024: expected WithSpouseAndChild VA ${:.2}, got ${:.2}",
            month, expected_child, income.va_usd,
        );
    }

    for month in [5u32, 6, 7, 8, 9, 10, 11, 12] {
        let date = iso(2024, month, 1);
        let income = cf_engine.get_incomes_usd(date);
        assert!(
            (income.va_usd - expected_spouse).abs() < 0.01,
            "Month {} 2024: expected WithSpouse VA ${:.2}, got ${:.2}",
            month, expected_spouse, income.va_usd,
        );
    }
}

/// The WithSpouseAndChild rate for rating 70 must exceed the WithSpouse rate.
#[test]
fn test_va_child_rate_exceeds_spouse_rate() {
    let with_child  = lookup_va_monthly_2026(70, VaDependentStatus::WithSpouseAndChild);
    let with_spouse = lookup_va_monthly_2026(70, VaDependentStatus::WithSpouse);
    assert!(
        with_child > with_spouse,
        "WithSpouseAndChild (${:.2}) must exceed WithSpouse (${:.2}) for 70% rating",
        with_child, with_spouse,
    );
}

// ─── Test (B): Jido Teate stops in the month after child turns 18 ────────────

/// April 2024 is an even payment month.
///   prev month = March 1, 2024: child age = 17 (Mar 1 < Apr 15). Rate: ¥10,000.
///   cur  month = April 1, 2024: child age = 17 (Apr 1 < Apr 15). Rate: ¥10,000.
///   April 2024 bundled payment: ¥20,000.
#[test]
fn test_jido_teate_april_2024_still_pays() {
    let payment = jido_teate_monthly_jpy(true, child_birth(), iso(2024, 4, 1));
    assert!(
        (payment - 20_000.0).abs() < 1.0,
        "April 2024 Jido Teate should be ¥20,000 (age 17 both months), got ¥{:.0}",
        payment,
    );
}

/// June 2024 is an even payment month.
///   prev month = May 1, 2024:  child age = 18 (May 1 > Apr 15). Rate: ¥0.
///   cur  month = June 1, 2024: child age = 18. Rate: ¥0.
///   June 2024 bundled payment: ¥0.
#[test]
fn test_jido_teate_june_2024_is_zero() {
    let payment = jido_teate_monthly_jpy(true, child_birth(), iso(2024, 6, 1));
    assert!(
        payment < 1.0,
        "June 2024 Jido Teate should be ¥0 (child age 18+), got ¥{:.0}",
        payment,
    );
}

// ─── Test (C): NHI per-capita prorates for fractional year coverage ───────────

/// Child born April 15, 2006 is under 18 on the 1st of January through April 2024
/// (4 months). May 1 onwards the child is 18+.
///
/// Precision mode: num_insured = 1 + 4/12 ≈ 1.333.
/// Legacy (Dec 31 snapshot): child is 18 on Dec 31, 2024 → num_insured = 1.0.
/// At zero income, only per-capita components are non-zero, so the NHI difference
/// equals exactly (4/12) × per_capita_total_for_one_person.
#[test]
fn test_nhi_fractional_insured_precision_exceeds_legacy() {
    let sagamihara = NhiModel::Calculated(NhiCalculatedRates::sagamihara_2026());
    let age = 49; // primary retiree born 1975, age during 2024

    // Precision mode: 4 of 12 months covered.
    let n_precision = 1.0 + 4.0 / 12.0;
    // Legacy mode: Dec 31 snapshot — child is 18, not counted.
    let n_legacy = 1.0_f64;

    let nhi_precision = NhiEngine::compute_annual(&sagamihara, 0.0, 0.0, 0.0, n_precision, age, false);
    let nhi_legacy    = NhiEngine::compute_annual(&sagamihara, 0.0, 0.0, 0.0, n_legacy,    age, false);

    assert!(
        nhi_precision > nhi_legacy,
        "Precision-mode NHI (¥{:.0}) must exceed legacy-mode NHI (¥{:.0})",
        nhi_precision, nhi_legacy,
    );

    // At zero income, age 49, per-capita for 1 person = 33,600 + 11,400 + 12,600 = 57,600.
    let per_capita_one = 57_600.0_f64;
    let expected_diff  = per_capita_one * (4.0 / 12.0);
    assert!(
        (nhi_precision - nhi_legacy - expected_diff).abs() < 1.0,
        "NHI difference should be ≈¥{:.0} (4/12 × ¥{:.0}), got ¥{:.0}",
        expected_diff, per_capita_one, nhi_precision - nhi_legacy,
    );
}

// ─── Test (D): next_12_months_income_jpy captures the cliff drop ─────────────

/// The rolling 12-month projection starting Dec 1, 2023 should be larger than
/// the one starting Dec 1, 2024 because the earlier window includes:
///   - 5 months of WithSpouseAndChild VA (Dec 2023 + Jan–Apr 2024).
///   - Jido Teate in Dec 2023, Feb 2024, and Apr 2024 (3 × ¥20,000 = ¥60,000).
/// The later window (Dec 2024–Nov 2025) uses WithSpouse for all 12 months and
/// receives zero Jido Teate (child is 18+).
#[test]
fn test_rolling_income_drops_after_cliff_year() {
    let cfg = scenario_cfg();
    let cf_engine = CashFlowEngine::new(cfg.clone());
    let fx = 150.0;

    let income_pre  = next_12_months_income_jpy(&cfg, &cf_engine, iso(2023, 12, 1), fx);
    let income_post = next_12_months_income_jpy(&cfg, &cf_engine, iso(2024, 12, 1), fx);

    assert!(
        income_pre > income_post,
        "Pre-cliff 12-month income (¥{:.0}) must exceed post-cliff (¥{:.0}). \
         VA child add-on and Jido Teate inflate the Dec-2023 window.",
        income_pre, income_post,
    );

    // VA difference: 5 months × (WithSpouseAndChild − WithSpouse) converted to JPY.
    let va_with_child  = lookup_va_monthly_2026(70, VaDependentStatus::WithSpouseAndChild);
    let va_with_spouse = lookup_va_monthly_2026(70, VaDependentStatus::WithSpouse);
    let expected_va_diff_jpy  = 5.0 * (va_with_child - va_with_spouse) * fx;
    let expected_jido_diff_jpy = 60_000.0;
    let expected_total_diff   = expected_va_diff_jpy + expected_jido_diff_jpy;

    assert!(
        (income_pre - income_post - expected_total_diff).abs() < 100.0,
        "Expected income cliff ≈¥{:.0}, got ¥{:.0}",
        expected_total_diff, income_pre - income_post,
    );
}

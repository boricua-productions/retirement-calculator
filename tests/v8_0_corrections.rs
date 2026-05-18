//! V8.0.0 Corrections — Integration Tests
//!
//! Covers the behavioural changes introduced by the V8.0.0 remediation:
//!   B.1 — Resident Tax Tokubetsu Choushuu (12-month) cadence in active employment.

use std::collections::HashMap;
use chrono::NaiveDate;

use retirement_calculator::models::assets::{
    Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, AssetClass,
    Currency, DividendCurrency, PficRegime,
};
use retirement_calculator::models::config::{
    BufferFundingTiming, Config, FamilyUnit, InvestmentLocation, NhiModel,
    RsuSellToCoverPolicy, ShockOrdering, SpouseProfile, TaxProtocol, TaxRules,
    UsTaxStrategy, VaDependentStatus, VisaType, WaterfallStrategy, WithdrawalRegime,
    WithdrawalStrategy,
};
use retirement_calculator::simulation::controller::SimulationController;

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn working_config(model_active_res_tax: bool) -> Config {
    Config {
        start_date:       iso(2026, 1, 1),
        end_date:         iso(2028, 12, 31),
        retirement_date:  iso(2030, 1, 1),
        rebalance_date:   iso(2026, 2, 1),
        usd_jpy:          150.0,
        inflation_cola:   0.0,
        inflation_japan:  0.0,
        ira_limit_growth: 0.0,
        fx_drift_enabled: false,
        fx_drift_rate:    0.0,
        fx_drift_cadence_months:      0,
        fx_drift_increase_amount_jpy: 0.0,
        recession_enabled:  false,
        recession_severity: 0.0,
        recession_events:   vec![],
        fx_shock_events:    vec![],
        base_expense_jpy:   300_000.0,
        min_expense_jpy:    200_000.0,
        nhi_spike_monthly_jpy: 0.0,
        nhi_model: NhiModel::default(),
        war_chest_enabled: false,
        war_chest_funding_timing: BufferFundingTiming::AtRetirement,
        war_chest_ramp_months: 0,
        war_chest_currency:   "JPY".into(),
        war_chest_target_jpy: 0.0,
        war_chest_target_usd: 0.0,
        bridge_fund_enabled: false,
        bridge_fund_funding_timing: BufferFundingTiming::AtRetirement,
        bridge_fund_ramp_months: 0,
        bridge_months_target: 0,
        bridge_fund_currency: "USD".into(),
        roth_start_limit:     7_000.0,
        roth_contribution_made_this_year: false,
        roth_contribution_so_far: 0.0,
        dc_monthly_jpy:   0.0,
        dc_growth_rate:   0.0,
        monthly_contribution_ticker: "VTI".into(),
        va_contribution_buffer_usd:  0.0,
        nenkin_baseline_annual_jpy:  0.0,
        growth_rates_annual: HashMap::new(),
        va_disability_rates: HashMap::new(),
        fers_monthly_start:  0.0,
        fers_start_date:     iso(2030, 1, 1),
        retirement_year_gross_income_jpy: 0.0,
        birth_date:        iso(1970, 6, 1),
        spouse_birth_date: iso(1972, 6, 1),
        child_birth_date:  iso(2000, 1, 1),
        va_child_cutoff_date: None,
        dc_payout_start_age: 99,
        dc_payout_method:    "LUMP_SUM".into(),
        pre_funded_war_chest_jpy: 0.0,
        pre_funded_bridge_jpy:   0.0,
        pre_funded_bridge_usd:   0.0,
        pre_funded_japan_tax_jpy: 0.0,
        pre_funded_us_tax_usd:    0.0,
        target_vti_pct:  1.0,
        target_schd_pct: 0.0,
        roth_rebalance_target_vti_pct:  1.0,
        roth_rebalance_target_schd_pct: 0.0,
        enable_roth_rebalance_at_59: false,
        buy_schd_last_year: false,
        rsu_tax_handling:   "SALARY".into(),
        total_annual_compensation_usd: 100_000.0,
        expense_rules: vec![],
        rsu_awards:    vec![],
        tax_rules:     TaxRules::default(),
        tax_jurisdiction:    TaxProtocol::Both,
        investment_location: InvestmentLocation::Us,
        us_tax_strategy:     UsTaxStrategy::FtcOnly,
        va_disability_rating: 0,
        va_dependent_status:  VaDependentStatus::VetOnly,
        va_monthly_override:  None,
        smc_monthly_override: None,
        ss_monthly_usd:    0.0,
        ss_start_age:      99,
        ssdi_monthly_usd:  0.0,
        is_married:        false,
        spouse_ss_monthly_usd: 0.0,
        spouse_ss_start_age:   99,
        spouse_ss_jurisdiction: TaxProtocol::Both,
        spouse_nenkin_monthly_jpy:  0.0,
        spouse_nenkin_start_age:    99,
        spouse_nenkin_jurisdiction: TaxProtocol::Both,
        family_unit: FamilyUnit { user_birth_year: 1970, spouse_birth_year: None, dependents: vec![] },
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
        accumulation_rules: vec![],
        target_allocations: HashMap::new(),
        rebalance_enabled: false,
        rebalance_frequency_months: 12,
        us_state_tax_rate: 0.0,
        withdrawal_strategy: WithdrawalStrategy::TotalReturn,
        withdrawal_waterfall: WaterfallStrategy::Defensive,
        fx_spread_penalty: 0.0,
        withdrawal_regime: WithdrawalRegime::Shielded,
        edu_savings_jpy_monthly: 0.0,
        jido_teate_enabled: false,
        japan_residency_start_date: None,
        exit_tax_include_tax_advantaged: false,
        annual_gift_jpy_per_recipient: 0.0,
        gift_recipient_count: 0,
        us_gift_exclusion_usd: 19_000.0,
        tlh_enabled: false,
        tlh_active_months: vec![],
        tlh_min_loss_usd: 500.0,
        enable_education_savings: false,
        enable_gift_sink: false,
        rsu_sell_to_cover_realism: false,
        rsu_sell_to_cover_policy: RsuSellToCoverPolicy::Strict,
        spouse_profile: SpouseProfile::UsPerson,
        spouse_japan_salary_jpy: 0.0,
        spouse_japan_misc_income_jpy: 0.0,
        monthly_dependent_precision: true,
        shock_ordering: ShockOrdering::DepreciateThenReprice,
        track_pfic_basis_drift: false,
        real_estate: vec![],
        enable_heloc_tier: false,
        enable_estate_planning: false,
        death_date: None,
        spouse_death_date: None,
        heirs: vec![],
        estate_planning_jurisdiction: TaxProtocol::Both,
        enable_gifting_optimiser: false,
        mc_use_correlated_paths: false,
        mc_correlation_matrix: HashMap::new(),
        crypto_tax_enabled: false,
        kaigo_hoken_enabled: false,
        kaigo_hoken_brackets: None,
        kaigo_care_scenario: retirement_calculator::engine::tax::kaigo_hoken::CareScenario::None,
        primary_taxpayer_visa: VisaType::Table1,
        model_active_phase_resident_tax: model_active_res_tax,
    }
}

fn minimal_accounts() -> HashMap<String, Account> {
    let mut asset = Asset {
        ticker: "VTI".into(),
        price: 100.0,
        yield_rate: 0.02,
        growth_rate: 0.05,
        currency: Currency::Usd,
        category: AssetCategory::Growth,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: 15_000.0,
        dividend_months: vec![3, 6, 9, 12],
        dividend_currency: DividendCurrency::Usd,
        pfic_regime: PficRegime::NotPfic,
        pfic_prior_year_fmv_per_share: 0.0,
        pfic_prior_year_fmv_per_share_jpy: 0.0,
        pfic_mtm_loss_carryforward_usd: 0.0,
        pfic_qef_election_year: None,
        asset_class: AssetClass::default(),
        return_profile: None,
        crypto_staking_apr: 0.0,
        lots: vec![],
    };
    asset.add_lot(iso(2020, 1, 1), 10_000.0, 1_000_000.0);

    let mut acc = Account::new_with_meta(
        "Taxable",
        Currency::Usd,
        AccountLocation::Us,
        AccountJurisdiction::Both,
    );
    acc.assets.insert("VTI".into(), asset);

    let mut map = HashMap::new();
    map.insert("Taxable".into(), acc);
    map
}

// ── B.1: Resident Tax Tokubetsu Choushuu cadence ─────────────────────────────

/// When model_active_phase_resident_tax = false (default), no resident tax is
/// charged in pre-retirement years (employer withholding implicitly netted).
#[test]
fn pre_retirement_resident_tax_off_by_default() {
    let results = SimulationController::new(working_config(false), minimal_accounts()).run();
    // All pre-retirement years (2026–2028) should show zero resident tax.
    for snap in &results.annual_summary {
        assert_eq!(
            snap.res_tax_jpy, 0.0,
            "Year {}: res_tax_jpy should be 0 when flag is off, got ¥{:.0}",
            snap.year, snap.res_tax_jpy
        );
    }
}

/// When model_active_phase_resident_tax = true, the engine schedules 12 monthly
/// Tokubetsu Choushuu installments for each working year. The second simulated
/// year (2027) should carry a non-zero resident tax charge from the N-1 salary.
#[test]
fn pre_retirement_tokubetsu_choushuu_fires_when_flag_set() {
    let results = SimulationController::new(working_config(true), minimal_accounts()).run();
    // Year 2027 uses 2026 salary (¥15M at 150 FX). Resident tax must be > 0.
    let snap_2027 = results.annual_summary.iter()
        .find(|s| s.year == 2027)
        .expect("simulation should produce 2027 snapshot");
    assert!(
        snap_2027.res_tax_jpy > 0.0,
        "Year 2027: expected Tokubetsu Choushuu resident tax, got ¥0"
    );
}

//! V8.2.0 Account Snapshot Integration Tests
//!
//! Verifies that `SimResults.account_snapshots` is populated with rows at the
//! expected events: `Retirement` (fired at the rebalance date) and `FinalYear`
//! (fired at the end of the last simulated year).

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
use retirement_calculator::models::snapshot::AccountSnapshotEvent;
use retirement_calculator::simulation::controller::SimulationController;

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Build a minimal Config where the rebalance/transition date fires within the
/// simulation window so that both `Retirement` and `FinalYear` snapshots are
/// captured.
fn snapshot_test_config() -> Config {
    Config {
        start_date:       iso(2026, 1, 1),
        end_date:         iso(2027, 12, 31),
        retirement_date:  iso(2026, 7, 1),
        rebalance_date:   iso(2026, 3, 1),
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
        expenses_detailed_mode: false,
        expense_categories: vec![],
        min_expense_buffer_jpy: 0.0,
        min_expense_buffer_pct: 0.0,
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
        fers_start_date:     iso(2026, 7, 1),
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
        model_active_phase_resident_tax: false,
        prefer_liquidation_over_belt_tightening: false,
    }
}

fn one_account_with_vti() -> HashMap<String, Account> {
    let mut asset = Asset {
        ticker: "VTI".into(),
        price: 200.0,
        yield_rate: 0.02,
        growth_rate: 0.07,
        currency: Currency::Usd,
        category: AssetCategory::Growth,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: 20_000.0,
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
    asset.add_lot(iso(2020, 1, 1), 500.0, 50_000.0);

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

#[test]
fn account_snapshots_contain_retirement_event() {
    let results = SimulationController::new(
        snapshot_test_config(),
        one_account_with_vti(),
    )
    .run();

    let retirement_rows: Vec<_> = results.account_snapshots.iter()
        .filter(|r| r.event == AccountSnapshotEvent::Retirement)
        .collect();

    assert!(
        !retirement_rows.is_empty(),
        "Expected at least one AccountSnapshotRow with event == Retirement, got none. \
         rebalance_date fires in 2026-03 which is within 2026-01..2027-12 simulation window."
    );

    for row in &retirement_rows {
        assert!(
            !row.composition.is_empty(),
            "Retirement snapshot for account '{}' has empty composition — \
             expected at least one asset row.",
            row.account_name
        );
    }
}

#[test]
fn account_snapshots_contain_final_year_event() {
    let results = SimulationController::new(
        snapshot_test_config(),
        one_account_with_vti(),
    )
    .run();

    let final_rows: Vec<_> = results.account_snapshots.iter()
        .filter(|r| r.event == AccountSnapshotEvent::FinalYear)
        .collect();

    assert!(
        !final_rows.is_empty(),
        "Expected at least one AccountSnapshotRow with event == FinalYear. \
         end_date is 2027-12-31 so FinalYear should fire at December 2027."
    );

    for row in &final_rows {
        assert!(
            !row.composition.is_empty(),
            "FinalYear snapshot for account '{}' has empty composition.",
            row.account_name
        );
        assert_eq!(
            row.date,
            iso(2027, 12, 31),
            "FinalYear snapshot date should be 2027-12-31, got {}",
            row.date
        );
    }
}

//! Stage 06 — Real Estate & Mortgage Amortization Engine Tests
//!
//! Acceptance criteria (from instructions/06_extension_real_estate.md):
//!
//!   (A) Unit: mortgage amortization formula produces payment within expected range;
//!       balance fully amortizes to zero after term.
//!   (B) Integration: toggling real-estate module on/off with empty `real_estate`
//!       array produces identical results (no-op).
//!   (C) Integration: a single rental property produces non-zero Income, Expense,
//!       and real-estate equity entries in the annual snapshot.
//!   (D) Integration: HELOC tier fires before Tier-8 stock liquidation when both
//!       `enable_heloc_tier` and at least one active HELOC line are present;
//!       does NOT fire when the master toggle is false.

use std::collections::HashMap;

use chrono::NaiveDate;

use retirement_calculator::engine::real_estate_engine::{
    monthly_pi_payment, mortgage_balance, elapsed_months,
};
use retirement_calculator::models::assets::{
    Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, AssetClass,
    Currency, DividendCurrency,
};
use retirement_calculator::models::config::{
    BufferFundingTiming,
    Config, FamilyUnit, InvestmentLocation, NhiModel, RsuSellToCoverPolicy, ShockOrdering,
    SpouseProfile, TaxJurisdiction, TaxProtocol, TaxRules, UsTaxStrategy, VaDependentStatus,
    WaterfallStrategy, WarChestCapPolicy, WithdrawalRegime, WithdrawalStrategy,
};
use retirement_calculator::models::real_estate::{
    HelocLine, MortgageCurrency, MortgageTerms, RealEstateHolding,
    RentalProfile, PropertyLocation, PropertyType, StructureType,
};
use retirement_calculator::simulation::controller::SimulationController;

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Minimal retired config with given real_estate and enable_heloc_tier.
fn base_config(
    real_estate: Vec<RealEstateHolding>,
    enable_heloc_tier: bool,
) -> Config {
    Config {
        start_date:       iso(2026, 1, 1),
        end_date:         iso(2035, 12, 31),
        retirement_date:  iso(2026, 1, 1),
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
        expenses_detailed_mode: false,
        expense_categories: vec![],
        min_expense_buffer_jpy: 0.0,
        min_expense_buffer_pct: 0.0,
        war_chest_enabled: true,
        war_chest_funding_timing: BufferFundingTiming::AtRetirement,
        war_chest_ramp_months: 24,
        war_chest_currency:   "JPY".into(),
        war_chest_target_jpy: 6_000_000.0,
        war_chest_target_usd: 0.0,
        war_chest_cap_policy: WarChestCapPolicy::Fixed,
        war_chest_cap_growth_pct: 0.0,
        war_chest_empty_date: None,
        bridge_fund_enabled: true,
        bridge_fund_funding_timing: BufferFundingTiming::AtRetirement,
        bridge_fund_ramp_months: 18,
        bridge_months_target: 12,
        bridge_fund_currency: "USD".into(),
        roth_start_limit:     0.0,
        roth_contribution_made_this_year: false,
        roth_contribution_so_far: 0.0,
        dc_monthly_jpy:   0.0,
        dc_growth_rate:   0.0,
        monthly_contribution_ticker: "VTI".into(),
        va_contribution_buffer_usd:  0.0,
        nenkin_baseline_annual_jpy:  0.0,
        growth_rates_annual: HashMap::new(),
        va_disability_rates: HashMap::new(),
        fers_monthly_start:  3_000.0,
        fers_start_date:     iso(2026, 1, 1),
        retirement_year_gross_income_jpy: 0.0,
        birth_date:        iso(1964, 1, 1),
        spouse_birth_date: iso(1966, 1, 1),
        child_birth_date:  iso(2005, 1, 1),
        va_child_cutoff_date: None,
        dc_payout_start_age:  99,
        dc_payout_method:     "LUMP_SUM".into(),
        pre_funded_war_chest_jpy: 6_000_000.0,
        pre_funded_bridge_jpy:   0.0,
        pre_funded_bridge_usd:   36_000.0,
        pre_funded_japan_tax_jpy: 0.0,
        pre_funded_us_tax_usd:    0.0,
        target_vti_pct:  0.7,
        target_schd_pct: 0.3,
        roth_rebalance_target_vti_pct:  0.7,
        roth_rebalance_target_schd_pct: 0.3,
        enable_roth_rebalance_at_59: false,
        buy_schd_last_year: false,
        rsu_tax_handling:   "SALARY".into(),
        total_annual_compensation_usd: 0.0,
        expense_rules: vec![],
        rsu_awards:    vec![],
        tax_rules:     TaxRules::default(),
        tax_jurisdiction: TaxJurisdiction::Both,
        investment_location: InvestmentLocation::Us,
        us_tax_strategy: UsTaxStrategy::FtcOnly,
        va_disability_rating: 0,
        va_dependent_status:  VaDependentStatus::VetOnly,
        ss_monthly_usd:    2_200.0,
        ss_start_age:      62,
        ssdi_monthly_usd:  0.0,
        is_married:        false,
        spouse_ss_monthly_usd: 0.0,
        spouse_ss_start_age:   99,
        spouse_ss_jurisdiction: TaxProtocol::Both,
        spouse_nenkin_monthly_jpy:  0.0,
        spouse_nenkin_start_age:    99,
        spouse_nenkin_jurisdiction: TaxProtocol::Both,
        family_unit: FamilyUnit { user_birth_year: 1964, spouse_birth_year: None, dependents: vec![] },
        nenkin_income_monthly_jpy: 100_000.0,
        nenkin_income_start_age:   62,
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
        jido_teate_enabled: false,
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
        rsu_sell_to_cover_policy: RsuSellToCoverPolicy::Strict,
        spouse_profile: SpouseProfile::UsPerson,
        spouse_japan_salary_jpy: 0.0,
        spouse_japan_misc_income_jpy: 0.0,
        monthly_dependent_precision: true,
        shock_ordering: ShockOrdering::DepreciateThenReprice,
        track_pfic_basis_drift: false,
        real_estate,
        enable_heloc_tier,
        enable_estate_planning: false,
        death_date: None,
        spouse_death_date: None,
        heirs: vec![],
        estate_planning_jurisdiction: TaxProtocol::Both,
        enable_gifting_optimiser: false,
        // Stage 08 defaults
        mc_use_correlated_paths: false,
        mc_correlation_matrix: std::collections::HashMap::new(),
        // Stage 09 defaults
        crypto_tax_enabled: true,
        // Stage 10 defaults
        kaigo_hoken_enabled: true,
        kaigo_hoken_brackets: None,
        kaigo_care_scenario: retirement_calculator::engine::tax::kaigo_hoken::CareScenario::None,
        primary_taxpayer_visa: retirement_calculator::models::config::VisaType::Table1,
        model_active_phase_resident_tax: false,
        prefer_liquidation_over_belt_tightening: false,
    }
}

/// Build a minimal set of accounts for integration tests.
fn minimal_accounts() -> HashMap<String, Account> {
    let mut accounts = HashMap::new();

    let mut taxable = Account::new_with_meta(
        "Taxable", Currency::Usd,
        AccountLocation::Us, AccountJurisdiction::Us,
    );
    let mut vti = Asset {
        ticker: "VTI".into(),
        price: 250.0,
        yield_rate: 0.013,
        growth_rate: 0.07,
        currency: Currency::Usd,
        category: AssetCategory::Income,
        drip_enabled: true,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: 250.0 * 150.0,
        dividend_months: vec![3, 6, 9, 12],
        dividend_currency: DividendCurrency::Usd,
        pfic_regime: retirement_calculator::models::assets::PficRegime::NotPfic,
        pfic_prior_year_fmv_per_share: 0.0,
        pfic_prior_year_fmv_per_share_jpy: 0.0,
        pfic_mtm_loss_carryforward_usd: 0.0,
        pfic_qef_election_year: None,
        asset_class: AssetClass::default(),
        return_profile: None,
        crypto_staking_apr: 0.0,
        lots: Vec::new(),
    };
    vti.add_lot(iso(2020, 1, 1), 500.0, 500.0 * 200.0);
    taxable.assets.insert("VTI".into(), vti);
    accounts.insert("Taxable".into(), taxable);

    let roth = Account::new_with_meta(
        "Roth", Currency::Usd,
        AccountLocation::Us, AccountJurisdiction::Us,
    );
    accounts.insert("Roth".into(), roth);

    let dc = Account::new_with_meta(
        "DC", Currency::Jpy,
        AccountLocation::Japan, AccountJurisdiction::Japan,
    );
    accounts.insert("DC".into(), dc);

    accounts
}

/// A Tokyo rental mansion holding (no mortgage, no HELOC).
fn tokyo_rental() -> RealEstateHolding {
    RealEstateHolding {
        name: "Tokyo Mansion".into(),
        location: PropertyLocation::Japan,
        property_type: PropertyType::Rental,
        structure_type: StructureType::ReinforcedConcrete,
        purchase_date: Some(iso(2010, 1, 1)),
        purchase_price_jpy: 50_000_000.0,
        purchase_price_usd: 0.0,
        current_fmv_jpy: 55_000_000.0,
        current_fmv_usd: 0.0,
        annual_property_tax_jpy: 935_000.0,   // ~1.7% of assessed value
        annual_property_tax_usd: 0.0,
        mortgage: None,
        heloc: None,
        reverse_mortgage: None,
        rental: Some(RentalProfile {
            monthly_rent_jpy: 200_000.0,
            monthly_rent_usd: 0.0,
            vacancy_pct: 0.05,
            annual_insurance_jpy: 120_000.0,
            annual_insurance_usd: 0.0,
            annual_repairs_pct_fmv: 0.01,
        }),
    }
}

/// A Tokyo rental mansion with a 30-year JPY mortgage and a HELOC line.
fn tokyo_rental_with_heloc() -> RealEstateHolding {
    let mut h = tokyo_rental();
    h.mortgage = Some(MortgageTerms {
        original_principal: 40_000_000.0,
        annual_rate: 0.01,
        term_months: 360,
        start_date: iso(2010, 1, 1),
        currency: MortgageCurrency::Jpy,
    });
    h.heloc = Some(HelocLine {
        credit_line_usd: 200_000.0,  // ~¥30M at 150 JPY/USD
        draw_rate: 0.06,
        ltv_cap: 0.80,
        enabled: true,
    });
    h
}

// ─── (A) Unit tests ──────────────────────────────────────────────────────────

#[test]
fn a_mortgage_payment_in_correct_range() {
    let terms = MortgageTerms {
        original_principal: 30_000_000.0,
        annual_rate: 0.01,
        term_months: 360,
        start_date: iso(2010, 1, 1),
        currency: MortgageCurrency::Jpy,
    };
    let payment = monthly_pi_payment(&terms);
    assert!(
        payment > 96_000.0 && payment < 97_000.0,
        "Expected ~¥96k monthly payment, got {payment:.0}"
    );
    // Total payments must exceed principal (interest was charged).
    assert!(payment * 360.0 > 30_000_000.0);
}

#[test]
fn a_mortgage_fully_amortizes() {
    let terms = MortgageTerms {
        original_principal: 30_000_000.0,
        annual_rate: 0.01,
        term_months: 360,
        start_date: iso(2010, 1, 1),
        currency: MortgageCurrency::Jpy,
    };
    let balance_at_end = mortgage_balance(&terms, 360);
    assert!(
        balance_at_end < 1.0,
        "Balance after 360 payments should be ≈0, got {balance_at_end:.2}"
    );
}

#[test]
fn a_elapsed_months_correct() {
    let terms = MortgageTerms {
        original_principal: 1.0,
        annual_rate: 0.0,
        term_months: 1,
        start_date: iso(2010, 1, 1),
        currency: MortgageCurrency::Jpy,
    };
    assert_eq!(elapsed_months(&terms, iso(2010, 1, 1)), 0);
    assert_eq!(elapsed_months(&terms, iso(2010, 7, 1)), 6);
    assert_eq!(elapsed_months(&terms, iso(2040, 1, 1)), 360);
}

// ─── (B) No-op when real_estate is empty ────────────────────────────────────

#[test]
fn b_empty_real_estate_no_change_to_results() {
    // Run the same simulation with and without the module enabled (but empty).
    let cfg_off = base_config(vec![], false);
    let cfg_on  = base_config(vec![], true);   // toggle on, but no properties

    let results_off = SimulationController::new(cfg_off, minimal_accounts())
        .run()
        .annual_summary;
    let results_on  = SimulationController::new(cfg_on, minimal_accounts())
        .run()
        .annual_summary;

    assert_eq!(results_off.len(), results_on.len(), "Simulation length should match");
    for (a, b) in results_off.iter().zip(results_on.iter()) {
        assert_eq!(a.year, b.year);
        // Portfolio values should be identical to within floating-point precision.
        assert!(
            (a.brokerage_usd - b.brokerage_usd).abs() < 0.01,
            "Year {}: brokerage differs: {:.2} vs {:.2}", a.year, a.brokerage_usd, b.brokerage_usd
        );
        // No rental income should be present.
        assert_eq!(b.rental_income_jpy, 0.0);
        assert_eq!(b.real_estate_exp_jpy, 0.0);
    }
}

// ─── (C) Rental property produces non-zero Income, Expense, Equity rows ─────

#[test]
fn c_rental_property_produces_non_zero_income_and_expense() {
    let cfg = base_config(vec![tokyo_rental()], false);
    let results = SimulationController::new(cfg, minimal_accounts()).run();

    // Every simulated year should have rental income and real estate expenses.
    for snap in &results.annual_summary {
        assert!(
            snap.rental_income_jpy > 0.0,
            "Year {}: expected non-zero rental income, got {:.0}", snap.year, snap.rental_income_jpy
        );
        assert!(
            snap.real_estate_exp_jpy > 0.0,
            "Year {}: expected non-zero real-estate expense (property tax), got {:.0}",
            snap.year, snap.real_estate_exp_jpy
        );
        // Equity should reflect FMV (no mortgage in this holding).
        assert!(
            snap.real_estate_equity_jpy > 0.0,
            "Year {}: expected positive equity, got {:.0}", snap.year, snap.real_estate_equity_jpy
        );
    }
}

#[test]
fn c_rental_income_increases_income_line() {
    // With rental income the simulation should show higher annual income totals.
    let cfg_no_re  = base_config(vec![], false);
    let cfg_with_re = base_config(vec![tokyo_rental()], false);

    let res_no  = SimulationController::new(cfg_no_re,  minimal_accounts()).run();
    let res_re  = SimulationController::new(cfg_with_re, minimal_accounts()).run();

    let first_no = &res_no.annual_summary[0];
    let first_re = &res_re.annual_summary[0];

    assert!(
        first_re.rental_income_jpy > 0.0,
        "Expected rental income but got {:.0}", first_re.rental_income_jpy
    );
    // Real estate expense (property tax) adds drag — total_exp_jpy should be higher.
    assert!(
        first_re.total_exp_jpy > first_no.total_exp_jpy,
        "Real estate property tax should increase total expenses: {:.0} vs {:.0}",
        first_re.total_exp_jpy, first_no.total_exp_jpy
    );
}

// ─── (D) HELOC tier fires only when configured ───────────────────────────────

/// Build a recession scenario where income is severely insufficient.
/// The war chest starts at zero so T0-T3 cannot cover the gap,
/// and the bridge fund is minimal so T6 drains quickly.
fn recession_config_with_heloc(heloc_enabled: bool, master_toggle: bool) -> Config {
    let property = if heloc_enabled {
        tokyo_rental_with_heloc()
    } else {
        // HELOC line present but not enabled
        let mut h = tokyo_rental_with_heloc();
        h.heloc = Some(HelocLine {
            credit_line_usd: 200_000.0,
            draw_rate: 0.06,
            ltv_cap: 0.80,
            enabled: false,  // explicitly disabled
        });
        h
    };

    let mut cfg = base_config(vec![property], master_toggle);
    // Make expenses extremely high relative to income so the waterfall runs deep.
    cfg.base_expense_jpy = 2_000_000.0;
    cfg.min_expense_jpy  = 1_500_000.0;
    // Force a recession in year 1 to stress-test the waterfall.
    cfg.recession_events = vec![
        retirement_calculator::models::config::RecessionEvent {
            year: 2026,
            severity: 0.40,
            duration_months: 6,
            recovery_months: 6,
        }
    ];
    cfg.recession_enabled = true;
    cfg.recession_severity = 0.40;
    // Small bridge fund so it depletes in a few months.
    cfg.pre_funded_bridge_usd = 5_000.0;
    cfg.pre_funded_war_chest_jpy = 0.0;
    cfg
}

#[test]
fn d_heloc_fires_when_toggle_and_line_enabled() {
    let cfg = recession_config_with_heloc(true, true);
    let results = SimulationController::new(cfg, minimal_accounts()).run();

    // In year 2026 with severe recession, the HELOC should have been drawn.
    let snap_2026 = results.annual_summary.iter().find(|s| s.year == 2026).unwrap();
    assert!(
        snap_2026.outstanding_heloc_usd > 0.0,
        "HELOC should have been drawn in 2026 recession, got ${:.2}", snap_2026.outstanding_heloc_usd
    );
}

#[test]
fn d_heloc_does_not_fire_when_master_toggle_off() {
    let cfg = recession_config_with_heloc(true, false);  // HELOC line enabled, master toggle OFF
    let results = SimulationController::new(cfg, minimal_accounts()).run();

    let snap_2026 = results.annual_summary.iter().find(|s| s.year == 2026).unwrap();
    assert_eq!(
        snap_2026.outstanding_heloc_usd, 0.0,
        "HELOC must not fire when master toggle is off, got ${:.2}", snap_2026.outstanding_heloc_usd
    );
}

#[test]
fn d_heloc_does_not_fire_when_line_disabled() {
    let cfg = recession_config_with_heloc(false, true);  // master toggle ON, HELOC line disabled
    let results = SimulationController::new(cfg, minimal_accounts()).run();

    let snap_2026 = results.annual_summary.iter().find(|s| s.year == 2026).unwrap();
    assert_eq!(
        snap_2026.outstanding_heloc_usd, 0.0,
        "HELOC must not fire when the property's HELOC line is disabled, got ${:.2}",
        snap_2026.outstanding_heloc_usd
    );
}

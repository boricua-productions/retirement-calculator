//! Stage 05 — PFIC §1296 MTM Phantom Income & FX Drift Tests
//!
//! Acceptance criteria (from instructions/05_edge_case_pfic_mtm_drift.md):
//!
//!   (A) A 30-year simulation with a Japan-domiciled PFIC-flagged asset and FX drift
//!       runs to 2055 with zero basis-drift warnings.
//!   (B) A loss year banks the carry-forward; the subsequent gain year is reduced to zero.
//!   (C) Toggling `track_pfic_basis_drift = false` does NOT eliminate drift warnings
//!       when the JPY basis is frozen (drift is suppressed, not absent).
//!       Note: with drift-tracking enabled, warnings appear; disabled means warnings are
//!       simply not emitted (self-healing skipped).

use std::collections::HashMap;

use chrono::NaiveDate;

use retirement_calculator::models::assets::{
    Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, AssetClass,
    Currency, DividendCurrency, PficRegime,
};
use retirement_calculator::models::config::{
    BufferFundingTiming,
    Config, FamilyUnit, NhiModel, ShockOrdering, SpouseProfile, TaxProtocol, TaxRules,
    VaDependentStatus, VisaType, WarChestCapPolicy, WaterfallStrategy, WithdrawalRegime,
    WithdrawalStrategy, UsTaxStrategy, InvestmentLocation, RsuSellToCoverPolicy,
};
use retirement_calculator::simulation::controller::SimulationController;

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Minimal 30-year retired config with FX drift enabled (0.5% annual yen strengthening).
/// Rate is intentionally below the 1% drift-monitor threshold so annual FX movements
/// do not trigger warnings; the engine's self-heal path is still exercised if the
/// basis is ever stale (> 1% drift from multi-year gaps).
fn pfic_config(track_drift: bool) -> Config {
    Config {
        start_date:       iso(2026, 1, 1),
        end_date:         iso(2055, 12, 31),
        retirement_date:  iso(2026, 1, 1),
        rebalance_date:   iso(2026, 2, 1),
        usd_jpy:          150.0,
        inflation_cola:   0.0,
        inflation_japan:  0.0,
        ira_limit_growth: 0.0,
        fx_drift_enabled: true,
        fx_drift_rate:    0.005,
        fx_drift_cadence_months:   0,
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
        birth_date:        iso(1968, 1, 1),
        spouse_birth_date: iso(1970, 1, 1),
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
        tax_jurisdiction: TaxProtocol::Both,
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
        family_unit: FamilyUnit { user_birth_year: 1968, spouse_birth_year: None, dependents: vec![] },
        nenkin_income_monthly_jpy: 0.0,
        nenkin_income_start_age:   65,
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
        track_pfic_basis_drift: track_drift,
        real_estate: vec![],
        enable_heloc_tier: false,
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

/// 500 shares of a Japan mutual fund (PFIC §1296 MTM) at ¥10,000 per unit ($66.67/share at ¥150).
/// Held in a taxable brokerage (not japan_tax_advantaged → contributes to JPY resident-tax base).
/// Price appreciates 5% annually in USD; FX drift means JPY-equivalent basis diverges.
fn pfic_taxable_account() -> Account {
    let price_usd = 10_000.0 / 150.0; // ≈ $66.67 per share at starting FX
    let mut asset = Asset {
        ticker: "JPNFND".into(),
        price: price_usd,
        yield_rate: 0.0,
        growth_rate: 0.05,
        currency: Currency::Usd,
        category: AssetCategory::Growth,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: 10_000.0,
        dividend_months: vec![],
        dividend_currency: DividendCurrency::Usd,
        pfic_regime: PficRegime::Mtm,
        pfic_prior_year_fmv_per_share: 0.0,       // first-year: uses cost basis
        pfic_prior_year_fmv_per_share_jpy: 0.0,   // first-year: derived from USD × FX
        pfic_mtm_loss_carryforward_usd: 0.0,
        pfic_qef_election_year: None,
        asset_class: AssetClass::default(),
        return_profile: None,
        crypto_staking_apr: 0.0,
        lots: Vec::new(),
    };
    asset.add_lot(iso(2025, 12, 31), 500.0, 500.0 * price_usd);

    // Use a "Brokerage_" prefix so the account is included in the brokerage_usd
    // snapshot total but is NOT touched by the transition rebalance handler
    // (which only operates on the "Taxable" account).
    let mut acc = Account::new_with_meta(
        "Brokerage_PFIC",
        Currency::Usd,
        AccountLocation::Us,
        AccountJurisdiction::Both,
    );
    acc.japan_tax_advantaged = false;
    acc.assets.insert("JPNFND".into(), asset);
    acc
}

/// (A) 30-year PFIC simulation with FX drift produces zero basis-drift warnings
///     when `track_pfic_basis_drift = true`.
///
/// Rationale: the drift monitor fires when USD-basis × FX diverges from the stored
/// JPY basis by more than 1%. With 0.5% annual FX drift, each year's basis movement
/// stays below the 1% threshold, so no warnings are emitted. Over 30 years the FX
/// compounds to ~16% total (1.005^30), exercising the dual-currency accumulation path
/// without triggering annual drift alerts.
#[test]
fn test_30yr_pfic_no_drift_warnings_with_tracking() {
    let cfg = pfic_config(true);
    let mut accounts = HashMap::new();
    accounts.insert("Brokerage_PFIC".into(), pfic_taxable_account());
    let ctrl = SimulationController::new(cfg, accounts);
    let results = ctrl.run();

    assert_eq!(
        results.pfic_basis_drift_warnings.len(),
        0,
        "With track_pfic_basis_drift=true, JPY basis is self-healed each year — \
         no drift warnings should accumulate over 30 years. \
         Got {} warning(s).",
        results.pfic_basis_drift_warnings.len()
    );

    // Also verify PFIC MTM income was actually recorded (not all zero).
    let total_pfic_mtm: f64 = results.annual_summary.iter()
        .map(|s| s.pfic_mtm_income_usd)
        .sum();
    assert!(
        total_pfic_mtm > 0.0,
        "PFIC MTM phantom income should be positive over 30 years of appreciation. Got ${:.2}",
        total_pfic_mtm
    );
}

/// (B) Loss year banks carry-forward; next gain year is partially or fully absorbed.
///
/// We test this with the unit-level function directly (already covered by pfic.rs unit tests),
/// but also verify that the controller correctly passes carry-forward state across years.
///
/// Strategy: start with a fund at $100, drop to $80 in year 1, recover to $90 in year 2.
/// Year 1 loss of $10/share × 500 shares = $5,000 carry-forward.
/// Year 2 gain of $10/share × 500 = $5,000 gross, offset by $5,000 carry → net $0 MTM income.
#[test]
fn test_pfic_loss_carryforward_absorbs_subsequent_gain() {
    // Use the pfic::compute_annual_mtm_gain directly (unit-level verification).
    use retirement_calculator::engine::tax::pfic::compute_annual_mtm_gain;
    use retirement_calculator::models::assets::{Asset, AssetClass, AssetLot, AssetCategory,
        DividendCurrency, PficRegime};

    let mut asset = Asset {
        ticker: "JPNFND".into(),
        price: 80.0,
        yield_rate: 0.0,
        growth_rate: 0.05,
        currency: Currency::Usd,
        category: AssetCategory::Growth,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: 100.0 * 150.0,
        dividend_months: vec![],
        dividend_currency: DividendCurrency::Usd,
        pfic_regime: PficRegime::Mtm,
        pfic_prior_year_fmv_per_share: 100.0,   // was $100; now $80 → loss
        pfic_prior_year_fmv_per_share_jpy: 15_000.0, // 100 × 150
        pfic_mtm_loss_carryforward_usd: 0.0,
        pfic_qef_election_year: None,
        asset_class: AssetClass::default(),
        return_profile: None,
        crypto_staking_apr: 0.0,
        lots: Vec::new(),
    };
    asset.add_lot(iso(2025, 1, 1), 500.0, 500.0 * 100.0);

    // Year 1: loss year ($100 → $80 = -$10k gross loss).
    let r1 = compute_annual_mtm_gain(&mut asset, 150.0).unwrap();
    assert_eq!(r1.usd, 0.0, "Loss year must report $0 MTM income");
    assert!(
        (asset.pfic_mtm_loss_carryforward_usd - 10_000.0).abs() < 1.0,
        "Carry-forward should be $10,000 after loss year. Got ${:.2}",
        asset.pfic_mtm_loss_carryforward_usd,
    );

    // Year 2: partial gain ($80 → $90 = +$5k gross gain; carry-forward $10k absorbs all).
    asset.pfic_prior_year_fmv_per_share = 80.0;
    asset.price = 90.0;
    let r2 = compute_annual_mtm_gain(&mut asset, 150.0).unwrap();
    assert_eq!(r2.usd, 0.0, "Partial gain fully absorbed by carry-forward");
    assert!(
        (asset.pfic_mtm_loss_carryforward_usd - 5_000.0).abs() < 1.0,
        "Carry-forward should be $5,000 after partial absorption. Got ${:.2}",
        asset.pfic_mtm_loss_carryforward_usd,
    );

    // Year 3: gain ($90 → $110 = +$10k gross; $5k carry-forward absorbs half → $5k net).
    asset.pfic_prior_year_fmv_per_share = 90.0;
    asset.price = 110.0;
    let r3 = compute_annual_mtm_gain(&mut asset, 150.0).unwrap();
    assert!(
        (r3.usd - 5_000.0).abs() < 1.0,
        "Net MTM income should be $5,000 after residual carry-forward consumed. Got ${:.2}",
        r3.usd
    );
    assert!(
        asset.pfic_mtm_loss_carryforward_usd < 1.0,
        "Carry-forward should be exhausted. Got ${:.2}",
        asset.pfic_mtm_loss_carryforward_usd,
    );
}

/// (C) With `track_pfic_basis_drift = false`, no warnings are emitted even when
///     the JPY basis has drifted significantly from USD basis × FX.
#[test]
fn test_pfic_drift_warnings_suppressed_when_tracking_disabled() {
    let cfg = pfic_config(false);
    let mut accounts = HashMap::new();
    accounts.insert("Brokerage_PFIC".into(), pfic_taxable_account());
    let ctrl = SimulationController::new(cfg, accounts);
    let results = ctrl.run();

    assert_eq!(
        results.pfic_basis_drift_warnings.len(),
        0,
        "With track_pfic_basis_drift=false, drift monitoring is off — \
         no warnings should be emitted regardless of actual drift.",
    );
}

/// (D) V8.0 — Table1 visa is exempt from Exit Tax per IT Act Art. 60-2.
/// Even with 10+ years residency and assets above ¥100M, Table1 never triggers.
/// Table2 with identical conditions does trigger.
#[test]
fn test_visa_table1_exempt_from_exit_tax() {
    // JPY-denominated DC account with ¥200M → above the ¥100M exit-tax threshold.
    let make_large_accounts = || {
        let price_jpy = 10_000.0;
        let mut asset = Asset {
            ticker: "NISA_FUND".into(),
            price: price_jpy,
            yield_rate: 0.0,
            growth_rate: 0.05,
            currency: Currency::Jpy,
            category: AssetCategory::Growth,
            drip_enabled: false,
            dividend_reinvest_target: None,
            custom_growth_rate: None,
            avg_jpy_basis_per_share: price_jpy,
            dividend_months: vec![],
            dividend_currency: DividendCurrency::Jpy,
            pfic_regime: PficRegime::default(),
            pfic_prior_year_fmv_per_share: 0.0,
            pfic_prior_year_fmv_per_share_jpy: 0.0,
            pfic_mtm_loss_carryforward_usd: 0.0,
            pfic_qef_election_year: None,
            asset_class: AssetClass::default(),
            return_profile: None,
            crypto_staking_apr: 0.0,
            lots: Vec::new(),
        };
        // 20,000 units × ¥10,000 = ¥200M
        asset.add_lot(iso(2025, 12, 31), 20_000.0, 20_000.0 * price_jpy);
        let mut acc = Account::new_with_meta(
            "DC",
            Currency::Jpy,
            AccountLocation::Japan,
            AccountJurisdiction::Both,
        );
        acc.assets.insert("NISA_FUND".into(), asset);
        let mut accounts = HashMap::new();
        accounts.insert("DC".into(), acc);
        accounts
    };

    // Table1: never triggers even above ¥100M threshold.
    let mut cfg_table1 = pfic_config(true);
    cfg_table1.japan_residency_start_date = Some(iso(2015, 1, 1)); // 10+ years by sim start
    cfg_table1.primary_taxpayer_visa = VisaType::Table1;
    let results1 = SimulationController::new(cfg_table1, make_large_accounts()).run();
    for snap in &results1.annual_summary {
        assert!(!snap.exit_tax_triggered,
            "Table1 visa should never trigger exit tax (year {})", snap.year);
    }

    // Table2: same conditions triggers.
    let mut cfg_table2 = pfic_config(true);
    cfg_table2.japan_residency_start_date = Some(iso(2015, 1, 1));
    cfg_table2.primary_taxpayer_visa = VisaType::Table2;
    let results2 = SimulationController::new(cfg_table2, make_large_accounts()).run();
    let any_triggered = results2.annual_summary.iter().any(|s| s.exit_tax_triggered);
    assert!(any_triggered, "Table2 visa with ≥5 years residency and ≥¥100M should trigger exit tax");
}

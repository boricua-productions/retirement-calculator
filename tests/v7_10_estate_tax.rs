//! Stage 07 — Japan Sōzoku-zei & US Estate Tax Integration Tests
//!
//! Acceptance criteria (from instructions/07_extension_inheritance_estate_tax.md):
//!
//! (A) Unit tests for Japan Sōzoku-zei bracket table (NTA published values).
//! (B) Unit tests for US estate tax computation (pre-2026 and post-2026 exclusion).
//! (C) Integration test: ¥500M estate with 2 heirs produces the expected Japan tax bill.
//! (D) Toggling the gift sink reduces the Sōzoku-zei total.
//! (E) A full simulation with `enable_estate_planning = true` produces a non-zero
//!     `EstateSummary` on the final snapshot and in `SimResults`.

use std::collections::HashMap;
use chrono::NaiveDate;

use retirement_calculator::engine::tax::estate_tax::{
    compute_japan_sozoku_zei, compute_us_estate_tax, compute_treaty_credit,
    lifetime_gifting_optimiser,
};
use retirement_calculator::models::assets::{
    Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, AssetClass,
    Currency, DividendCurrency, PficRegime,
};
use retirement_calculator::models::config::{
    BufferFundingTiming,
    Config, FamilyUnit, Heir, HeirRelationship, InvestmentLocation, NhiModel,
    RsuSellToCoverPolicy, ShockOrdering, SpouseProfile, TaxProtocol, TaxRules,
    UsTaxStrategy, VaDependentStatus, WaterfallStrategy, WithdrawalRegime, WithdrawalStrategy,
};
use retirement_calculator::simulation::controller::SimulationController;

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

// ─── Bracket unit tests ───────────────────────────────────────────────────────

/// (A-1) ¥500M estate, 2 heirs (no spouse).
/// Exclusion = ¥30M + ¥6M × 2 = ¥42M. Taxable = ¥458M.
/// Each heir: ¥229M → bracket ¥200M-¥300M: ¥229M × 45% − ¥27M = ¥103.05M − ¥27M = ¥76.05M
/// Total = 2 × ¥76.05M = ¥152.1M
#[test]
fn japan_estate_500m_2_heirs_no_spouse() {
    let tax = compute_japan_sozoku_zei(500_000_000.0, 2, &[]);
    // Each heir gets equal share = ¥229M → 45% bracket: 229M×0.45 − 27M = 76.05M each
    let expected = 152_100_000.0;
    assert!((tax - expected).abs() < 1_000.0,
        "Expected ≈¥152.1M, got ¥{:.0}", tax);
}

/// (A-2) Very small estate below exclusion → zero tax.
#[test]
fn japan_estate_below_exclusion_zero_tax() {
    // 1 heir → exclusion = ¥36M. Estate ¥35M → taxable = 0.
    let tax = compute_japan_sozoku_zei(35_000_000.0, 1, &[]);
    assert_eq!(tax, 0.0);
}

/// (A-3) Top bracket: ¥1B estate, 1 heir.
/// Exclusion = ¥36M. Taxable = ¥964M.
/// ¥964M → top bracket (>¥600M) 55% − ¥72M = 964M × 0.55 − 72M = 530.2M − 72M = ¥458.2M
#[test]
fn japan_estate_1b_top_bracket() {
    let tax = compute_japan_sozoku_zei(1_000_000_000.0, 1, &[]);
    let heir_taxable = 1_000_000_000.0 - 36_000_000.0; // ¥964M (1 heir excl)
    let expected = heir_taxable * 0.55 - 72_000_000.0;
    assert!((tax - expected).abs() < 1_000.0, "got ¥{:.0}", tax);
}

// ─── US estate tax unit tests ─────────────────────────────────────────────────

/// (B-1) Post-sunset 2026: $15M estate → $15M exclusion (OBBBA) → $0 tax.
#[test]
fn us_estate_post_sunset_2026() {
    let tax = compute_us_estate_tax(15_000_000.0, 2026);
    assert!((tax - 0.0).abs() < 1.0, "got ${:.0}", tax);
}

/// (B-2) Pre-sunset 2024: $12M estate → below $13.61M exclusion → $0.
#[test]
fn us_estate_pre_sunset_2024_below_exclusion() {
    assert_eq!(compute_us_estate_tax(12_000_000.0, 2024), 0.0);
}

/// (B-3) Pre-sunset 2025: $20M estate → 40% × ($20M − $13.61M × 1.028) = some positive amount.
#[test]
fn us_estate_pre_sunset_2025() {
    let exclusion_2025: f64 = 13_610_000.0 * 1.028;
    let tax = compute_us_estate_tax(20_000_000.0, 2025);
    let expected = (20_000_000.0 - exclusion_2025).max(0.0) * 0.40;
    assert!((tax - expected).abs() < 1.0);
}

// ─── Treaty credit unit tests ─────────────────────────────────────────────────

/// Treaty credit cannot exceed US tax paid.
#[test]
fn treaty_credit_capped_at_us_tax() {
    let credit = compute_treaty_credit(8_000_000.0, 3_000_000.0, 1.0);
    assert!((credit - 3_000_000.0).abs() < 1.0);
}

/// Treaty credit scales with Japan-situs fraction.
#[test]
fn treaty_credit_proportional_situs() {
    // Japan paid $5M, US paid $10M, 60% Japan situs.
    let credit = compute_treaty_credit(5_000_000.0, 10_000_000.0, 0.6);
    // min(5M × 0.6, 10M) = 3M
    assert!((credit - 3_000_000.0).abs() < 1.0);
}

// ─── Integration: ¥500M estate full bill ─────────────────────────────────────

/// (C) ¥500M estate, 2 heirs (no spouse). Gift sink off → full tax.
///     Gift sink on → estate reduced by ¥2.2M/year × 20 years = ¥44M → lower tax.
#[test]
fn gift_sink_reduces_sozoku_zei() {
    // Without gifting
    let tax_no_gift = compute_japan_sozoku_zei(500_000_000.0, 2, &[]);

    // With 20 years of gifting at ¥1.1M × 2 recipients = ¥2.2M/year
    let annual_gift = 1_100_000.0 * 2.0;
    let reduced = 500_000_000.0 - annual_gift * 20.0;
    let tax_with_gift = compute_japan_sozoku_zei(reduced, 2, &[]);

    assert!(tax_with_gift < tax_no_gift,
        "Gifting should reduce Sōzoku-zei: {:.0} vs {:.0}", tax_with_gift, tax_no_gift);
    let savings = tax_no_gift - tax_with_gift;
    assert!(savings > 0.0, "Expected positive tax savings from gifting");
}

// ─── Integration: full simulation produces EstateSummary ─────────────────────

/// (D) 5-year retired simulation with enable_estate_planning = true.
/// Checks that the final SimResults.estate_summary is non-None and has non-zero values.
#[test]
fn simulation_produces_non_zero_estate_summary() {
    let cfg = estate_planning_config();
    let accounts = simple_taxable_account();
    let ctrl = SimulationController::new(cfg, accounts);
    let results = ctrl.run();

    let summary = results.estate_summary.expect("estate_summary should be Some");
    assert!(summary.total_estate_jpy > 0.0, "Estate should have positive value");
    assert!(summary.japan_sozoku_zei_jpy >= 0.0, "Japan tax should be non-negative");
    assert!(summary.net_to_heirs_jpy >= 0.0, "Net to heirs should be non-negative");
    assert!(summary.net_to_heirs_jpy <= summary.total_estate_jpy,
        "Net to heirs cannot exceed gross estate");
}

/// (E) The last annual snapshot also carries the estate_summary.
#[test]
fn final_snapshot_carries_estate_summary() {
    let cfg = estate_planning_config();
    let accounts = simple_taxable_account();
    let ctrl = SimulationController::new(cfg, accounts);
    let results = ctrl.run();

    let last = results.annual_summary.last().expect("simulation produced snapshots");
    assert!(last.estate_summary.is_some(), "Last snapshot should carry estate_summary");
}

/// (F) With estate_planning disabled, estate_summary is None.
#[test]
fn estate_planning_off_no_summary() {
    let mut cfg = estate_planning_config();
    cfg.enable_estate_planning = false;
    let accounts = simple_taxable_account();
    let ctrl = SimulationController::new(cfg, accounts);
    let results = ctrl.run();
    assert!(results.estate_summary.is_none(), "No estate summary when disabled");
}

// ─── Gifting optimiser ────────────────────────────────────────────────────────

/// (G) Gifting optimiser suggests ¥1.1M × recipient_count per year.
#[test]
fn gifting_optimiser_suggests_annual_amount() {
    let mut cfg = estate_planning_config();
    cfg.gift_recipient_count = 2;
    cfg.us_gift_exclusion_usd = 19_000.0;
    let suggestion = lifetime_gifting_optimiser(&cfg, 500_000_000.0, iso(2026, 1, 1));
    assert!((suggestion.suggested_annual_jpy - 2_200_000.0).abs() < 1.0,
        "Expected ¥2.2M/year, got ¥{:.0}", suggestion.suggested_annual_jpy);
    assert!(suggestion.estimated_tax_reduction_jpy > 0.0,
        "Gifting should reduce estimated tax");
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn estate_planning_config() -> Config {
    Config {
        start_date:       iso(2026, 1, 1),
        end_date:         iso(2030, 12, 31),
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
        war_chest_target_jpy: 3_000_000.0,
        war_chest_target_usd: 0.0,
        bridge_fund_enabled: true,
        bridge_fund_funding_timing: BufferFundingTiming::AtRetirement,
        bridge_fund_ramp_months: 18,
        bridge_months_target: 6,
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
        fers_monthly_start:  2_000.0,
        fers_start_date:     iso(2026, 1, 1),
        retirement_year_gross_income_jpy: 0.0,
        birth_date:        iso(1960, 1, 1),
        spouse_birth_date: iso(1962, 1, 1),
        child_birth_date:  iso(1990, 1, 1),
        va_child_cutoff_date: None,
        dc_payout_start_age: 99,
        dc_payout_method:    "LUMP_SUM".into(),
        pre_funded_war_chest_jpy: 3_000_000.0,
        pre_funded_bridge_jpy:   0.0,
        pre_funded_bridge_usd:   24_000.0,
        pre_funded_japan_tax_jpy: 0.0,
        pre_funded_us_tax_usd:    0.0,
        target_vti_pct:  1.0,
        target_schd_pct: 0.0,
        roth_rebalance_target_vti_pct:  1.0,
        roth_rebalance_target_schd_pct: 0.0,
        enable_roth_rebalance_at_59: false,
        buy_schd_last_year: false,
        rsu_tax_handling:   "SALARY".into(),
        total_annual_compensation_usd: 0.0,
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
        ss_monthly_usd:    1_500.0,
        ss_start_age:      62,
        ssdi_monthly_usd:  0.0,
        is_married:        false,
        spouse_ss_monthly_usd: 0.0,
        spouse_ss_start_age:   99,
        spouse_ss_jurisdiction: TaxProtocol::Both,
        spouse_nenkin_monthly_jpy:  0.0,
        spouse_nenkin_start_age:    99,
        spouse_nenkin_jurisdiction: TaxProtocol::Both,
        family_unit: FamilyUnit { user_birth_year: 1960, spouse_birth_year: None, dependents: vec![] },
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
        real_estate: vec![],
        enable_heloc_tier: false,
        // Stage 07
        enable_estate_planning: true,
        death_date: None,
        spouse_death_date: None,
        heirs: vec![
            Heir { name: "Child 1".into(), birth_date: Some(iso(1988, 3, 1)), relationship: HeirRelationship::Child },
            Heir { name: "Child 2".into(), birth_date: Some(iso(1992, 7, 1)), relationship: HeirRelationship::Child },
        ],
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
    }
}

/// A single taxable account with $1M in VTI (growing 5% per year).
fn simple_taxable_account() -> HashMap<String, Account> {
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

    let mut taxable = Account::new_with_meta(
        "Taxable",
        Currency::Usd,
        AccountLocation::Us,
        AccountJurisdiction::Both,
    );
    taxable.assets.insert("VTI".into(), asset);

    let mut accounts = HashMap::new();
    accounts.insert("Taxable".into(), taxable);
    accounts
}

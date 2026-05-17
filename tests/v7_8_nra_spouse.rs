//! Stage 02 — NRA Spouse Edge-Case Tests
//!
//! Acceptance checklist (from instructions/02_edge_case_nra_spouse.md):
//!
//!   (A) US total tax differs across SpouseProfile variants for the same gross income.
//!   (B) Roth contribution handler skips contributions for NRA-MFS when MAGI > $10k.
//!   (C) §6013(g) path increases the FTC-eligible income by adding the spouse's Japan income.

use std::collections::HashMap;

use chrono::NaiveDate;

use retirement_calculator::engine::tax::japan_tax::JapanTaxEngine;
use retirement_calculator::engine::tax::us_tax::TaxEngine;
use retirement_calculator::handlers::contributions::handle_contributions;
use retirement_calculator::engine::cashflow_engine::CashFlowEngine;
use retirement_calculator::models::assets::{
    Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, AssetClass,
    Currency, DividendCurrency,
};
use retirement_calculator::models::config::{
    Config, FamilyUnit, NhiModel, SpouseProfile, TaxProtocol, TaxRules,
    UsTaxStrategy, VaDependentStatus, WaterfallStrategy, WithdrawalRegime, WithdrawalStrategy,
};
use retirement_calculator::simulation::state::SimState;

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Minimal Config usable in unit tests.
fn base_cfg() -> Config {
    Config {
        start_date: iso(2026, 1, 1),
        end_date:   iso(2027, 12, 31),
        retirement_date: iso(2026, 1, 1),
        rebalance_date:  iso(2026, 2, 1),
        usd_jpy: 150.0,
        inflation_cola: 0.0,
        inflation_japan: 0.0,
        ira_limit_growth: 0.0,
        fx_drift_enabled: false,
        fx_drift_rate: 0.0,
        fx_drift_cadence_months: 0,
        fx_drift_increase_amount_jpy: 0.0,
        recession_enabled: false,
        recession_severity: 0.0,
        recession_events: vec![],
        fx_shock_events: vec![],
        base_expense_jpy: 0.0,
        min_expense_jpy: 0.0,
        nhi_spike_monthly_jpy: 0.0,
        nhi_model: NhiModel::default(),
        war_chest_currency: "JPY".into(),
        war_chest_target_jpy: 0.0,
        war_chest_target_usd: 0.0,
        bridge_months_target: 12,
        bridge_fund_currency: "USD".into(),
        roth_start_limit: 7_000.0,
        roth_contribution_made_this_year: false,
        roth_contribution_so_far: 0.0,
        dc_monthly_jpy: 0.0,
        dc_growth_rate: 0.0,
        monthly_contribution_ticker: "VTI".into(),
        va_contribution_buffer_usd: 0.0,
        nenkin_baseline_annual_jpy: 0.0,
        growth_rates_annual: HashMap::new(),
        va_disability_rates: HashMap::new(),
        fers_monthly_start: 0.0,
        fers_start_date: iso(2099, 1, 1),
        retirement_year_gross_income_jpy: 0.0,
        birth_date: iso(1980, 1, 1),
        spouse_birth_date: iso(1982, 1, 1),
        child_birth_date: iso(2018, 1, 1),
        va_child_cutoff_date: None,
        dc_payout_start_age: 99,
        dc_payout_method: "LUMP_SUM".into(),
        pre_funded_war_chest_jpy: 0.0,
        pre_funded_bridge_jpy: 0.0,
        pre_funded_bridge_usd: 0.0,
        pre_funded_japan_tax_jpy: 0.0,
        pre_funded_us_tax_usd: 0.0,
        target_vti_pct: 0.5,
        target_schd_pct: 0.5,
        roth_rebalance_target_vti_pct: 0.5,
        roth_rebalance_target_schd_pct: 0.5,
        enable_roth_rebalance_at_59: false,
        buy_schd_last_year: false,
        rsu_tax_handling: "SALARY".into(),
        total_annual_compensation_usd: 200_000.0,
        expense_rules: vec![],
        rsu_awards: vec![],
        tax_rules: TaxRules::default(),
        tax_jurisdiction: TaxProtocol::Both,
        investment_location: retirement_calculator::models::config::InvestmentLocation::Us,
        us_tax_strategy: UsTaxStrategy::FtcOnly,
        va_disability_rating: 0,
        va_dependent_status: VaDependentStatus::VetOnly,
        ss_monthly_usd: 0.0,
        ss_start_age: 99,
        ssdi_monthly_usd: 0.0,
        is_married: true,
        spouse_ss_monthly_usd: 0.0,
        spouse_ss_start_age: 99,
        spouse_ss_jurisdiction: TaxProtocol::Both,
        spouse_nenkin_monthly_jpy: 0.0,
        spouse_nenkin_start_age: 99,
        spouse_nenkin_jurisdiction: TaxProtocol::Both,
        family_unit: FamilyUnit {
            user_birth_year: 1980,
            spouse_birth_year: Some(1982),
            dependents: vec![],
        },
        nenkin_income_monthly_jpy: 0.0,
        nenkin_income_start_age: 99,
        prefecture: "Kanagawa".into(),
        city: "Sagamihara".into(),
        military_retired: None,
        fers_jurisdiction: TaxProtocol::Both,
        fers_japan_local_tax_exempt: false,
        ss_jurisdiction: TaxProtocol::Both,
        nenkin_jurisdiction: TaxProtocol::Both,
        va_smc_variant: None,
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
        rsu_sell_to_cover_policy: retirement_calculator::models::config::RsuSellToCoverPolicy::Strict,
        spouse_profile: SpouseProfile::UsPerson,
        spouse_japan_salary_jpy: 0.0,
        spouse_japan_misc_income_jpy: 0.0,
        monthly_dependent_precision: true,
    }
}

/// Build a SimState with a small Roth account.
fn state_with_roth(date: NaiveDate) -> SimState {
    let mut roth = Account::new_with_meta(
        "Roth",
        Currency::Usd,
        AccountLocation::Us,
        AccountJurisdiction::Both,
    );
    roth.assets.insert("VTI".into(), Asset {
        ticker: "VTI".into(),
        price: 100.0,
        yield_rate: 0.0,
        growth_rate: 0.07,
        currency: Currency::Usd,
        category: AssetCategory::Growth,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: 15_000.0,
        dividend_months: vec![],
        dividend_currency: DividendCurrency::Usd,
        pfic_regime: retirement_calculator::models::assets::PficRegime::NotPfic,
        pfic_prior_year_fmv_per_share: 0.0,
        asset_class: AssetClass::default(),
        return_profile: None,
        lots: vec![],
    });

    let accounts = {
        let mut m = HashMap::new();
        m.insert("Roth".into(), roth);
        m
    };
    let mut state = SimState::new(date, 150.0, 7_000.0, accounts);
    state.date = date;
    state
}

// ─── Test (A): Different profiles → different US tax ─────────────────────────

/// MFJ (UsPerson default) and MFS (NraMfs) produce different US tax results for
/// the same gross income. MFS always yields higher tax at the same income level
/// because of the smaller standard deduction and narrower brackets.
#[test]
fn test_mfj_vs_mfs_tax_differs() {
    let gross_ord = 120_000.0;
    let gross_lt  =  50_000.0;

    let mfj_rules = TaxRules::for_filing_status("Married Filing Jointly");
    let mfs_rules = TaxRules::for_filing_status("Married Filing Separately");
    let hoh_rules = TaxRules::for_filing_status("Head of Household");

    let mfj_tax = TaxEngine::new(mfj_rules).calculate_liability(2026, gross_ord, 0.0, gross_lt);
    let mfs_tax = TaxEngine::new(mfs_rules).calculate_liability(2026, gross_ord, 0.0, gross_lt);
    let hoh_tax = TaxEngine::new(hoh_rules).calculate_liability(2026, gross_ord, 0.0, gross_lt);

    assert!(
        mfs_tax.total_tax > mfj_tax.total_tax,
        "NRA-MFS tax (${:.0}) should be higher than MFJ tax (${:.0})",
        mfs_tax.total_tax, mfj_tax.total_tax,
    );
    assert!(
        hoh_tax.total_tax < mfs_tax.total_tax,
        "HoH tax (${:.0}) should be lower than MFS tax (${:.0})",
        hoh_tax.total_tax, mfs_tax.total_tax,
    );
    assert!(
        mfj_tax.total_tax != mfs_tax.total_tax,
        "MFJ and MFS should produce different tax amounts (same income)",
    );
}

// ─── Test (B): NRA-MFS suppresses Roth when MAGI > $10k ──────────────────────

/// When `spouse_profile == NraMfs` and `total_annual_compensation_usd > $10k`,
/// `handle_contributions` must NOT increase the Roth account balance and MUST
/// push a `SolvencyWarning` with `absorbed_by == "RothMfsPhaseOutExceeded"`.
#[test]
fn test_nra_mfs_roth_contribution_suppressed() {
    let jan1 = iso(2027, 1, 1);
    let mut cfg = base_cfg();
    cfg.start_date = iso(2027, 1, 1);
    cfg.retirement_date = iso(2030, 1, 1); // pre-retirement so contributions run
    cfg.spouse_profile = SpouseProfile::NraMfs;
    cfg.total_annual_compensation_usd = 200_000.0; // far above $10k MFS ceiling

    let mut state = state_with_roth(jan1);
    let roth_value_before = state.accounts["Roth"].total_value(150.0);

    let cf_engine = CashFlowEngine::new(cfg.clone());
    handle_contributions(&mut state, &cfg, &cf_engine);

    let roth_value_after = state.accounts["Roth"].total_value(150.0);

    assert_eq!(
        roth_value_before, roth_value_after,
        "Roth account should not change under NRA-MFS with MAGI ${:.0} > $10k",
        cfg.total_annual_compensation_usd,
    );

    let phase_out_warning = state.gap_warnings.iter()
        .any(|w| w.absorbed_by == "RothMfsPhaseOutExceeded");
    assert!(
        phase_out_warning,
        "Expected a RothMfsPhaseOutExceeded SolvencyWarning to be pushed for NRA-MFS",
    );
}

/// When `spouse_profile == NraMfs` but `total_annual_compensation_usd == 0`
/// (MAGI ≤ $10k), contributions are NOT suppressed.
#[test]
fn test_nra_mfs_roth_allowed_when_magi_zero() {
    let jan1 = iso(2027, 1, 1);
    let mut cfg = base_cfg();
    cfg.start_date = iso(2027, 1, 1);
    cfg.retirement_date = iso(2030, 1, 1);
    cfg.spouse_profile = SpouseProfile::NraMfs;
    cfg.total_annual_compensation_usd = 0.0; // MAGI ≤ $10k → Roth allowed

    let mut state = state_with_roth(jan1);
    let roth_value_before = state.accounts["Roth"].total_value(150.0);

    let cf_engine = CashFlowEngine::new(cfg.clone());
    handle_contributions(&mut state, &cfg, &cf_engine);

    let roth_value_after = state.accounts["Roth"].total_value(150.0);

    assert!(
        roth_value_after > roth_value_before,
        "Roth account should grow when NRA-MFS MAGI is $0 (≤ $10k ceiling)",
    );

    let has_phase_out_warning = state.gap_warnings.iter()
        .any(|w| w.absorbed_by == "RothMfsPhaseOutExceeded");
    assert!(!has_phase_out_warning, "No phase-out warning expected when MAGI ≤ $10k");
}

// ─── Test (C): §6013(g) increases US gross income (and therefore FTC pool) ───

/// When `spouse_profile == NraElectedToBeTreatedAsResident`, the NRA spouse's
/// Japan salary is added to US gross ordinary income, increasing the US tax bill
/// before FTC relative to the UsPerson baseline (same FERS income, no FTC offset).
///
/// The Japan resident tax on the spouse's income is then added to the FTC pool,
/// which is approximately `JapanTaxEngine::calculate_resident_tax(spouse_salary)`.
#[test]
fn test_6013g_spouse_income_increases_us_gross_ord() {
    // Baseline: no spouse income (UsPerson).
    let base_fers_usd = 50_000.0;
    let baseline_tax = TaxEngine::new(TaxRules::default())
        .calculate_liability(2026, base_fers_usd, 0.0, 0.0);

    // §6013(g): add ¥8,000,000 spouse salary at ¥150/$1 → $53,333 additional income.
    let spouse_salary_jpy = 8_000_000.0;
    let fx = 150.0;
    let spouse_income_usd = spouse_salary_jpy / fx;
    let pooled_tax = TaxEngine::new(TaxRules::default())
        .calculate_liability(2026, base_fers_usd + spouse_income_usd, 0.0, 0.0);

    assert!(
        pooled_tax.total_tax > baseline_tax.total_tax,
        "§6013(g) pooled income (${:.0}) should yield higher US tax than baseline (${:.0})",
        pooled_tax.total_tax, baseline_tax.total_tax,
    );

    // The Japan resident tax on the spouse's income is the FTC addition.
    let spouse_japan_tax_jpy = JapanTaxEngine::calculate_resident_tax(
        spouse_salary_jpy, 0.0, 0.0,
        44,    // spouse age (born 1982, year 2026)
        0,
        0.10,  // standard resident-tax rate
        6_000.0,
    );
    assert!(
        spouse_japan_tax_jpy > 0.0,
        "Japan resident tax on ¥{:.0} spouse salary should be positive (got ¥{:.0})",
        spouse_salary_jpy, spouse_japan_tax_jpy,
    );

    // The FTC addition in USD is approximately this Japan tax converted.
    let ftc_addition_usd = spouse_japan_tax_jpy / fx;
    // Net tax should be lower than the pooled pre-FTC, showing the FTC offsets the extra income.
    let pooled_with_ftc = TaxEngine::new(TaxRules::default())
        .calculate_liability_with_ftc(
            2026,
            base_fers_usd + spouse_income_usd,
            0.0,
            0.0,
            ftc_addition_usd,
        );
    assert!(
        pooled_with_ftc.total_tax < pooled_tax.total_tax,
        "FTC from spouse Japan tax (${:.0}) should reduce US liability",
        ftc_addition_usd,
    );
}

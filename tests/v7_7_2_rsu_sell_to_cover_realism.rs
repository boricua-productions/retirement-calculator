//! V7.7.2 — RSU Sell-to-Cover Realism Integration Tests
//!
//! These tests verify the RSU margin-call realism layer described in Stage 01 of
//! the edge-case spec. The acceptance criteria are:
//!
//!   (A) When `rsu_sell_to_cover_realism = true` and combined US + Japan tax exceeds
//!       the vest proceeds, the deficit cascade drains Bridge Fund → War Chest in order
//!       and records any residual as `unpaid_rsu_tax_liability_usd`.
//!
//!   (B) When `rsu_sell_to_cover_realism = false` (legacy), only the US marginal tax
//!       is considered. The `buy_amount` floors at zero silently; buffers are untouched.
//!
//! To trigger the deficit path we use a synthetic TaxEngine with a flat 99% bracket
//! and zero standard deduction. Combined with the Japan marginal tax at the given
//! salary level this pushes the combined rate above 100% of the vest value.

use std::collections::HashMap;

use chrono::NaiveDate;

use retirement_calculator::engine::rsu_engine::RsuEngine;
use retirement_calculator::engine::tax::us_tax::TaxEngine;
use retirement_calculator::handlers::cashflow_manager::cover_usd_deficit_from_buffers;
use retirement_calculator::handlers::rsu_vesting::handle_rsu_vesting;
use retirement_calculator::models::assets::{
    Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, Currency, DividendCurrency,
};
use retirement_calculator::models::config::{
    Config, FamilyUnit, NhiModel, RsuSellToCoverPolicy, ShockOrdering, SpouseProfile, TaxProtocol,
    TaxRules, VaDependentStatus, WithdrawalRegime, WithdrawalStrategy, WaterfallStrategy,
    UsTaxStrategy, InvestmentLocation,
};
use retirement_calculator::models::rsu::{RsuAward, VestingCadence};
use retirement_calculator::simulation::state::SimState;

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Builds a TaxEngine with a flat 99% bracket and zero standard deduction so that
/// US marginal tax ≈ 99% of any additional income, making it easy to trigger the
/// combined-tax > vest-value condition.
fn extreme_tax_engine() -> TaxEngine {
    let mut rules = TaxRules::default();
    rules.std_deduction = 0.0;
    rules.brackets = vec![(f64::INFINITY, 0.99)];
    TaxEngine::new(rules)
}

/// Builds a minimal SELL_TO_COVER Config with the specified realism setting.
fn stc_config(realism: bool) -> Config {
    Config {
        start_date:      iso(2030, 1, 1),
        end_date:        iso(2040, 12, 31),
        retirement_date: iso(2035, 1, 1),
        rebalance_date:  iso(2035, 2, 1),
        usd_jpy:         150.0,
        inflation_cola:  0.0,
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
        roth_start_limit: 0.0,
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
        child_birth_date: iso(2010, 1, 1),
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
        rsu_tax_handling: "SELL_TO_COVER".into(),
        total_annual_compensation_usd: 0.0,
        expense_rules: vec![],
        rsu_awards: vec![],
        tax_rules: TaxRules::default(),
        tax_jurisdiction: TaxProtocol::Both,
        investment_location: InvestmentLocation::Us,
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
        rsu_sell_to_cover_realism: realism,
        rsu_sell_to_cover_policy: RsuSellToCoverPolicy::Strict,
        // Stage 02 defaults
        spouse_profile: SpouseProfile::UsPerson,
        spouse_japan_salary_jpy: 0.0,
        spouse_japan_misc_income_jpy: 0.0,
        // Stage 03 defaults
        monthly_dependent_precision: true,
        // Stage 04 defaults
        shock_ordering: ShockOrdering::DepreciateThenReprice,
        // Stage 05 defaults
        track_pfic_basis_drift: true,
    }
}

/// Build an RSU award that vests 100 shares of a synthetic "TCKR" ticker in
/// January 2030, using unit_value = $10 as the price oracle.
fn panw_cliff_award() -> RsuAward {
    RsuAward {
        grant_date:          iso(2029, 12, 1),
        vesting_start_date:  Some(iso(2030, 1, 1)),
        ticker:              "TCKR".into(),
        total_shares:        100.0,
        vesting_years:       1,
        vesting_months_total: None,
        vesting_months:      vec![1],  // vest once in January
        vesting_cadence:     VestingCadence::Annually,
        cliff_vest_months:   0,
        unit_value:          Some(10.0),  // $10/share post-recession price
        growth_rate:         None,
        return_profile:      None,
        migrate_on_retirement: false,
    }
}

/// Build a SimState with buffers set for the cascade test:
///   bridge_fund_usd = $50, war_chest_jpy = ¥10,000, no T8 equity.
/// year_salary_jpy = ¥5M to push Japan income into tax-paying territory.
fn cascade_state(accounts: HashMap<String, Account>) -> SimState {
    let mut state = SimState::new(iso(2030, 1, 1), 150.0, 7_000.0, accounts);
    state.date = iso(2030, 1, 1);
    state.current_fx = 150.0;
    state.bridge_fund_usd = 50.0;
    state.war_chest_jpy = 10_000.0;
    // Simulate ¥5M salary already accumulated for this year (raises Japan marginal rate).
    state.stats.year_salary_jpy = 5_000_000.0;
    state
}

fn empty_accounts() -> HashMap<String, Account> {
    let mut m = HashMap::new();
    m.insert(
        "Taxable".into(),
        Account::new_with_meta("Taxable", Currency::Usd, AccountLocation::Us, AccountJurisdiction::Both),
    );
    m
}

// ── (A) Deficit cascade drains Bridge Fund → War Chest in order ──────────────

/// With extreme tax brackets, the combined US + Japan tax exceeds the $1,000 vest.
/// The cascade drains Bridge Fund first, then War Chest. Any residual that cannot
/// be covered by the (empty) Taxable account is recorded as an unpaid IRS liability.
#[test]
fn rsu_stc_realism_deficit_triggers_cascade_and_unpaid_liability() {
    let cfg = stc_config(true);
    let award = panw_cliff_award();
    let rsu_engine = RsuEngine::new(vec![award], Some(cfg.retirement_date));
    let tax_engine = extreme_tax_engine();

    let mut state = cascade_state(empty_accounts());
    let bridge_before = state.bridge_fund_usd;
    let wc_before     = state.war_chest_jpy;

    handle_rsu_vesting(
        &mut state,
        &cfg,
        &rsu_engine,
        &tax_engine,
        |s, _yr| s.stats.acc_ord_inc,
    );

    // --- Combined tax should exceed $1,000 vest, triggering the deficit path ---
    // US marginal (99% flat) = $990, Japan ≈ $161, combined > $1,000.
    assert!(
        state.rsu_sell_to_cover_warnings.len() >= 1,
        "Expected at least one RSU sell-to-cover warning (deficit event recorded). Got: {:?}",
        state.rsu_sell_to_cover_warnings,
    );

    // Bridge Fund was drained (partially or fully) before War Chest.
    assert!(
        state.bridge_fund_usd < bridge_before,
        "Bridge Fund should have been drained (before=${:.2}, after=${:.2}).",
        bridge_before, state.bridge_fund_usd,
    );

    // War Chest was also drawn on (cascade continues after bridge exhausted).
    assert!(
        state.war_chest_jpy < wc_before,
        "War Chest should have been drawn on (before=¥{:.0}, after=¥{:.0}).",
        wc_before, state.war_chest_jpy,
    );

    // Unpaid liability is positive because no T8 equity was available.
    assert!(
        state.unpaid_rsu_tax_liability_usd > 0.0,
        "Expected unpaid_rsu_tax_liability_usd > 0 (no T8 equity to cover residual). \
         Got: {:.2}",
        state.unpaid_rsu_tax_liability_usd,
    );

    // RSU vest value is still tracked in stats (gross income recorded regardless of tax).
    let vest_value = 100.0 * 10.0;
    assert!(
        (state.stats.year_rsu_vest_usd - vest_value).abs() < 0.01,
        "year_rsu_vest_usd should equal the gross vest (${:.2}).", vest_value,
    );
}

// ── (B) Legacy path (realism = false) leaves buffers untouched ───────────────

/// With `rsu_sell_to_cover_realism = false`, the engine reverts to the pre-V7.7.2
/// behaviour: only US marginal tax is considered, `net` is floored at zero, and
/// no buffer drain or warning is recorded.
#[test]
fn rsu_stc_legacy_path_does_not_drain_buffers() {
    let cfg = stc_config(false);
    let award = panw_cliff_award();
    let rsu_engine = RsuEngine::new(vec![award], Some(cfg.retirement_date));
    let tax_engine = extreme_tax_engine();

    let mut state = cascade_state(empty_accounts());
    let bridge_before = state.bridge_fund_usd;
    let wc_before     = state.war_chest_jpy;

    handle_rsu_vesting(
        &mut state,
        &cfg,
        &rsu_engine,
        &tax_engine,
        |s, _yr| s.stats.acc_ord_inc,
    );

    // Legacy path: no warnings, no unpaid liability.
    assert!(
        state.rsu_sell_to_cover_warnings.is_empty(),
        "Legacy path must not emit RSU warnings (got: {:?})", state.rsu_sell_to_cover_warnings,
    );
    assert_eq!(
        state.unpaid_rsu_tax_liability_usd, 0.0,
        "Legacy path must not record any unpaid liability.",
    );

    // Buffers must be completely untouched.
    assert_eq!(
        state.bridge_fund_usd, bridge_before,
        "Bridge Fund must be unchanged in legacy path.",
    );
    assert_eq!(
        state.war_chest_jpy, wc_before,
        "War Chest must be unchanged in legacy path.",
    );
}

// ── (C) Cascade helper drains Bridge → War Chest in strict order ─────────────

/// Unit-level test for `cover_usd_deficit_from_buffers`.
/// Verifies the three-step cascade: Bridge first, then War Chest, then T8.
/// Uses no T8 equity so the residual becomes the uncovered amount.
#[test]
fn cover_usd_deficit_drains_bridge_then_warchest() {
    let cfg = stc_config(true);
    let mut accounts = empty_accounts();

    // Add a small VTI position to verify T8 fires as step 3 when needed.
    let mut vti = Asset {
        ticker: "VTI".into(),
        price: 200.0,
        yield_rate: 0.0,
        growth_rate: 0.07,
        currency: Currency::Usd,
        category: AssetCategory::Income,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: 15_000.0,
        dividend_months: vec![3, 6, 9, 12],
        dividend_currency: DividendCurrency::Usd,
        pfic_regime: retirement_calculator::models::assets::PficRegime::NotPfic,
        pfic_prior_year_fmv_per_share: 0.0,
        pfic_prior_year_fmv_per_share_jpy: 0.0,
        pfic_mtm_loss_carryforward_usd: 0.0,
        pfic_qef_election_year: None,
        asset_class: retirement_calculator::models::assets::AssetClass::default(),
        return_profile: None,
        lots: Vec::new(),
    };
    vti.add_lot(iso(2025, 1, 1), 5.0, 750.0); // 5 shares at $150 avg cost
    accounts.get_mut("Taxable").unwrap().assets.insert("VTI".into(), vti);

    let mut state = SimState::new(iso(2030, 1, 1), 150.0, 7_000.0, accounts);
    state.date = iso(2030, 1, 1);
    state.current_fx = 150.0;
    state.bridge_fund_usd = 30.0;
    state.war_chest_jpy = 6_000.0;

    let penalty = 0.005_f64;
    let deficit = 100.0_f64; // $100 to cover

    let uncovered = cover_usd_deficit_from_buffers(&mut state, &cfg, deficit, penalty);

    // Bridge should be fully drained (was $30 < $100 deficit).
    assert!(
        state.bridge_fund_usd <= 0.001,
        "Bridge Fund should be exhausted after cascade. Got: ${:.4}", state.bridge_fund_usd,
    );

    // War Chest should be reduced (was ¥6,000, contributed to covering the gap).
    assert!(
        state.war_chest_jpy < 6_000.0,
        "War Chest should have been drawn on. Got: ¥{:.0}", state.war_chest_jpy,
    );

    // Remaining deficit covered by T8 — uncovered should be ~0 with enough VTI.
    assert!(
        uncovered < 5.0,
        "Expected T8 liquidation to cover most of the residual. Uncovered: ${:.4}", uncovered,
    );
}

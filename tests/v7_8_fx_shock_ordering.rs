//! Stage 04 — FX Shock + Recession Ordering Tests
//!
//! Acceptance criteria (from instructions/04_edge_case_fx_market_shock_order.md):
//!
//!   (A) All three `ShockOrdering` variants populate `pre_shock_net_worth_jpy` and
//!       `post_shock_net_worth_jpy` in the snapshot for any year that has both a
//!       recession event and an FX shock event.
//!   (B) The pre-shock and post-shock JPY values are mathematically predictable from
//!       the initial portfolio, severity, and target FX.
//!   (C) The `debug_assert!` in `liquidate_for_jpy_gap` does not fire — verifying that
//!       Tier 8 uses the post-shock FX rate.
//!   (D) Years without a combined shock do NOT have pre/post fields set.

use std::collections::HashMap;

use chrono::NaiveDate;

use retirement_calculator::models::assets::{
    Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, AssetClass,
    Currency, DividendCurrency, PficRegime,
};
use retirement_calculator::models::config::{
    Config, FamilyUnit, FXShockEvent, NhiModel, RecessionEvent, ShockOrdering,
    SpouseProfile, TaxProtocol, TaxRules, VaDependentStatus, WaterfallStrategy,
    WithdrawalRegime, WithdrawalStrategy, UsTaxStrategy, InvestmentLocation,
    RsuSellToCoverPolicy,
};
use retirement_calculator::simulation::controller::SimulationController;

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Build a minimal retired Config with both recession and FX shock in 2030.
/// $100k in VTI at ¥145/$; recession -35%; FX shock → ¥80/$.
fn shock_config(ordering: ShockOrdering) -> Config {
    Config {
        start_date:       iso(2028, 1, 1),
        end_date:         iso(2032, 12, 31),
        retirement_date:  iso(2028, 1, 1),
        rebalance_date:   iso(2028, 2, 1),
        usd_jpy:          145.0,
        inflation_cola:   0.0,
        inflation_japan:  0.0,
        ira_limit_growth: 0.0,
        fx_drift_enabled: false,
        fx_drift_rate: 0.0,
        fx_drift_cadence_months: 0,
        fx_drift_increase_amount_jpy: 0.0,
        recession_enabled: false,
        recession_severity: 0.0,
        recession_events: vec![RecessionEvent {
            year: 2030,
            severity: 0.35,
            duration_months: 1,
            recovery_months: 0,
        }],
        fx_shock_events: vec![FXShockEvent {
            year: 2030,
            target_fx: 80.0,
            description: "test yen surge".into(),
        }],
        base_expense_jpy: 200_000.0,
        min_expense_jpy:  100_000.0,
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
        fers_monthly_start: 2_000.0,
        fers_start_date: iso(2028, 1, 1),
        retirement_year_gross_income_jpy: 0.0,
        birth_date: iso(1975, 1, 1),
        spouse_birth_date: iso(1977, 1, 1),
        child_birth_date: iso(2010, 1, 1),
        va_child_cutoff_date: None,
        dc_payout_start_age: 99,
        dc_payout_method: "LUMP_SUM".into(),
        pre_funded_war_chest_jpy: 0.0,
        pre_funded_bridge_jpy: 0.0,
        pre_funded_bridge_usd: 5_000.0,
        pre_funded_japan_tax_jpy: 0.0,
        pre_funded_us_tax_usd: 0.0,
        target_vti_pct: 1.0,
        target_schd_pct: 0.0,
        roth_rebalance_target_vti_pct: 1.0,
        roth_rebalance_target_schd_pct: 0.0,
        enable_roth_rebalance_at_59: false,
        buy_schd_last_year: false,
        rsu_tax_handling: "SALARY".into(),
        total_annual_compensation_usd: 0.0,
        expense_rules: vec![],
        rsu_awards: vec![],
        tax_rules: TaxRules::default(),
        tax_jurisdiction: TaxProtocol::JapanOnly,
        investment_location: InvestmentLocation::Us,
        us_tax_strategy: UsTaxStrategy::FtcOnly,
        va_disability_rating: 0,
        va_dependent_status: VaDependentStatus::VetOnly,
        ss_monthly_usd: 0.0,
        ss_start_age: 99,
        ssdi_monthly_usd: 0.0,
        is_married: false,
        spouse_ss_monthly_usd: 0.0,
        spouse_ss_start_age: 99,
        spouse_ss_jurisdiction: TaxProtocol::Both,
        spouse_nenkin_monthly_jpy: 0.0,
        spouse_nenkin_start_age: 99,
        spouse_nenkin_jurisdiction: TaxProtocol::Both,
        family_unit: FamilyUnit { user_birth_year: 1975, spouse_birth_year: None, dependents: vec![] },
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
        rsu_sell_to_cover_policy: RsuSellToCoverPolicy::Strict,
        spouse_profile: SpouseProfile::UsPerson,
        spouse_japan_salary_jpy: 0.0,
        spouse_japan_misc_income_jpy: 0.0,
        monthly_dependent_precision: true,
        shock_ordering: ordering,
        track_pfic_basis_drift: true,
        real_estate: vec![],
        enable_heloc_tier: false,
    }
}

/// Build a Taxable account with 1000 shares of VTI at $100/share.
/// Pre-shock USD value: $100,000. Pre-shock JPY value at ¥145: ¥14,500,000.
fn taxable_vti_100k() -> Account {
    let mut asset = Asset {
        ticker: "VTI".into(),
        price: 100.0,
        yield_rate: 0.0,
        growth_rate: 0.0,
        currency: Currency::Usd,
        category: AssetCategory::Income,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: 80.0 * 145.0,
        dividend_months: vec![3, 6, 9, 12],
        dividend_currency: DividendCurrency::Usd,
        pfic_regime: PficRegime::NotPfic,
        pfic_prior_year_fmv_per_share: 0.0,
        pfic_prior_year_fmv_per_share_jpy: 0.0,
        pfic_mtm_loss_carryforward_usd: 0.0,
        pfic_qef_election_year: None,
        asset_class: AssetClass::Etf,
        return_profile: None,
        lots: Vec::new(),
    };
    // 1000 shares at $80/share basis
    asset.add_lot(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(), 1000.0, 80_000.0);
    let mut acc = Account::new_with_meta(
        "Taxable",
        Currency::Usd,
        AccountLocation::Us,
        AccountJurisdiction::Us,
    );
    acc.assets.insert("VTI".into(), asset);
    acc
}

/// Run a simulation with the given ordering and return the annual snapshots.
fn run_with_ordering(ordering: ShockOrdering) -> Vec<retirement_calculator::models::snapshot::AnnualSnapshot> {
    let cfg = shock_config(ordering);
    let mut accounts = HashMap::new();
    accounts.insert("Taxable".into(), taxable_vti_100k());
    let ctrl = SimulationController::new(cfg, accounts);
    ctrl.run().annual_summary
}

/// (A) Both pre_shock and post_shock fields are populated in 2030 for all orderings.
#[test]
fn test_shock_year_snapshot_populated() {
    for ordering in [
        ShockOrdering::DepreciateThenReprice,
        ShockOrdering::RepriceThenDepreciate,
        ShockOrdering::Simultaneous,
    ] {
        let snaps = run_with_ordering(ordering);
        let snap_2030 = snaps.iter().find(|s| s.year == 2030)
            .expect("2030 snapshot missing");

        assert!(
            snap_2030.pre_shock_net_worth_jpy.is_some(),
            "ordering={:?}: pre_shock_net_worth_jpy should be Some in 2030", ordering
        );
        assert!(
            snap_2030.post_shock_net_worth_jpy.is_some(),
            "ordering={:?}: post_shock_net_worth_jpy should be Some in 2030", ordering
        );
    }
}

/// (B) Pre-shock value ≈ $100k × ¥145 = ¥14,500,000 (before any shocks).
///     Post-shock value ≈ $65k × ¥80 = ¥5,200,000 (after −35% recession and FX 145→80).
///     All three orderings converge to the same pre and post values.
#[test]
fn test_shock_values_predictable_and_equal_across_orderings() {
    let expected_pre_jpy  = 100_000.0 * 145.0; // ¥14,500,000
    let expected_post_jpy = 65_000.0  * 80.0;  // ¥5,200,000

    for ordering in [
        ShockOrdering::DepreciateThenReprice,
        ShockOrdering::RepriceThenDepreciate,
        ShockOrdering::Simultaneous,
    ] {
        let snaps = run_with_ordering(ordering);
        let snap = snaps.iter().find(|s| s.year == 2030)
            .expect("2030 snapshot missing");

        let pre  = snap.pre_shock_net_worth_jpy.unwrap();
        let post = snap.post_shock_net_worth_jpy.unwrap();

        // Allow ±5% tolerance: bridge fund contributions, expense payments, and
        // dividends between January (shock) and December (snapshot) shift the totals.
        let tol = 0.05;
        assert!(
            (pre - expected_pre_jpy).abs() / expected_pre_jpy < tol,
            "ordering={:?}: pre_shock ¥{:.0} differs from expected ¥{:.0} by more than {:.0}%",
            ordering, pre, expected_pre_jpy, tol * 100.0
        );
        assert!(
            (post - expected_post_jpy).abs() / expected_post_jpy < tol,
            "ordering={:?}: post_shock ¥{:.0} differs from expected ¥{:.0} by more than {:.0}%",
            ordering, post, expected_post_jpy, tol * 100.0
        );
    }
}

/// (D) Years without a combined shock do NOT get pre/post fields set.
#[test]
fn test_non_shock_years_have_none() {
    let snaps = run_with_ordering(ShockOrdering::DepreciateThenReprice);
    for snap in &snaps {
        if snap.year == 2030 { continue; }
        assert!(
            snap.pre_shock_net_worth_jpy.is_none(),
            "year {}: pre_shock_net_worth_jpy should be None (no combined shock)", snap.year
        );
        assert!(
            snap.post_shock_net_worth_jpy.is_none(),
            "year {}: post_shock_net_worth_jpy should be None (no combined shock)", snap.year
        );
    }
}

/// (B) jpy_purchasing_power_index is 1.0 at start and increases with Japan CPI.
/// With inflation_japan=0.0 it stays at 1.0 throughout.
#[test]
fn test_jpy_purchasing_power_index_zero_inflation() {
    let snaps = run_with_ordering(ShockOrdering::DepreciateThenReprice);
    for snap in &snaps {
        assert!(
            (snap.jpy_purchasing_power_index - 1.0).abs() < 1e-9,
            "year {}: jpy_purchasing_power_index should be 1.0 with 0% inflation, got {}",
            snap.year, snap.jpy_purchasing_power_index
        );
    }
}

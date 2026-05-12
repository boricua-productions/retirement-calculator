//! V7.0 — Integration tests for the high-basis-first liquidation engine and
//! state-tax gross-up.
//!
//! Both tests construct a synthetic `SimState` with a controlled Taxable account
//! and a deficit `bridge_fund_usd`, then call `v7_liquidate_for_deficit` directly
//! so the assertions stay focused on the V7.0 contract:
//!
//!   (A) Setting `us_state_tax_rate > 0` makes the engine sell MORE shares to
//!       cover the same shortfall (so the post-year-end state-tax bill still
//!       leaves the bridge buffer net-flat).
//!   (B) When two tickers share the same price/qty, the one with the HIGHER JPY
//!       cost basis is sold first, preserving the low-basis lots for later (the
//!       "minimise realised gains in the early years" promise).

use chrono::NaiveDate;
use std::collections::HashMap;

use retirement_calculator::config::loader::load_scenario;
use retirement_calculator::handlers::cashflow_manager::v7_liquidate_for_deficit;
use retirement_calculator::models::assets::{
    Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, Currency, DividendCurrency,
};
use retirement_calculator::models::config::{Config, WithdrawalStrategy};
use retirement_calculator::simulation::state::SimState;

/// Builds a Taxable account with two tickers at the same price+qty, but
/// distinct JPY cost bases so we can observe the highest-basis-first ordering.
fn build_two_ticker_taxable(price_usd: f64, qty: f64, high_jpy: f64, low_jpy: f64) -> Account {
    let lot_date = NaiveDate::from_ymd_opt(2020, 1, 1).unwrap();
    let mut acct = Account::new_with_meta(
        "Taxable",
        Currency::Usd,
        AccountLocation::Us,
        AccountJurisdiction::Both,
    );

    let mut high = Asset {
        ticker: "HIGHB".into(),
        price: price_usd,
        yield_rate: 0.0,
        growth_rate: 0.07,
        currency: Currency::Usd,
        category: AssetCategory::Income,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: high_jpy,
        dividend_months: vec![3, 6, 9, 12],
        dividend_currency: DividendCurrency::Usd,
        lots: Vec::new(),
    };
    high.add_lot(lot_date, qty, qty * (high_jpy / 150.0));
    acct.assets.insert("HIGHB".into(), high);

    let mut low = Asset {
        ticker: "LOWB".into(),
        price: price_usd,
        yield_rate: 0.0,
        growth_rate: 0.07,
        currency: Currency::Usd,
        category: AssetCategory::Income,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: low_jpy,
        dividend_months: vec![3, 6, 9, 12],
        dividend_currency: DividendCurrency::Usd,
        lots: Vec::new(),
    };
    low.add_lot(lot_date, qty, qty * (low_jpy / 150.0));
    acct.assets.insert("LOWB".into(), low);
    acct
}

/// Loads the published FX-shock scenario, then strips down its cfg + accounts
/// to the synthetic harness used by both V7.0 tests.
fn fresh_test_state(state_rate: f64, strategy: WithdrawalStrategy) -> (Config, SimState) {
    let loaded = load_scenario("input/test_fx_shock_2032.json")
        .expect("test scenario should load");

    let mut cfg = loaded.config;
    cfg.us_state_tax_rate = state_rate;
    cfg.tax_rules.us_state_rate = state_rate;
    cfg.withdrawal_strategy = strategy;

    let fx = 150.0_f64;
    // HIGHB: ¥18,000 basis (gain ~¥1,500/sh after 20.315% → ~¥305 tax).
    // LOWB:  ¥7,500  basis (gain ~¥12,000/sh → ~¥2,438 tax — bigger drag).
    let mut accounts: HashMap<String, Account> = HashMap::new();
    accounts.insert(
        "Taxable".into(),
        build_two_ticker_taxable(/*price*/ 130.0, /*qty*/ 100.0, /*high*/ 18_000.0, /*low*/ 7_500.0),
    );
    accounts.insert(
        "Roth".into(),
        Account::new_with_meta("Roth", Currency::Usd, AccountLocation::Us, AccountJurisdiction::Us),
    );

    let mut state = SimState::new(NaiveDate::from_ymd_opt(2032, 6, 1).unwrap(), fx, 7_000.0, accounts);
    state.date = NaiveDate::from_ymd_opt(2032, 6, 1).unwrap();
    state.current_fx = fx;
    (cfg, state)
}

/// (A) State-tax raises the gross amount sold to cover an identical shortfall.
#[test]
fn v7_state_tax_increases_withdrawal_amount() {
    let shortfall_usd = 1_000.0;

    let (cfg_a, mut state_a) = fresh_test_state(0.00, WithdrawalStrategy::TotalReturn);
    state_a.bridge_fund_usd = -shortfall_usd;
    v7_liquidate_for_deficit(&mut state_a, &cfg_a);
    let proceeds_no_state_tax = state_a.stats.year_forced_liquidations_usd;

    let (cfg_b, mut state_b) = fresh_test_state(0.10, WithdrawalStrategy::TotalReturn);
    state_b.bridge_fund_usd = -shortfall_usd;
    v7_liquidate_for_deficit(&mut state_b, &cfg_b);
    let proceeds_with_state_tax = state_b.stats.year_forced_liquidations_usd;

    assert!(
        proceeds_with_state_tax > proceeds_no_state_tax,
        "V7.0: a 10% state tax must force a larger gross sale to cover the same \
         shortfall (no_state=${:.2}, with_state=${:.2})",
        proceeds_no_state_tax, proceeds_with_state_tax,
    );
    assert!(
        state_b.stats.year_state_cap_gains_tax_usd > 0.0,
        "year_state_cap_gains_tax_usd must be positive when us_state_tax_rate > 0",
    );
}

/// (B) The highest-JPY-basis ticker is liquidated before the lower-basis one.
#[test]
fn v7_highest_basis_stocks_are_sold_first() {
    let shortfall_usd = 1_000.0;
    let (cfg, mut state) = fresh_test_state(0.00, WithdrawalStrategy::TotalReturn);
    state.bridge_fund_usd = -shortfall_usd;

    let qty_high_before = state.accounts["Taxable"].assets["HIGHB"].qty();
    let qty_low_before  = state.accounts["Taxable"].assets["LOWB"].qty();
    assert!(qty_high_before > 0.0 && qty_low_before > 0.0);

    v7_liquidate_for_deficit(&mut state, &cfg);

    let qty_high_after = state.accounts["Taxable"].assets["HIGHB"].qty();
    let qty_low_after  = state.accounts["Taxable"].assets["LOWB"].qty();

    let sold_high = qty_high_before - qty_high_after;
    let sold_low  = qty_low_before  - qty_low_after;

    assert!(sold_high > 0.0,
        "Highest-basis ticker must be sold first (sold_high={:.4})", sold_high);
    assert!(
        sold_low <= 1e-6,
        "Low-basis ticker should remain untouched while high-basis qty is available \
         (sold_high={:.4}, sold_low={:.4})",
        sold_high, sold_low,
    );

    // Spec sanity: V7.0 records Japan capital-gains tax on the realised JPY gain.
    assert!(
        state.stats.year_japan_cap_gains_tax_jpy > 0.0,
        "Japan capital-gains tax must be recorded when shares are liquidated.",
    );
}

/// (C) DividendOnly strategy short-circuits the liquidation entirely — the
/// deficit remains negative and no shares are sold.
#[test]
fn v7_dividend_only_strategy_does_not_liquidate() {
    let shortfall_usd = 1_000.0;
    let (cfg, mut state) = fresh_test_state(0.05, WithdrawalStrategy::DividendOnly);
    state.bridge_fund_usd = -shortfall_usd;

    let qty_high_before = state.accounts["Taxable"].assets["HIGHB"].qty();
    let qty_low_before  = state.accounts["Taxable"].assets["LOWB"].qty();

    v7_liquidate_for_deficit(&mut state, &cfg);

    assert_eq!(state.accounts["Taxable"].assets["HIGHB"].qty(), qty_high_before);
    assert_eq!(state.accounts["Taxable"].assets["LOWB"].qty(),  qty_low_before);
    assert!(state.bridge_fund_usd < 0.0,
        "DividendOnly must leave the deficit unliquidated (cash_buffer={:.2})",
        state.bridge_fund_usd);
    assert_eq!(state.stats.year_forced_liquidations_usd, 0.0);
}

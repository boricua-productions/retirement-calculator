use chrono::{Datelike, NaiveDate};
use log::{info, warn};

use crate::engine::cashflow_engine::CashFlowEngine;
use crate::engine::market_data::MarketDataService;
use crate::engine::rsu_engine::add_years;
use crate::engine::tax::japan_tax::JapanTaxEngine;
use crate::engine::tax::us_tax::TaxEngine;
use crate::models::assets::{AssetLot, Asset};
use crate::models::config::Config;
use crate::models::constants::SimConstants;
use crate::models::snapshot::{BuyRecord, SellRecord, TransitionAllocation, TransitionReport};
use crate::simulation::state::SimState;

/// Executes the one-time retirement portfolio rebalance and cash-funding event.
///
/// Steps:
///   1. Apply recession shock if configured.
///   2. Calculate cash needed: war chest + bridge fund + Japan resident tax + US capital gains tax.
///   3. Iteratively sell from Taxable (up to 15 iterations) to cover all needs,
///      accounting for the tax on the realized gains themselves.
///   4. Smart rebalance of remaining assets to target VTI/SCHD allocation.
///   5. Buy underweighted assets with remaining cash.
///   6. Record full transition report.
///
/// Mirrors Python's `RetirementTransitionHandler.handle_transition()`.
pub fn handle_transition(
    state: &mut SimState,
    cfg: &Config,
    cf_engine: &CashFlowEngine,
    tax_engine: &TaxEngine,
) -> TransitionReport {
    let current_date = state.date;
    info!("\n>>> EVENT: RETIREMENT TRANSITION ({})", current_date);

    // Step 0: Optional recession shock.
    if cfg.recession_enabled {
        warn!("   [!!!] SIMULATING RECESSION: Market Shock -{:.1}%", cfg.recession_severity * 100.0);
        for acc in state.accounts.values_mut() {
            acc.shock(cfg.recession_severity);
        }
    }

    // Step 1: Calculate cash targets.
    // V7.1: war_chest_jpy is always JPY-denominated. Legacy "USD" configs are
    // converted at transition-date FX to preserve the intended reserve level.
    let new_wc_jpy = if cfg.war_chest_currency == "USD" {
        cfg.war_chest_target_usd * state.current_fx  // convert to JPY at today's rate
    } else {
        cfg.war_chest_target_jpy
    };
    let wc_needed_usd = {
        let pre_jpy = cfg.pre_funded_war_chest_jpy;
        ((new_wc_jpy - pre_jpy).max(0.0)) / state.current_fx
    };
    state.war_chest_jpy = new_wc_jpy;

    let exp_breakdown = cf_engine.get_expenses_breakdown(current_date);
    let income = cf_engine.get_incomes_usd(current_date);
    let guaranteed_income_jpy = (income.va_usd + income.fers_usd) * state.current_fx;
    let shortfall_monthly = (exp_breakdown.total_desired - guaranteed_income_jpy).max(0.0);
    let bridge_target = shortfall_monthly * cfg.bridge_months_target as f64;
    let nhi_buffer = cfg.nhi_spike_monthly_jpy * SimConstants::MEDICAL_BUFFER_MONTHS as f64;
    let bridge_general_target = bridge_target.max(nhi_buffer);

    let jp_tax_details = JapanTaxEngine::estimate_resident_tax_transition(cfg.retirement_year_gross_income_jpy);
    let resident_tax_total = jp_tax_details.tax_bill;

    let bridge_pre_general = if cfg.bridge_fund_currency == "USD" {
        cfg.pre_funded_bridge_usd * state.current_fx
    } else {
        cfg.pre_funded_bridge_jpy
    };
    let jp_tax_pre = cfg.pre_funded_japan_tax_jpy;

    let bridge_total_pull_usd = ((bridge_general_target - bridge_pre_general).max(0.0)
        + (resident_tax_total - jp_tax_pre).max(0.0))
        / state.current_fx;

    state.bridge_fund_usd = (bridge_general_target + resident_tax_total) / state.current_fx;

    // Step 2: Estimate ordinary income for the retirement year.
    let months_worked = (cfg.retirement_date.month() as i32 - 1).max(0) as f64;
    let prorated_salary = (cfg.total_annual_compensation_usd / 12.0) * months_worked;
    let mut ord_income_ret_year = prorated_salary;

    // Build virtual RSU lot history and count YTD vesting income.
    let ret_year = cfg.retirement_date.year();
    let mut rsu_ytd_income = 0.0_f64;

    for award in &cfg.rsu_awards {
        if award.vesting_years == 0 || award.vesting_months.is_empty() {
            continue;
        }
        let total_events = award.vesting_years as usize * award.vesting_months.len();
        if total_events == 0 {
            continue;
        }
        let shares_per_vest = award.total_shares / total_events as f64;
        let end_date = add_years(award.grant_date, award.vesting_years);

        for year_offset in 0..award.vesting_years {
            let base_date = add_years(award.grant_date, year_offset);
            for &month in &award.vesting_months {
                // Defensive: loader already filters to 1..=12, but guard against any
                // path that bypasses validation (e.g. programmatic construction).
                let Some(mut vest_date) = NaiveDate::from_ymd_opt(base_date.year(), month, 1) else {
                    continue;
                };
                if vest_date < base_date {
                    let Some(next) = NaiveDate::from_ymd_opt(base_date.year() + 1, month, 1) else {
                        continue;
                    };
                    vest_date = next;
                }
                if vest_date >= end_date || vest_date >= cfg.retirement_date {
                    continue;
                }

                let price = state.accounts.get("Taxable")
                    .and_then(|a| a.assets.get(&award.ticker))
                    .map(|a| a.price)
                    .unwrap_or_else(|| MarketDataService::fallback_price(&award.ticker));

                let vest_value = shares_per_vest * price;

                // Add to virtual lot history in the Taxable account.
                if let Some(taxable) = state.accounts.get_mut("Taxable") {
                    let fallback_p = price;
                    let fallback_g = cfg.growth_rates_annual.get(&award.ticker).copied()
                        .unwrap_or_else(|| MarketDataService::fallback_growth(&award.ticker));
                    taxable.assets
                        .entry(award.ticker.clone())
                        .or_insert_with(|| Asset::new(&award.ticker, fallback_p, 0.0, fallback_g))
                        .lots.push(AssetLot {
                            purchase_date: vest_date,
                            qty: shares_per_vest,
                            basis: vest_value,
                        });
                }

                if vest_date.year() == ret_year {
                    rsu_ytd_income += vest_value;
                }
            }
        }
    }
    ord_income_ret_year += rsu_ytd_income;

    // Step 3: Iterative sell loop to cover all cash needs (mirrors Python's ≤15 iteration loop).
    let mut total_cash_raised = 0.0_f64;
    let mut total_st_gains = 0.0_f64;
    let mut total_lt_gains = 0.0_f64;
    let mut sells_snapshot: Vec<SellRecord> = Vec::new();

    // Record pre-rebalance value via a deep-clone simulation.
    let pre_val = state.accounts.get("Taxable").map(|a| a.total_value(state.current_fx)).unwrap_or(0.0);

    // Convergence threshold $100: well below any tax-relevant precision and
    // tight enough that a degenerate $1-per-iteration drip can't spin all 15
    // iterations.
    const GAINS_CONVERGENCE_USD: f64 = 100.0;
    let mut prev_total_gains = f64::NEG_INFINITY;
    for _iteration in 0..15 {
        let total_gains = total_st_gains + total_lt_gains;
        if (total_gains - prev_total_gains).abs() < GAINS_CONVERGENCE_USD {
            break;
        }
        prev_total_gains = total_gains;

        let tax_result = tax_engine.calculate_liability(
            current_date.year(), ord_income_ret_year, total_st_gains, total_lt_gains,
        );
        let tax_pull_needed = (tax_result.total_tax - cfg.pre_funded_us_tax_usd).max(0.0);
        let total_cash_pull = wc_needed_usd + bridge_total_pull_usd + tax_pull_needed;

        let cash_shortfall = total_cash_pull - total_cash_raised;
        if cash_shortfall <= 0.0 {
            break;
        }

        let portfolio_value: f64 = state.accounts.get("Taxable")
            .map(|a| a.assets.values().map(|ast| ast.market_value()).sum())
            .unwrap_or(0.0);

        if portfolio_value <= 0.0 {
            warn!("   Portfolio exhausted during tax stabilization.");
            break;
        }

        let snapshot: Vec<(String, f64, f64)> = state.accounts.get("Taxable")
            .expect("Taxable account must exist when portfolio_value > 0")
            .assets
            .iter()
            .map(|(t, a)| (t.clone(), a.market_value(), a.price))
            .collect();

        for (ticker, mv, price) in &snapshot {
            let proportional = cash_shortfall * (mv / portfolio_value);
            let gain = state.accounts.get_mut("Taxable")
                .expect("Taxable account must exist for cash-shortfall sell")
                .sell_value(ticker, proportional, current_date);
            sells_snapshot.push(SellRecord {
                ticker: ticker.clone(),
                action: "SOLD_FOR_CASH".into(),
                qty_sold: if *price > 0.0 { gain.proceeds / price } else { 0.0 },
                price: *price,
                proceeds: gain.proceeds,
            });
            total_cash_raised += gain.proceeds;
            total_st_gains += gain.short_term_gain;
            total_lt_gains += gain.long_term_gain;
        }
    }

    let mut cash_for_reinvest = total_cash_raised - (wc_needed_usd + bridge_total_pull_usd);

    // Step 4: Smart rebalance of remaining portfolio.
    let target_allocations = [
        ("VTI", cfg.target_vti_pct),
        ("SCHD", cfg.target_schd_pct),
    ];

    let equity_to_rebalance = state.accounts.get("Taxable")
        .map(|a| a.total_value(state.current_fx))
        .unwrap_or(0.0) + cash_for_reinvest;

    if equity_to_rebalance > 0.0 {
        let portfolio_snapshot: Vec<(String, f64, f64)> = state.accounts.get("Taxable")
            .expect("Taxable account must exist when equity_to_rebalance > 0")
            .assets.iter()
            .map(|(t, a)| (t.clone(), a.market_value(), a.price))
            .collect();

        for (ticker, mv, price) in &portfolio_snapshot {
            let target_pct = target_allocations.iter()
                .find(|(t, _)| t == ticker)
                .map(|(_, p)| *p)
                .unwrap_or(0.0);
            let target_value = equity_to_rebalance * target_pct;
            let overweight = mv - target_value;
            if overweight > 0.0 {
                let action = if target_pct > 0.0 { "SOLD_REBALANCE" } else { "SOLD_NON_TARGET" };
                let gain = state.accounts.get_mut("Taxable")
                    .expect("Taxable account must exist for rebalance sell")
                    .sell_value(ticker, overweight, current_date);
                sells_snapshot.push(SellRecord {
                    ticker: ticker.clone(),
                    action: action.into(),
                    qty_sold: if *price > 0.0 { gain.proceeds / price } else { 0.0 },
                    price: *price,
                    proceeds: gain.proceeds,
                });
                cash_for_reinvest += gain.proceeds;
                total_st_gains += gain.short_term_gain;
                total_lt_gains += gain.long_term_gain;
            }
        }
    }

    // Final tax calculation with total realized gains.
    let final_tax = tax_engine.calculate_liability(
        current_date.year(), ord_income_ret_year, total_st_gains, total_lt_gains,
    );
    let final_us_tax_breakdown = final_tax.breakdown.clone();
    let final_tax_pull = (final_tax.total_tax - cfg.pre_funded_us_tax_usd).max(0.0);
    state.stats.year_st_cap_gains = total_st_gains;
    state.stats.year_lt_cap_gains = total_lt_gains;
    state.stats.year_tax_routed += final_tax.total_tax;
    cash_for_reinvest -= final_tax_pull;

    // Step 5: Buy underweighted assets.
    let mut buys_snapshot: Vec<BuyRecord> = Vec::new();

    if equity_to_rebalance > 0.0 {
        for (ticker, target_pct) in &target_allocations {
            let current_val = state.accounts.get("Taxable")
                .and_then(|a| a.assets.get(*ticker))
                .map(|a| a.market_value())
                .unwrap_or(0.0);
            let target_value = equity_to_rebalance * target_pct;
            let underweight = target_value - current_val;

            if underweight > 0.0 && cash_for_reinvest > 0.0 {
                let buy_amount = underweight.min(cash_for_reinvest);
                let price = MarketDataService::fallback_price(ticker);
                let growth = cfg.growth_rates_annual.get(*ticker).copied()
                    .unwrap_or_else(|| MarketDataService::fallback_growth(ticker));
                let spent = state.accounts.get_mut("Taxable")
                    .expect("Taxable account must exist for rebalance buy")
                    .buy(ticker, buy_amount, current_date, price, growth);
                let qty_bought = if price > 0.0 { spent / price } else { 0.0 };
                buys_snapshot.push(BuyRecord { ticker: ticker.to_string(), qty_bought, cost: spent });
                cash_for_reinvest -= spent;
            }
        }

        // Reinvest residual cash into primary target (highest allocation).
        // Use Ordering::Equal as a NaN fallback so a stray NaN allocation pct
        // doesn't panic — max_by simply picks the first ticker in that case.
        if cash_for_reinvest > 1.0 {
            let primary = target_allocations.iter()
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .expect("target_allocations is non-empty")
                .0;
            let price = MarketDataService::fallback_price(primary);
            let growth = cfg.growth_rates_annual.get(primary).copied()
                .unwrap_or_else(|| MarketDataService::fallback_growth(primary));
            let spent = state.accounts.get_mut("Taxable").unwrap()
                .buy(primary, cash_for_reinvest, current_date, price, growth);
            let qty_bought = if price > 0.0 { spent / price } else { 0.0 };
            buys_snapshot.push(BuyRecord { ticker: primary.to_string(), qty_bought, cost: spent });
        }
    }

    let final_balance = state.accounts.get("Taxable")
        .map(|a| a.total_value(state.current_fx))
        .unwrap_or(0.0);

    let weighted_yield = state.accounts.get("Taxable").map(|a| {
        if final_balance <= 0.0 { return 0.0; }
        a.assets.values().map(|ast| (ast.market_value() / final_balance) * ast.yield_rate).sum()
    }).unwrap_or(0.0);

    let reinvested_cash: f64 = buys_snapshot.iter().map(|b| b.cost).sum();
    info!("   Pre-rebalance: ${:.2} | Post-rebalance: ${:.2}", pre_val, final_balance);

    TransitionReport {
        date: current_date,
        fx_rate: state.current_fx,
        pre_val,
        post_val: final_balance,
        yield_post: weighted_yield,
        sells: sells_snapshot,
        buys: buys_snapshot,
        allocation: TransitionAllocation {
            prorated_base_income: ord_income_ret_year,
            us_tax_bill: final_tax.total_tax,
            us_tax_breakdown: final_us_tax_breakdown,
            total_st_gains,
            total_lt_gains,
            total_niit: final_tax.breakdown.get("niit_on_gains").copied().unwrap_or(0.0),
            us_tax_pre: cfg.pre_funded_us_tax_usd,
            us_tax_paid_from_portfolio: final_tax_pull,
            wc_target: if cfg.war_chest_currency == "USD" { cfg.war_chest_target_usd } else { cfg.war_chest_target_jpy },
            wc_currency: cfg.war_chest_currency.clone(),
            wc_paid_from_portfolio_usd: wc_needed_usd,
            wc_pre: cfg.pre_funded_war_chest_jpy,
            bridge_total_jpy: bridge_general_target + resident_tax_total,
            bridge_pre_general_jpy: bridge_pre_general,
            bridge_fund_currency: cfg.bridge_fund_currency.clone(),
            jp_tax_pre_jpy: jp_tax_pre,
            bridge_pull_usd: bridge_total_pull_usd,
            jp_tax_bill: resident_tax_total,
            reinvested_cash,
        },
    }
}

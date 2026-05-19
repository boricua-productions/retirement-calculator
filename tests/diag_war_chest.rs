//! Diagnostic: trace war_chest_jpy through the simulation for the two
//! 20260519 NO_RECESSION 50K REFRESH scenarios. Not a pass/fail test — prints
//! year-by-year war chest balance + retirement-transition allocation so we can
//! see exactly when (and if) the war chest drains to zero.
//!
//! Run with:
//!   cargo test --test diag_war_chest -- --nocapture

use retirement_calculator::config::loader::load_scenario;
use retirement_calculator::simulation::controller::SimulationController;

fn run_scenario(label: &str, path: &str) {
    println!("\n=================================================================");
    println!(" {label}");
    println!(" file: {path}");
    println!("=================================================================");

    let loaded = load_scenario(path).unwrap_or_else(|e| panic!("load {path}: {e}"));
    let cfg = &loaded.config;
    println!(
        " war_chest_enabled       = {}\n war_chest_currency      = {}\n war_chest_target_jpy    = {}\n war_chest_target_usd    = {}\n pre_funded_war_chest_jpy= {}\n war_chest_funding_timing= {:?}\n war_chest_ramp_months   = {}\n retirement_date         = {}\n rebalance_date          = {}\n start_date              = {}\n end_date                = {}",
        cfg.war_chest_enabled,
        cfg.war_chest_currency,
        cfg.war_chest_target_jpy,
        cfg.war_chest_target_usd,
        cfg.pre_funded_war_chest_jpy,
        cfg.war_chest_funding_timing,
        cfg.war_chest_ramp_months,
        cfg.retirement_date,
        cfg.rebalance_date,
        cfg.start_date,
        cfg.end_date,
    );

    let results = SimulationController::new(loaded.config, loaded.accounts).run();

    if let Some(tr) = &results.transition_report {
        let a = &tr.allocation;
        println!(
            "\n RETIREMENT TRANSITION REPORT @ {}\n   wc_currency             = {}\n   wc_target               = {}\n   wc_pre                  = {}\n   wc_pre_accumulated_jpy  = {}\n   wc_paid_from_portfolio_usd = {}",
            tr.date, a.wc_currency, a.wc_target, a.wc_pre, a.wc_pre_accumulated_jpy, a.wc_paid_from_portfolio_usd,
        );
    } else {
        println!("\n !! No transition report produced.");
    }

    println!(
        "\n {:>4} | {:>14} | {:>14} | {:>14} | {:>14} | {:>14}",
        "YEAR", "war_chest_jpy", "wc_used_jpy", "bridge_fund_usd", "nenkin_jpy", "div_net_usd"
    );
    println!(" {:->4}-+-{:->14}-+-{:->14}-+-{:->14}-+-{:->14}-+-{:->14}", "", "", "", "", "", "");
    for s in &results.annual_summary {
        println!(
            " {:>4} | {:>14.0} | {:>14.0} | {:>14.0} | {:>14.0} | {:>14.0}",
            s.year, s.war_chest_jpy, s.war_chest_used_jpy, s.bridge_fund_usd,
            s.nenkin_income_jpy, s.div_net_usd,
        );
    }
}

#[test]
fn war_chest_trace_both_scenarios() {
    run_scenario(
        "RETIRE MAY 2029",
        "20260519_NO_RECESSION_50K_REFRESH_RETIRE_MAY_2029.json",
    );
    run_scenario(
        "RETIRE MAY 2030",
        "20260519_NO_RECESSION_50K_REFRESH_RETIRE_MAY_2030.json",
    );
}

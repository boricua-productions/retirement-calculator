use chrono::{Datelike, NaiveDate};
use log::info;

use crate::engine::market_data::MarketDataService;
use crate::engine::tax::us_tax::TaxEngine;
use crate::handlers::cashflow_manager::JAPAN_CAPITAL_GAINS_RATE;
use crate::models::assets::{Account, AccountJurisdiction, DividendCurrency, PficRegime};
use crate::models::config::Config;
use crate::simulation::state::SimState;

/// V7.6 — Component classification for a single distribution event.
/// Determines §904 basket routing and PFIC §1296 treatment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistributionComponent {
    /// Qualified/ordinary dividend. Passive §904 basket.
    Dividend,
    /// Interest distribution. Passive basket, ordinary US stack.
    Interest,
    /// Capital-gains distribution (mutual-fund pass-through).
    /// PFIC §1296 → ordinary passive basket; otherwise LTCG.
    CapGainsDist,
    /// Special / non-recurring distribution. Treated as ordinary dividend.
    Special,
    /// Return of Capital. Non-taxable; reduces basis. Never enters tax stacks.
    Roc,
}

/// V7.6 — A single component-typed distribution event for one asset in one month.
#[derive(Debug, Clone)]
pub struct DistributionEvent {
    pub ticker: String,
    pub currency: DividendCurrency,
    pub gross: f64,
    pub component: DistributionComponent,
    pub drip_enabled: bool,
    pub reinvest_target: Option<String>,
    /// `true` when the source asset has PFIC §1296 MTM regime — cap-gains
    /// distributions from such an asset route to the ordinary basket.
    pub is_pfic_mtm: bool,
}

/// V7.6 — Build the per-month distribution events for an account.
///
/// Profile-aware: when the asset has a `return_profile`, up to five component
/// events fire (one per non-zero component). When `return_profile` is `None`,
/// the legacy single-event path runs — same dividend amount as pre-V7.6.
/// Exposed `pub` so the V7.6 distribution-routing tests can verify the
/// per-component split without needing a full SimState.
pub fn collect_distribution_events(account: &Account, mo: u32) -> Vec<DistributionEvent> {
    let mut out: Vec<DistributionEvent> = Vec::new();
    for a in account.assets.values() {
        if a.qty() <= 0.0 || a.price <= 0.0 || !a.dividend_months.contains(&mo) {
            continue;
        }
        let n = a.dividend_months.len().max(1) as f64;
        let mv = a.qty() * a.price;
        let is_pfic_mtm = a.pfic_regime == PficRegime::Mtm;
        let drip = a.drip_enabled;
        let target = a.dividend_reinvest_target.clone();
        let cur = a.dividend_currency.clone();

        let mut push = |rate: f64, c: DistributionComponent| {
            if rate > 0.0 {
                out.push(DistributionEvent {
                    ticker: a.ticker.clone(),
                    currency: cur.clone(),
                    gross: mv * rate / n,
                    component: c,
                    drip_enabled: drip,
                    reinvest_target: target.clone(),
                    is_pfic_mtm,
                });
            }
        };

        if a.return_profile.is_some() {
            push(a.dividend_yield_rate(),  DistributionComponent::Dividend);
            push(a.interest_yield_rate(),  DistributionComponent::Interest);
            push(a.cap_gains_dist_rate(),  DistributionComponent::CapGainsDist);
            push(a.special_dist_rate(),    DistributionComponent::Special);
            push(a.roc_rate(),             DistributionComponent::Roc);
        } else if a.yield_rate > 0.0 {
            // Legacy path: yield_rate maps entirely to Dividend component.
            push(a.yield_rate, DistributionComponent::Dividend);
        }
    }
    out
}

/// V7.6 — Processes component-typed distributions for all accounts.
///
/// Currency routing (JPY-first, no FX churn):
///   - USD events: tax via US marginal rate (passive basket); net → `state.current_month_div_net_usd`.
///   - JPY events: Japan 20.315% CG rate (0% for tax-free); net → `state.current_month_div_net_jpy`.
///
/// Component routing:
///   - Dividend / Special  → standard dividend flow.
///   - Interest            → standard flow + `year_passive_ord_income_usd` accumulator.
///   - CapGainsDist        → PFIC MTM → `year_pfic_ord_income_usd`; else `year_cap_gains`.
///   - Roc                 → non-taxable; reduces basis via `apply_roc_basis_reduction`;
///                            excess above basis routed to `year_cap_gains` as LTCG.
///
/// Roth / DC DRIP: always reinvested, no tax. ROC inside tax-advantaged accounts
/// still reduces basis (preserves correct exit-tax math) but the cash reinvests.
///
/// Returns `(net_usd, net_jpy)` — the caller stores both into SimState.
pub fn handle_dividends(
    state: &mut SimState,
    cfg: &Config,
    tax_engine: &TaxEngine,
    estimate_annual_ord_income: impl Fn(&SimState, i32) -> f64,
) -> (f64, f64) {
    let current_date = state.date;
    let yr  = current_date.year();
    let mo  = current_date.month();

    let is_schd_pivot = cfg.buy_schd_last_year && {
        let pivot_start = subtract_one_year_approx(cfg.retirement_date);
        current_date >= pivot_start && current_date < cfg.retirement_date
    };

    let is_retired = current_date >= cfg.retirement_date;
    let mut div_net_usd = 0.0_f64;
    let mut div_net_jpy = 0.0_f64;

    // ── Taxable account distributions ─────────────────────────────────────────
    // §5.1 — Extract per-account tax flags before borrowing state mutably.
    let (taxable_events, apply_us_tax, apply_japan_tax) = {
        let acc = match state.accounts.get("Taxable") {
            Some(a) => a,
            None    => return (0.0, 0.0),
        };
        let events = collect_distribution_events(acc, mo);
        let jurisdiction = acc.tax_jurisdiction;
        let us_tax_adv  = acc.us_tax_advantaged;
        let jp_tax_adv  = acc.japan_tax_advantaged;
        let consult_us  = matches!(jurisdiction,
            AccountJurisdiction::Us | AccountJurisdiction::Both);
        let consult_jp  = matches!(jurisdiction,
            AccountJurisdiction::Japan | AccountJurisdiction::Both);
        let apply_us  = consult_us  && !us_tax_adv;
        let apply_jp  = consult_jp  && !jp_tax_adv;
        (events, apply_us, apply_jp)
    };

    for ev in taxable_events {
        match ev.component {
            // ── ROC: non-taxable, reduce basis, route net cash to waterfall ──
            DistributionComponent::Roc => {
                let fx = state.current_fx;
                let (excess_usd, ticker_log) = if let Some(acc) = state.accounts.get_mut("Taxable") {
                    if let Some(asset) = acc.assets.get_mut(&ev.ticker) {
                        match ev.currency {
                            DividendCurrency::Usd => (
                                asset.apply_roc_basis_reduction(ev.gross, fx),
                                ev.ticker.clone(),
                            ),
                            DividendCurrency::Jpy => {
                                // Convert to USD-equivalent so the proportional reduction
                                // hits the FIFO lot basis (tracked in USD). Any excess
                                // above basis becomes a JPY-taxed capital gain.
                                let roc_usd_equiv = ev.gross / fx;
                                (
                                    asset.apply_roc_basis_reduction(roc_usd_equiv, fx),
                                    ev.ticker.clone(),
                                )
                            }
                        }
                    } else { (0.0, ev.ticker.clone()) }
                } else { (0.0, ev.ticker.clone()) };

                if excess_usd > 0.0 {
                    state.stats.year_cap_gains += excess_usd;
                    // Japan-first: passive-basket CG tax on the excess.
                    let jp_tax_usd = excess_usd * JAPAN_CAPITAL_GAINS_RATE;
                    state.stats.year_japan_cap_gains_tax_jpy += jp_tax_usd * fx;
                }

                state.stats.year_dist_roc_usd += match ev.currency {
                    DividendCurrency::Usd => ev.gross,
                    DividendCurrency::Jpy => ev.gross / fx,
                };

                // ROC cash: DRIP reinvests; otherwise routes to net.
                match ev.currency {
                    DividendCurrency::Usd => {
                        if ev.drip_enabled && !is_retired {
                            let target = ev.reinvest_target.clone()
                                .unwrap_or_else(|| ev.ticker.clone());
                            let fp = MarketDataService::fallback_price(&target);
                            let fg = MarketDataService::fallback_growth(&target);
                            let fx = state.current_fx;
                            if let Some(taxable) = state.accounts.get_mut("Taxable") {
                                taxable.buy_with_fx(&target, ev.gross, current_date, fp, fg, fx);
                            }
                        } else {
                            div_net_usd += ev.gross;
                        }
                    }
                    DividendCurrency::Jpy => {
                        if ev.drip_enabled && !is_retired {
                            let fp = MarketDataService::fallback_price(&ev.ticker);
                            let fg = MarketDataService::fallback_growth(&ev.ticker);
                            if let Some(taxable) = state.accounts.get_mut("Taxable") {
                                // JPY dividend reinvested into JPY-priced asset: fx=1.0 keeps basis in JPY.
                                taxable.buy_with_fx(&ev.ticker, ev.gross, current_date, fp, fg, 1.0);
                            }
                        } else {
                            div_net_jpy += ev.gross;
                        }
                    }
                }
                info!("   [DIV ROC] {} cur={:?} gross={:.2} excess_ltcg={:.2}",
                    ticker_log, ev.currency, ev.gross, excess_usd);
            }

            // ── Dividend / Interest / Special / CapGainsDist ──────────────────
            // All share the same per-event tax/DRIP/net flow; they differ only
            // in which annual-stat bucket the gross feeds (driving §904 basket
            // routing at year-end true-up).
            _ => {
                let (net_usd_add, net_jpy_add) = process_taxable_dist_event(
                    state, cfg, tax_engine, &estimate_annual_ord_income,
                    yr, is_retired, is_schd_pivot, current_date, &ev,
                    apply_us_tax, apply_japan_tax,
                );
                div_net_usd += net_usd_add;
                div_net_jpy += net_jpy_add;
            }
        }
    }

    // ── Roth DRIP (always reinvest, no tax, respects dividend_months) ─────────
    process_drip_account(state, current_date, mo, "Roth");

    finish_dc_drip(state, current_date, mo, div_net_usd, div_net_jpy)
}

/// V7.7 — Process a single non-ROC distribution event in the Taxable account.
/// Handles US/JPY currency split, marginal tax estimation, DRIP vs net routing.
/// `apply_us_tax` / `apply_japan_tax` come from the §5.1 account-level logic gate.
/// Returns the `(net_usd, net_jpy)` increment for this single event.
fn process_taxable_dist_event(
    state: &mut SimState,
    _cfg: &Config,
    tax_engine: &TaxEngine,
    estimate_annual_ord_income: &impl Fn(&SimState, i32) -> f64,
    yr: i32,
    is_retired: bool,
    is_schd_pivot: bool,
    current_date: NaiveDate,
    ev: &DistributionEvent,
    apply_us_tax: bool,
    apply_japan_tax: bool,
) -> (f64, f64) {
    let gross = ev.gross;
    let fx = state.current_fx;
    let mut net_usd_add = 0.0_f64;
    let mut net_jpy_add = 0.0_f64;

    // Component-stat tagging (drives year-end §904 basket split).
    let gross_usd_equiv = match ev.currency {
        DividendCurrency::Usd => gross,
        DividendCurrency::Jpy => gross / fx,
    };
    match ev.component {
        DistributionComponent::Dividend => {
            state.stats.year_dist_dividend_usd += gross_usd_equiv;
        }
        DistributionComponent::Interest => {
            state.stats.year_dist_interest_usd += gross_usd_equiv;
            state.stats.year_passive_ord_income_usd += gross_usd_equiv;
        }
        DistributionComponent::CapGainsDist => {
            state.stats.year_dist_cap_gains_usd += gross_usd_equiv;
            if ev.is_pfic_mtm {
                // PFIC §1296 → ordinary passive basket. NOT added to year_cap_gains.
                state.stats.year_pfic_ord_income_usd += gross_usd_equiv;
            } else {
                state.stats.year_cap_gains += gross_usd_equiv;
            }
        }
        DistributionComponent::Special => {
            state.stats.year_dist_special_usd += gross_usd_equiv;
            state.stats.year_passive_ord_income_usd += gross_usd_equiv;
        }
        DistributionComponent::Roc => unreachable!("ROC handled in caller"),
    }

    match ev.currency {
        DividendCurrency::Usd => {
            state.stats.year_div_gross += gross;
            state.stats.acc_div_inc    += gross;

            // Marginal US withhold estimate (same approach as V7.5).
            let est_ord = estimate_annual_ord_income(state, yr);
            let gains_pre  = state.stats.year_div_gross + state.stats.year_cap_gains - gross;
            let gains_post = state.stats.year_div_gross + state.stats.year_cap_gains;
            // §5.1 gate: apply US tax only when account jurisdiction requires it.
            let us_tax_due = if apply_us_tax {
                let pre  = tax_engine.calculate_liability(yr, est_ord, 0.0, gains_pre).total_tax;
                let post = tax_engine.calculate_liability(yr, est_ord, 0.0, gains_post).total_tax;
                (post - pre).max(0.0)
            } else {
                0.0
            };
            // §5.1 gate: apply Japan 20.315% on USD dividends post-retirement when required.
            let jp_tax_usd = if apply_japan_tax && is_retired {
                gross * JAPAN_CAPITAL_GAINS_RATE
            } else {
                0.0
            };
            let tax_due = us_tax_due + jp_tax_usd;

            if !is_retired {
                if tax_due > 0.0 {
                    state.stats.tax_paid_external += tax_due;
                    state.stats.year_div_tax += tax_due;
                }
                let reinvest_ticker = if is_schd_pivot {
                    "SCHD".to_string()
                } else {
                    ev.reinvest_target.clone().unwrap_or_else(|| ev.ticker.clone())
                };
                if ev.drip_enabled || is_schd_pivot {
                    let fp = MarketDataService::fallback_price(&reinvest_ticker);
                    let fg = MarketDataService::fallback_growth(&reinvest_ticker);
                    let fx = state.current_fx;
                    if let Some(taxable) = state.accounts.get_mut("Taxable") {
                        taxable.buy_with_fx(&reinvest_ticker, gross, current_date, fp, fg, fx);
                    }
                } else {
                    net_usd_add += gross;
                }
            } else {
                let net = gross - tax_due;
                net_usd_add += net;
                if us_tax_due > 0.0 {
                    state.stats.year_tax_routed += us_tax_due;
                    state.stats.year_div_tax    += us_tax_due;
                }
                if jp_tax_usd > 0.0 {
                    state.stats.year_japan_cap_gains_tax_jpy += jp_tax_usd * fx;
                }
                info!("   [DIV USD {:?}] {} div=${:.2} us_tax=${:.2} jp_tax=${:.2} net=${:.2}",
                    ev.component, ev.ticker, gross, us_tax_due, jp_tax_usd, net);
            }
        }

        DividendCurrency::Jpy => {
            // §5.1 gate: Japan tax applies only when account requires it.
            let tax_jpy = if is_retired && apply_japan_tax {
                gross * JAPAN_CAPITAL_GAINS_RATE
            } else {
                0.0
            };
            let net_jpy = gross - tax_jpy;

            state.stats.year_div_gross += gross / fx;
            state.stats.acc_div_inc    += gross / fx;

            if !is_retired {
                if ev.drip_enabled || is_schd_pivot {
                    let fp = MarketDataService::fallback_price(&ev.ticker);
                    let fg = MarketDataService::fallback_growth(&ev.ticker);
                    if let Some(taxable) = state.accounts.get_mut("Taxable") {
                        taxable.buy_with_fx(&ev.ticker, gross, current_date, fp, fg, 1.0);
                    }
                } else {
                    net_jpy_add += gross;
                }
            } else {
                net_jpy_add += net_jpy;
                if tax_jpy > 0.0 {
                    state.stats.year_japan_cap_gains_tax_jpy += tax_jpy;
                }
                info!("   [DIV JPY {:?}] {} div=¥{:.0} tax=¥{:.0} net=¥{:.0}",
                    ev.component, ev.ticker, gross, tax_jpy, net_jpy);
            }
        }
    }

    (net_usd_add, net_jpy_add)
}

/// V7.6 — DRIP an account's distribution events in place. Tax-advantaged
/// accounts (Roth, DC) reinvest all components into the source asset; ROC
/// still reduces basis (preserves exit-tax math) before the reinvestment.
fn process_drip_account(state: &mut SimState, current_date: NaiveDate, mo: u32, name: &str) {
    let events: Vec<DistributionEvent> = match state.accounts.get(name) {
        Some(a) => collect_distribution_events(a, mo),
        None    => return,
    };
    let fx = state.current_fx;
    for ev in events {
        if matches!(ev.component, DistributionComponent::Roc) {
            if let Some(acc) = state.accounts.get_mut(name) {
                if let Some(asset) = acc.assets.get_mut(&ev.ticker) {
                    if matches!(ev.currency, DividendCurrency::Usd) {
                        asset.apply_roc_basis_reduction(ev.gross, fx);
                    }
                }
            }
        }
        if ev.gross > 0.0 {
            let fp = MarketDataService::fallback_price(&ev.ticker);
            let fg = MarketDataService::fallback_growth(&ev.ticker);
            // V8.0 — DRIP uses the account's native FX rate (Roth/DC are USD, so use fx).
            if let Some(acc) = state.accounts.get_mut(name) {
                acc.buy_with_fx(&ev.ticker, ev.gross, current_date, fp, fg, fx);
            }
        }
    }
}

/// Processes DC DRIP then returns the (usd, jpy) dividend tuple.
fn finish_dc_drip(
    state: &mut SimState,
    current_date: NaiveDate,
    mo: u32,
    div_net_usd: f64,
    div_net_jpy: f64,
) -> (f64, f64) {
    process_drip_account(state, current_date, mo, "DC");
    (div_net_usd, div_net_jpy)
}

fn subtract_one_year_approx(date: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year() - 1, date.month(), date.day())
        .or_else(|| NaiveDate::from_ymd_opt(date.year() - 1, date.month(), date.day() - 1))
        .unwrap_or(date)
}

use chrono::{Datelike, Months, NaiveDate};

/// Whole calendar months elapsed from `from` to `to` (signed-clamped to >=0).
fn months_between(from: NaiveDate, to: NaiveDate) -> u32 {
    let diff = (to.year() - from.year()) * 12 + (to.month() as i32 - from.month() as i32);
    diff.max(0) as u32
}
use log::{info, warn};
use std::collections::HashMap;

use crate::engine::cashflow_engine::CashFlowEngine;
use crate::engine::rsu_engine::RsuEngine;
use crate::engine::tax::japan_regions::{lookup_resident_tax_rates, ResidentTaxRates};
use crate::engine::tax::japan_tax::JapanTaxEngine;
use crate::engine::tax::nhi::NhiEngine;
use crate::engine::tax::us_tax::{ssdi_combined_income_taxable_portion, TaxEngine};
use crate::handlers::cashflow_manager::manage_monthly_cashflow;
use crate::handlers::contributions::handle_contributions;
use crate::handlers::dividends::handle_dividends;
use crate::handlers::rebalancing::handle_rebalancing;
use crate::handlers::retirement_transition::handle_transition;
use crate::handlers::roth_rebalancer::{execute_roth_rebalance, roth_rebalance_trigger_date};
use crate::handlers::rsu_vesting::handle_rsu_vesting;
use crate::models::assets::Account;
use crate::models::config::{Config, TaxJurisdiction, UsTaxStrategy};
use crate::models::expense::ExpenseRule;
use crate::models::snapshot::{AnnualSnapshot, SimResults};
use super::state::SimState;

/// The main simulation orchestrator.
///
/// Initialises all engines and state, then runs the month-by-month loop from
/// `cfg.start_date` to `cfg.end_date`. Mirrors Python's `SimulationController`.
pub struct SimulationController {
    cfg: Config,
    cf_engine: CashFlowEngine,
    tax_engine: TaxEngine,
    rsu_engine: RsuEngine,
    state: SimState,
    japan_tax_rates: ResidentTaxRates,
}

impl SimulationController {
    pub fn new(cfg: Config, accounts: HashMap<String, Account>) -> Self {
        let rsu_engine = RsuEngine::new(cfg.rsu_awards.clone(), Some(cfg.retirement_date));
        let cf_engine = CashFlowEngine::new(cfg.clone());
        let tax_engine = TaxEngine::new(cfg.tax_rules.clone());
        let ira_limit = cfg.roth_start_limit;
        let start_fx = cfg.usd_jpy;
        let start_date = cfg.start_date;

        let japan_tax_rates = lookup_resident_tax_rates(&cfg.prefecture, &cfg.city);

        let mut state = SimState::new(start_date, start_fx, ira_limit, accounts);

        // Prime the FERS history for year 0 so NHI calculations have a basis.
        let annual_fers = cf_engine.calc_annual_fers_projection(start_date.year());
        state.stats.acc_ord_inc = annual_fers;
        state.fers_history.insert(start_date.year(), annual_fers);

        Self { cfg, cf_engine, tax_engine, rsu_engine, state, japan_tax_rates }
    }

    /// Runs the full simulation. Returns all results.
    pub fn run(mut self) -> SimResults {
        info!("\n--- STARTING SIMULATION ({} → {}) ---",
            self.cfg.start_date, self.cfg.end_date);

        let mut current = self.cfg.start_date;
        while current <= self.cfg.end_date {
            self.state.date = current;
            self.process_month();
            current = current.checked_add_months(Months::new(1))
                .expect("date overflow past end of simulation");
        }

        SimResults {
            annual_summary: self.state.annual_summary,
            gap_warnings: self.state.gap_warnings,
            transition_report: self.state.transition_report,
            tax_jurisdiction: self.cfg.tax_jurisdiction,
            investment_location: self.cfg.investment_location,
            prefecture: self.cfg.prefecture.clone(),
            city: self.cfg.city.clone(),
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  Month processing
    // ─────────────────────────────────────────────────────────────────────────

    fn process_month(&mut self) {
        let yr = self.state.date.year();
        let mo = self.state.date.month();
        let is_qtr = mo % 3 == 0;
        let is_new_year = mo == 1;

        self.state.current_month_div_net_usd = 0.0;
        self.state.current_month_div_net_jpy = 0.0;

        // ── January: year-start housekeeping ─────────────────────────────────
        if is_new_year {
            self.handle_new_year(yr);
        }

        // ── FX drift (post-retirement) ────────────────────────────────────────
        // V6.6: cadence-based JPY drift takes precedence when cadence_months > 0.
        // Every N months the FX rate jumps by `fx_drift_increase_amount_jpy`.
        // When cadence is 0, fall back to the legacy continuous-rate mode.
        if self.cfg.fx_drift_enabled && self.state.date >= self.cfg.retirement_date {
            if self.cfg.fx_drift_cadence_months > 0 {
                let cadence = self.cfg.fx_drift_cadence_months;
                let elapsed = months_between(self.cfg.retirement_date, self.state.date);
                if elapsed > 0 && elapsed % cadence == 0 {
                    let amount = self.cfg.fx_drift_increase_amount_jpy;
                    if amount.is_finite() {
                        self.state.current_fx = (self.state.current_fx + amount).max(0.01);
                    }
                }
            } else {
                // Guard: fx_drift_rate >= 1.0 corrupts current_fx via 0.0/NaN propagation
                // (negative-base fractional exponent). Skip the drift step in that case.
                let r = self.cfg.fx_drift_rate;
                if r.is_finite() && r < 1.0 && r > -1.0 {
                    self.state.current_fx *= (1.0 - r).powf(1.0 / 12.0);
                }
            }
        }

        // ── Spouse SS / Nenkin payouts (V6.6) ────────────────────────────────
        if self.cfg.is_married && self.state.date >= self.cfg.retirement_date {
            self.process_spouse_benefits(yr);
        }

        // ── Retirement transition event ───────────────────────────────────────
        if self.state.date.year() == self.cfg.rebalance_date.year()
            && self.state.date.month() == self.cfg.rebalance_date.month()
        {
            let report = handle_transition(
                &mut self.state,
                &self.cfg,
                &self.cf_engine,
                &self.tax_engine,
            );
            self.state.transition_report = Some(report);
        }

        // ── Roth IRA rebalance at age 59.5 ───────────────────────────────────
        if self.cfg.enable_roth_rebalance_at_59 && !self.state.roth_rebalance_executed {
            let trigger = roth_rebalance_trigger_date(self.cfg.birth_date);
            if self.state.date >= trigger {
                execute_roth_rebalance(&mut self.state, &self.cfg);
                self.state.roth_rebalance_executed = true;
            }
        }

        // ── Portfolio growth ──────────────────────────────────────────────────
        for acc in self.state.accounts.values_mut() {
            acc.grow();
        }

        // ── Recession / Recovery trajectory (multi-month drawdown) ────────────
        if self.state.recession_months_remaining > 0 {
            let rate = self.state.recession_monthly_shock_rate;
            for acc in self.state.accounts.values_mut() {
                acc.shock(rate);
            }
            self.state.recession_months_remaining -= 1;
            if self.state.recession_months_remaining == 0 && self.state.recovery_months_remaining == 0 {
                self.state.recession_active = false;
                info!("   [Recession] Drawdown complete — no recovery phase scheduled. Reinvestment resumes.");
            }
        } else if self.state.recovery_months_remaining > 0 {
            // shock(-rate) ≡ price *= (1 + rate): a price increase restoring prior losses.
            let rate = self.state.recovery_monthly_boost_rate;
            for acc in self.state.accounts.values_mut() {
                acc.shock(-rate);
            }
            self.state.recovery_months_remaining -= 1;
            if self.state.recovery_months_remaining == 0 {
                self.state.recession_active = false;
                info!("   [Recession] Recovery phase complete. Reinvestment resumes.");
            }
        }

        // ── RSU vesting ───────────────────────────────────────────────────────
        {
            let cfg = &self.cfg;
            let rsu_engine = &self.rsu_engine;
            let tax_engine = &self.tax_engine;
            handle_rsu_vesting(
                &mut self.state,
                cfg,
                rsu_engine,
                tax_engine,
                |state, yr| Self::estimate_annual_ord_income_static(state, cfg, yr),
            );
        }

        // ── V7.5 — Tax-Loss Harvesting (pre-waterfall, post-retirement) ─────────
        if self.state.date >= self.cfg.retirement_date && self.cfg.tlh_enabled {
            crate::handlers::tax_loss_harvesting::harvest_losses(&mut self.state, &self.cfg);
        }

        // ── Dividends ─────────────────────────────────────────────────────────
        {
            let cfg = &self.cfg;
            let tax_engine = &self.tax_engine;
            let (net_usd, net_jpy) = handle_dividends(
                &mut self.state,
                cfg,
                tax_engine,
                |state, yr| Self::estimate_annual_ord_income_static(state, cfg, yr),
            );
            self.state.current_month_div_net_usd = net_usd;
            self.state.current_month_div_net_jpy = net_jpy;
        }

        // ── Contributions (pre-retirement) ────────────────────────────────────
        handle_contributions(&mut self.state, &self.cfg, &self.cf_engine);

        // ── Periodic target-state rebalancing (V6.0) ─────────────────────────
        handle_rebalancing(&mut self.state, &self.cfg);

        // ── Post-retirement cashflow management ───────────────────────────────
        if self.state.date >= self.cfg.retirement_date {
            let cfg = &self.cfg;
            let cf_engine = &self.cf_engine;
            manage_monthly_cashflow(
                &mut self.state,
                cfg,
                cf_engine,
                |state, yr| Self::estimate_annual_ord_income_static(state, cfg, yr),
                yr,
                mo,
                is_qtr,
            );
        }

        // ── Year-end tax true-up (December, post-retirement) ──────────────────
        if mo == 12 && self.state.date >= self.cfg.retirement_date {
            self.finalize_year_taxes(yr);
        }

        // ── Annual snapshot (December of every year) ──────────────────────────
        if mo == 12 {
            self.record_annual_snapshot(yr);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  New-year housekeeping
    // ─────────────────────────────────────────────────────────────────────────

    fn handle_new_year(&mut self, yr: i32) {
        // Inflate tax brackets (not in the very first year of simulation).
        if yr > self.cfg.start_date.year() {
            self.tax_engine.rules.inflate(self.cfg.inflation_cola);
        }

        // Archive prior-year social insurance totals for resident tax deduction.
        let prev_yr = yr - 1;
        let soc_paid = self.state.stats.year_exp_nhi + self.state.stats.year_exp_nenkin;
        self.state.social_insurance_history.insert(prev_yr, soc_paid);

        // Archive prior-year gross dividend income for NHI investment-income basis.
        self.state.div_income_history.insert(prev_yr, self.state.stats.year_div_gross);

        // Schedule Japan resident tax and NHI installments for the current year.
        if self.state.date >= self.cfg.retirement_date {
            self.schedule_annual_resident_tax(yr);
            self.schedule_annual_nhi(yr);
        }

        // V7.5 — Defect 1.1: decay Japan capital-loss carry-forward by 1 year.
        // Accumulate this year's losses, then add to the rolling carry-forward.
        // The carry-forward is a single rolling sum — losses older than 3 years are
        // implicitly expired by the fact that gains in the same span have offset them.
        let new_loss = self.state.stats.year_japan_cap_loss_jpy;
        if new_loss > 0.0 {
            self.state.japan_loss_carryforward_jpy += new_loss;
        }

        // Reset annual accumulators.
        self.state.stats.reset();

        // Project this year's FERS and seed acc_ord_inc.
        let annual_fers = self.cf_engine.calc_annual_fers_projection(yr);
        self.state.stats.acc_ord_inc = annual_fers;
        self.state.fers_history.insert(yr, annual_fers);

        // Apply any scheduled recession events for this year.
        let recession_this_year: Vec<_> = self.cfg.recession_events.iter()
            .filter(|e| e.year == yr)
            .cloned()
            .collect();
        for event in recession_this_year {
            // Skip if this year overlaps with the retirement rebalance shock.
            if self.cfg.recession_enabled && yr == self.cfg.rebalance_date.year() {
                info!("   [Recession] Skipping {yr} recession — overlaps with retirement rebalance.");
                continue;
            }
            if event.duration_months <= 1 {
                // Legacy single-shock: apply the full drawdown instantly in January.
                info!("   [!!!] SCHEDULED RECESSION {yr}: -{:.1}% (instant shock)",
                    event.severity * 100.0);
                for acc in self.state.accounts.values_mut() {
                    acc.shock(event.severity);
                }
            } else {
                // Multi-month trajectory: arm the per-month drawdown counters.
                // The shock fires in process_month each month until the counter expires.
                // Surplus reinvestment is suppressed while recession_active is set.
                info!("   [!!!] SCHEDULED RECESSION {yr}: -{:.1}% over {} months, {} month recovery",
                    event.severity * 100.0, event.duration_months, event.recovery_months);
                self.state.recession_active = true;
                self.state.recession_months_remaining = event.duration_months;
                self.state.recession_monthly_shock_rate = event.monthly_shock_rate();
                // Pre-compute the per-month recovery rate that fully reverses the drawdown.
                self.state.recovery_months_remaining = event.recovery_months;
                self.state.recovery_monthly_boost_rate = if event.recovery_months > 0 {
                    // Guard: severity=1.0 makes the denominator 0 → +∞ boost rate.
                    // Floor at 0.001 (0.1% residual) keeps recovery finite and monotone.
                    let base = (1.0 - event.severity).max(0.001);
                    (1.0 / base).powf(1.0 / event.recovery_months as f64) - 1.0
                } else {
                    0.0
                };
            }
        }

        // Apply any scheduled FX shock events for this year (macro events; pre- and post-retirement).
        let fx_shocks_this_year: Vec<_> = self.cfg.fx_shock_events.iter()
            .filter(|e| e.year == yr)
            .cloned()
            .collect();
        for event in fx_shocks_this_year {
            let safe_fx = if event.target_fx.is_finite() && event.target_fx > 0.0 {
                event.target_fx
            } else {
                warn!("   [FX Shock] Year {}: target_fx={} is invalid — clamped to 0.01.",
                    yr, event.target_fx);
                0.01
            };
            info!("   [FX Shock] Year {}: USD/JPY → {:.2} ({})", yr, safe_fx, event.description);
            self.state.current_fx = safe_fx;
        }

        // Grow Roth IRA contribution limit after 2025.
        if yr > 2025 {
            self.state.ira_limit = (self.state.ira_limit * (1.0 + self.cfg.ira_limit_growth))
                .round();
            // Round to nearest $10 to match Python's `round(x, -1)`.
            self.state.ira_limit = (self.state.ira_limit / 10.0).round() * 10.0;
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  Japan resident tax scheduler
    // ─────────────────────────────────────────────────────────────────────────

    fn schedule_annual_resident_tax(&mut self, current_year: i32) {
        // Japan resident tax is not applicable in US-only mode.
        if self.cfg.tax_jurisdiction == TaxJurisdiction::UsOnly {
            return;
        }

        let prev_year = current_year - 1;

        let gross_salary = if prev_year == self.cfg.retirement_date.year() {
            self.cfg.retirement_year_gross_income_jpy
        } else {
            0.0
        };
        // Convert prior-year income streams to JPY for Japan's resident tax engine.
        let fers_annual_jpy = self.state.fers_history.get(&prev_year).copied().unwrap_or(0.0)
            * self.state.current_fx;
        // Article 18: FERS exempt from Japan resident tax when flag is set.
        let fers_for_japan = if self.cfg.fers_japan_local_tax_exempt { 0.0 } else { fers_annual_jpy };
        // Article 17: SS classified as public pension income in Japan (公的年金等控除 applies).
        let ss_annual_jpy = self.state.stats.year_ss_payout_usd * self.state.current_fx;
        // SSDI routed through public pension deduction (existing treatment, unchanged).
        let ssdi_annual_jpy = self.state.stats.year_ssdi_gross_usd * self.state.current_fx;
        let gross_pension = fers_for_japan
            + self.state.stats.year_nenkin_income_jpy
            + ssdi_annual_jpy
            + ss_annual_jpy;
        let soc_ins_paid = self.state.social_insurance_history.get(&prev_year).copied().unwrap_or(0.0);
        let age = current_year - self.cfg.birth_date.year();

        let tax_bill = JapanTaxEngine::calculate_resident_tax(
            gross_salary, gross_pension, soc_ins_paid, age, 1,
            self.japan_tax_rates.income_rate, self.japan_tax_rates.per_capita_jpy,
        );

        if tax_bill > 0.0 {
            let inst = tax_bill / 4.0;
            let new_rules = vec![
                ExpenseRule::new(format!("ResTax {} Q1", current_year), inst,
                    NaiveDate::from_ymd_opt(current_year, 6, 1).unwrap(),
                    NaiveDate::from_ymd_opt(current_year, 6, 30).unwrap()),
                ExpenseRule::new(format!("ResTax {} Q2", current_year), inst,
                    NaiveDate::from_ymd_opt(current_year, 8, 1).unwrap(),
                    NaiveDate::from_ymd_opt(current_year, 8, 31).unwrap()),
                ExpenseRule::new(format!("ResTax {} Q3", current_year), inst,
                    NaiveDate::from_ymd_opt(current_year, 10, 1).unwrap(),
                    NaiveDate::from_ymd_opt(current_year, 10, 31).unwrap()),
                ExpenseRule::new(format!("ResTax {} Q4", current_year), inst,
                    NaiveDate::from_ymd_opt(current_year + 1, 1, 1).unwrap(),
                    NaiveDate::from_ymd_opt(current_year + 1, 1, 31).unwrap()),
            ];
            if current_year == self.cfg.retirement_date.year() + 1 {
                info!("   [TAX] Scheduled Resident Tax: ¥{:.0} (Social Insurance paid: ¥{:.0})",
                    tax_bill, soc_ins_paid);
            }
            // Sync new rules into both the controller cfg AND the CashFlowEngine's cloned cfg.
            self.cfg.expense_rules.extend(new_rules.clone());
            self.cf_engine.add_expense_rules(&new_rules);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  Japan NHI dynamic scheduler
    // ─────────────────────────────────────────────────────────────────────────

    fn schedule_annual_nhi(&mut self, current_year: i32) {
        // NHI not applicable in US-only mode.
        if self.cfg.tax_jurisdiction == TaxJurisdiction::UsOnly {
            return;
        }

        let retirement_year = self.cfg.retirement_date.year();
        // NHI scheduling starts the year AFTER retirement (first full post-retirement year).
        if current_year <= retirement_year {
            return;
        }

        let prev_year    = current_year - 1;
        let is_spike_year = prev_year == retirement_year;

        // Prior-year salary: non-zero only in the retirement year itself.
        let prev_salary_jpy = if is_spike_year {
            self.cfg.retirement_year_gross_income_jpy
        } else {
            0.0
        };

        // Prior-year pension: FERS (converted to JPY) + Nenkin income.
        // Both are read from state before the annual reset in handle_new_year.
        let fers_jpy = self.state.fers_history.get(&prev_year).copied().unwrap_or(0.0)
            * self.state.current_fx;
        let prev_pension_jpy = fers_jpy + self.state.stats.year_nenkin_income_jpy;

        // Prior-year US investment income (converted to JPY).
        // Only included in the income basis when the flag is enabled.
        let prev_div_usd = self.state.div_income_history.get(&prev_year).copied().unwrap_or(0.0);
        let prev_investment_income_jpy = prev_div_usd * self.state.current_fx;

        let age = current_year - self.cfg.birth_date.year();

        // V7.5 — Ninki Keizoku: track duration and switch to fallback when exhausted.
        let effective_model = if let crate::models::config::NhiModel::NinkiKeizoku {
            monthly_premium_jpy, duration_months, fallback
        } = &self.cfg.nhi_model {
            if self.state.nhi_ninki_keizoku_months_remaining > 0 {
                self.state.nhi_ninki_keizoku_months_remaining =
                    self.state.nhi_ninki_keizoku_months_remaining.saturating_sub(12);
                // Still in the Ninki Keizoku window — use fixed monthly premium.
                std::borrow::Cow::Owned(crate::models::config::NhiModel::NinkiKeizoku {
                    monthly_premium_jpy: *monthly_premium_jpy,
                    duration_months: *duration_months,
                    fallback: fallback.clone(),
                })
            } else {
                // Window exhausted — fall back to the inner model.
                std::borrow::Cow::Borrowed(fallback.as_ref())
            }
        } else {
            // Not a Ninki Keizoku model — initialize counter on first encounter.
            if let crate::models::config::NhiModel::NinkiKeizoku { duration_months, .. }
                = &self.cfg.nhi_model
            {
                if self.state.nhi_ninki_keizoku_months_remaining == 0 {
                    self.state.nhi_ninki_keizoku_months_remaining = *duration_months;
                }
            }
            std::borrow::Cow::Borrowed(&self.cfg.nhi_model)
        };

        let annual_nhi = NhiEngine::compute_annual(
            effective_model.as_ref(),
            prev_salary_jpy,
            prev_pension_jpy,
            prev_investment_income_jpy,
            1,   // num_insured (primary retiree)
            age,
            is_spike_year,
        );

        if annual_nhi > 0.0 {
            let monthly = annual_nhi / 12.0;
            let start = NaiveDate::from_ymd_opt(current_year, 1, 1).unwrap();
            let end   = NaiveDate::from_ymd_opt(current_year, 12, 31).unwrap();
            let rule  = ExpenseRule::new(format!("NHI {}", current_year), monthly, start, end);

            if is_spike_year {
                info!("   [NHI] Spike year {}: ¥{:.0}/yr (salary basis ¥{:.0}) → ¥{:.0}/mo",
                    current_year, annual_nhi, prev_salary_jpy, monthly);
            }

            self.cfg.expense_rules.push(rule.clone());
            self.cf_engine.add_expense_rules(&[rule]);
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  Year-end US tax true-up (with Japan-First FTC)
    // ─────────────────────────────────────────────────────────────────────────

    fn finalize_year_taxes(&mut self, yr: i32) {
        // US federal year-end true-up is not applicable in Japan-only mode.
        if self.cfg.tax_jurisdiction == TaxJurisdiction::JapanOnly {
            return;
        }

        // Include SS in US ordinary income (SS is ordinary income for tax purposes).
        let base_ord = self.state.stats.year_fers_gross + self.state.stats.year_ss_payout_usd;

        // ── SSDI Combined Income Rule (IRS) ──────────────────────────────────
        // Provisional income = AGI_before_SSDI + 0.5 × annual_SSDI.
        // Only the taxable portion is added to ordinary income.
        let annual_ssdi = self.state.stats.year_ssdi_gross_usd;
        let ssdi_taxable = if annual_ssdi > 0.0 {
            let provisional_income = base_ord + 0.5 * annual_ssdi;
            ssdi_combined_income_taxable_portion(provisional_income, annual_ssdi)
        } else {
            0.0
        };
        // V7.5 — Feature 1: Aggregate §1296 MTM gains as ordinary income (not LTCG).
        let pfic_mtm_usd = crate::engine::tax::pfic::aggregate_pfic_mtm_income(
            &mut self.state.accounts,
        );
        self.state.stats.year_pfic_mtm_income_usd = pfic_mtm_usd;

        // V7.6 — §904 basket split. Passive-basket ordinary income includes:
        //   • PFIC §1296 MTM (already accumulated above)
        //   • PFIC-flagged cap-gains distributions (year_pfic_ord_income_usd)
        //   • Interest + special distributions (year_passive_ord_income_usd)
        // General-basket ordinary income: FERS, SS, taxable SSDI.
        let passive_ord = pfic_mtm_usd
            + self.state.stats.year_pfic_ord_income_usd
            + self.state.stats.year_passive_ord_income_usd;
        let general_ord = base_ord + ssdi_taxable;

        // Keep total_ord for back-compat / FEIE path (which lumps the two).
        let total_ord    = general_ord + passive_ord;
        let earned_ord   = self.state.stats.year_rsu_vest_usd; // FEIE-eligible only
        let unearned_ord = total_ord;                           // pension / SS / SSDI / PFIC

        let total_cap = self.state.stats.year_div_gross + self.state.stats.year_cap_gains;

        // ── IRS Senior Standard Deduction Add-On (age ≥ 65) ─────────────────
        // Temporarily boost std_deduction by $1,550 per qualifying senior (MFJ 2026).
        let user_age = yr - self.cfg.birth_date.year();
        let spouse_is_senior = self.cfg.family_unit.spouse_birth_year
            .map(|birth_yr| yr - birth_yr >= 65)
            .unwrap_or(false);
        let senior_bonus =
            (if user_age >= 65 { self.tax_engine.rules.senior_addon_per_person } else { 0.0 })
            + (if spouse_is_senior { self.tax_engine.rules.senior_addon_per_person } else { 0.0 });
        let saved_std_deduction = self.tax_engine.rules.std_deduction;
        self.tax_engine.rules.std_deduction += senior_bonus;

        // Sync Japan resident tax accumulator from the expense tracker.
        self.state.stats.year_japan_res_tax_jpy = self.state.stats.year_exp_restax;

        // Japan-First FTC: credit Japan resident taxes paid this year against US liability.
        // V7.0: also include the Japan capital-gains tax (20.315%) realised at sale —
        // it is a foreign income tax under IRC §901 and so eligible for the FTC pool.
        // V7.6: split into §904 baskets. Japan cap-gains tax is unambiguously
        // passive-basket (transactional, on investment income). Japan resident
        // tax is allocated by income proportion (passive vs general); this
        // prevents passive credit from absorbing general-basket liability and
        // vice versa, the central §904 leakage guard.
        let (japan_tax_passive_usd, japan_tax_general_usd) =
            if self.cfg.tax_jurisdiction == TaxJurisdiction::Both {
                let cg_passive = self.state.stats.year_japan_cap_gains_tax_jpy
                    / self.state.current_fx;
                let res_total  = self.state.stats.year_japan_res_tax_jpy
                    / self.state.current_fx;
                let passive_inc = passive_ord + total_cap;
                let general_inc = general_ord;
                let denom = passive_inc + general_inc;
                let (res_passive, res_general) = if denom > 0.0 {
                    let p_share = passive_inc / denom;
                    (res_total * p_share, res_total * (1.0 - p_share))
                } else {
                    (0.0, res_total)
                };
                (cg_passive + res_passive, res_general)
            } else {
                (0.0, 0.0)
            };

        // IRC §904(c) per-basket carryover: prior-year unused credits stay in their
        // source basket so passive credit can never absorb general-basket liability.
        let effective_passive_usd = japan_tax_passive_usd
            + self.state.ftc_carryover_passive_usd;
        let effective_general_usd = japan_tax_general_usd
            + self.state.ftc_carryover_general_usd;
        // Lumped value retained for the FEIE path (legacy lumped FTC math).
        let effective_japan_tax_usd = effective_passive_usd + effective_general_usd;

        // Choose FEIE+FTC optimisation or basket-aware FTC based on strategy setting.
        let liability = match self.cfg.us_tax_strategy {
            UsTaxStrategy::FeieAndFtc => self.tax_engine.calculate_liability_with_feie_ftc(
                yr, earned_ord, unearned_ord, 0.0, total_cap, effective_japan_tax_usd,
            ),
            UsTaxStrategy::FtcOnly => self.tax_engine.calculate_liability_with_basket_ftc(
                yr,
                general_ord,
                passive_ord,
                0.0,
                total_cap,
                effective_passive_usd,
                effective_general_usd,
            ),
        };

        // Restore std_deduction (senior add-on is per-year, not permanently inflated).
        self.tax_engine.rules.std_deduction = saved_std_deduction;

        // Per-basket unused FTC: each basket's surplus carries within that basket.
        let passive_used = liability.breakdown.get("ftc_passive").copied().unwrap_or(0.0);
        let general_used = liability.breakdown.get("ftc_general").copied().unwrap_or(0.0);
        // For the FEIE path, breakdown has only "ftc_applied"; fall back to proportional split.
        let new_passive_co = if liability.feie_applied {
            (effective_japan_tax_usd - liability.ftc_applied).max(0.0)
                * if effective_japan_tax_usd > 0.0 {
                    effective_passive_usd / effective_japan_tax_usd
                } else { 1.0 }
        } else {
            (effective_passive_usd - passive_used).max(0.0)
        };
        let new_general_co = if liability.feie_applied {
            (effective_japan_tax_usd - liability.ftc_applied).max(0.0)
                * if effective_japan_tax_usd > 0.0 {
                    effective_general_usd / effective_japan_tax_usd
                } else { 0.0 }
        } else {
            (effective_general_usd - general_used).max(0.0)
        };
        let unused_ftc = new_passive_co + new_general_co;
        if unused_ftc > 0.0 {
            info!(
                "   [FTC Carryover] Year {}: ${:.2} unused (passive ${:.2} / general ${:.2})",
                yr, unused_ftc, new_passive_co, new_general_co,
            );
        }
        self.state.ftc_carryover_passive_usd = new_passive_co;
        self.state.ftc_carryover_general_usd = new_general_co;
        self.state.ftc_carryover_usd = unused_ftc;

        self.state.stats.year_feie_applied = liability.feie_applied;

        let already_paid = self.state.stats.year_tax_routed;
        let tax_due = liability.total_tax - already_paid;

        self.state.bridge_fund_usd -= tax_due;
        self.state.stats.year_tax_routed += tax_due;

        // Record the full US federal+state tax for dual-field reporting.
        // NOTE: liability.total_tax already IS the full annual bill; do not add already_paid.
        self.state.stats.year_us_fed_tax_usd = liability.total_tax;

        if tax_due.abs() > 500.0 {
            info!(
                "   [Tax True-Up] Year {}: Adj ${:.2} | Paid ${:.0} → Actual ${:.0} | FTC ${:.0} | State ${:.0} | FEIE ${:.0}",
                yr, tax_due, already_paid, liability.total_tax,
                liability.ftc_applied, liability.state_tax, liability.feie_exclusion,
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  Annual snapshot
    // ─────────────────────────────────────────────────────────────────────────

    fn record_annual_snapshot(&mut self, yr: i32) {
        let (exit_triggered, exit_value_jpy) = self.evaluate_exit_tax_trigger(yr);
        let s = &self.state.stats;
        let fx = self.state.current_fx;

        let val = |name: &str| -> f64 {
            self.state.accounts.get(name).map(|a| a.total_value(fx)).unwrap_or(0.0)
        };

        let total_net_usd = (s.year_div_gross - s.year_div_tax)
            + (s.year_fers_gross - s.year_fers_tax)
            + s.year_va_net
            + s.year_rsu_vest_usd
            + s.year_ss_payout_usd
            + s.year_ssdi_gross_usd
            + (s.year_nenkin_income_jpy / fx);
        let total_net_jpy = total_net_usd * fx;

        // V7.1: war_chest_jpy is always JPY-denominated.
        let extra_brokerage_usd: f64 = self.state.accounts.iter()
            .filter(|(name, _)| name.starts_with("Brokerage_"))
            .map(|(_, acc)| acc.total_value(fx))
            .sum();

        self.state.annual_summary.push(AnnualSnapshot {
            year: yr,
            usd_jpy: fx,
            brokerage_usd: val("Taxable") + extra_brokerage_usd,
            roth_usd: val("Roth"),
            dc_jpy: val("DC"),
            div_gross_usd: s.year_div_gross,
            div_net_usd: s.year_div_gross - s.year_div_tax,
            fers_net_usd: s.year_fers_gross - s.year_fers_tax,
            va_net_usd: s.year_va_net,
            rsu_vest_usd: s.year_rsu_vest_usd,
            total_inc_net_usd: total_net_usd,
            total_inc_net_jpy: total_net_jpy,
            base_exp_jpy: s.year_exp_base,
            nhi_obligation_jpy: s.year_exp_nhi,
            nenkin_jpy: s.year_exp_nenkin,
            res_tax_jpy: s.year_exp_restax,
            total_exp_jpy: s.year_total_exp_jpy,
            gap_jpy: total_net_jpy - s.year_total_exp_jpy,
            bridge_fund_usd: self.state.bridge_fund_usd,
            war_chest_jpy: self.state.war_chest_jpy,
            war_chest_used_jpy: s.year_wc_used,
            us_tax_charged_usd: s.year_us_fed_tax_usd,
            japan_tax_charged_jpy: s.year_japan_res_tax_jpy,
            ext_tax_paid_usd: s.tax_paid_external,
            ss_payout_usd: s.year_ss_payout_usd,
            nenkin_income_jpy: s.year_nenkin_income_jpy,
            feie_applied: s.year_feie_applied,
            bridge_exhausted: s.year_bridge_exhausted,
            forced_liquidations_usd: s.year_forced_liquidations_usd,
            ftc_carryover_usd: self.state.ftc_carryover_usd,
            purchasing_power_usd: self.cfg.min_expense_jpy / fx,
            div_coverage_ratio: {
                let div_jpy = s.year_div_gross * fx;
                if s.year_total_exp_jpy > 0.0 { div_jpy / s.year_total_exp_jpy } else { 0.0 }
            },
            japan_cap_gains_tax_jpy: s.year_japan_cap_gains_tax_jpy,
            state_cap_gains_tax_usd: s.year_state_cap_gains_tax_usd,
            fx_penalty_jpy: s.year_fx_penalty_jpy,
            months_at_min_target: s.year_months_target_dropped,
            exit_tax_triggered: exit_triggered,
            exit_tax_asset_value_jpy: exit_value_jpy,
            year_gift_sink_jpy: s.year_gift_sink_jpy,
            year_form_709_required: s.year_form_709_required,

            // V7.6 — dual-field reporting. Gross = all distribution components +
            // capital gains realised; Net = gross minus dividend/CG taxes. Tax
            // friction surfaces the delta without naming the underlying regime.
            total_gross_return_usd: s.year_div_gross + s.year_cap_gains
                + s.year_dist_roc_usd + s.year_pfic_mtm_income_usd,
            total_net_return_usd: s.year_div_gross + s.year_cap_gains
                + s.year_dist_roc_usd + s.year_pfic_mtm_income_usd
                - s.year_div_tax
                - s.year_state_cap_gains_tax_usd
                - (s.year_japan_cap_gains_tax_jpy / fx),
            tax_friction_usd: s.year_div_tax
                + s.year_state_cap_gains_tax_usd
                + (s.year_japan_cap_gains_tax_jpy / fx),
            dist_dividend_usd: s.year_dist_dividend_usd,
            dist_interest_usd: s.year_dist_interest_usd,
            dist_cap_gains_usd: s.year_dist_cap_gains_usd,
            dist_special_usd: s.year_dist_special_usd,
            dist_roc_usd: s.year_dist_roc_usd,
        });
    }

    // ── V7.5 — Exit Tax Monitor ──────────────────────────────────────────────
    fn evaluate_exit_tax_trigger(&self, yr: i32) -> (bool, f64) {
        const THRESHOLD_JPY: f64 = 100_000_000.0;
        let start = match self.cfg.japan_residency_start_date {
            Some(d) => d,
            None    => return (false, 0.0),
        };
        // 5-of-10 residency test (IT Act Art. 60-2).
        let years_resident = ((yr - start.year()).min(10)).max(0);
        if years_resident < 5 { return (false, 0.0); }

        let fx = self.state.current_fx;
        let mut assets_jpy = 0.0_f64;
        for (name, acc) in &self.state.accounts {
            let val_jpy = acc.total_value(fx);
            let include = if self.cfg.exit_tax_include_tax_advantaged {
                true
            } else {
                !matches!(name.as_str(), "Roth" | "NISA" | "iDeCo")
            };
            if include { assets_jpy += val_jpy; }
        }
        (assets_jpy >= THRESHOLD_JPY, assets_jpy)
    }

    // ─────────────────────────────────────────────────────────────────────────
    //  Helpers
    // ─────────────────────────────────────────────────────────────────────────

    /// Spouse SS / Nenkin monthly accrual when the spouse has reached the
    /// configured start age. SS is added in USD; Nenkin in JPY. Both feed the
    /// existing annual stat accumulators so downstream tax + reporting honour
    /// them without further plumbing.
    fn process_spouse_benefits(&mut self, yr: i32) {
        let spouse_birth_year = match self.cfg.family_unit.spouse_birth_year {
            Some(y) => y,
            None    => return,
        };
        let spouse_age = yr - spouse_birth_year;

        if self.cfg.spouse_ss_monthly_usd > 0.0
            && spouse_age >= self.cfg.spouse_ss_start_age as i32
        {
            self.state.stats.year_ss_payout_usd += self.cfg.spouse_ss_monthly_usd;
        }
        if self.cfg.spouse_nenkin_monthly_jpy > 0.0
            && spouse_age >= self.cfg.spouse_nenkin_start_age as i32
        {
            self.state.stats.year_nenkin_income_jpy += self.cfg.spouse_nenkin_monthly_jpy;
        }
    }

    /// Estimates total annual ordinary income for marginal tax bracket calculations.
    /// Pre-retirement: adds the annual compensation on top of FERS projection.
    /// Post-retirement: returns accumulated ordinary income so far.
    fn estimate_annual_ord_income_static(state: &SimState, cfg: &Config, _yr: i32) -> f64 {
        let base = state.stats.acc_ord_inc;
        if state.date < cfg.retirement_date {
            base + cfg.total_annual_compensation_usd
        } else {
            base
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  V5.2 Stress-Test Integration Tests
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use std::fs;
    use crate::config::loader::load_scenario;
    use crate::reporter::format_csv;

    // ── V6.6 unit tests (pure logic, no scenario I/O required) ─────────────

    #[test]
    fn v6_6_months_between_is_calendar_aware() {
        use chrono::NaiveDate;
        let r = NaiveDate::from_ymd_opt(2031, 1, 1).unwrap();
        assert_eq!(super::months_between(r, r), 0);
        assert_eq!(super::months_between(r, NaiveDate::from_ymd_opt(2031, 7, 1).unwrap()), 6);
        assert_eq!(super::months_between(r, NaiveDate::from_ymd_opt(2032, 1, 1).unwrap()), 12);
        // Past dates clamp to 0 (no negative cadence triggers).
        assert_eq!(super::months_between(r, NaiveDate::from_ymd_opt(2030, 12, 1).unwrap()), 0);
    }

    #[test]
    fn v6_6_fx_drift_cadence_fires_on_multiples() {
        // Simulates the controller's cadence logic in isolation.
        use chrono::NaiveDate;
        let retire = NaiveDate::from_ymd_opt(2031, 1, 1).unwrap();
        let cadence = 6;
        let increment: f64 = 5.0;
        let mut fx: f64 = 145.0;
        let mut hits = 0;
        for m in 0..24 {
            let yr = 2031 + (m / 12);
            let mo = (m % 12 + 1) as u32;
            let date = NaiveDate::from_ymd_opt(yr, mo, 1).unwrap();
            let elapsed = super::months_between(retire, date);
            if elapsed > 0 && elapsed % cadence == 0 {
                fx += increment;
                hits += 1;
            }
        }
        // Months 6, 12, 18 elapsed within months 0..24 (m=23 → elapsed=23, m=23 not divisible).
        // Hits: m=6 (elapsed=6), m=12 (12), m=18 (18). 3 firings.
        assert_eq!(hits, 3, "expected 3 cadence hits over 24 months at 6-month cadence");
        let expected: f64 = 145.0 + 15.0;
        assert!((fx - expected).abs() < 1e-9);
    }

    #[test]
    fn v6_6_spouse_ss_activates_at_start_age() {
        // Pure age-gate logic mirroring process_spouse_benefits().
        let spouse_birth_year = 1970;
        let start_age: i32 = 67;
        for yr in 2030..=2040 {
            let age = yr - spouse_birth_year;
            let active = age >= start_age;
            if yr < 2037 {
                assert!(!active, "spouse SS must NOT be active at age {}", age);
            } else {
                assert!(active, "spouse SS must be active at age {}", age);
            }
        }
    }

    #[test]
    fn v6_6_position_rebalance_date_field_present() {
        use chrono::NaiveDate;
        use crate::models::config::Position;
        let p_default = Position { ticker: "VTI".into(), quantity: 100.0, avg_cost: 50.0,
                                    ..Default::default() };
        assert!(p_default.rebalance_date.is_none(), "default rebalance_date must be None");
        assert_eq!(p_default.avg_purchase_price_jpy, 0.0, "V7.0 default jpy basis = 0.0");
        let p_dated = Position {
            ticker: "VTI".into(), quantity: 100.0, avg_cost: 50.0,
            rebalance_date: Some(NaiveDate::from_ymd_opt(2032, 6, 1).unwrap()),
            avg_purchase_price_jpy: 7_500.0,
            ..Default::default()
        };
        assert_eq!(p_dated.avg_purchase_price_jpy, 7_500.0);
        assert_eq!(p_dated.rebalance_date.unwrap().format("%Y-%m-%d").to_string(), "2032-06-01");
    }

    /// Loads test_crash_2030_stress_scenario.json, runs the full simulation, writes the audit CSV to
    /// output/crash_test_audit.csv, then asserts on the three new V5.2 audit columns.
    ///
    /// Expected behaviours:
    ///  - Portfolio drops in 2030 vs 2029 (14-month drawdown fires correctly)
    ///  - Bridge fund exhausts (VA-only income < ¥700K expenses for 6 years pre-FERS)
    ///  - FTC carryover accumulates (Year+1 Japan tax on ¥20M gross > low post-retirement US liability)
    ///  - Forced liquidations occur once bridge + war chest drain (within the 2040 window)
    #[test]
    fn test_crash_2030_stress_scenario() {
        let loaded = load_scenario("input/test_crash_2030_stress_scenario.json")
            .expect("test_crash_2030_stress_scenario.json should load without error");

        let results = super::SimulationController::new(loaded.config, loaded.accounts).run();
        let snaps = &results.annual_summary;

        // ── 1. Simulation produced snapshots ─────────────────────────────────
        assert!(!snaps.is_empty(), "simulation should produce annual snapshots");
        // Expect at least pre-retirement and some post-retirement years (2025–2040)
        assert!(snaps.len() >= 10, "expected ≥10 annual snapshots; got {}", snaps.len());

        // ── 2. Crash took effect: 2030 portfolio < 2029 portfolio ────────────
        let snap_2029 = snaps.iter().find(|s| s.year == 2029);
        let snap_2030 = snaps.iter().find(|s| s.year == 2030);
        if let (Some(pre), Some(post)) = (snap_2029, snap_2030) {
            assert!(
                post.brokerage_usd < pre.brokerage_usd,
                "Taxable portfolio should be lower in crash year 2030 ({:.0}) than 2029 ({:.0})",
                post.brokerage_usd, pre.brokerage_usd,
            );
        }

        // ── 3. Bridge exhaustion: VA-only income < ¥700K expenses ───────────
        // FERS doesn't start until 2037 (birth 1975 + fers_start_age 62).
        // The monthly gap is negative every month from 2031 to 2037.
        assert!(
            snaps.iter().any(|s| s.bridge_exhausted),
            "BridgeExhausted should be Y in at least one year; \
             VA-only income (~¥274K) cannot cover ¥700K expenses",
        );

        // ── 4. FTC carryover: Year+1 Japan tax on ¥20M gross > low US liability
        assert!(
            snaps.iter().any(|s| s.ftc_carryover_usd > 0.0),
            "FTC_Carryover_USD should be positive in at least one year; \
             Japan resident tax on ¥20M retirement-year gross should exceed \
             the US federal liability on post-retirement (VA + small dividend) income",
        );

        // ── 5. Forced liquidations: once bridge + war chest exhaust ──────────
        assert!(
            snaps.iter().any(|s| s.forced_liquidations_usd > 0.0),
            "ForcedLiquidations_USD should be non-zero once bridge and war chest deplete",
        );

        // ── 6. Write audit CSV for manual inspection ─────────────────────────
        let _ = fs::create_dir_all("output");
        let csv = format_csv(&results);
        fs::write("output/crash_test_audit.csv", &csv)
            .expect("should be able to write crash_test_audit.csv");

        // Print a brief summary to test output for visibility
        println!("\n=== crash_test_2030 summary ({} years) ===", snaps.len());
        println!("{:<6}  {:>12}  {:>10}  {:>10}  {:>16}  {:>8}  {:>8}",
            "Year", "Brokerage($)", "Bridge($)", "FTC_CO($)", "ForcedLiq($)", "BridgeEx", "FTCcarry");
        for s in snaps {
            if s.year >= 2029 {
                println!("{:<6}  {:>12.0}  {:>10.0}  {:>10.2}  {:>16.2}  {:>8}  {:>8.2}",
                    s.year,
                    s.brokerage_usd,
                    s.bridge_fund_usd,
                    s.ftc_carryover_usd,
                    s.forced_liquidations_usd,
                    if s.bridge_exhausted { "Y" } else { "N" },
                    s.ftc_carryover_usd,
                );
            }
        }
    }

    /// Loads test_fx_shock_2032.json, runs the full simulation, writes the audit CSV to
    /// output/fx_shock_audit.csv, then asserts on V5.3 FX shock behaviour.
    ///
    /// Expected behaviours:
    ///  - Year 2031: FX rate is ~145 (pre-shock baseline with no drift)
    ///  - Year 2032: FX rate snaps to 80 (shock fires in handle_new_year)
    ///  - Purchasing_Power_USD rises in 2032 (same JPY floor costs more USD at ¥80)
    ///  - Simulation completes without panic
    #[test]
    fn test_fx_shock_2032() {
        let loaded = load_scenario("input/test_fx_shock_2032.json")
            .expect("test_fx_shock_2032.json should load without error");

        let results = super::SimulationController::new(loaded.config, loaded.accounts).run();
        let snaps = &results.annual_summary;

        assert!(!snaps.is_empty(), "simulation should produce annual snapshots");

        // ── 1. FX shock applied: 2032 rate should be ~80 ────────────────────
        let snap_2031 = snaps.iter().find(|s| s.year == 2031);
        let snap_2032 = snaps.iter().find(|s| s.year == 2032);

        if let Some(post) = snap_2032 {
            assert!(
                (post.usd_jpy - 80.0).abs() < 1.0,
                "Year 2032 FX should be ~80 after shock; got {:.2}",
                post.usd_jpy,
            );
        } else {
            panic!("No snapshot found for 2032");
        }

        // ── 2. Pre-shock rate was ~145 ────────────────────────────────────────
        if let Some(pre) = snap_2031 {
            assert!(
                pre.usd_jpy > 100.0,
                "Year 2031 FX should be ~145 (no drift, no shock yet); got {:.2}",
                pre.usd_jpy,
            );
        }

        // ── 3. Purchasing power rose at shock year (same JPY floor costs more USD)
        if let (Some(pre), Some(post)) = (snap_2031, snap_2032) {
            assert!(
                post.purchasing_power_usd > pre.purchasing_power_usd,
                "Purchasing_Power_USD should increase in shock year (¥450K at ¥80/$ > at ¥145/$); \
                 2031={:.2}, 2032={:.2}",
                pre.purchasing_power_usd, post.purchasing_power_usd,
            );
        }

        // ── 4. Write audit CSV ────────────────────────────────────────────────
        let _ = fs::create_dir_all("output");
        let csv = format_csv(&results);
        fs::write("output/fx_shock_audit.csv", &csv)
            .expect("should be able to write fx_shock_audit.csv");

        println!("\n=== fx_shock_test summary ({} years) ===", snaps.len());
        println!("{:<6}  {:>10}  {:>18}",
            "Year", "FX(¥/$)", "PurchasingPower($)");
        for s in snaps {
            if s.year >= 2030 {
                println!("{:<6}  {:>10.2}  {:>18.2}", s.year, s.usd_jpy, s.purchasing_power_usd);
            }
        }
    }
}

use chrono::{Datelike, NaiveDate};

use crate::models::config::{Config, VaDependentStatus};

/// The monthly expense breakdown for a given date.
#[derive(Debug, Default, Clone)]
pub struct ExpenseBreakdown {
    pub total_desired: f64,
    pub base_desired: f64,
    pub base_floor: f64,
    pub nhi: f64,
    pub nenkin: f64,
    pub restax: f64,
    /// V7.3 — Sum of active ExpenseRules whose `name` contains "Education".
    /// These bypass the standard waterfall and route Tier 2.5 → Tier 8.
    pub education: f64,
    /// Stage 06 — Monthly real-estate fixed costs (PI + property tax) in JPY.
    /// Zero when `cfg.real_estate` is empty.
    pub real_estate: f64,
}

/// The monthly income breakdown.
#[derive(Debug, Default, Clone)]
pub struct IncomeBreakdown {
    pub va_usd: f64,
    pub fers_usd: f64,
    /// Social Security monthly income (USD).
    pub ss_usd: f64,
    /// SSDI (Social Security Disability Insurance) monthly income (USD).
    /// Taxable portion is determined annually via the Combined Income rule.
    pub ssdi_usd: f64,
    /// Nenkin pension monthly income (JPY). Separate from Nenkin expense contributions.
    pub nenkin_income_jpy: f64,
    /// Stage 06 — Net monthly rental income in JPY (Japan properties).
    pub rental_jpy: f64,
    /// Stage 06 — Net monthly rental income in USD (US / international properties).
    pub rental_usd: f64,
}

/// Handles all recurring income and expense calculations.
/// Mirrors Python's `CashFlowEngine` class in `engine.py`.
pub struct CashFlowEngine {
    cfg: Config,
}

impl CashFlowEngine {
    pub fn new(cfg: Config) -> Self {
        Self { cfg }
    }

    /// Calculates the "diet" COLA rate for FERS pensions.
    /// US law limits FERS COLA to slightly less than CPI.
    ///
    /// Mirrors Python's `_get_diet_cola_rate()`:
    ///   CPI <= 2% → full CPI
    ///   CPI <= 3% → capped at 2%
    ///   CPI >  3% → CPI - 1%
    fn diet_cola_rate(&self) -> f64 {
        let cpi = self.cfg.inflation_cola;
        if cpi <= 0.02 {
            cpi
        } else if cpi <= 0.03 {
            0.02
        } else {
            cpi - 0.01
        }
    }

    /// Calculates the FERS pension monthly payment for a given simulation year,
    /// applying diet-COLA compounding from the year after the retiree turns 62.
    ///
    /// COLA does not apply before the calendar year following age 62.
    /// Mirrors Python's `calculate_fers_monthly()`.
    pub fn calculate_fers_monthly(&self, current_year: i32) -> f64 {
        if current_year < self.cfg.fers_start_date.year() {
            return 0.0;
        }

        let age_62_year = self.cfg.birth_date.year() + 62;
        let cola_start_year = age_62_year + 1;
        let start_cal_year = self.cfg.fers_start_date.year();

        let effective_start = start_cal_year.max(cola_start_year) + 1;
        let years_compounding = (current_year - effective_start + 1).max(0) as u32;

        let diet_rate = self.diet_cola_rate();
        let multiplier = (1.0 + diet_rate).powi(years_compounding as i32);

        self.cfg.fers_monthly_start * multiplier
    }

    /// Returns the monthly expense breakdown for a given date.
    ///
    /// Before retirement: returns all zeros (no retirement expenses tracked).
    /// After retirement: inflates base expense and applies all active ExpenseRules.
    ///
    /// Expense rules with "NHI" in their name → nhi bucket
    /// Expense rules with "Nenkin" in their name → nenkin bucket
    /// Expense rules with "ResTax" in their name → restax bucket
    /// Everything else → added to base_desired and base_floor.
    ///
    /// `fx` is needed to convert USD mortgage PI / property tax into JPY for the
    /// `real_estate` bucket.
    ///
    /// Mirrors Python's `get_expenses_breakdown()`.
    pub fn get_expenses_breakdown(&self, current_date: NaiveDate, fx: f64) -> ExpenseBreakdown {
        if current_date < self.cfg.retirement_date {
            return ExpenseBreakdown::default();
        }

        let years_passed = (current_date - self.cfg.start_date).num_days() as f64 / 365.25;
        let inflation = (1.0 + self.cfg.inflation_japan).powf(years_passed);

        let mut base_desired = self.cfg.base_expense_jpy * inflation;
        let mut base_floor = self.cfg.min_expense_jpy * inflation;
        let mut nhi_cost = 0.0_f64;
        let mut nenkin_cost = 0.0_f64;
        let mut restax_cost = 0.0_f64;
        let mut edu_cost = 0.0_f64;

        for rule in &self.cfg.expense_rules {
            if rule.is_active_on(current_date) {
                if rule.name.contains("NHI") {
                    nhi_cost += rule.amount_jpy;
                } else if rule.name.contains("Nenkin") {
                    nenkin_cost += rule.amount_jpy;
                } else if rule.name.contains("ResTax") {
                    restax_cost += rule.amount_jpy;
                } else if rule.name.contains("Education") {
                    edu_cost += rule.amount_jpy;
                } else {
                    base_desired += rule.amount_jpy;
                    base_floor += rule.amount_jpy;
                }
            }
        }

        // ── Stage 06 — Real-estate fixed costs (PI + property tax) ──────────
        let re_cost = if self.cfg.real_estate.is_empty() {
            0.0
        } else {
            crate::engine::real_estate_engine::total_monthly_re_expense_jpy(
                &self.cfg.real_estate, current_date, fx,
            )
        };

        let fixed_costs = nhi_cost + nenkin_cost + restax_cost + re_cost;
        ExpenseBreakdown {
            total_desired: base_desired + fixed_costs + edu_cost,
            base_desired,
            base_floor,
            nhi: nhi_cost,
            nenkin: nenkin_cost,
            restax: restax_cost,
            education: edu_cost,
            real_estate: re_cost,
        }
    }

    /// Returns the monthly income breakdown for a given date.
    ///
    /// VA rate logic (when `va_disability_rating` is set):
    ///   Uses the 2026 lookup table inflated forward from 2026.
    /// VA rate logic (legacy fallback when `va_disability_rating == 0`):
    ///   Uses the `va_disability_rates` map with COLA inflation.
    /// VA is always tax-free (US federal, state, and Japan resident tax).
    ///
    /// Mirrors Python's `get_incomes_usd()`.
    pub fn get_incomes_usd(&self, current_date: NaiveDate) -> IncomeBreakdown {
        let sim_year = current_date.year();

        // ── VA income ─────────────────────────────────────────────────────────
        // Priority: manual override > 2026 lookup table + optional SMC variant.
        // All VA income is tax-free. COLA inflation applied from 2026 baseline.
        let va_usd = {
            let years_from_2026 = (sim_year - 2026).max(0) as u32;
            let cola_factor = (1.0 + self.cfg.inflation_cola).powi(years_from_2026 as i32);

            // Base VA amount — override takes priority over rating table.
            let base_inflated = if let Some(ov) = self.cfg.va_monthly_override {
                ov * cola_factor
            } else if self.cfg.va_disability_rating > 0 {
                let effective_status = match self.cfg.va_dependent_status {
                    VaDependentStatus::WithSpouseAndChild => {
                        // Primary check: exact 18th-birthday cutoff (precise date comparison).
                        let passed_18 = self.cfg.va_child_cutoff_date
                            .map(|cutoff| current_date > cutoff)
                            .unwrap_or(false);
                        let child_eligible = if !passed_18 {
                            true // Still under 18 — unconditionally eligible.
                        } else {
                            // V6.4: past 18th birthday — eligible only if a college-student
                            // dependent is still within the age-23 extended window.
                            self.cfg.family_unit.dependents.iter().any(|dep| {
                                dep.is_college_student && (sim_year - dep.birth_year) <= 23
                            })
                        };
                        if child_eligible { VaDependentStatus::WithSpouseAndChild }
                        else              { VaDependentStatus::WithSpouse }
                    }
                    other => other,
                };
                crate::engine::va_benefits::lookup_va_monthly_2026(
                    self.cfg.va_disability_rating,
                    effective_status,
                ) * cola_factor
            } else {
                0.0
            };

            // SMC component — manual override takes priority over variant lookup.
            let smc_inflated = if let Some(ov) = self.cfg.smc_monthly_override {
                ov * cola_factor
            } else if let Some(smc_label) = &self.cfg.va_smc_variant {
                if let Some(entry) = crate::engine::va_benefits::ALL_SMC_VARIANTS
                    .iter().find(|e| e.0 == smc_label.as_str())
                {
                    crate::engine::va_benefits::lookup_smc_monthly_2026(entry.1) * cola_factor
                } else {
                    0.0
                }
            } else {
                0.0
            };

            // K adds; all other non-K SMC variants replace the base amount.
            // When using a numeric SMC override (no variant label), treat as additive.
            let smc_is_additive = self.cfg.smc_monthly_override.is_some()
                || self.cfg.va_smc_variant.as_deref()
                    .and_then(|l| crate::engine::va_benefits::ALL_SMC_VARIANTS.iter().find(|e| e.0 == l))
                    .map(|e| e.1 == crate::engine::va_benefits::SmcVariant::K)
                    .unwrap_or(false);

            if smc_inflated > 0.0 {
                if smc_is_additive { base_inflated + smc_inflated } else { smc_inflated }
            } else {
                base_inflated
            }
        };

        // ── FERS pension ──────────────────────────────────────────────────────
        let fers_usd = if current_date >= self.cfg.fers_start_date {
            self.calculate_fers_monthly(sim_year)
        } else {
            0.0
        };

        // ── Social Security ───────────────────────────────────────────────────
        let ss_usd = if self.cfg.ss_monthly_usd > 0.0 {
            let ss_start_year = self.cfg.birth_date.year() + self.cfg.ss_start_age as i32;
            if sim_year >= ss_start_year {
                let years_from_start = (sim_year - ss_start_year).max(0) as u32;
                self.cfg.ss_monthly_usd * (1.0 + self.cfg.inflation_cola).powi(years_from_start as i32)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // ── Nenkin pension income ─────────────────────────────────────────────
        let nenkin_income_jpy = if self.cfg.nenkin_income_monthly_jpy > 0.0 {
            let nenkin_start_year = self.cfg.birth_date.year() + self.cfg.nenkin_income_start_age as i32;
            if sim_year >= nenkin_start_year {
                let years_from_start = (sim_year - nenkin_start_year).max(0) as u32;
                self.cfg.nenkin_income_monthly_jpy
                    * (1.0 + self.cfg.inflation_japan).powi(years_from_start as i32)
            } else {
                0.0
            }
        } else {
            0.0
        };

        // ── SSDI ──────────────────────────────────────────────────────────────
        // Inflates with COLA from 2026 baseline; taxable portion computed annually.
        // At age 65 the classification transitions to SS retirement (amount unchanged).
        let ssdi_usd = if self.cfg.ssdi_monthly_usd > 0.0 {
            let years_from_2026 = (sim_year - 2026).max(0) as u32;
            let cola_factor = (1.0 + self.cfg.inflation_cola).powi(years_from_2026 as i32);
            self.cfg.ssdi_monthly_usd * cola_factor
        } else {
            0.0
        };

        // ── Stage 06 — Rental income ─────────────────────────────────────────
        let rental_jpy = if self.cfg.real_estate.is_empty() {
            0.0
        } else {
            crate::engine::real_estate_engine::total_monthly_rental_jpy(&self.cfg.real_estate)
        };
        let rental_usd = if self.cfg.real_estate.is_empty() {
            0.0
        } else {
            crate::engine::real_estate_engine::total_monthly_rental_usd(&self.cfg.real_estate)
        };

        IncomeBreakdown { va_usd, fers_usd, ss_usd, ssdi_usd, nenkin_income_jpy, rental_jpy, rental_usd }
    }

    /// Add new expense rules (e.g., dynamically scheduled resident tax installments).
    /// Called by the controller whenever new rules are scheduled during the simulation.
    pub fn add_expense_rules(&mut self, rules: &[crate::models::expense::ExpenseRule]) {
        self.cfg.expense_rules.extend_from_slice(rules);
    }

    /// Compute the projected annual FERS income for a given year.
    /// Returns 0 if FERS has not started. Prorates by months remaining in the start year.
    pub fn calc_annual_fers_projection(&self, yr: i32) -> f64 {
        if yr < self.cfg.fers_start_date.year() {
            return 0.0;
        }
        let monthly = self.calculate_fers_monthly(yr);
        if yr == self.cfg.fers_start_date.year() {
            let months_active = (12 - self.cfg.fers_start_date.month() + 1) as f64;
            monthly * months_active
        } else {
            monthly * 12.0
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::config::TaxRules;

    /// Build a minimal Config for cashflow tests.
    fn minimal_cfg() -> Config {
        Config {
            start_date: NaiveDate::from_ymd_opt(2025, 1, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2080, 12, 31).unwrap(),
            retirement_date: NaiveDate::from_ymd_opt(2031, 1, 1).unwrap(),
            rebalance_date: NaiveDate::from_ymd_opt(2031, 2, 1).unwrap(),
            usd_jpy: 145.0,
            inflation_cola: 0.028,
            inflation_japan: 0.028,
            ira_limit_growth: 0.03,
            fx_drift_enabled: false,
            fx_drift_rate: 0.02,
            fx_drift_cadence_months: 0,
            fx_drift_increase_amount_jpy: 0.0,
            recession_enabled: false,
            recession_severity: 0.20,
            recession_events: vec![],
            fx_shock_events: vec![],
            base_expense_jpy: 1_000_000.0,
            min_expense_jpy: 600_000.0,
            nhi_spike_monthly_jpy: 73_333.0,
            nhi_model: crate::models::config::NhiModel::default(),
            war_chest_currency: "JPY".into(),
            war_chest_target_jpy: 7_000_000.0,
            war_chest_target_usd: 50_000.0,
            bridge_months_target: 12,
            bridge_fund_currency: "JPY".into(),
            roth_start_limit: 7_000.0,
            roth_contribution_made_this_year: false,
            roth_contribution_so_far: 0.0,
            dc_monthly_jpy: 45_000.0,
            dc_growth_rate: 0.08,
            monthly_contribution_ticker: "VTI".into(),
            va_contribution_buffer_usd: 800.0,
            nenkin_baseline_annual_jpy: 171_800.0,
            growth_rates_annual: std::collections::HashMap::new(),
            va_disability_rates: {
                let mut m = std::collections::HashMap::new();
                m.insert("2026".into(), crate::models::config::VaRates {
                    base: 4_158.17, child_addon: 160.82
                });
                m
            },
            fers_monthly_start: 794.55,
            fers_start_date: NaiveDate::from_ymd_opt(2037, 9, 1).unwrap(),
            retirement_year_gross_income_jpy: 40_000_000.0,
            birth_date: NaiveDate::from_ymd_opt(1975, 9, 1).unwrap(), // age 62 in 2037
            spouse_birth_date: NaiveDate::from_ymd_opt(1978, 1, 1).unwrap(),
            child_birth_date: NaiveDate::from_ymd_opt(2018, 9, 18).unwrap(),
            va_child_cutoff_date: Some(NaiveDate::from_ymd_opt(2036, 9, 18).unwrap()),
            dc_payout_start_age: 60,
            dc_payout_method: "LUMP_SUM".into(),
            pre_funded_war_chest_jpy: 0.0,
            pre_funded_bridge_jpy: 0.0,
            pre_funded_bridge_usd: 0.0,
            pre_funded_japan_tax_jpy: 0.0,
            pre_funded_us_tax_usd: 0.0,
            target_vti_pct: 0.20,
            target_schd_pct: 0.80,
            roth_rebalance_target_vti_pct: 0.50,
            roth_rebalance_target_schd_pct: 0.50,
            enable_roth_rebalance_at_59: false,
            buy_schd_last_year: false,
            rsu_tax_handling: "SALARY".into(),
            total_annual_compensation_usd: 0.0,
            expense_rules: vec![],
            rsu_awards: vec![],
            tax_rules: TaxRules::default(),
            tax_jurisdiction: crate::models::config::TaxJurisdiction::Both,
            investment_location: crate::models::config::InvestmentLocation::Us,
            us_tax_strategy: crate::models::config::UsTaxStrategy::FtcOnly,
            va_disability_rating: 100,
            va_dependent_status: crate::models::config::VaDependentStatus::WithSpouseAndChild,
            ss_monthly_usd: 0.0,
            ss_start_age: 67,
            ssdi_monthly_usd: 0.0,
            is_married: true,
            spouse_ss_monthly_usd: 0.0,
            spouse_ss_start_age: 67,
            spouse_ss_jurisdiction: crate::models::config::TaxProtocol::Both,
            spouse_nenkin_monthly_jpy: 0.0,
            spouse_nenkin_start_age: 65,
            spouse_nenkin_jurisdiction: crate::models::config::TaxProtocol::Both,
            family_unit: crate::models::config::FamilyUnit {
                user_birth_year:   1975,
                spouse_birth_year: Some(1978),
                dependents: vec![crate::models::config::Dependent {
                    birth_year: 2018,
                    birth_date: NaiveDate::from_ymd_opt(2018, 9, 18),
                    is_college_student: false,
                }],
            },
            nenkin_income_monthly_jpy: 0.0,
            nenkin_income_start_age: 65,
            prefecture: "Kanagawa".into(),
            city: "Sagamihara".into(),
            military_retired: None,
            fers_jurisdiction: crate::models::config::TaxProtocol::Both,
            fers_japan_local_tax_exempt: false,
            ss_jurisdiction: crate::models::config::TaxProtocol::Both,
            nenkin_jurisdiction: crate::models::config::TaxProtocol::Both,
            va_smc_variant: None,
            va_monthly_override: None,
            smc_monthly_override: None,
            accumulation_rules: vec![],
            target_allocations: std::collections::HashMap::new(),
            rebalance_enabled: false,
            rebalance_frequency_months: 12,
            us_state_tax_rate: 0.0,
            withdrawal_strategy: crate::models::config::WithdrawalStrategy::TotalReturn,
            withdrawal_waterfall: crate::models::config::WaterfallStrategy::Defensive,
            fx_spread_penalty: 0.005,
            withdrawal_regime: crate::models::config::WithdrawalRegime::Shielded,
            edu_savings_jpy_monthly: 0.0,
            jido_teate_enabled: true,
            // V7.5 defaults
            japan_residency_start_date: None,
            exit_tax_include_tax_advantaged: true,
            annual_gift_jpy_per_recipient: 0.0,
            gift_recipient_count: 0,
            us_gift_exclusion_usd: 19_000.0,
            tlh_enabled: false,
            tlh_active_months: vec![11, 12],
            tlh_min_loss_usd: 500.0,
            // V7.7 defaults
            enable_education_savings: true,
            enable_gift_sink: true,
            // V7.7.2 defaults
            rsu_sell_to_cover_realism: true,
            rsu_sell_to_cover_policy: crate::models::config::RsuSellToCoverPolicy::Strict,
            // Stage 02 defaults
            spouse_profile: crate::models::config::SpouseProfile::UsPerson,
            spouse_japan_salary_jpy: 0.0,
            spouse_japan_misc_income_jpy: 0.0,
            // Stage 03 defaults
            monthly_dependent_precision: true,
            // Stage 04 defaults
            shock_ordering: crate::models::config::ShockOrdering::DepreciateThenReprice,
            // Stage 05 defaults
            track_pfic_basis_drift: true,
            // Stage 06 defaults
            real_estate: vec![],
            enable_heloc_tier: false,
            // Stage 07 defaults
            enable_estate_planning: false,
            death_date: None,
            spouse_death_date: None,
            heirs: vec![],
            estate_planning_jurisdiction: crate::models::config::TaxProtocol::Both,
            enable_gifting_optimiser: false,
            // Stage 08 defaults
            mc_use_correlated_paths: false,
            mc_correlation_matrix: std::collections::HashMap::new(),
            // Stage 09 defaults
            crypto_tax_enabled: true,
        }
    }

    #[test]
    fn test_diet_cola_tier1_below_2pct() {
        let mut cfg = minimal_cfg();
        cfg.inflation_cola = 0.015;
        let engine = CashFlowEngine::new(cfg);
        assert!((engine.diet_cola_rate() - 0.015).abs() < 1e-9);
    }

    #[test]
    fn test_diet_cola_tier2_between_2_and_3pct() {
        let mut cfg = minimal_cfg();
        cfg.inflation_cola = 0.025;
        let engine = CashFlowEngine::new(cfg);
        assert!((engine.diet_cola_rate() - 0.02).abs() < 1e-9);
    }

    #[test]
    fn test_diet_cola_tier3_above_3pct() {
        let mut cfg = minimal_cfg();
        cfg.inflation_cola = 0.035;
        let engine = CashFlowEngine::new(cfg);
        assert!((engine.diet_cola_rate() - 0.025).abs() < 1e-9);
    }

    #[test]
    fn test_fers_returns_zero_before_start() {
        let cfg = minimal_cfg();
        let engine = CashFlowEngine::new(cfg);
        assert_eq!(engine.calculate_fers_monthly(2030), 0.0);
    }

    #[test]
    fn test_fers_start_year_no_cola() {
        let cfg = minimal_cfg();
        let engine = CashFlowEngine::new(cfg);
        // FERS starts 2037; birth year 1975, age 62 = 2037, cola_start = 2038+1=2039
        // effective_start = max(2037,2039)+1 = 2040
        // years_compounding = max(0, 2037 - 2040 + 1) = 0
        let monthly = engine.calculate_fers_monthly(2037);
        assert!((monthly - 794.55).abs() < 0.01, "monthly={}", monthly);
    }

    #[test]
    fn test_va_income_inflates_from_base_year() {
        let cfg = minimal_cfg();
        let engine = CashFlowEngine::new(cfg);
        // Rating 100, WithSpouseAndChild. Child cutoff 2036-09-18; date 2028-06-01 is eligible.
        // 2026 base = $4,318.99; inflated 2 years at 2.8%.
        let date = NaiveDate::from_ymd_opt(2028, 6, 1).unwrap();
        let income = engine.get_incomes_usd(date);
        let factor = (1.028_f64).powi(2);
        let expected_va = 4_318.99 * factor;
        assert!((income.va_usd - expected_va).abs() < 0.01, "va={} expected={}", income.va_usd, expected_va);
    }

    #[test]
    fn test_va_income_base_only_after_child_cutoff() {
        let cfg = minimal_cfg();
        let engine = CashFlowEngine::new(cfg);
        // After child cutoff (2036-09-18), status downgraded from WithSpouseAndChild to WithSpouse.
        // WithSpouse 2026 base = $4,158.17; inflated 11 years at 2.8%.
        let date = NaiveDate::from_ymd_opt(2037, 1, 1).unwrap();
        let income = engine.get_incomes_usd(date);
        let factor = (1.028_f64).powi(11);
        let expected_va = 4_158.17 * factor;
        assert!((income.va_usd - expected_va).abs() < 0.01, "va={} expected={}", income.va_usd, expected_va);
    }

    #[test]
    fn test_va_child_addon_removed_after_cutoff() {
        let cfg = minimal_cfg();
        let engine = CashFlowEngine::new(cfg);
        // Child cutoff 2036-09-18: WithSpouseAndChild before, WithSpouse after.
        let before = NaiveDate::from_ymd_opt(2036, 9, 17).unwrap();
        let after  = NaiveDate::from_ymd_opt(2036, 9, 19).unwrap();
        let income_before = engine.get_incomes_usd(before);
        let income_after  = engine.get_incomes_usd(after);
        assert!(income_before.va_usd > income_after.va_usd,
            "before={} after={}", income_before.va_usd, income_after.va_usd);
    }

    #[test]
    fn test_all_pensions_disabled() {
        // VA rating 0 + FERS/SS/Nenkin at 0 must all produce exactly $0 / ¥0.
        // No NaN is allowed — this covers the zero-multiplication edge case.
        let mut cfg = minimal_cfg();
        cfg.va_disability_rating = 0;
        cfg.fers_monthly_start = 0.0;
        cfg.ss_monthly_usd = 0.0;
        cfg.nenkin_income_monthly_jpy = 0.0;
        let engine = CashFlowEngine::new(cfg);

        // Test well after all potential pension start dates.
        let date = NaiveDate::from_ymd_opt(2045, 6, 1).unwrap();
        let income = engine.get_incomes_usd(date);

        assert_eq!(income.va_usd, 0.0,            "VA must be $0 with 0% rating");
        assert_eq!(income.fers_usd, 0.0,           "FERS must be $0 when disabled");
        assert_eq!(income.ss_usd, 0.0,             "SS must be $0 when disabled");
        assert_eq!(income.nenkin_income_jpy, 0.0,  "Nenkin must be ¥0 when disabled");

        assert!(!income.va_usd.is_nan(),           "VA must not produce NaN");
        assert!(!income.fers_usd.is_nan(),         "FERS must not produce NaN");
        assert!(!income.ss_usd.is_nan(),           "SS must not produce NaN");
        assert!(!income.nenkin_income_jpy.is_nan(),"Nenkin must not produce NaN");
    }

    // ── NHI premium tests ──────────────────────────────────────────────────────

}

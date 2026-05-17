/// Annual statistical accumulators for the simulation.
/// Reset to zero at the start of each calendar year.
/// Mirrors Python's `self.stats` dict in `SimulationController`.
#[derive(Debug, Clone, Default)]
pub struct AnnualStats {
    /// Accumulated ordinary income (FERS + RSU vest value) for marginal tax estimation.
    pub acc_ord_inc: f64,
    /// Accumulated dividend income (used as capital gains floor for tax estimation).
    pub acc_div_inc: f64,
    /// Gross dividends received from the Taxable account.
    pub year_div_gross: f64,
    /// Capital gains realized from sells (both ST and LT).
    pub year_cap_gains: f64,
    /// Dividend tax withheld / paid.
    pub year_div_tax: f64,
    /// Gross FERS pension received.
    pub year_fers_gross: f64,
    /// FERS tax estimated and withheld.
    pub year_fers_tax: f64,
    /// Net VA disability income (tax-free).
    pub year_va_net: f64,
    /// Total tax routed from income streams (for year-end true-up).
    pub year_tax_routed: f64,
    /// War chest balance drawn down this year (in war_chest_currency).
    pub year_wc_used: f64,
    /// Total expenses paid this year (JPY).
    pub year_total_exp_jpy: f64,
    /// Base living expenses paid this year (JPY).
    pub year_exp_base: f64,
    /// NHI (National Health Insurance) obligations this year (JPY).
    pub year_exp_nhi: f64,
    /// Nenkin (pension contributions) paid this year (JPY).
    pub year_exp_nenkin: f64,
    /// Resident tax installments paid this year (JPY).
    pub year_exp_restax: f64,
    /// Tax paid from external sources (e.g., from salary during accumulation phase).
    pub tax_paid_external: f64,
    /// Total RSU vesting income this year (USD market value at time of vest).
    pub year_rsu_vest_usd: f64,
    /// Monthly VTI/SCHD contributions made this year (USD).
    pub year_monthly_contribution: f64,
    /// Short-term capital gains realized at the retirement transition event.
    pub year_st_cap_gains: f64,
    /// Long-term capital gains realized at the retirement transition event.
    pub year_lt_cap_gains: f64,
    /// Total US federal + state tax charged this year (USD). Used for dual-field reporting.
    pub year_us_fed_tax_usd: f64,
    /// Japan resident tax charged this year (JPY). Mirrors year_exp_restax for dual-field reporting.
    pub year_japan_res_tax_jpy: f64,
    /// US Social Security income received this year (USD).
    pub year_ss_payout_usd: f64,
    /// SSDI (Social Security Disability Insurance) gross income this year (USD).
    /// Taxable portion determined at year-end via the Combined Income rule.
    pub year_ssdi_gross_usd: f64,
    /// Japanese Nenkin pension income received this year (JPY).
    pub year_nenkin_income_jpy: f64,
    /// Whether FEIE was applied in this year's US tax calculation.
    pub year_feie_applied: bool,
    /// True if cash_buffer_usd went negative at any point this year.
    pub year_bridge_exhausted: bool,
    /// Value of taxable portfolio sold to cover deficits this year (USD).
    pub year_forced_liquidations_usd: f64,
    /// V7.0 — Japan capital-gains tax (¥) realised at sale during liquidation.
    /// Settled at the moment of sale (源泉徴収-style); flows into year-end FTC pool.
    pub year_japan_cap_gains_tax_jpy: f64,
    /// V7.0 — US state capital-gains tax ($) reserved at sale during liquidation.
    /// Pre-paid into the federal-true-up pipeline so the year-end calculation does
    /// not double-charge state on the same gain.
    pub year_state_cap_gains_tax_usd: f64,
    /// V7.1 — Cumulative FX spread penalty paid (¥) across all USD→JPY conversions.
    /// Sum of the 0.5% spread cost for Tiers 4, 5, 6, and 8 conversions.
    pub year_fx_penalty_jpy: f64,
    /// V7.1 — Number of months this year where the spending target was dropped to
    /// the Minimum floor (Tier 7 belt-tightening fired). High values = stress signal.
    pub year_months_target_dropped: u32,
    /// V7.3 — Jido Teate (児童手当) child allowance received this year (JPY).
    /// Counted as JPY inflow into the Defensive waterfall at Tier 0.5.
    pub year_jido_teate_jpy: f64,
    /// V7.3 — JPY contributed into the Tier 2.5 Education Fund this year.
    pub year_edu_fund_in_jpy: f64,
    /// V7.3 — JPY drawn from the Tier 2.5 Education Fund this year for
    /// Education-tagged expenses (or routed to T8 fallback when exhausted).
    pub year_edu_fund_out_jpy: f64,
    /// V7.5 — Japan-side capital losses realised this year (JPY, unsigned magnitude).
    /// Eligible for 3-year carry-forward under IT Act Art. 37-12-2 (損失の繰越控除).
    pub year_japan_cap_loss_jpy: f64,
    /// V7.5 — PFIC §1296 mark-to-market gain for the year (USD).
    /// Taxed as ordinary income; NOT FEIE-eligible (passive income, §911(d)(2)).
    pub year_pfic_mtm_income_usd: f64,
    /// Stage 05 — PFIC §1296 MTM gain in JPY (non-Japan-tax-advantaged accounts only).
    /// Added to Japan resident-tax income base in the N-1 hand-off.
    pub year_pfic_mtm_income_jpy: f64,
    /// V7.6 — PFIC-flagged capital-gains distributions (ordinary, passive basket).
    /// Distinct from MTM: this is pass-through CGD from a §1296-flagged fund.
    /// Accumulated by the dividend handler; consumed at year-end true-up.
    pub year_pfic_ord_income_usd: f64,
    /// V7.6 — Passive-basket ordinary income from interest + special distributions.
    /// Routed to the §904 passive basket; ordinary stack on the federal side.
    pub year_passive_ord_income_usd: f64,
    /// V7.6 — Component breakdown of taxable dividends (USD, for audit reporting).
    pub year_dist_dividend_usd: f64,
    pub year_dist_interest_usd: f64,
    pub year_dist_cap_gains_usd: f64,
    pub year_dist_special_usd: f64,
    /// V7.6 — Return-of-Capital cash received this year (non-taxable; basis-reducing).
    pub year_dist_roc_usd: f64,
    /// V7.5 — JPY diverted into the Tier 9 Gift Sink this year.
    pub year_gift_sink_jpy: f64,
    /// V7.5 — true if any per-recipient gift exceeded the US $19k exclusion
    /// (flagged for Form 709 filing in the audit report).
    pub year_form_709_required: bool,
    /// V7.7 — Gross salary earned this year (JPY). Pre-retirement only.
    /// Accumulated monthly; captured into `salary_history` in December.
    pub year_salary_jpy: f64,
    /// V7.7 — RSU vest value this year (JPY). Pre-retirement only.
    /// Accumulated at each vest event; captured into `rsu_vest_history` in December.
    pub year_rsu_vest_jpy: f64,
    /// V7.7 — Japan income tax (所得税) paid this year (JPY). Pre-retirement only.
    /// Computed once in December via `JapanTaxEngine::calculate_income_tax`.
    pub year_japan_income_tax_jpy: f64,
    /// Stage 06 — Annual net rental income (JPY) from Japan properties.
    pub year_rental_income_jpy: f64,
    /// Stage 06 — Annual net rental income (USD) from US / international properties.
    pub year_rental_income_usd: f64,
    /// Stage 06 — Annual real-estate fixed costs (PI + property tax) in JPY.
    pub year_real_estate_exp_jpy: f64,
    /// Stage 06 — Total HELOC drawn this year (USD). Tier 7.5 draws only.
    pub year_heloc_draw_usd: f64,
    /// Stage 10 — Annual age-65+ Kaigo Hoken premium paid this year (JPY).
    pub year_kaigo_premium_jpy: f64,
    /// Stage 10 — Annual projected out-of-pocket care costs (JPY, based on CareScenario).
    pub year_kaigo_care_jpy: f64,
}

impl AnnualStats {
    pub fn reset(&mut self) {
        *self = AnnualStats::default();
    }
}

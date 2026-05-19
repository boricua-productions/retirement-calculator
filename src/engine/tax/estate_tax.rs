/// Stage 07 — Japan Sōzoku-zei (相続税) and US Estate Tax engine.
///
/// Computes end-of-life wealth-transfer tax liability for a US citizen who is a
/// long-term resident of Japan.  Both countries tax the **global** estate; the
/// US-Japan Estate Tax Treaty (2004) provides a pro-rata credit to avoid double
/// taxation, but the coordination is imperfect — this module models the full
/// bilateral bill.
use chrono::{Datelike, NaiveDate};

use crate::models::config::{Config, HeirRelationship};

// ─── Japan Sōzoku-zei bracket table ─────────────────────────────────────────

/// Per-heir taxable-share amount brackets per NTA (国税庁) published schedule.
/// Each entry is (upper_limit_jpy, marginal_rate, flat_deduction_jpy).
/// The final entry has f64::INFINITY as the upper limit.
const JAPAN_ESTATE_BRACKETS: &[(f64, f64, f64)] = &[
    (10_000_000.0,  0.10,          0.0),
    (30_000_000.0,  0.15,    500_000.0),
    (50_000_000.0,  0.20,  2_000_000.0),
    (100_000_000.0, 0.30,  7_000_000.0),
    (200_000_000.0, 0.40, 17_000_000.0),
    (300_000_000.0, 0.45, 27_000_000.0),
    (600_000_000.0, 0.50, 42_000_000.0),
    (f64::INFINITY, 0.55, 72_000_000.0),
];

/// Apply the Japan Sōzoku-zei bracket table to a single heir's share amount.
/// Returns the tax owed by that heir before any spouse-deduction or other adjustments.
fn japan_bracket_tax(share_amount: f64) -> f64 {
    if share_amount <= 0.0 { return 0.0; }
    for &(limit, rate, deduction) in JAPAN_ESTATE_BRACKETS {
        if share_amount <= limit {
            return (share_amount * rate - deduction).max(0.0);
        }
    }
    // Fallback: top bracket (should be unreachable due to INFINITY sentinel)
    (share_amount * 0.55 - 72_000_000.0).max(0.0)
}

/// Compute the **total** Japan Sōzoku-zei (相続税) bill.
///
/// # Arguments
/// * `total_assets_jpy` — Gross estate value in JPY (before exclusions).
/// * `heir_count`       — Number of statutory heirs (including spouse, children).
/// * `heir_shares`      — Per-heir fractional shares of the taxable estate (must sum ≤ 1.0).
///   If empty, heirs share equally.
///
/// # Method (mirrors NTA calculation sheet)
/// 1. Basic exclusion = ¥30M + ¥6M × heir_count.
/// 2. Taxable estate = max(0, total_assets − exclusion).
/// 3. Each heir's notional share = taxable_estate × their_fraction.
/// 4. Apply bracket tax to each notional share → sum → **total tentative tax**.
///
/// This function does **not** apply the spousal 1/2 deduction (配偶者の税額軽減) because
/// the actual deduction depends on what the spouse actually inherits.  The caller may
/// reduce the result by 50 % when the spouse is the sole or primary heir.
pub fn compute_japan_sozoku_zei(
    total_assets_jpy: f64,
    heir_count: u32,
    heir_shares: &[f64],
) -> f64 {
    let n = heir_count.max(1);
    let basic_exclusion = 30_000_000.0 + 6_000_000.0 * n as f64;
    let taxable = (total_assets_jpy - basic_exclusion).max(0.0);
    if taxable <= 0.0 { return 0.0; }

    // Build equal shares if none provided.
    let equal_share = 1.0 / n as f64;
    let total: f64 = if heir_shares.is_empty() {
        (0..n).map(|_| japan_bracket_tax(taxable * equal_share)).sum()
    } else {
        heir_shares.iter().map(|&s| japan_bracket_tax(taxable * s)).sum()
    };
    total.max(0.0)
}

// ─── US Estate Tax ───────────────────────────────────────────────────────────

/// Federal estate-tax exclusion amounts (USD), inflation-adjusted at 2.8 % per year.
/// TCJA sunsets on 2026-01-01; the exclusion roughly halves.
fn us_estate_exclusion(year: i32) -> f64 {
    if year < 2026 {
        // TCJA era: ~$13.61M in 2024, indexed at ~2.8 % per year.
        13_610_000.0 * (1.028_f64).powi((year - 2024).max(0))
    } else {
        // V8.0 — 2026 Unified Credit Guidelines (OBBBA permanent extension).
        15_000_000.0 * (1.028_f64).powi((year - 2026).max(0))
    }
}

/// Compute the US federal estate tax bill.
///
/// * Estates below the exclusion owe $0.
/// * The marginal rate above the exclusion is a flat 40 %.
/// * State estate taxes are NOT modeled (varies by domicile; consult counsel).
pub fn compute_us_estate_tax(total_assets_usd: f64, year: i32) -> f64 {
    let exclusion = us_estate_exclusion(year);
    let taxable = (total_assets_usd - exclusion).max(0.0);
    taxable * 0.40
}

// ─── US-Japan Treaty credit ──────────────────────────────────────────────────

/// Compute the US-Japan Estate Tax Treaty credit (Article 6 pro-rata mechanism).
///
/// The treaty allows the US to credit Japan inheritance tax paid on property that is
/// also included in the US taxable estate, in proportion to Japanese-situs assets.
///
/// For a US citizen who has lived in Japan and holds primarily Japan-situs assets,
/// the credit approaches `min(japan_paid, us_paid)` — this function implements that
/// ceiling.  When the estate has significant US-situs assets (e.g., Roth IRA,
/// brokerage held with a US custodian), apply `japan_situs_fraction < 1.0`.
///
/// # Arguments
/// * `japan_paid`          — Japan Sōzoku-zei already computed.
/// * `us_paid`             — US estate tax already computed (before credit).
/// * `japan_situs_fraction` — Fraction of the total estate that is Japan-situs (0.0–1.0).
///   Use 1.0 as a conservative default for long-term Japan residents.
pub fn compute_treaty_credit(
    japan_paid: f64,
    us_paid: f64,
    japan_situs_fraction: f64,
) -> f64 {
    let max_credit = japan_paid * japan_situs_fraction.clamp(0.0, 1.0);
    max_credit.min(us_paid).max(0.0)
}

// ─── Gifting optimiser ───────────────────────────────────────────────────────

/// Suggested annual-gifting amounts and estimated estate-tax benefit.
#[derive(Debug, Clone)]
pub struct GiftingSuggestion {
    /// Annual JPY gift amount across all recipients (¥1.1M × recipient_count).
    pub suggested_annual_jpy: f64,
    /// Annual USD gift amount across all recipients ($19k × recipient_count).
    pub suggested_annual_usd: f64,
    /// Estimated years remaining until `death_date` (or end_date).
    pub projected_years_remaining: u32,
    /// Total JPY removed from the estate via annual gifting over remaining years.
    pub estimated_estate_reduction_jpy: f64,
    /// Estimated reduction in Japan Sōzoku-zei from the gifted assets.
    pub estimated_tax_reduction_jpy: f64,
}

/// Conservative lifetime-gifting optimiser.
///
/// Suggests how much to pre-gift each year using the Japan 暦年贈与 (¥1.1M/recipient)
/// and US §2503(b) ($19k/recipient) annual exclusions, and estimates the resulting
/// reduction in Japan Sōzoku-zei.
///
/// Labeled "rough guidance, not legal advice" — actual tax savings depend on the
/// order of inheritance, asset appreciation, and recipient-specific deductions.
pub fn lifetime_gifting_optimiser(
    cfg: &Config,
    projected_estate_jpy: f64,
    as_of_date: NaiveDate,
) -> GiftingSuggestion {
    let death_date = cfg.death_date.unwrap_or(cfg.end_date);
    let years_remaining = ((death_date.year() - as_of_date.year()).max(0)) as u32;

    let recipient_count = cfg.gift_recipient_count.max(1) as f64;
    let suggested_annual_jpy = 1_100_000.0 * recipient_count;
    let suggested_annual_usd = cfg.us_gift_exclusion_usd * recipient_count;

    let total_reduction = suggested_annual_jpy * years_remaining as f64;
    let heir_count = cfg.heirs.len().max(1) as u32;

    // Build equal heir shares.
    let heir_shares: Vec<f64> = vec![1.0 / heir_count as f64; heir_count as usize];

    let tax_before = compute_japan_sozoku_zei(projected_estate_jpy, heir_count, &heir_shares);
    let reduced_estate = (projected_estate_jpy - total_reduction).max(0.0);
    let tax_after  = compute_japan_sozoku_zei(reduced_estate, heir_count, &heir_shares);
    let tax_reduction = (tax_before - tax_after).max(0.0);

    GiftingSuggestion {
        suggested_annual_jpy,
        suggested_annual_usd,
        projected_years_remaining: years_remaining,
        estimated_estate_reduction_jpy: total_reduction,
        estimated_tax_reduction_jpy: tax_reduction,
    }
}

// ─── EstatePlanningEngine ────────────────────────────────────────────────────

/// Estate tax calculation context containing final simulation state.
#[derive(Debug, Clone)]
pub struct EstateTaxContext {
    /// Final taxable portfolio value (USD).
    pub brokerage_usd: f64,
    /// Final Roth IRA value (USD).
    pub roth_usd: f64,
    /// Final Japan DC/NISA/iDeCo value (JPY).
    pub dc_jpy: f64,
    /// Final real-estate equity (JPY).
    pub real_estate_equity_jpy: f64,
    /// Final real-estate equity (USD).
    pub real_estate_equity_usd: f64,
    /// Final USD/JPY exchange rate.
    pub final_fx: f64,
    /// Calendar year of death event.
    pub death_year: i32,
}

/// Full estate-planning projection at end-of-horizon (or death_date).
pub struct EstatePlanningEngine;

impl EstatePlanningEngine {
    /// Compute the estate-tax projection from the last simulation state.
    ///
    /// # Arguments
    /// * `context` — Estate tax calculation context containing asset values and exchange rate.
    /// * `cfg`     — Simulation configuration (heir structure, jurisdiction settings).
    pub fn project_at_death(
        context: &EstateTaxContext,
        cfg: &Config,
    ) -> crate::models::snapshot::EstateSummary {
        use crate::models::snapshot::EstateSummary;

        // ── Total estate in both currencies ───────────────────────────────────
        let us_assets_usd = context.brokerage_usd + context.roth_usd + context.real_estate_equity_usd;
        let japan_assets_jpy = context.dc_jpy + context.real_estate_equity_jpy;
        let total_usd = us_assets_usd + japan_assets_jpy / context.final_fx.max(1.0);
        let total_jpy = total_usd * context.final_fx;

        // ── Heirs & shares ────────────────────────────────────────────────────
        let heir_count = cfg.heirs.len().max(1) as u32;
        let has_spouse = cfg.heirs.iter().any(|h| h.relationship == HeirRelationship::Spouse);

        // Build statutory heir shares: spouse ½, remaining children split equally.
        let heir_shares: Vec<f64> = if has_spouse {
            let spouse_share = 0.5_f64;
            let child_count = cfg.heirs.iter()
                .filter(|h| h.relationship != HeirRelationship::Spouse)
                .count()
                .max(1) as f64;
            cfg.heirs.iter().map(|h| {
                if h.relationship == HeirRelationship::Spouse {
                    spouse_share
                } else {
                    (1.0 - spouse_share) / child_count
                }
            }).collect()
        } else {
            vec![1.0 / heir_count as f64; heir_count as usize]
        };

        // ── Japan Sōzoku-zei ──────────────────────────────────────────────────
        // V8.0 — Article 19-2 (配偶者の税額軽減) full implementation.
        // Spouse's tax exemption credit = tax due on max(statutory share, ¥160M).
        //
        // Algorithm:
        // 1. Compute total tentative inheritance tax across heirs.
        // 2. Identify spouse's actual share fraction.
        // 3. Compute the credit = tax on the larger of (statutory share value, ¥160M).
        // 4. Subtract credit from spouse's personal liability (cannot go below ¥0).
        let japan_tax_jpy = if has_spouse {
            // Recompute by walking each heir individually.
            let basic_exclusion = 30_000_000.0 + 6_000_000.0 * heir_count as f64;
            let taxable = (total_jpy - basic_exclusion).max(0.0);
            let mut total_after_credit = 0.0_f64;
            for (i, heir) in cfg.heirs.iter().enumerate() {
                let share = heir_shares[i];
                let share_value = taxable * share;
                let heir_tax = japan_bracket_tax(share_value);
                if heir.relationship == HeirRelationship::Spouse {
                    // Article 19-2 ceiling: tax on max(statutory_share_value, ¥160M).
                    let ceiling_value = share_value.max(160_000_000.0);
                    let credit_basis = japan_bracket_tax(ceiling_value.min(taxable));
                    let credit = credit_basis.min(heir_tax);
                    total_after_credit += (heir_tax - credit).max(0.0);
                } else {
                    total_after_credit += heir_tax;
                }
            }
            total_after_credit
        } else {
            compute_japan_sozoku_zei(total_jpy, heir_count, &heir_shares)
        };

        // ── US Estate Tax ─────────────────────────────────────────────────────
        let us_tax_usd = compute_us_estate_tax(total_usd, context.death_year);

        // ── Treaty credit ─────────────────────────────────────────────────────
        // Japan-situs fraction: JP DC/NISA + JP real estate / total estate
        let japan_situs_usd = japan_assets_jpy / context.final_fx.max(1.0);
        let japan_situs_fraction = if total_usd > 0.0 {
            (japan_situs_usd / total_usd).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let japan_tax_usd = japan_tax_jpy / context.final_fx.max(1.0);
        let treaty_credit_usd = compute_treaty_credit(japan_tax_usd, us_tax_usd, japan_situs_fraction);
        let net_us_tax_usd = (us_tax_usd - treaty_credit_usd).max(0.0);

        // ── Net wealth to heirs ───────────────────────────────────────────────
        let total_tax_usd = japan_tax_usd + net_us_tax_usd;
        let net_to_heirs_usd = (total_usd - total_tax_usd).max(0.0);
        let net_to_heirs_jpy = net_to_heirs_usd * context.final_fx;

        EstateSummary {
            year: context.death_year,
            total_estate_jpy: total_jpy,
            total_estate_usd: total_usd,
            japan_sozoku_zei_jpy: japan_tax_jpy,
            japan_sozoku_zei_pct: if total_jpy > 0.0 { japan_tax_jpy / total_jpy * 100.0 } else { 0.0 },
            us_estate_tax_usd:    us_tax_usd,
            us_estate_tax_pct:    if total_usd > 0.0 { us_tax_usd / total_usd * 100.0 } else { 0.0 },
            treaty_credit_usd,
            net_us_estate_tax_usd: net_us_tax_usd,
            net_to_heirs_jpy,
            net_to_heirs_usd,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// NTA bracket example: ¥200M estate, 3 heirs (spouse + 2 children)
    /// Exclusion = ¥30M + ¥6M × 3 = ¥48M. Taxable = ¥152M.
    /// Statutory heir shares: spouse ½ = ¥76M, each child ¼ = ¥38M.
    /// Spouse ¥76M → 30% bracket: ¥76M × 30% − ¥7M = ¥15.8M
    /// Each child ¥38M → 20% bracket: ¥38M × 20% − ¥2M = ¥5.6M
    /// Total tentative = ¥15.8M + 2 × ¥5.6M = ¥27.0M
    #[test]
    fn japan_estate_200m_3_heirs() {
        let total_jpy = 200_000_000.0_f64;
        let heir_shares = vec![0.5, 0.25, 0.25];
        let tax = compute_japan_sozoku_zei(total_jpy, 3, &heir_shares);
        // Tentative tax (before spousal ½ deduction): ¥27.0M
        let expected = 27_000_000.0;
        assert!((tax - expected).abs() < 10_000.0,
            "Expected ≈¥27.0M, got ¥{:.0}", tax);
    }

    /// NTA bracket smoke test: ¥10M heir share → 10% × ¥10M = ¥1M
    #[test]
    fn bracket_bottom() {
        let tax = japan_bracket_tax(10_000_000.0);
        assert!((tax - 1_000_000.0).abs() < 1.0);
    }

    /// NTA bracket smoke test: ¥600M → 55% − ¥72M = ¥258M
    #[test]
    fn bracket_top() {
        let tax = japan_bracket_tax(600_000_000.0);
        let expected = 600_000_000.0 * 0.55 - 72_000_000.0;
        assert!((tax - expected).abs() < 1.0, "got {:.0}", tax);
    }

    /// Zero estate → zero tax.
    #[test]
    fn japan_estate_zero() {
        assert_eq!(compute_japan_sozoku_zei(0.0, 2, &[]), 0.0);
    }

    /// Estate below exclusion → zero tax.
    #[test]
    fn japan_estate_below_exclusion() {
        // ¥42M estate, 2 heirs → exclusion = ¥30M + ¥12M = ¥42M → taxable = ¥0
        let tax = compute_japan_sozoku_zei(42_000_000.0, 2, &[]);
        assert_eq!(tax, 0.0);
    }

    /// US estate tax: $20M estate in 2026 (OBBBA permanent extension, $15M exclusion).
    #[test]
    fn us_estate_post_sunset() {
        let tax = compute_us_estate_tax(20_000_000.0, 2026);
        let expected = (20_000_000.0 - 15_000_000.0) * 0.40;
        assert!((tax - expected).abs() < 1.0);
    }

    /// US estate tax: $5M estate in 2026 → below $7M exclusion → $0.
    #[test]
    fn us_estate_below_exclusion() {
        assert_eq!(compute_us_estate_tax(5_000_000.0, 2026), 0.0);
    }

    /// US estate tax: $10M in 2024 (pre-sunset, $13.61M exclusion) → $0.
    #[test]
    fn us_estate_pre_sunset_below_exclusion() {
        assert_eq!(compute_us_estate_tax(10_000_000.0, 2024), 0.0);
    }

    /// Treaty credit caps at US tax paid.
    #[test]
    fn treaty_credit_capped_at_us_tax() {
        let credit = compute_treaty_credit(5_000_000.0, 3_000_000.0, 1.0);
        assert!((credit - 3_000_000.0).abs() < 1.0);
    }

    /// Treaty credit scales by Japan-situs fraction.
    #[test]
    fn treaty_credit_partial_situs() {
        let credit = compute_treaty_credit(4_000_000.0, 10_000_000.0, 0.5);
        assert!((credit - 2_000_000.0).abs() < 1.0);
    }

    /// V8.0 — Article 19-2: pure-spouse heir under ¥160M → spouse credit fully absorbs liability.
    #[test]
    fn article_19_2_pure_spouse_under_160m() {
        use crate::models::config::{Heir, HeirRelationship};
        let mut cfg = minimal_config_for_estate_test();
        cfg.heirs = vec![Heir {
            name: "Spouse".into(),
            birth_date: None,
            relationship: HeirRelationship::Spouse,
        }];

        // Estate = ¥150M → basic exclusion = ¥30M + ¥6M = ¥36M → taxable = ¥114M
        // Spouse share = 100% → ¥114M → bracket tax ≈ ¥31.7M
        // Article 19-2 credit ceiling = max(¥114M, ¥160M) = ¥160M → bracket tax ≈ ¥48.5M
        // Since credit (¥48.5M) > liability (¥31.7M), net = ¥0
        let context = EstateTaxContext {
            brokerage_usd: 150_000_000.0 / 150.0,
            roth_usd: 0.0,
            dc_jpy: 0.0,
            real_estate_equity_jpy: 0.0,
            real_estate_equity_usd: 0.0,
            final_fx: 150.0,
            death_year: 2026,
        };
        let summary = EstatePlanningEngine::project_at_death(&context, &cfg);
        assert!(summary.japan_sozoku_zei_jpy < 1.0,
            "Spouse-only estate under ¥160M should have zero tax after Article 19-2 credit, got ¥{:.0}",
            summary.japan_sozoku_zei_jpy);
    }

    /// V8.0 — Article 19-2: spouse + 1 child, estate ¥300M → verify spouse's liability reduced.
    #[test]
    fn article_19_2_spouse_and_child_300m() {
        use crate::models::config::{Heir, HeirRelationship};
        let mut cfg = minimal_config_for_estate_test();
        cfg.heirs = vec![
            Heir {
                name: "Spouse".into(),
                birth_date: None,
                relationship: HeirRelationship::Spouse,
            },
            Heir {
                name: "Child".into(),
                birth_date: None,
                relationship: HeirRelationship::Child,
            },
        ];

        // Estate = ¥300M → basic exclusion = ¥30M + ¥12M = ¥42M → taxable = ¥258M
        // Statutory shares: spouse ½ = ¥129M, child ½ = ¥129M
        // Without Article 19-2:
        //   Spouse ¥129M → bracket tax ≈ ¥38.9M
        //   Child ¥129M → bracket tax ≈ ¥38.9M
        //   Total = ¥77.8M
        // With Article 19-2:
        //   Spouse credit ceiling = max(¥129M, ¥160M) = ¥160M → bracket tax ≈ ¥48.5M
        //   Spouse net = max(¥38.9M - ¥48.5M, 0) = ¥0
        //   Child = ¥38.9M (unchanged)
        //   Total ≈ ¥38.9M
        let context = EstateTaxContext {
            brokerage_usd: 300_000_000.0 / 150.0,
            roth_usd: 0.0,
            dc_jpy: 0.0,
            real_estate_equity_jpy: 0.0,
            real_estate_equity_usd: 0.0,
            final_fx: 150.0,
            death_year: 2026,
        };
        let summary = EstatePlanningEngine::project_at_death(&context, &cfg);
        // Expect roughly half the pre-credit tax (child's liability only).
        // Actual: ¥34.6M (child's portion after spousal credit fully absorbs spouse's liability).
        let expected_approx = 34_600_000.0;
        assert!((summary.japan_sozoku_zei_jpy - expected_approx).abs() < 1_000_000.0,
            "Expected ≈¥34.6M (child only), got ¥{:.0}", summary.japan_sozoku_zei_jpy);
        // Verify it's less than the pre-credit total of ~¥70M
        assert!(summary.japan_sozoku_zei_jpy < 40_000_000.0,
            "Article 19-2 should reduce total tax significantly");
    }

    /// Minimal Config for estate tax unit tests — just proxies what's already
    /// in the tests/v7_10_estate_tax.rs integration test helper.
    fn minimal_config_for_estate_test() -> crate::models::config::Config {
        use crate::models::config::*;
        use chrono::NaiveDate;
        use std::collections::HashMap;
        let iso = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();

        Config {
            start_date: iso(2026, 1, 1),
            end_date: iso(2030, 12, 31),
            retirement_date: iso(2026, 1, 1),
            rebalance_date: iso(2026, 2, 1),
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
            base_expense_jpy: 300_000.0,
            min_expense_jpy: 200_000.0,
            nhi_spike_monthly_jpy: 0.0,
            nhi_model: NhiModel::default(),
            expenses_detailed_mode: false,
            expense_categories: vec![],
            min_expense_buffer_jpy: 0.0,
            min_expense_buffer_pct: 0.0,
            war_chest_enabled: false,
            war_chest_funding_timing: BufferFundingTiming::AtRetirement,
            war_chest_ramp_months: 24,
            war_chest_currency: "JPY".into(),
            war_chest_target_jpy: 0.0,
            war_chest_target_usd: 0.0,
            war_chest_cap_policy: crate::models::config::WarChestCapPolicy::Fixed,
            war_chest_cap_growth_pct: 0.0,
            war_chest_empty_date: None,
            bridge_fund_enabled: false,
            bridge_fund_funding_timing: BufferFundingTiming::AtRetirement,
            bridge_fund_ramp_months: 18,
            bridge_months_target: 0,
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
            fers_start_date: iso(2026, 1, 1),
            retirement_year_gross_income_jpy: 0.0,
            birth_date: iso(1960, 1, 1),
            spouse_birth_date: iso(1962, 1, 1),
            child_birth_date: iso(1990, 1, 1),
            va_child_cutoff_date: None,
            dc_payout_start_age: 99,
            dc_payout_method: "LUMP_SUM".into(),
            pre_funded_war_chest_jpy: 0.0,
            pre_funded_bridge_jpy: 0.0,
            pre_funded_bridge_usd: 0.0,
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
            tax_jurisdiction: TaxProtocol::Both,
            investment_location: InvestmentLocation::Us,
            us_tax_strategy: UsTaxStrategy::FtcOnly,
            va_disability_rating: 0,
            va_dependent_status: VaDependentStatus::VetOnly,
            va_monthly_override: None,
            smc_monthly_override: None,
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
            family_unit: FamilyUnit {
                user_birth_year: 1960,
                spouse_birth_year: None,
                dependents: vec![],
            },
            nenkin_income_monthly_jpy: 0.0,
            nenkin_income_start_age: 65,
            prefecture: "Tokyo".into(),
            city: "Chiyoda".into(),
            military_retired: None,
            fers_jurisdiction: TaxProtocol::Both,
            fers_japan_local_tax_exempt: false,
            ss_jurisdiction: TaxProtocol::Both,
            nenkin_jurisdiction: TaxProtocol::Both,
            va_smc_variant: None,
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
            tlh_active_months: vec![],
            tlh_min_loss_usd: 0.0,
            enable_education_savings: false,
            enable_gift_sink: false,
            rsu_sell_to_cover_realism: false,
            rsu_sell_to_cover_policy: RsuSellToCoverPolicy::Strict,
            spouse_profile: SpouseProfile::UsPerson,
            spouse_japan_salary_jpy: 0.0,
            spouse_japan_misc_income_jpy: 0.0,
            monthly_dependent_precision: true,
            shock_ordering: ShockOrdering::DepreciateThenReprice,
            track_pfic_basis_drift: false,
            real_estate: vec![],
            enable_heloc_tier: false,
            enable_estate_planning: true,
            death_date: None,
            spouse_death_date: None,
            heirs: vec![],
            model_active_phase_resident_tax: false,
            primary_taxpayer_visa: VisaType::default(),
            crypto_tax_enabled: true,
            enable_gifting_optimiser: false,
            estate_planning_jurisdiction: TaxProtocol::Both,
            mc_use_correlated_paths: false,
            mc_correlation_matrix: HashMap::new(),
            kaigo_hoken_enabled: true,
            kaigo_hoken_brackets: None,
            kaigo_care_scenario: crate::engine::tax::kaigo_hoken::CareScenario::None,
            prefer_liquidation_over_belt_tightening: false,
        }
    }
}

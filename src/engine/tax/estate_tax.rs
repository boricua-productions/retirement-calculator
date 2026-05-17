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
        // Post-sunset: ~$7M in 2026, indexed at 2.8 % per year thereafter.
        7_000_000.0 * (1.028_f64).powi((year - 2026).max(0))
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

/// Full estate-planning projection at end-of-horizon (or death_date).
pub struct EstatePlanningEngine;

impl EstatePlanningEngine {
    /// Compute the estate-tax projection from the last simulation state.
    ///
    /// # Arguments
    /// * `brokerage_usd`  — Final taxable portfolio value (USD).
    /// * `roth_usd`       — Final Roth IRA value (USD).
    /// * `dc_jpy`         — Final Japan DC/NISA/iDeCo value (JPY).
    /// * `real_estate_equity_jpy` — Final real-estate equity (JPY).
    /// * `real_estate_equity_usd` — Final real-estate equity (USD).
    /// * `final_fx`       — Final USD/JPY rate.
    /// * `death_year`     — Calendar year of the event.
    /// * `cfg`            — Simulation configuration.
    pub fn project_at_death(
        brokerage_usd: f64,
        roth_usd: f64,
        dc_jpy: f64,
        real_estate_equity_jpy: f64,
        real_estate_equity_usd: f64,
        final_fx: f64,
        death_year: i32,
        cfg: &Config,
    ) -> crate::models::snapshot::EstateSummary {
        use crate::models::snapshot::EstateSummary;

        // ── Total estate in both currencies ───────────────────────────────────
        let us_assets_usd = brokerage_usd + roth_usd + real_estate_equity_usd;
        let japan_assets_jpy = dc_jpy + real_estate_equity_jpy;
        let total_usd = us_assets_usd + japan_assets_jpy / final_fx.max(1.0);
        let total_jpy = total_usd * final_fx;

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
        let mut japan_tax_jpy = compute_japan_sozoku_zei(total_jpy, heir_count, &heir_shares);

        // Apply spousal 1/2 deduction (配偶者の税額軽減) when a spouse is present.
        if has_spouse {
            japan_tax_jpy *= 0.5;
        }

        // ── US Estate Tax ─────────────────────────────────────────────────────
        let us_tax_usd = compute_us_estate_tax(total_usd, death_year);

        // ── Treaty credit ─────────────────────────────────────────────────────
        // Japan-situs fraction: JP DC/NISA + JP real estate / total estate
        let japan_situs_usd = japan_assets_jpy / final_fx.max(1.0);
        let japan_situs_fraction = if total_usd > 0.0 {
            (japan_situs_usd / total_usd).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let japan_tax_usd = japan_tax_jpy / final_fx.max(1.0);
        let treaty_credit_usd = compute_treaty_credit(japan_tax_usd, us_tax_usd, japan_situs_fraction);
        let net_us_tax_usd = (us_tax_usd - treaty_credit_usd).max(0.0);

        // ── Net wealth to heirs ───────────────────────────────────────────────
        let total_tax_usd = japan_tax_usd + net_us_tax_usd;
        let net_to_heirs_usd = (total_usd - total_tax_usd).max(0.0);
        let net_to_heirs_jpy = net_to_heirs_usd * final_fx;

        EstateSummary {
            year: death_year,
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

    /// US estate tax: $20M estate in 2026 (post-sunset, $7M exclusion).
    #[test]
    fn us_estate_post_sunset() {
        let tax = compute_us_estate_tax(20_000_000.0, 2026);
        let expected = (20_000_000.0 - 7_000_000.0) * 0.40;
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
}

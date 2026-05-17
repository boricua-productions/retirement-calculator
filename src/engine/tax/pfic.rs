// US-Japan Treaty (Article 1(5) Savings Clause): The US retains the right to
// tax its citizens as if the Treaty did not exist. Japan-side tax-free
// status (NISA, iDeCo) does NOT shelter from §1296 MTM. However, the
// FTC pool does not grow from these assets — Japan collects no offsetting
// tax — so the PFIC drag is fully unhedged.

use crate::models::assets::{Asset, PficRegime};
use crate::models::snapshot::PficDriftWarning;

/// Dual-currency MTM gain result for a single PFIC-flagged asset.
pub struct MtmGainResult {
    pub usd: f64,
    pub jpy: f64,
}

/// Compute the annual §1296 MTM gain for a single PFIC-flagged asset.
///
/// Mutates `asset.pfic_mtm_loss_carryforward_usd`:
/// - Loss years: the absolute loss is added to the carry-forward; net gain = 0.
/// - Gain years: available carry-forward is drawn down first; net gain is the residual.
///
/// `fx` is the current USD/JPY rate, used to derive the JPY gain from the
/// JPY prior-year FMV field (`pfic_prior_year_fmv_per_share_jpy`).
///
/// Returns `Some(MtmGainResult)` when the asset is `Mtm`, `None` otherwise.
pub fn compute_annual_mtm_gain(asset: &mut Asset, fx: f64) -> Option<MtmGainResult> {
    if asset.pfic_regime != PficRegime::Mtm {
        return None;
    }
    let qty = asset.qty();
    if qty <= 0.0 {
        return Some(MtmGainResult { usd: 0.0, jpy: 0.0 });
    }

    let prior_usd = if asset.pfic_prior_year_fmv_per_share > 0.0 {
        asset.pfic_prior_year_fmv_per_share
    } else {
        // First-year MTM: treat cost basis per share as prior FMV.
        if qty > 0.0 { asset.basis() / qty } else { 0.0 }
    };

    let prior_jpy = if asset.pfic_prior_year_fmv_per_share_jpy > 0.0 {
        asset.pfic_prior_year_fmv_per_share_jpy
    } else {
        prior_usd * fx
    };

    let raw_gain_usd = (asset.price - prior_usd) * qty;
    let current_price_jpy = asset.price * fx;
    let raw_gain_jpy = (current_price_jpy - prior_jpy) * qty;

    let (net_usd, net_jpy) = if raw_gain_usd < 0.0 {
        // §1296(d): bank the loss as carry-forward; report zero to the US income stack.
        asset.pfic_mtm_loss_carryforward_usd += raw_gain_usd.abs();
        log::info!(
            "   [PFIC §1296] MTM loss of ${:.2} on {} absorbed by §1296(d) carry-forward (balance now: ${:.2}).",
            raw_gain_usd.abs(),
            asset.ticker,
            asset.pfic_mtm_loss_carryforward_usd,
        );
        (0.0, 0.0_f64.min(raw_gain_jpy))
    } else if asset.pfic_mtm_loss_carryforward_usd > 0.0 {
        // Gain year with carry-forward: apply carry-forward offset first.
        let offset_usd = asset.pfic_mtm_loss_carryforward_usd.min(raw_gain_usd);
        asset.pfic_mtm_loss_carryforward_usd -= offset_usd;
        let remaining_usd = raw_gain_usd - offset_usd;
        let offset_jpy = offset_usd * fx;
        let remaining_jpy = (raw_gain_jpy - offset_jpy).max(0.0);
        log::info!(
            "   [PFIC §1296] Carry-forward of ${:.2} applied on {} — net reportable gain: ${:.2}.",
            offset_usd,
            asset.ticker,
            remaining_usd,
        );
        (remaining_usd, remaining_jpy)
    } else {
        (raw_gain_usd, raw_gain_jpy.max(0.0))
    };

    Some(MtmGainResult { usd: net_usd.max(0.0), jpy: net_jpy.max(0.0) })
}

/// Aggregate §1296 MTM gains across all accounts.
///
/// For each Mtm-flagged asset:
/// 1. Cross-checks USD×FX vs JPY prior-FMV basis; if drift > 1% and `track_drift` is
///    true, emits a `PficDriftWarning` and self-heals by resetting the JPY basis.
/// 2. Calls `compute_annual_mtm_gain` (applies carry-forward).
/// 3. Advances both `pfic_prior_year_fmv_per_share` and `pfic_prior_year_fmv_per_share_jpy`
///    to the current price for next year's mark.
///
/// Returns `(total_mtm_usd, total_mtm_jpy_non_advantaged, drift_warnings)`:
/// - `total_mtm_usd`:               all PFIC MTM income for US §1296 reporting.
/// - `total_mtm_jpy_non_advantaged`: PFIC MTM income in JPY for accounts where
///   `japan_tax_advantaged == false` (feeds Japan resident-tax base).
/// - `drift_warnings`:               events where JPY basis required self-healing.
pub fn aggregate_pfic_mtm_income(
    accounts: &mut std::collections::HashMap<String, crate::models::assets::Account>,
    fx: f64,
    track_drift: bool,
    year: i32,
) -> (f64, f64, Vec<PficDriftWarning>) {
    let mut total_mtm_usd = 0.0_f64;
    let mut total_mtm_jpy_non_advantaged = 0.0_f64;
    let mut warnings: Vec<PficDriftWarning> = Vec::new();

    for account in accounts.values_mut() {
        let japan_exempt = account.japan_tax_advantaged;

        for asset in account.assets.values_mut() {
            if asset.pfic_regime != PficRegime::Mtm {
                continue;
            }

            // ── Drift cross-check ────────────────────────────────────────────
            if track_drift
                && asset.pfic_prior_year_fmv_per_share > 0.0
                && asset.pfic_prior_year_fmv_per_share_jpy > 0.0
                && fx > 0.0
            {
                let derived_jpy = asset.pfic_prior_year_fmv_per_share * fx;
                let stored_jpy  = asset.pfic_prior_year_fmv_per_share_jpy;
                let drift = (derived_jpy - stored_jpy).abs() / stored_jpy;
                if drift > 0.01 {
                    log::warn!(
                        "   [PFIC drift] {} basis drift {:.2}% — self-healing: JPY basis {:.0} → {:.0}.",
                        asset.ticker, drift * 100.0, stored_jpy, derived_jpy,
                    );
                    asset.pfic_prior_year_fmv_per_share_jpy = derived_jpy;
                    warnings.push(PficDriftWarning {
                        year,
                        ticker: asset.ticker.clone(),
                        drift_pct: drift * 100.0,
                    });
                }
            }

            // ── MTM gain computation (with carry-forward) ────────────────────
            if let Some(result) = compute_annual_mtm_gain(asset, fx) {
                total_mtm_usd += result.usd;
                if !japan_exempt {
                    total_mtm_jpy_non_advantaged += result.jpy;
                }
            }

            // ── Advance both prior-FMV fields for next year's mark ───────────
            asset.pfic_prior_year_fmv_per_share = asset.price;
            if fx > 0.0 {
                asset.pfic_prior_year_fmv_per_share_jpy = asset.price * fx;
            }
        }
    }

    (total_mtm_usd, total_mtm_jpy_non_advantaged, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::assets::{Asset, AssetCategory, AssetClass, AssetLot, DividendCurrency, PficRegime};
    use chrono::NaiveDate;
    use std::collections::HashMap;

    fn jan1(y: i32) -> NaiveDate { NaiveDate::from_ymd_opt(y, 1, 1).unwrap() }

    fn pfic_asset(price: f64) -> Asset {
        let mut a = Asset::new("JPNFUND", price, 0.0, 0.06);
        a.pfic_regime = PficRegime::Mtm;
        a.add_lot(jan1(2025), 100.0, price * 100.0);
        a
    }

    #[test]
    fn test_carry_forward_suppresses_gain() {
        let mut asset = pfic_asset(100.0);
        // Year 1: price drops from ¥100 → ¥90 → loss = -1,000 USD → carry-forward = 1,000
        asset.pfic_prior_year_fmv_per_share = 100.0;
        asset.price = 90.0;
        let r1 = compute_annual_mtm_gain(&mut asset, 150.0).unwrap();
        assert_eq!(r1.usd, 0.0, "loss year should report 0");
        assert!((asset.pfic_mtm_loss_carryforward_usd - 1_000.0).abs() < 0.01);

        // Year 2: price rises from ¥90 → ¥100 → gross gain = +1,000; carry-forward absorbs all
        asset.pfic_prior_year_fmv_per_share = 90.0;
        asset.price = 100.0;
        let r2 = compute_annual_mtm_gain(&mut asset, 150.0).unwrap();
        assert_eq!(r2.usd, 0.0, "carry-forward should fully absorb equal gain");
        assert!(asset.pfic_mtm_loss_carryforward_usd < 0.01);

        // Year 3: price rises from ¥100 → ¥110 → carry-forward exhausted → net = 1,000
        asset.pfic_prior_year_fmv_per_share = 100.0;
        asset.price = 110.0;
        let r3 = compute_annual_mtm_gain(&mut asset, 150.0).unwrap();
        assert!((r3.usd - 1_000.0).abs() < 0.01, "year 3 gain should be net 1,000");
    }

    #[test]
    fn test_drift_check_emits_warning_and_self_heals() {
        let mut asset = pfic_asset(100.0);
        asset.pfic_prior_year_fmv_per_share     = 100.0;
        asset.pfic_prior_year_fmv_per_share_jpy = 10_000.0; // implies 100 JPY/USD
        asset.price = 105.0;

        let mut accounts: HashMap<String, crate::models::assets::Account> = HashMap::new();
        let mut acct = crate::models::assets::Account::new_with_meta(
            "Taxable", crate::models::assets::Currency::Usd,
            crate::models::assets::AccountLocation::Us,
            crate::models::assets::AccountJurisdiction::Both,
        );
        acct.assets.insert("JPNFUND".into(), asset);
        accounts.insert("Taxable".into(), acct);

        // FX = 200 → derived_jpy = 100 * 200 = 20,000; stored_jpy = 10,000 → drift = 100% > 1%
        let (_usd, _jpy, warnings) = aggregate_pfic_mtm_income(&mut accounts, 200.0, true, 2026);
        assert_eq!(warnings.len(), 1, "drift > 1% should emit one warning");
        assert_eq!(warnings[0].ticker, "JPNFUND");

        // After self-healing, JPY basis should be re-derived
        let healed_jpy = accounts["Taxable"].assets["JPNFUND"].pfic_prior_year_fmv_per_share_jpy;
        // After the mark-advance step: pfic_prior_year_fmv_per_share_jpy = 105.0 * 200.0 = 21,000
        assert!((healed_jpy - 21_000.0).abs() < 1.0, "JPY basis should advance to current price × fx");
    }

    #[test]
    fn test_ftc_passive_basket_carry_forward_suppresses_income() {
        // Validates that the carry-forward reduces passive-basket income so the
        // us_tax engine sees zero PFIC MTM in a loss year followed by a partial gain.
        let mut asset = pfic_asset(100.0);
        asset.pfic_prior_year_fmv_per_share = 100.0;

        // Loss year: price falls to 80 → loss = 2,000 → carry-forward = 2,000
        asset.price = 80.0;
        let r_loss = compute_annual_mtm_gain(&mut asset, 150.0).unwrap();
        assert_eq!(r_loss.usd, 0.0);
        assert!((asset.pfic_mtm_loss_carryforward_usd - 2_000.0).abs() < 0.01);

        // Gain year: price rises to 90 → gross gain = 1,000 → carry-forward absorbs → 0 net
        asset.pfic_prior_year_fmv_per_share = 80.0;
        asset.price = 90.0;
        let r_gain = compute_annual_mtm_gain(&mut asset, 150.0).unwrap();
        assert_eq!(r_gain.usd, 0.0, "partial gain should be fully absorbed");
        assert!((asset.pfic_mtm_loss_carryforward_usd - 1_000.0).abs() < 0.01);
    }
}

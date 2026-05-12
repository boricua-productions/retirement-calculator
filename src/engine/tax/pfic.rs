// US-Japan Treaty (Article 1(5) Savings Clause): The US retains the right to
// tax its citizens as if the Treaty did not exist. Japan-side tax-free
// status (NISA, iDeCo) does NOT shelter from §1296 MTM. However, the
// FTC pool does not grow from these assets — Japan collects no offsetting
// tax — so the PFIC drag is fully unhedged.

use crate::models::assets::{Asset, PficRegime};

/// Compute the annual §1296 MTM gain for a single asset.
///
/// Returns `Some(gain_usd)` when the asset is flagged `Mtm`, `None` otherwise.
/// MTM losses are limited to prior MTM-included income under §1296(d). For a
/// long-only retail portfolio this is effectively zero in early years; losses
/// are floored at 0 for V7.5 and a warning is emitted so the user knows that
/// §1296(d) carry-forward is not modelled.
pub fn compute_annual_mtm_gain_usd(asset: &Asset) -> Option<f64> {
    if asset.pfic_regime != PficRegime::Mtm {
        return None;
    }
    let qty = asset.qty();
    if qty <= 0.0 {
        return Some(0.0);
    }
    let prior = if asset.pfic_prior_year_fmv_per_share > 0.0 {
        asset.pfic_prior_year_fmv_per_share
    } else {
        // First-year MTM: treat cost basis as prior FMV.
        if qty > 0.0 { asset.basis() / qty } else { 0.0 }
    };
    let delta_per_share = asset.price - prior;
    let gain = delta_per_share * qty;
    // §1296(d) V7.5 simplification: floor MTM losses at zero.
    if gain < 0.0 {
        log::warn!(
            "   [PFIC §1296] MTM loss of ${:.2} on {} discarded — §1296(d) carry-forward not modelled in V7.5.",
            gain.abs(),
            asset.ticker,
        );
        return Some(0.0);
    }
    Some(gain)
}

/// Aggregate §1296 MTM gains across all accounts, update `pfic_prior_year_fmv_per_share`
/// for next year, and return the total USD ordinary income to add to `gross_ord`.
pub fn aggregate_pfic_mtm_income(accounts: &mut std::collections::HashMap<String, crate::models::assets::Account>) -> f64 {
    let mut total_mtm_usd = 0.0_f64;
    for account in accounts.values_mut() {
        for asset in account.assets.values_mut() {
            if let Some(gain) = compute_annual_mtm_gain_usd(asset) {
                total_mtm_usd += gain;
                // Advance the prior-year FMV for the next annual mark.
                asset.pfic_prior_year_fmv_per_share = asset.price;
            }
        }
    }
    total_mtm_usd
}

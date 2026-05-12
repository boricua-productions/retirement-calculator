//! V7.5 — Tax-Loss Harvesting (IRC §1091 wash-sale aware).
//!
//! Fires in months listed in `cfg.tlh_active_months` (typically [11, 12]).
//! For each Taxable asset with at least one lot at a loss, records the
//! harvestable USD loss for current-year capital-loss offset and accumulates
//! the corresponding JPY loss into `state.japan_loss_carryforward_jpy` under
//! IT Act Art. 37-12-2 (3-year carry-forward).
//!
//! Wash-sale detection: if a replacement lot was acquired within 30 calendar
//! days before or after the sale, the loss is disallowed and the disallowed
//! amount is added back to the replacement lot's basis (§1091(d)).

use chrono::{Datelike, NaiveDate};
use log::{info, warn};

use crate::models::config::Config;
use crate::simulation::state::SimState;

/// Entry point: called from `process_month` before `handle_dividends`.
/// Only fires post-retirement and when TLH is enabled for this month.
pub fn harvest_losses(state: &mut SimState, cfg: &Config) {
    if !cfg.tlh_enabled { return; }
    let mo = state.date.month();
    if !cfg.tlh_active_months.contains(&mo) { return; }

    let fx = state.current_fx;
    let current_date = state.date;

    // Collect tickers with harvestable losses to avoid borrow conflicts.
    let tickers: Vec<String> = state.accounts.get("Taxable")
        .map(|a| a.assets.keys().cloned().collect())
        .unwrap_or_default();

    for ticker in tickers {
        let (price, lots) = match state.accounts.get("Taxable").and_then(|a| a.assets.get(&ticker)) {
            Some(asset) if asset.price > 0.0 => (asset.price, asset.lots.clone()),
            _ => continue,
        };

        for lot in &lots {
            if lot.qty <= 0.0 { continue; }

            let lot_basis_per_share = lot.basis / lot.qty;
            let usd_gain_per_share  = price - lot_basis_per_share;
            let usd_loss            = usd_gain_per_share * lot.qty;

            // Only process loss lots.
            if usd_loss >= 0.0 { continue; }

            let loss_usd = usd_loss.abs();
            if loss_usd < cfg.tlh_min_loss_usd { continue; }

            // §1091 wash-sale check: is this lot still tainted?
            if let Some(clean_date) = lot.wash_sale_clean_after {
                if current_date < clean_date {
                    warn!("   [TLH] {} lot {:?} — wash-sale tainted until {}; skipping.",
                        ticker, lot.purchase_date, clean_date);
                    continue;
                }
            }

            // Check for a recent replacement purchase within 30 days.
            let wash_window_start = current_date - chrono::Days::new(30);
            let wash_window_end   = current_date + chrono::Days::new(30);
            let replacement_in_window = lots.iter().any(|other| {
                other.purchase_date != lot.purchase_date
                    && other.purchase_date >= wash_window_start
                    && other.purchase_date <= wash_window_end
                    && other.qty > 0.0
            });

            if replacement_in_window {
                warn!("   [TLH] {} — wash-sale: replacement acquired within 30-day window; loss disallowed.",
                    ticker);
                // Mark the current lot as tainted (basis adjustment deferred; tracked on lot).
                if let Some(acct) = state.accounts.get_mut("Taxable") {
                    if let Some(asset) = acct.assets.get_mut(&ticker) {
                        for l in asset.lots.iter_mut() {
                            if l.purchase_date == lot.purchase_date {
                                l.disallowed_loss_usd += loss_usd;
                                l.wash_sale_clean_after = Some(wash_window_end);
                            }
                        }
                    }
                }
                continue;
            }

            // Recognized loss — record for US and Japan carry-forward.
            let jpy_basis_sold = lot.basis * fx;
            let jpy_proceeds   = lot.qty * price * fx;
            let jpy_loss       = (jpy_basis_sold - jpy_proceeds).max(0.0);

            state.japan_loss_carryforward_jpy += jpy_loss;
            // US capital loss offsets current-year gains (net into year_cap_gains as negative).
            state.stats.year_cap_gains -= loss_usd;
            state.stats.year_japan_cap_loss_jpy += jpy_loss;

            info!("   [TLH] Harvested ${:.2} USD loss / ¥{:.0} JPY loss on {} (lot {:?})",
                loss_usd, jpy_loss, ticker, lot.purchase_date);
        }
    }
}

/// Check whether `date` falls within the 30-calendar-day wash-sale window around `sale_date`.
#[allow(dead_code)]
pub fn is_wash_sale_window(sale_date: NaiveDate, check_date: NaiveDate) -> bool {
    let diff = (check_date - sale_date).num_days().abs();
    diff <= 30
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wash_sale_window_boundary() {
        let sale = NaiveDate::from_ymd_opt(2026, 11, 15).unwrap();
        assert!(is_wash_sale_window(sale, NaiveDate::from_ymd_opt(2026, 11, 15).unwrap()));
        assert!(is_wash_sale_window(sale, NaiveDate::from_ymd_opt(2026, 10, 16).unwrap())); // 30 days before
        assert!(is_wash_sale_window(sale, NaiveDate::from_ymd_opt(2026, 12, 15).unwrap())); // 30 days after
        assert!(!is_wash_sale_window(sale, NaiveDate::from_ymd_opt(2026, 12, 16).unwrap())); // 31 days after
    }
}

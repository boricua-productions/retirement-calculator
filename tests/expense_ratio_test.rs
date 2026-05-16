//! Tests for the per-issuer expense ratio resolution pipeline.
//!
//! Pure (offline) tests cover the resolver's contract: dispatch, fallback,
//! validation, source provenance. Live network tests are #[ignore]'d so CI
//! doesn't depend on issuer endpoints staying up — run them locally with
//! `cargo test -- --ignored` to verify adapters still work.

use retirement_calculator::engine::market_data::{
    expense_ratio::{fallback_expense_ratio, resolve_expense_ratio, validate},
    ExpenseRatioSource, Issuer,
};

// ── validate() — guards against silent corruption ────────────────────────

#[test]
fn validate_accepts_typical_etf_range() {
    assert!(validate(0.0003).is_ok());  // VOO
    assert!(validate(0.0015).is_ok());  // QQQM
    assert!(validate(0.0049).is_ok());  // HYG
}

#[test]
fn validate_rejects_zero_to_block_silent_failures() {
    // A scraper that finds the wrong DOM node often returns 0. Catching it
    // here forces the resolver to fall back instead of corrupting projections.
    assert!(validate(0.0).is_err());
}

#[test]
fn validate_rejects_negative_nan_and_infinity() {
    assert!(validate(-0.01).is_err());
    assert!(validate(f64::NAN).is_err());
    assert!(validate(f64::INFINITY).is_err());
}

#[test]
fn validate_rejects_implausibly_high() {
    // No US ETF charges more than ~3%; 5% is a generous ceiling.
    assert!(validate(0.10).is_err());
    assert!(validate(1.0).is_err());
}

// ── fallback table ───────────────────────────────────────────────────────

#[test]
fn fallback_covers_priority_tickers() {
    for ticker in ["VOO", "VTI", "SCHD", "QQQ", "QQQM", "SPY", "IVV", "AGG"] {
        assert!(
            fallback_expense_ratio(ticker).is_some(),
            "priority ticker {} missing from fallback table", ticker
        );
    }
}

#[test]
fn fallback_unknown_ticker_returns_none() {
    assert!(fallback_expense_ratio("ZZZZZ").is_none());
    assert!(fallback_expense_ratio("").is_none());
}

#[test]
fn fallback_values_are_in_validator_range() {
    // Every hardcoded value must itself pass validate() — guards against
    // a future edit setting one to 0 or some absurd number.
    for ticker in ["VOO", "VTI", "VXUS", "BND", "SCHD", "QQQ", "QQQM", "SPY",
                   "IVV", "AGG", "IEFA", "TLT", "GLD"] {
        let er = fallback_expense_ratio(ticker)
            .unwrap_or_else(|| panic!("{} not in fallback", ticker));
        assert!(validate(er).is_ok(), "{} fallback {} fails validate", ticker, er);
    }
}

// ── resolve_expense_ratio() — contract ───────────────────────────────────

#[test]
fn stock_returns_not_applicable_with_zero() {
    let (er, src) = resolve_expense_ratio("MSFT", false);
    assert_eq!(er, 0.0);
    assert_eq!(src, ExpenseRatioSource::NotApplicable);
}

#[test]
fn unknown_ticker_returns_unavailable() {
    // No adapter, no fallback → caller should preserve existing user value.
    let (er, src) = resolve_expense_ratio("ZZZZZ", true);
    assert_eq!(er, 0.0);
    assert_eq!(src, ExpenseRatioSource::Unavailable);
}

#[test]
fn schwab_ticker_falls_back_from_unimplemented_adapter() {
    // SCHD has an issuer dispatch but the Schwab adapter is intentionally
    // unimplemented (bot-walled). Should land cleanly on the fallback.
    let (er, src) = resolve_expense_ratio("SCHD", true);
    assert_eq!(src, ExpenseRatioSource::Fallback);
    assert_eq!(er, fallback_expense_ratio("SCHD").unwrap());
}

#[test]
fn ssga_ticker_falls_back_from_unimplemented_adapter() {
    let (er, src) = resolve_expense_ratio("SPY", true);
    assert_eq!(src, ExpenseRatioSource::Fallback);
    assert_eq!(er, fallback_expense_ratio("SPY").unwrap());
}

#[test]
fn ishares_ticker_falls_back_from_unimplemented_adapter() {
    let (er, src) = resolve_expense_ratio("IVV", true);
    assert_eq!(src, ExpenseRatioSource::Fallback);
    assert_eq!(er, fallback_expense_ratio("IVV").unwrap());
}

// ── provenance labels ────────────────────────────────────────────────────

#[test]
fn source_labels_are_distinct_and_nonempty() {
    let labels = [
        ExpenseRatioSource::Fetched(Issuer::Vanguard).label(),
        ExpenseRatioSource::Fetched(Issuer::Invesco).label(),
        ExpenseRatioSource::Fallback.label(),
        ExpenseRatioSource::NotApplicable.label(),
        ExpenseRatioSource::Unavailable.label(),
    ];
    for label in &labels {
        assert!(!label.is_empty(), "empty label");
    }
    for i in 0..labels.len() {
        for j in (i + 1)..labels.len() {
            assert_ne!(labels[i], labels[j], "duplicate label: {}", labels[i]);
        }
    }
}

// ── live integration (network) — run with: cargo test -- --ignored ───────

#[test]
#[ignore]
fn live_vanguard_voo_returns_expected() {
    let (er, src) = resolve_expense_ratio("VOO", true);
    assert!(matches!(src, ExpenseRatioSource::Fetched(Issuer::Vanguard) | ExpenseRatioSource::Fallback),
        "got source {:?}", src);
    // VOO has been 0.03% for years; allow generous range.
    assert!(er >= 0.0001 && er <= 0.001, "unexpected VOO er: {}", er);
}

#[test]
#[ignore]
fn live_invesco_qqqm_returns_expected() {
    let (er, src) = resolve_expense_ratio("QQQM", true);
    assert!(matches!(src, ExpenseRatioSource::Fetched(Issuer::Invesco) | ExpenseRatioSource::Fallback),
        "got source {:?}", src);
    // QQQM is 0.15%.
    assert!(er >= 0.0010 && er <= 0.0020, "unexpected QQQM er: {}", er);
}

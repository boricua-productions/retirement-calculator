use log::{info, warn};

/// Which firm issues an ETF, used to dispatch to the right adapter and to
/// label the provenance of a fetched value in logs and the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Issuer {
    Vanguard,
    Schwab,
    Invesco,
    IShares,
    SSGA,
}

impl Issuer {
    pub fn as_str(self) -> &'static str {
        match self {
            Issuer::Vanguard => "Vanguard",
            Issuer::Schwab   => "Schwab",
            Issuer::Invesco  => "Invesco",
            Issuer::IShares  => "iShares",
            Issuer::SSGA     => "SSGA",
        }
    }
}

/// Provenance of an expense_ratio value after resolution. The UI uses this to
/// label the field and to decide whether the existing user-entered value
/// should be preserved (Unavailable) or overwritten (every other variant).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExpenseRatioSource {
    Fetched(Issuer),
    Fallback,
    /// Ticker is an individual stock — no expense ratio applies.
    #[default]
    NotApplicable,
    /// Fetch failed AND no fallback exists. Callers should preserve the
    /// existing user value rather than overwrite with 0.0.
    Unavailable,
}

impl ExpenseRatioSource {
    pub fn label(self) -> String {
        match self {
            ExpenseRatioSource::Fetched(i)    => format!("fetched: {}", i.as_str()),
            ExpenseRatioSource::Fallback      => "fallback (hardcoded)".to_string(),
            ExpenseRatioSource::NotApplicable => "n/a (not a fund)".to_string(),
            ExpenseRatioSource::Unavailable   => "unavailable — value preserved".to_string(),
        }
    }
}

/// Hardcoded fallback expense ratios for well-known ETFs. Values are annual
/// fractions (0.0003 = 0.03%). Used when the live adapter fails or no adapter
/// covers the ticker. Update annually from issuer fact sheets.
pub fn fallback_expense_ratio(ticker: &str) -> Option<f64> {
    match ticker {
        // Vanguard
        "VOO"  => Some(0.0003),
        "VTI"  => Some(0.0003),
        "VXUS" => Some(0.0005),
        "BND"  => Some(0.0003),
        "VEA"  => Some(0.0005),
        "VWO"  => Some(0.0007),
        "VYM"  => Some(0.0006),
        "VGT"  => Some(0.0009),
        "VIG"  => Some(0.0005),
        "VNQ"  => Some(0.0013),
        "VTV"  => Some(0.0004),
        "VUG"  => Some(0.0004),
        "BNDX" => Some(0.0007),
        "VT"   => Some(0.0006),
        // Schwab
        "SCHD" => Some(0.0006),
        "SCHB" => Some(0.0003),
        "SCHX" => Some(0.0003),
        "SCHF" => Some(0.0006),
        "SCHE" => Some(0.0011),
        "SCHG" => Some(0.0004),
        "SCHV" => Some(0.0004),
        "SCHH" => Some(0.0007),
        "SCHA" => Some(0.0004),
        "SCHM" => Some(0.0004),
        "SCHO" => Some(0.0003),
        "SCHR" => Some(0.0003),
        "SCHZ" => Some(0.0003),
        // Invesco
        "QQQ"  => Some(0.0020),
        "QQQM" => Some(0.0015),
        "RSP"  => Some(0.0020),
        // iShares
        "IVV"  => Some(0.0003),
        "AGG"  => Some(0.0003),
        "IEFA" => Some(0.0007),
        "IEMG" => Some(0.0009),
        "ITOT" => Some(0.0003),
        "IJR"  => Some(0.0006),
        "IJH"  => Some(0.0005),
        "IWM"  => Some(0.0019),
        "IWF"  => Some(0.0019),
        "IWD"  => Some(0.0019),
        "EFA"  => Some(0.0033),
        "EEM"  => Some(0.0070),
        "TLT"  => Some(0.0015),
        "HYG"  => Some(0.0049),
        "LQD"  => Some(0.0014),
        // SSGA / State Street
        "SPY"  => Some(0.0009),
        "DIA"  => Some(0.0016),
        "MDY"  => Some(0.0023),
        "XLK"  => Some(0.0009),
        "XLF"  => Some(0.0009),
        "XLE"  => Some(0.0009),
        "XLV"  => Some(0.0009),
        // Other
        "GLD"  => Some(0.0040),
        _ => None,
    }
}

/// Map a ticker to the issuer adapter responsible for it. Returns None for
/// tickers without a registered adapter (which fall straight to the fallback
/// table or to Unavailable).
fn dispatch(ticker: &str) -> Option<Issuer> {
    match ticker {
        "VOO" | "VTI" | "VXUS" | "BND" | "VEA" | "VWO" | "VYM" | "VGT"
        | "VIG" | "VNQ" | "VTV" | "VUG" | "BNDX" | "VT" => Some(Issuer::Vanguard),

        "SCHD" | "SCHB" | "SCHX" | "SCHF" | "SCHE" | "SCHG" | "SCHV"
        | "SCHH" | "SCHA" | "SCHM" | "SCHO" | "SCHR" | "SCHZ" => Some(Issuer::Schwab),

        "QQQ" | "QQQM" | "RSP" | "BKLN" | "PDP" | "SPHQ" | "SPLV" => Some(Issuer::Invesco),

        "IVV" | "AGG" | "IEFA" | "IEMG" | "ITOT" | "IJR" | "IJH" | "IWM"
        | "IWF" | "IWD" | "EFA" | "EEM" | "TLT" | "HYG" | "LQD" => Some(Issuer::IShares),

        "SPY" | "DIA" | "MDY" | "XLK" | "XLF" | "XLE" | "XLV" | "XLI"
        | "XLY" | "XLP" | "XLU" | "XLB" | "XLRE" | "XLC" | "GLD" => Some(Issuer::SSGA),

        _ => None,
    }
}

/// Each issuer adapter implements this. The static `try_fetch` does one
/// network call, parses the response, and range-validates before returning.
pub trait IssuerAdapter {
    const ISSUER: Issuer;
    fn try_fetch(ticker: &str) -> Result<f64, String>;
}

/// Range-validate a fetched expense ratio. Anything outside [0.0001, 0.05]
/// (0.01% to 5%) is treated as a fetch error to guard against silent
/// DOM-change failures returning 0 or absurd values that would otherwise
/// corrupt a 30-year retirement projection.
pub fn validate(er: f64) -> Result<f64, String> {
    if !er.is_finite() {
        return Err(format!("non-finite value: {}", er));
    }
    if !(0.0001..=0.05).contains(&er) {
        return Err(format!("{:.4} out of plausible range [0.0001, 0.05]", er));
    }
    Ok(er)
}

/// Try issuer adapter → hardcoded fallback → Unavailable. Returns the
/// resolved annual fraction and its provenance so callers can label the
/// value and decide overwrite vs preserve semantics.
///
/// `include_fund_data` is false for individual stocks — skips network and
/// returns (0.0, NotApplicable).
pub fn resolve_expense_ratio(ticker: &str, include_fund_data: bool) -> (f64, ExpenseRatioSource) {
    if !include_fund_data {
        return (0.0, ExpenseRatioSource::NotApplicable);
    }

    if let Some(issuer) = dispatch(ticker) {
        // Intentional non-implementations skip straight to fallback at INFO level.
        let is_intentional_skip = matches!(issuer, Issuer::Schwab | Issuer::IShares | Issuer::SSGA);
        if is_intentional_skip {
            info!("[ExpenseRatio] {}: {} has no live adapter — using hardcoded fallback",
                ticker, issuer.as_str());
        } else {
            let result = match issuer {
                Issuer::Vanguard => super::adapters::vanguard::Vanguard::try_fetch(ticker),
                Issuer::Invesco  => super::adapters::invesco::Invesco::try_fetch(ticker),
                _ => unreachable!(),
            };
            match result {
                Ok(er) => {
                    info!("[ExpenseRatio] {}: fetched {:.3}% from {}",
                        ticker, er * 100.0, issuer.as_str());
                    return (er, ExpenseRatioSource::Fetched(issuer));
                }
                Err(e) => {
                    warn!("[ExpenseRatio] {}: {} fetch failed ({}), trying fallback",
                        ticker, issuer.as_str(), e);
                }
            }
        }
    }

    if let Some(er) = fallback_expense_ratio(ticker) {
        info!("[ExpenseRatio] {}: using hardcoded fallback {:.3}%",
            ticker, er * 100.0);
        return (er, ExpenseRatioSource::Fallback);
    }

    info!("[ExpenseRatio] {}: no adapter, no fallback — preserving existing user value",
        ticker);
    (0.0, ExpenseRatioSource::Unavailable)
}

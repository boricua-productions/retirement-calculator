use super::super::expense_ratio::{validate, Issuer, IssuerAdapter};

/// Vanguard returns the expense ratio as a percentage string in the
/// `fundProfile.expenseRatio` field of its unauthenticated profile API.
/// Verified against VOO=0.03%, VTI=0.03%, VNQ=0.13% on 2026-05-16.
pub struct Vanguard;

impl IssuerAdapter for Vanguard {
    const ISSUER: Issuer = Issuer::Vanguard;

    fn try_fetch(ticker: &str) -> Result<f64, String> {
        let url = format!(
            "https://investor.vanguard.com/investment-products/etfs/profile/api/{}/profile",
            ticker.to_lowercase()
        );
        let resp = ureq::get(&url)
            .set("User-Agent", "Mozilla/5.0 retirement-calculator/1.0")
            .call()
            .map_err(|e| format!("http: {}", e))?;
        let body = resp.into_string().map_err(|e| format!("read body: {}", e))?;
        let json: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| format!("parse json: {}", e))?;
        let er_str = json["fundProfile"]["expenseRatio"]
            .as_str()
            .ok_or_else(|| "missing fundProfile.expenseRatio".to_string())?;
        // Vanguard returns a percentage as a string (e.g. "0.0300" = 0.03%).
        let er_pct: f64 = er_str
            .parse()
            .map_err(|e| format!("parse '{}': {}", er_str, e))?;
        validate(er_pct / 100.0)
    }
}

use super::super::expense_ratio::{validate, Issuer, IssuerAdapter};

/// Invesco embeds fund metadata as escaped JSON inside the product-detail
/// HTML page. The expense ratio field is `netExpenseRatio.value`, encoded
/// as a percentage (e.g. `"value":0.15` = 0.15%). Verified against QQQM=0.15%
/// on 2026-05-16.
///
/// We anchor on the pattern `"netExpenseRatio"` followed closely by `"value":`,
/// then extract the next numeric literal. This is stricter than searching for
/// bare `value` (which previously matched unrelated fields after a page layout
/// change, returning 34 instead of 0.15 for QQQM).
pub struct Invesco;

impl IssuerAdapter for Invesco {
    const ISSUER: Issuer = Issuer::Invesco;

    fn try_fetch(ticker: &str) -> Result<f64, String> {
        let url = format!(
            "https://www.invesco.com/us/financial-products/etfs/product-detail?audienceType=Investor&ticker={}",
            ticker
        );
        let resp = ureq::get(&url)
            .set("User-Agent", "Mozilla/5.0 retirement-calculator/1.0")
            .call()
            .map_err(|e| format!("http: {}", e))?;
        let body = resp.into_string().map_err(|e| format!("read body: {}", e))?;

        // Find the netExpenseRatio object and then anchor on `"value":` inside it.
        // The pattern we expect: `"netExpenseRatio":{...,"value":0.15,...}`
        let key_idx = body.find("netExpenseRatio")
            .ok_or_else(|| "netExpenseRatio key not present in page".to_string())?;
        let window_end = (key_idx + 300).min(body.len());
        let window = &body[key_idx..window_end];

        // Require `"value":` (with colon) immediately after a quote, anchored
        // within the netExpenseRatio object — prevents matching `"netAssetValue"`.
        let value_pattern = "\"value\":";
        let val_idx = window.find(value_pattern)
            .ok_or_else(|| "\"value\": not found within 300 chars of netExpenseRatio".to_string())?;
        let after_colon = &window[val_idx + value_pattern.len()..];

        let num_start = after_colon.find(|c: char| c.is_ascii_digit())
            .ok_or_else(|| "no numeric value after \"value\":".to_string())?;
        let after_num = &after_colon[num_start..];
        let num_end = after_num.find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(after_num.len());
        let pct_str = &after_num[..num_end];

        let pct: f64 = pct_str
            .parse()
            .map_err(|e| format!("parse '{}': {}", pct_str, e))?;
        validate(pct / 100.0)
    }
}

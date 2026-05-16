use super::super::expense_ratio::{validate, Issuer, IssuerAdapter};

/// Invesco embeds fund metadata as escaped JSON inside the product-detail
/// HTML page. The expense ratio field is `netExpenseRatio.value`, encoded
/// as a percentage string (e.g. "0.15" = 0.15%). Verified against QQQM=0.15%
/// on 2026-05-16.
///
/// We scan for the `netExpenseRatio` literal and walk forward to the next
/// `value`-keyed numeric — avoids parsing the entire 600KB page and is
/// resilient to both raw (`"`) and HTML-entity-encoded (`&#34;`) quoting.
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

        let key_idx = body.find("netExpenseRatio")
            .ok_or_else(|| "netExpenseRatio key not present in page".to_string())?;
        let window_end = (key_idx + 600).min(body.len());
        let window = &body[key_idx..window_end];

        let val_idx = window.find("value")
            .ok_or_else(|| "value field not found within 600 chars of netExpenseRatio".to_string())?;
        let after_val = &window[val_idx..];

        let num_start = after_val.find(|c: char| c.is_ascii_digit())
            .ok_or_else(|| "no numeric value after 'value' marker".to_string())?;
        let after_num = &after_val[num_start..];
        let num_end = after_num.find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(after_num.len());
        let pct_str = &after_num[..num_end];

        let pct: f64 = pct_str
            .parse()
            .map_err(|e| format!("parse '{}': {}", pct_str, e))?;
        validate(pct / 100.0)
    }
}

use std::collections::HashMap;
use log::warn;
use crate::models::config::Position;

/// Global market average CAGR fallback when live data is unavailable (7%).
pub const GLOBAL_MARKET_FALLBACK_GROWTH: f64 = 0.07;

/// Provides fallback market data and live 10-year CAGR fetching.
/// Live data is fetched from Yahoo Finance; any network/parse error falls back
/// gracefully to the global market average.
pub struct MarketDataService;

impl MarketDataService {
    /// Fallback prices (USD) used when live data is unavailable.
    pub fn fallback_price(ticker: &str) -> f64 {
        match ticker {
            "VTI"  => 280.0,
            "QQQM" => 195.0,
            "MSFT" => 430.0,
            "SCHD" => 83.0,
            "PANW" => 360.0,
            _      => 100.0,
        }
    }

    /// Fallback annual growth rates (CAGR) used when live data is unavailable.
    pub fn fallback_growth(ticker: &str) -> f64 {
        match ticker {
            "VTI"  => 0.08,
            "QQQM" => 0.10,
            "SCHD" => 0.09,
            "MSFT" => 0.12,
            "PANW" => 0.15,
            _      => GLOBAL_MARKET_FALLBACK_GROWTH,
        }
    }

    /// Fallback dividend yields used when live data is unavailable.
    pub fn fallback_yield(ticker: &str) -> f64 {
        match ticker {
            "VTI"  => 0.015,
            "QQQM" => 0.006,
            "SCHD" => 0.034,
            "MSFT" => 0.008,
            "PANW" => 0.0,
            _      => 0.015,
        }
    }

    /// Fallback USD/JPY exchange rate.
    pub fn fallback_fx_rate() -> f64 {
        145.0
    }

    /// Returns known Roth IRA annual contribution limits, with projection for future years.
    pub fn roth_limit(year: i32) -> f64 {
        match year {
            2023 => 6_500.0,
            2024 => 7_000.0,
            2025 => 7_000.0,
            2026 => 7_000.0,
            y if y > 2026 => {
                let years_diff = y - 2026;
                7_000.0 + 500.0 * (years_diff / 2) as f64
            }
            _ => 7_000.0,
        }
    }

    /// Returns the hardcoded 10-year historical CAGR for well-known index tickers.
    #[allow(dead_code)]
    ///
    /// Values represent annualised CAGR over the decade ending ~2024. Use this
    /// in "Historical" mode when live data is disabled and you want data-grounded
    /// assumptions rather than arbitrary growth inputs.
    ///
    /// Falls back to `GLOBAL_MARKET_FALLBACK_GROWTH` (7%) for unknown tickers.
    pub fn historical_10y_cagr(ticker: &str) -> f64 {
        match ticker {
            "VTI"  => 0.121,  // US total market: ~12.1% 10-year CAGR (2015–2024)
            "VXUS" => 0.043,  // Intl ex-US total market: ~4.3% (2015–2024)
            "SCHD" => 0.114,  // US dividend: ~11.4% (2015–2024)
            "QQQM" | "QQQ" => 0.185,  // NASDAQ-100: ~18.5% (2015–2024)
            "VNQ"  => 0.072,  // US REIT: ~7.2% (2015–2024)
            "BND"  => 0.012,  // US bond aggregate: ~1.2% (2015–2024)
            _      => GLOBAL_MARKET_FALLBACK_GROWTH,
        }
    }

    /// Fetches the most recent adjusted close price for `ticker` from Yahoo Finance.
    /// Uses the v8 chart API with a 5-day daily window to get the latest price.
    /// Falls back to `fallback_price` on any network or parse error.
    pub fn fetch_current_price(ticker: &str) -> f64 {
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1d&range=5d",
            ticker
        );

        let result = (|| -> Result<f64, Box<dyn std::error::Error>> {
            let resp = ureq::get(&url)
                .set("User-Agent", "Mozilla/5.0 retirement-calculator/1.0")
                .call()?;
            let body = resp.into_string()?;
            let json: serde_json::Value = serde_json::from_str(&body)?;
            let closes = json["chart"]["result"][0]["indicators"]["adjclose"][0]["adjclose"]
                .as_array()
                .ok_or("missing adjclose")?;
            let price = closes.iter().rev()
                .find_map(|v| v.as_f64())
                .ok_or("no valid price")?;
            if price <= 0.0 { return Err("non-positive price".into()); }
            Ok(price)
        })();

        match result {
            Ok(p) => p,
            Err(e) => {
                warn!("[MarketData] {}: price fetch failed ({}), using fallback ${:.2}",
                    ticker, e, Self::fallback_price(ticker));
                Self::fallback_price(ticker)
            }
        }
    }

    /// Fetches the 10-year annualised CAGR for `ticker` from Yahoo Finance.
    ///
    /// Uses the v8 chart API with monthly intervals over a 10-year range.
    /// Calculates CAGR as `(last_adj_close / first_adj_close) ^ (1/10) - 1`.
    ///
    /// Falls back to `GLOBAL_MARKET_FALLBACK_GROWTH` (7%) on any error.
    pub fn fetch_10y_cagr(ticker: &str) -> f64 {
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1mo&range=10y",
            ticker
        );

        let result = (|| -> Result<f64, Box<dyn std::error::Error>> {
            let resp = ureq::get(&url)
                .set("User-Agent", "Mozilla/5.0 retirement-calculator/1.0")
                .call()?;

            let body = resp.into_string()?;
            let json: serde_json::Value = serde_json::from_str(&body)?;

            let closes = json["chart"]["result"][0]["indicators"]["adjclose"][0]["adjclose"]
                .as_array()
                .ok_or("missing adjclose array")?;

            let first = closes.iter()
                .find_map(|v| v.as_f64())
                .ok_or("no valid first price")?;

            let last = closes.iter()
                .rev()
                .find_map(|v| v.as_f64())
                .ok_or("no valid last price")?;

            if first <= 0.0 || last <= 0.0 {
                return Err("non-positive price".into());
            }

            let years = (closes.len() as f64) / 12.0;
            let cagr = (last / first).powf(1.0 / years) - 1.0;
            Ok(cagr)
        })();

        match result {
            Ok(cagr) => {
                // Sanity-check: clamp to [-50%, +100%] to filter bad data
                let clamped = cagr.clamp(-0.50, 1.00);
                if (clamped - cagr).abs() > 0.001 {
                    warn!("[MarketData] {}: CAGR {:.2}% out of range, using fallback", ticker, cagr * 100.0);
                    Self::fallback_growth(ticker)
                } else {
                    clamped
                }
            }
            Err(e) => {
                warn!("[MarketData] {}: fetch failed ({}), using fallback {:.0}%",
                    ticker, e, Self::fallback_growth(ticker) * 100.0);
                Self::fallback_growth(ticker)
            }
        }
    }

    /// Calculate total cost basis and current value for a slice of positions.
    /// Returns `(cost_basis_usd, current_value_usd)`.
    /// Current price uses `fallback_price`; call site may substitute live prices.
    #[allow(dead_code)]
    pub fn calculate_account_value(positions: &[Position]) -> (f64, f64) {
        let mut total_basis = 0.0;
        let mut total_value = 0.0;
        for pos in positions {
            total_basis += pos.cost_basis();
            total_value += pos.quantity * Self::fallback_price(&pos.ticker);
        }
        (total_basis, total_value)
    }

    /// Resolve the final price for each ticker in the portfolio.
    /// Priority: manual_overrides > fallback.
    #[allow(dead_code)]
    pub fn resolve_prices(
        tickers: &[String],
        manual_overrides: &HashMap<String, f64>,
    ) -> HashMap<String, f64> {
        let mut prices: HashMap<String, f64> = HashMap::new();

        for ticker in tickers {
            if ticker.starts_with("//") || ticker.starts_with('_') {
                continue;
            }
            let price = manual_overrides.get(ticker.as_str()).copied().unwrap_or(0.0);
            if price > 0.0 {
                prices.insert(ticker.clone(), price);
            } else {
                let fallback = Self::fallback_price(ticker);
                warn!("Price for '{}' not provided or zero; using fallback ${}", ticker, fallback);
                prices.insert(ticker.clone(), fallback);
            }
        }
        prices
    }
}

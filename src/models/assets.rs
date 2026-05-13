use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Which tax jurisdiction(s) apply to an individual account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountJurisdiction {
    /// Taxed in the US only (e.g. FERS pension, VA disability, Roth IRA).
    Us,
    /// Taxed in Japan only (e.g. Japan DC / iDeCo).
    Japan,
    /// Subject to both US and Japan tax rules (e.g. main taxable brokerage).
    #[default]
    Both,
    /// Tax-exempt in both jurisdictions.
    None,
}

impl std::fmt::Display for AccountJurisdiction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountJurisdiction::Us    => write!(f, "US"),
            AccountJurisdiction::Japan => write!(f, "Japan"),
            AccountJurisdiction::Both  => write!(f, "Both"),
            AccountJurisdiction::None  => write!(f, "None"),
        }
    }
}

/// Physical / regulatory location of an account's assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccountLocation {
    #[default]
    Us,
    Japan,
    Both,
    None,
}

impl std::fmt::Display for AccountLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AccountLocation::Us    => write!(f, "US"),
            AccountLocation::Japan => write!(f, "Japan"),
            AccountLocation::Both  => write!(f, "Both"),
            AccountLocation::None  => write!(f, "None"),
        }
    }
}

/// Asset category for portfolio management strategy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetCategory {
    Growth,
    Income,
}

impl Default for AssetCategory {
    fn default() -> Self {
        AssetCategory::Income
    }
}

/// Currency denomination of an asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Currency {
    Usd,
    Jpy,
}

impl Default for Currency {
    fn default() -> Self {
        Currency::Usd
    }
}

/// V7.1 — Currency in which an asset pays its dividends.
/// Drives the JPY-first waterfall: JPY dividends land in the War Chest bucket;
/// USD dividends land in the Bridge Fund bucket (after FX spread penalty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DividendCurrency {
    #[default]
    Usd,
    Jpy,
}

fn default_dividend_months() -> Vec<u32> { vec![3, 6, 9, 12] }

/// V7.5 — PFIC tax regime election for a Japan-domiciled fund asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PficRegime {
    #[default]
    NotPfic,
    /// IRC §1296 — annual mark-to-market; FMV − basis taxed as ordinary income.
    Mtm,
    /// IRC §1291 — default excess distribution treatment.
    /// Out-of-scope for V7.5 (flag and warn; no multi-year reconstruction).
    ExcessDistribution,
}

/// V7.6 — Asset class drives distribution routing and PFIC defaults.
/// Funds domiciled outside the US flow through the PFIC check; stocks bypass it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssetClass {
    #[default]
    Stock,
    Etf,
    MutualFund,
    Other,
}

/// V7.6 — Component-based return profile. Decomposes a flat yield into
/// tax-aware sub-streams so the distribution handler can route each component
/// through the correct §904 basket and §1296 check.
///
/// Invariants:
///   - `cap_growth` is price-only; reinvested distributions are NOT baked in.
///   - `nav_growth` is reserved for fund NAV accounting; stocks/ETFs leave at 0.
///   - All yields are annual fractions (0.04 = 4%).
///   - `expense_ratio` is deducted from `cap_growth` before the price update.
///   - `roc` (Return of Capital) is non-taxable in the year received; it
///     reduces both USD and JPY cost basis proportionally.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DetailedReturnProfile {
    #[serde(default)]
    pub cap_growth: f64,
    #[serde(default)]
    pub nav_growth: f64,
    #[serde(default)]
    pub dividend_yield: f64,
    #[serde(default)]
    pub interest_yield: f64,
    #[serde(default)]
    pub cap_gains_dist: f64,
    #[serde(default)]
    pub special_dist: f64,
    #[serde(default)]
    pub roc: f64,
    #[serde(default)]
    pub expense_ratio: f64,
}

impl DetailedReturnProfile {
    /// Total annual taxable distribution yield (excludes ROC, which is basis return).
    pub fn total_taxable_yield(&self) -> f64 {
        self.dividend_yield + self.interest_yield + self.cap_gains_dist + self.special_dist
    }
    /// Net effective price growth after expense-ratio drag.
    pub fn net_growth(&self) -> f64 {
        (self.cap_growth - self.expense_ratio).max(-0.999)
    }
}

/// A single tax lot of an asset, tracking purchase date and cost basis.
/// Mirrors Python's `AssetLot` dataclass. Used for FIFO capital gains tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetLot {
    pub purchase_date: NaiveDate,
    pub qty: f64,
    pub basis: f64,
    /// V7.5 — §1091 wash-sale taint: disallowed loss amount (USD) added back to basis.
    /// Set when a replacement security is acquired within the 30-day window around the sale.
    #[serde(default)]
    pub disallowed_loss_usd: f64,
    /// V7.5 — Date after which this lot is no longer wash-sale tainted.
    #[serde(default)]
    pub wash_sale_clean_after: Option<NaiveDate>,
}

/// A financial asset (e.g., a stock or ETF) held within an account.
/// Mirrors Python's `Asset` dataclass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub ticker: String,
    pub price: f64,
    pub yield_rate: f64,
    pub growth_rate: f64,
    pub currency: Currency,
    pub category: AssetCategory,
    pub drip_enabled: bool,
    pub dividend_reinvest_target: Option<String>,
    /// Manual override for the annual growth rate (takes precedence over market-fetched rate).
    #[serde(default)]
    pub custom_growth_rate: Option<f64>,
    /// V7.0 — Japan-resident cost basis (¥/share). Populated at load time from
    /// `Position.avg_purchase_price_jpy` or the fallback `avg_cost × usd_jpy_at_load`.
    /// Drives the highest-basis-first liquidation order and the JPY capital-gains
    /// computation. 0.0 sentinel = unset (callers should fall back).
    #[serde(default)]
    pub avg_jpy_basis_per_share: f64,
    /// V7.1 — Calendar months (1..=12) in which this asset pays dividends.
    /// Default: [3, 6, 9, 12] (quarterly). Zero-smoothing: dividends only fire
    /// in months present in this list; all other months produce zero income.
    #[serde(default = "default_dividend_months")]
    pub dividend_months: Vec<u32>,
    /// V7.1 — Currency denomination of this asset's dividend payout.
    /// Usd dividends route through the Bridge Fund (with FX penalty on spend);
    /// Jpy dividends route directly into the War Chest bucket (no FX churn).
    #[serde(default)]
    pub dividend_currency: DividendCurrency,
    /// V7.5 — PFIC tax regime. NotPfic (default) = standard capital-gains treatment.
    #[serde(default)]
    pub pfic_regime: PficRegime,
    /// V7.5 — Prior-year FMV per share used by §1296 MTM to compute annual gain.
    /// Initialized to cost basis per share on the first MTM mark.
    #[serde(default)]
    pub pfic_prior_year_fmv_per_share: f64,
    /// V7.6 — Asset classification (Stock / Etf / MutualFund / Other).
    /// Defaults to Stock for backward compatibility with pre-V7.6 configs.
    #[serde(default)]
    pub asset_class: AssetClass,
    /// V7.6 — Component-based return profile. When `Some`, supersedes the flat
    /// `yield_rate` / `growth_rate` fields for all per-month calculations.
    /// When `None`, the legacy single-yield model is used (back-compat).
    #[serde(default)]
    pub return_profile: Option<DetailedReturnProfile>,
    /// All tax lots, maintained in FIFO (purchase date ascending) order.
    pub lots: Vec<AssetLot>,
}

impl Asset {
    pub fn new(ticker: impl Into<String>, price: f64, yield_rate: f64, growth_rate: f64) -> Self {
        Self {
            ticker: ticker.into(),
            price,
            yield_rate,
            growth_rate,
            currency: Currency::Usd,
            category: AssetCategory::Income,
            drip_enabled: true,
            dividend_reinvest_target: None,
            custom_growth_rate: None,
            avg_jpy_basis_per_share: 0.0,
            dividend_months: default_dividend_months(),
            dividend_currency: DividendCurrency::Usd,
            pfic_regime: PficRegime::NotPfic,
            pfic_prior_year_fmv_per_share: 0.0,
            asset_class: AssetClass::Stock,
            return_profile: None,
            lots: Vec::new(),
        }
    }

    /// JPY basis per share, falling back to USD-basis × `fx_fallback` when unset.
    /// `fx_fallback` should be the FX rate at portfolio-load time so the fallback
    /// represents an honest "what we paid in yen if we'd paid today" estimate.
    pub fn jpy_basis_per_share(&self, fx_fallback: f64) -> f64 {
        if self.avg_jpy_basis_per_share > 0.0 {
            return self.avg_jpy_basis_per_share;
        }
        let q = self.qty();
        if q <= 0.0 { return 0.0; }
        let usd_per_share = self.basis() / q;
        usd_per_share * fx_fallback
    }

    /// Returns the effective growth rate, preferring custom_growth_rate if set.
    pub fn effective_growth_rate(&self) -> f64 {
        self.custom_growth_rate.unwrap_or(self.growth_rate)
    }

    /// V7.6 — Profile-aware effective price growth. Falls back to the legacy
    /// `custom_growth_rate` / `growth_rate` path when no profile is attached.
    pub fn effective_cap_growth(&self) -> f64 {
        match &self.return_profile {
            Some(p) => p.net_growth(),
            None    => self.custom_growth_rate.unwrap_or(self.growth_rate),
        }
    }

    /// V7.6 — Dividend (qualified/ordinary) yield. Passive §904 basket.
    pub fn dividend_yield_rate(&self) -> f64 {
        match &self.return_profile {
            Some(p) => p.dividend_yield,
            None    => self.yield_rate,  // legacy: all yield treated as dividends
        }
    }

    /// V7.6 — Interest distribution yield. Passive §904 basket, ordinary US stack.
    pub fn interest_yield_rate(&self) -> f64 {
        self.return_profile.as_ref().map(|p| p.interest_yield).unwrap_or(0.0)
    }

    /// V7.6 — Capital-gains distribution yield (mutual-fund pass-through).
    /// PFIC §1296 → ordinary basket; otherwise LTCG passive basket.
    pub fn cap_gains_dist_rate(&self) -> f64 {
        self.return_profile.as_ref().map(|p| p.cap_gains_dist).unwrap_or(0.0)
    }

    /// V7.6 — Special / non-recurring distribution yield.
    pub fn special_dist_rate(&self) -> f64 {
        self.return_profile.as_ref().map(|p| p.special_dist).unwrap_or(0.0)
    }

    /// V7.6 — Return-of-Capital yield. Non-taxable in the year received;
    /// reduces cost basis (both USD and JPY).
    pub fn roc_rate(&self) -> f64 {
        self.return_profile.as_ref().map(|p| p.roc).unwrap_or(0.0)
    }

    /// V7.6 — Total taxable distribution yield (sum of all taxable components,
    /// excluding ROC). Used by the Mode B oracle to size forward draws.
    pub fn total_distribution_yield(&self) -> f64 {
        match &self.return_profile {
            Some(p) => p.total_taxable_yield(),
            None    => self.yield_rate,
        }
    }

    /// Total shares across all lots.
    pub fn qty(&self) -> f64 {
        self.lots.iter().map(|l| l.qty).sum()
    }

    /// Total cost basis across all lots.
    #[allow(dead_code)]
    pub fn basis(&self) -> f64 {
        self.lots.iter().map(|l| l.basis).sum()
    }

    /// Current market value (price × total qty).
    pub fn market_value(&self) -> f64 {
        self.price * self.qty()
    }

    /// Monthly compounded growth factor derived from the effective annual growth rate.
    /// V7.6 — uses the profile-aware `effective_cap_growth()`, which subtracts the
    /// expense ratio before compounding so the drag is automatic.
    pub fn monthly_growth_factor(&self) -> f64 {
        (1.0 + self.effective_cap_growth()).powf(1.0 / 12.0)
    }

    /// Apply one month of price growth.
    pub fn grow(&mut self) {
        self.price *= self.monthly_growth_factor();
    }

    /// Add a new lot (purchase). Lots are kept in ascending purchase_date order.
    pub fn add_lot(&mut self, purchase_date: NaiveDate, qty: f64, basis: f64) {
        let lot = AssetLot { purchase_date, qty, basis, disallowed_loss_usd: 0.0, wash_sale_clean_after: None };
        // Insert in sorted order to maintain FIFO invariant.
        let pos = self.lots.partition_point(|l| l.purchase_date <= lot.purchase_date);
        self.lots.insert(pos, lot);
    }

    /// V7.6 — Apply a Return-of-Capital event. ROC is non-taxable in the year
    /// received and reduces both USD and JPY cost basis proportionally across
    /// all FIFO lots. Any excess above total basis is returned as an LTCG
    /// magnitude (USD) for the caller to route through the standard CG path.
    ///
    /// `fx_at_event` is used only to lazily sync `avg_jpy_basis_per_share` when
    /// it has not yet been populated; the proportional reduction itself does
    /// not depend on FX.
    pub fn apply_roc_basis_reduction(&mut self, roc_usd: f64, fx_at_event: f64) -> f64 {
        if roc_usd <= 0.0 || self.lots.is_empty() {
            return 0.0;
        }
        let total_basis: f64 = self.lots.iter().map(|l| l.basis).sum();
        if total_basis <= 0.0 {
            return roc_usd;  // no basis to reduce — entire amount becomes gain
        }
        let absorbed = roc_usd.min(total_basis);
        let excess   = (roc_usd - absorbed).max(0.0);
        let ratio    = absorbed / total_basis;
        for lot in &mut self.lots {
            lot.basis *= 1.0 - ratio;
        }
        if self.avg_jpy_basis_per_share > 0.0 {
            self.avg_jpy_basis_per_share *= 1.0 - ratio;
        } else {
            let q = self.qty();
            if q > 0.0 {
                let new_usd_per_share = self.lots.iter().map(|l| l.basis).sum::<f64>() / q;
                self.avg_jpy_basis_per_share = new_usd_per_share * fx_at_event;
            }
        }
        excess
    }
}

/// The capital-gains breakdown from a sell operation.
#[derive(Debug, Default, Clone)]
pub struct GainBreakdown {
    pub proceeds: f64,
    pub short_term_gain: f64,
    pub long_term_gain: f64,
}

impl GainBreakdown {
    pub fn total_gain(&self) -> f64 {
        self.short_term_gain + self.long_term_gain
    }
}

/// A financial account holding multiple assets.
/// Mirrors Python's `Account` class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub currency: Currency,
    pub assets: HashMap<String, Asset>,
    /// Physical/regulatory location of this account's assets.
    #[serde(default)]
    pub location: AccountLocation,
    /// Which tax jurisdiction(s) apply to gains/income from this account.
    #[serde(default)]
    pub tax_jurisdiction: AccountJurisdiction,
}

impl Account {
    #[allow(dead_code)]
    pub fn new(name: impl Into<String>, currency: Currency) -> Self {
        Self {
            name: name.into(),
            currency,
            assets: HashMap::new(),
            location: AccountLocation::default(),
            tax_jurisdiction: AccountJurisdiction::default(),
        }
    }

    /// Convenience constructor with explicit location and jurisdiction metadata.
    pub fn new_with_meta(
        name: impl Into<String>,
        currency: Currency,
        location: AccountLocation,
        tax_jurisdiction: AccountJurisdiction,
    ) -> Self {
        Self {
            name: name.into(),
            currency,
            assets: HashMap::new(),
            location,
            tax_jurisdiction,
        }
    }

    /// Total market value of all assets in the account's native currency.
    /// Assets denominated in a foreign currency are converted using `fx_usd_to_jpy`.
    pub fn total_value(&self, fx_usd_to_jpy: f64) -> f64 {
        self.assets.values().map(|a| {
            let mv = a.market_value();
            match (&self.currency, &a.currency) {
                (Currency::Usd, Currency::Jpy) => mv / fx_usd_to_jpy,
                (Currency::Jpy, Currency::Usd) => mv * fx_usd_to_jpy,
                _ => mv,
            }
        }).sum()
    }

    /// Apply one month of growth to all held assets.
    pub fn grow(&mut self) {
        for asset in self.assets.values_mut() {
            asset.grow();
        }
    }

    /// Apply a market shock (percentage drop) to all asset prices.
    pub fn shock(&mut self, pct: f64) {
        for asset in self.assets.values_mut() {
            asset.price *= 1.0 - pct;
        }
    }

    /// Buy `amount` (in account currency) of the given `ticker`.
    /// Creates the asset if it does not yet exist, using the given fallback price/growth_rate.
    /// Returns the amount actually spent.
    pub fn buy(
        &mut self,
        ticker: &str,
        amount: f64,
        purchase_date: NaiveDate,
        fallback_price: f64,
        fallback_growth: f64,
    ) -> f64 {
        if amount <= 0.0 {
            return 0.0;
        }
        let asset = self.assets.entry(ticker.to_string()).or_insert_with(|| {
            Asset::new(ticker, fallback_price, 0.0, fallback_growth)
        });
        let price = asset.price;
        if price <= 0.0 {
            return 0.0;
        }
        let qty = amount / price;
        asset.add_lot(purchase_date, qty, amount);
        amount
    }

    /// Sell `amount_to_sell` (in account currency) of `ticker` using FIFO.
    /// Returns the gain breakdown.
    pub fn sell_value(
        &mut self,
        ticker: &str,
        amount_to_sell: f64,
        current_date: NaiveDate,
    ) -> GainBreakdown {
        if amount_to_sell <= 0.0 {
            return GainBreakdown::default();
        }
        let asset = match self.assets.get_mut(ticker) {
            Some(a) => a,
            None => return GainBreakdown::default(),
        };
        if asset.price <= 0.0 || asset.qty() <= 0.0 {
            return GainBreakdown::default();
        }

        let shares_to_sell = amount_to_sell / asset.price;
        if shares_to_sell >= asset.qty() {
            return self.liquidate_asset(ticker, current_date);
        }

        let price = asset.price;
        let one_year_ago = subtract_one_year(current_date);
        let mut result = GainBreakdown::default();
        let mut shares_left = shares_to_sell;
        let mut remaining_lots: Vec<AssetLot> = Vec::new();

        for mut lot in std::mem::take(&mut asset.lots) {
            if shares_left <= 0.0 {
                remaining_lots.push(lot);
                continue;
            }
            if lot.qty <= shares_left {
                // Sell this entire lot
                let proceeds = lot.qty * price;
                let gain = proceeds - lot.basis;
                result.proceeds += proceeds;
                if lot.purchase_date > one_year_ago {
                    result.short_term_gain += gain;
                } else {
                    result.long_term_gain += gain;
                }
                shares_left -= lot.qty;
            } else {
                // Sell a fraction of this lot
                let fraction = shares_left / lot.qty;
                let lot_proceeds = shares_left * price;
                let basis_sold = lot.basis * fraction;
                let gain = lot_proceeds - basis_sold;
                result.proceeds += lot_proceeds;
                if lot.purchase_date > one_year_ago {
                    result.short_term_gain += gain;
                } else {
                    result.long_term_gain += gain;
                }
                lot.qty -= shares_left;
                lot.basis -= basis_sold;
                shares_left = 0.0;
                remaining_lots.push(lot);
            }
        }

        // Restore asset's lot list
        if let Some(asset) = self.assets.get_mut(ticker) {
            asset.lots = remaining_lots;
        }
        result
    }

    /// Liquidate all shares of a single asset. Removes it from the account.
    pub fn liquidate_asset(&mut self, ticker: &str, current_date: NaiveDate) -> GainBreakdown {
        let asset = match self.assets.remove(ticker) {
            Some(a) => a,
            None => return GainBreakdown::default(),
        };
        let price = asset.price;
        let one_year_ago = subtract_one_year(current_date);
        let mut result = GainBreakdown::default();
        for lot in &asset.lots {
            let proceeds = lot.qty * price;
            let gain = proceeds - lot.basis;
            result.proceeds += proceeds;
            if lot.purchase_date > one_year_ago {
                result.short_term_gain += gain;
            } else {
                result.long_term_gain += gain;
            }
        }
        result
    }

    /// Liquidate all assets in the account.
    pub fn liquidate_all(&mut self, current_date: NaiveDate) -> GainBreakdown {
        let tickers: Vec<String> = self.assets.keys().cloned().collect();
        let mut total = GainBreakdown::default();
        for ticker in tickers {
            let g = self.liquidate_asset(&ticker, current_date);
            total.proceeds += g.proceeds;
            total.short_term_gain += g.short_term_gain;
            total.long_term_gain += g.long_term_gain;
        }
        total
    }
}

/// Returns the date exactly one year before `date`, handling leap years gracefully.
/// Mirrors Python's `date - relativedelta(years=1)`.
fn subtract_one_year(date: NaiveDate) -> NaiveDate {
    // Use checked arithmetic; fall back to same-month last day on Feb 29 → Feb 28.
    NaiveDate::from_ymd_opt(date.year() - 1, date.month(), date.day())
        .or_else(|| NaiveDate::from_ymd_opt(date.year() - 1, date.month(), date.day() - 1))
        .unwrap_or(date)
}

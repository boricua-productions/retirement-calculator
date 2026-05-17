use chrono::{Datelike, NaiveDate};

use crate::models::real_estate::{HelocLine, MortgageCurrency, MortgageTerms, RealEstateHolding, StructureType, PropertyType};

// ─── Mortgage amortization ───────────────────────────────────────────────────

/// Standard fixed-rate mortgage monthly P&I payment.
///
/// Formula: M = P × r(1+r)^n / ((1+r)^n − 1)
///   P = original_principal, r = annual_rate/12, n = term_months
///
/// Returns 0.0 if term_months is 0.  When annual_rate is 0 (interest-free),
/// returns equal principal installments: P / n.
pub fn monthly_pi_payment(terms: &MortgageTerms) -> f64 {
    let n = terms.term_months as f64;
    if n <= 0.0 { return 0.0; }
    let r = terms.annual_rate / 12.0;
    if r <= 0.0 {
        return terms.original_principal / n;
    }
    let factor = (1.0 + r).powf(n);
    terms.original_principal * r * factor / (factor - 1.0)
}

/// Outstanding principal balance after `elapsed_months` payments.
///
/// Formula: B(k) = P × ((1+r)^n − (1+r)^k) / ((1+r)^n − 1)
///   k = elapsed months, clamped to [0, n].
pub fn mortgage_balance(terms: &MortgageTerms, elapsed_months: u32) -> f64 {
    let n = terms.term_months as f64;
    let k = (elapsed_months as f64).min(n);
    if n <= 0.0 || k >= n { return 0.0; }
    let r = terms.annual_rate / 12.0;
    if r <= 0.0 {
        return terms.original_principal * (1.0 - k / n);
    }
    let factor_n = (1.0 + r).powf(n);
    let factor_k = (1.0 + r).powf(k);
    terms.original_principal * (factor_n - factor_k) / (factor_n - 1.0)
}

/// Number of full months elapsed from mortgage start to simulation_date.
pub fn elapsed_months(terms: &MortgageTerms, simulation_date: NaiveDate) -> u32 {
    let start = terms.start_date;
    if simulation_date <= start { return 0; }
    let year_diff  = simulation_date.year()  - start.year();
    let month_diff = simulation_date.month() as i32 - start.month() as i32;
    let total = year_diff * 12 + month_diff;
    total.max(0) as u32
}

/// Outstanding mortgage balance in the loan's native currency at `simulation_date`.
pub fn mortgage_balance_at_date(terms: &MortgageTerms, simulation_date: NaiveDate) -> f64 {
    mortgage_balance(terms, elapsed_months(terms, simulation_date))
}

// ─── Property tax ────────────────────────────────────────────────────────────

/// Monthly property tax in JPY (annual_property_tax_jpy ÷ 12).
pub fn monthly_property_tax_jpy(holding: &RealEstateHolding) -> f64 {
    holding.annual_property_tax_jpy / 12.0
}

/// Monthly property tax in USD (annual_property_tax_usd ÷ 12).
pub fn monthly_property_tax_usd(holding: &RealEstateHolding) -> f64 {
    holding.annual_property_tax_usd / 12.0
}

// ─── Rental income ──────────────────────────────────────────────────────────

/// Net monthly rental income in JPY:
///   gross_rent × (1 − vacancy) − annual_insurance/12 − repairs_pct × fmv/12
/// Returns 0.0 when no RentalProfile or monthly_rent_jpy == 0.
pub fn monthly_rental_net_jpy(holding: &RealEstateHolding) -> f64 {
    let rental = match &holding.rental { Some(r) => r, None => return 0.0 };
    if rental.monthly_rent_jpy <= 0.0 { return 0.0; }
    let gross     = rental.monthly_rent_jpy * (1.0 - rental.vacancy_pct);
    let insurance = rental.annual_insurance_jpy / 12.0;
    let repairs   = rental.annual_repairs_pct_fmv * holding.current_fmv_jpy / 12.0;
    (gross - insurance - repairs).max(0.0)
}

/// Net monthly rental income in USD.
/// Returns 0.0 when no RentalProfile or monthly_rent_usd == 0.
pub fn monthly_rental_net_usd(holding: &RealEstateHolding) -> f64 {
    let rental = match &holding.rental { Some(r) => r, None => return 0.0 };
    if rental.monthly_rent_usd <= 0.0 { return 0.0; }
    let gross     = rental.monthly_rent_usd * (1.0 - rental.vacancy_pct);
    let insurance = rental.annual_insurance_usd / 12.0;
    let repairs   = rental.annual_repairs_pct_fmv * holding.current_fmv_usd / 12.0;
    (gross - insurance - repairs).max(0.0)
}

// ─── HELOC ───────────────────────────────────────────────────────────────────

/// Maximum additional HELOC draw available in USD.
///
/// draw_available = min(credit_line − outstanding, ltv_cap × fmv_usd − mortgage_balance_usd)
/// Returns 0.0 if the HELOC is disabled or the LTV cap is already breached.
///
/// `fmv_usd` — current FMV in USD (convert JPY at current FX if needed).
/// `mortgage_balance_usd` — outstanding first-mortgage balance in USD.
/// `outstanding_heloc_usd` — total HELOC already drawn across all properties.
pub fn heloc_available_usd(
    line: &HelocLine,
    fmv_usd: f64,
    mortgage_balance_usd: f64,
    outstanding_heloc_usd: f64,
) -> f64 {
    if !line.enabled { return 0.0; }
    let equity_headroom = (line.ltv_cap * fmv_usd - mortgage_balance_usd).max(0.0);
    let credit_remaining = (line.credit_line_usd - outstanding_heloc_usd).max(0.0);
    equity_headroom.min(credit_remaining)
}

// ─── Depreciation ────────────────────────────────────────────────────────────

/// Annual tax depreciation in JPY — Japan declining-balance simplified to straight-line.
///
/// Useful lives (法定耐用年数):
///   Wood:               22 years
///   Reinforced Concrete: 47 years
///   Steel:              34 years (light steel)
///   Other:              22 years (conservative)
///
/// Land is assumed non-depreciable at 10% of purchase price;
/// building (90%) is straight-line over the useful life.
pub fn annual_depreciation_jpy(holding: &RealEstateHolding) -> f64 {
    if holding.purchase_price_jpy <= 0.0 { return 0.0; }
    let life: f64 = match holding.structure_type {
        StructureType::Wood               => 22.0,
        StructureType::ReinforcedConcrete => 47.0,
        StructureType::Steel              => 34.0,
        StructureType::Other              => 22.0,
    };
    holding.purchase_price_jpy * 0.90 / life
}

/// Annual tax depreciation in USD — US MACRS straight-line.
///
/// Residential rental (§168): 27.5 years
/// Non-residential (§168):     39.0 years
///
/// Land assumed non-depreciable at 20% of purchase price;
/// building (80%) is straight-line over the MACRS life.
pub fn annual_depreciation_usd(holding: &RealEstateHolding) -> f64 {
    if holding.purchase_price_usd <= 0.0 { return 0.0; }
    let life: f64 = match holding.property_type {
        PropertyType::Rental => 27.5,
        _                    => 39.0,
    };
    holding.purchase_price_usd * 0.80 / life
}

// ─── Portfolio aggregators ────────────────────────────────────────────────────

/// Total monthly real-estate expense in JPY across all holdings:
///   (property_tax_jpy + PI_jpy) + (property_tax_usd + PI_usd) × fx
///
/// Mortgage PI is zeroed once the loan term is fully amortized.
pub fn total_monthly_re_expense_jpy(
    holdings: &[RealEstateHolding],
    simulation_date: NaiveDate,
    fx: f64,
) -> f64 {
    holdings.iter().map(|h| {
        let tax_jpy = monthly_property_tax_jpy(h);
        let tax_usd = monthly_property_tax_usd(h) * fx;

        let pi_jpy = h.mortgage.as_ref().map(|m| {
            let elapsed = elapsed_months(m, simulation_date);
            if elapsed >= m.term_months { return 0.0; }
            let pi = monthly_pi_payment(m);
            match m.currency {
                MortgageCurrency::Jpy => pi,
                MortgageCurrency::Usd => pi * fx,
            }
        }).unwrap_or(0.0);

        tax_jpy + tax_usd + pi_jpy
    }).sum()
}

/// Total net monthly rental income in JPY across all Japan holdings.
pub fn total_monthly_rental_jpy(holdings: &[RealEstateHolding]) -> f64 {
    holdings.iter().map(|h| monthly_rental_net_jpy(h)).sum()
}

/// Total net monthly rental income in USD across all US / international holdings.
pub fn total_monthly_rental_usd(holdings: &[RealEstateHolding]) -> f64 {
    holdings.iter().map(|h| monthly_rental_net_usd(h)).sum()
}

/// Total real-estate equity in JPY (sum of Japan-property FMVs minus JPY mortgages).
/// `outstanding_heloc_usd` is deducted (converted at `fx`) from the equity total.
pub fn total_equity_jpy(
    holdings: &[RealEstateHolding],
    simulation_date: NaiveDate,
    outstanding_heloc_usd: f64,
    fx: f64,
) -> f64 {
    let fmv: f64 = holdings.iter().map(|h| h.current_fmv_jpy).sum();
    let mortgages: f64 = holdings.iter()
        .filter_map(|h| h.mortgage.as_ref().filter(|m| m.currency == MortgageCurrency::Jpy))
        .map(|m| mortgage_balance_at_date(m, simulation_date))
        .sum();
    (fmv - mortgages - outstanding_heloc_usd * fx).max(0.0)
}

/// Total real-estate equity in USD (sum of US-property FMVs minus USD mortgages).
pub fn total_equity_usd(
    holdings: &[RealEstateHolding],
    simulation_date: NaiveDate,
    outstanding_heloc_usd: f64,
) -> f64 {
    let fmv: f64 = holdings.iter().map(|h| h.current_fmv_usd).sum();
    let mortgages: f64 = holdings.iter()
        .filter_map(|h| h.mortgage.as_ref().filter(|m| m.currency == MortgageCurrency::Usd))
        .map(|m| mortgage_balance_at_date(m, simulation_date))
        .sum();
    (fmv - mortgages - outstanding_heloc_usd).max(0.0)
}

/// Maximum HELOC draw available across all holdings, in USD.
/// Used in the Tier 7.5 waterfall step to size the draw.
pub fn total_heloc_available_usd(
    holdings: &[RealEstateHolding],
    simulation_date: NaiveDate,
    outstanding_heloc_usd: f64,
    fx: f64,
) -> f64 {
    holdings.iter().map(|h| {
        let line = match &h.heloc { Some(l) if l.enabled => l, _ => return 0.0 };
        let fmv_usd = if h.current_fmv_usd > 0.0 {
            h.current_fmv_usd
        } else {
            h.current_fmv_jpy / fx.max(1.0)
        };
        let mort_balance_usd = h.mortgage.as_ref().map(|m| {
            let bal = mortgage_balance_at_date(m, simulation_date);
            match m.currency {
                MortgageCurrency::Usd => bal,
                MortgageCurrency::Jpy => bal / fx.max(1.0),
            }
        }).unwrap_or(0.0);
        heloc_available_usd(line, fmv_usd, mort_balance_usd, outstanding_heloc_usd)
    }).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use crate::models::real_estate::{MortgageCurrency, MortgageTerms, RealEstateHolding,
        RentalProfile, HelocLine, PropertyLocation, PropertyType, StructureType};

    fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn sample_terms(principal: f64, rate: f64, months: u32, start: NaiveDate) -> MortgageTerms {
        MortgageTerms { original_principal: principal, annual_rate: rate, term_months: months, start_date: start, currency: MortgageCurrency::Jpy }
    }

    // ── Amortization unit tests ──────────────────────────────────────────────

    #[test]
    fn test_monthly_pi_payment_known_reference() {
        // 30M JPY, 1% annual, 360 months.
        // Standard formula: M = P×r×(1+r)^n / ((1+r)^n − 1)
        // r = 0.01/12, n = 360.  Expected ≈ ¥96,491 (varies by calculator precision).
        let terms = sample_terms(30_000_000.0, 0.01, 360, iso(2010, 1, 1));
        let payment = monthly_pi_payment(&terms);
        // Verify the payment is positive and in the correct range.
        assert!(payment > 96_000.0 && payment < 97_000.0,
            "PI payment {payment:.2} is outside the expected ¥96,000–¥97,000 range");
        // Verify total payments exceed principal (interest paid over life of loan).
        let total_paid = payment * 360.0;
        assert!(total_paid > 30_000_000.0,
            "Total payments {total_paid:.0} should exceed principal ¥30M");
    }

    #[test]
    fn test_monthly_pi_payment_interest_free() {
        // Zero-rate mortgage: should return P/n.
        let terms = sample_terms(12_000_000.0, 0.0, 240, iso(2010, 1, 1));
        let payment = monthly_pi_payment(&terms);
        assert!((payment - 50_000.0).abs() < 0.01);
    }

    #[test]
    fn test_mortgage_balance_fully_amortized() {
        // After all payments are made, balance should be ≈ 0.
        let terms = sample_terms(30_000_000.0, 0.01, 360, iso(2010, 1, 1));
        let bal = mortgage_balance(&terms, 360);
        assert!(bal < 1.0, "Balance after full term should be ~0, got {bal:.2}");
    }

    #[test]
    fn test_mortgage_balance_at_midpoint() {
        // After 180 months (half term) the balance should be > P/2 for a positive-rate loan
        // (interest front-loads so you haven't paid off half the principal yet).
        let terms = sample_terms(30_000_000.0, 0.01, 360, iso(2010, 1, 1));
        let bal = mortgage_balance(&terms, 180);
        assert!(bal > 15_000_000.0, "Balance at midpoint ({bal:.0}) should exceed P/2");
    }

    #[test]
    fn test_elapsed_months_calculation() {
        let terms = sample_terms(1.0, 0.0, 1, iso(2010, 1, 1));
        assert_eq!(elapsed_months(&terms, iso(2010, 1, 1)), 0);
        assert_eq!(elapsed_months(&terms, iso(2010, 7, 1)), 6);
        assert_eq!(elapsed_months(&terms, iso(2020, 1, 1)), 120);
    }

    #[test]
    fn test_heloc_available_ltv_cap() {
        let line = HelocLine { credit_line_usd: 500_000.0, draw_rate: 0.06, ltv_cap: 0.80, enabled: true };
        // FMV $1M, mortgage $600k → max new equity draw = 0.80*1M - 600k = $200k
        let avail = heloc_available_usd(&line, 1_000_000.0, 600_000.0, 0.0);
        assert!((avail - 200_000.0).abs() < 0.01);
    }

    #[test]
    fn test_heloc_available_credit_cap() {
        let line = HelocLine { credit_line_usd: 100_000.0, draw_rate: 0.06, ltv_cap: 0.80, enabled: true };
        // Equity allows $200k but credit line is $100k.
        let avail = heloc_available_usd(&line, 1_000_000.0, 600_000.0, 0.0);
        assert!((avail - 100_000.0).abs() < 0.01);
    }

    #[test]
    fn test_heloc_disabled() {
        let line = HelocLine { credit_line_usd: 500_000.0, draw_rate: 0.06, ltv_cap: 0.80, enabled: false };
        assert_eq!(heloc_available_usd(&line, 1_000_000.0, 0.0, 0.0), 0.0);
    }

    #[test]
    fn test_monthly_rental_net_jpy() {
        let h = RealEstateHolding {
            name: "test".into(),
            location: PropertyLocation::Japan,
            property_type: PropertyType::Rental,
            structure_type: StructureType::ReinforcedConcrete,
            purchase_date: None,
            purchase_price_jpy: 50_000_000.0,
            purchase_price_usd: 0.0,
            current_fmv_jpy: 50_000_000.0,
            current_fmv_usd: 0.0,
            annual_property_tax_jpy: 850_000.0,
            annual_property_tax_usd: 0.0,
            mortgage: None,
            heloc: None,
            reverse_mortgage: None,
            rental: Some(RentalProfile {
                monthly_rent_jpy: 150_000.0,
                monthly_rent_usd: 0.0,
                vacancy_pct: 0.05,
                annual_insurance_jpy: 60_000.0,
                annual_insurance_usd: 0.0,
                annual_repairs_pct_fmv: 0.01,
            }),
        };
        let net = monthly_rental_net_jpy(&h);
        // gross = 150k×0.95 = 142,500 − insurance 5,000 − repairs 41,667 ≈ 95,833
        assert!(net > 90_000.0 && net < 100_000.0, "rental net {net:.0} outside expected range");
    }
}

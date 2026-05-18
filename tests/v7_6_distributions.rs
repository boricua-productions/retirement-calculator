//! V7.6 Granular Distribution & Basket-FTC Regression Tests
//!
//! Five tests covering the V7.6 surgical refactor:
//!   1. ROC reduces basis proportionally across FIFO lots.
//!   2. ROC above total basis returns the excess as LTCG magnitude.
//!   3. PFIC §1296 capital-gains distributions emit `is_pfic_mtm = true`.
//!   4. Expense-ratio drag is automatically applied via effective_cap_growth.
//!   5. Basket-aware FTC caps passive Japan tax at the §904 passive limit
//!      so the credit cannot bleed into the general (FERS/SS) basket.

use chrono::NaiveDate;

use retirement_calculator::handlers::dividends::{
    collect_distribution_events, DistributionComponent,
};
use retirement_calculator::models::assets::{
    Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, AssetClass, Currency,
    DetailedReturnProfile, DividendCurrency, PficRegime,
};
use retirement_calculator::models::config::TaxRules;
use retirement_calculator::engine::tax::us_tax::TaxEngine;

fn iso(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// Build a single-asset Taxable account for distribution tests.
fn taxable_with(asset: Asset) -> Account {
    let mut acc = Account::new_with_meta(
        "Taxable", Currency::Usd, AccountLocation::Us, AccountJurisdiction::Both,
    );
    acc.assets.insert(asset.ticker.clone(), asset);
    acc
}

fn base_asset(ticker: &str) -> Asset {
    Asset {
        ticker: ticker.into(),
        price: 100.0,
        yield_rate: 0.0,
        growth_rate: 0.0,
        currency: Currency::Usd,
        category: AssetCategory::Income,
        drip_enabled: false,
        dividend_reinvest_target: None,
        custom_growth_rate: None,
        avg_jpy_basis_per_share: 0.0,
        dividend_months: vec![3, 6, 9, 12],
        dividend_currency: DividendCurrency::Usd,
        pfic_regime: PficRegime::NotPfic,
        pfic_prior_year_fmv_per_share: 0.0,
        pfic_prior_year_fmv_per_share_jpy: 0.0,
        pfic_mtm_loss_carryforward_usd: 0.0,
        pfic_qef_election_year: None,
        asset_class: AssetClass::default(),
        return_profile: None,
        crypto_staking_apr: 0.0,
        lots: Vec::new(),
    }
}

// ── 1. ROC reduces basis proportionally ──────────────────────────────────────
#[test]
fn roc_reduces_basis_proportionally() {
    let mut a = base_asset("ROC1");
    // Two lots: $5,000 basis and $20,000 basis (total $25,000).
    a.add_lot(iso(2020, 1, 1), 100.0,  5_000.0);
    a.add_lot(iso(2021, 1, 1), 200.0, 20_000.0);

    // ROC of $5,000 → ratio = 5,000 / 25,000 = 0.20 → each lot reduced by 20%.
    let excess = a.apply_roc_basis_reduction(5_000.0, 150.0);

    assert!((excess - 0.0).abs() < 1e-9, "no excess when ROC <= total basis");
    assert!((a.lots[0].basis -  4_000.0).abs() < 1e-6, "lot 0 basis = 4000");
    assert!((a.lots[1].basis - 16_000.0).abs() < 1e-6, "lot 1 basis = 16000");
    let total: f64 = a.lots.iter().map(|l| l.basis).sum();
    assert!((total - 20_000.0).abs() < 1e-6, "total basis = 25,000 - 5,000");
}

// ── 2. ROC above total basis becomes LTCG excess ─────────────────────────────
#[test]
fn roc_excess_becomes_ltcg() {
    let mut a = base_asset("ROC2");
    a.add_lot(iso(2020, 1, 1), 10.0, 1_000.0); // total basis = $1,000

    // ROC of $1,500 → all basis absorbed, $500 excess returned.
    let excess = a.apply_roc_basis_reduction(1_500.0, 150.0);

    assert!((excess - 500.0).abs() < 1e-6, "excess = ROC - basis = 500, got {}", excess);
    let total: f64 = a.lots.iter().map(|l| l.basis).sum();
    assert!(total.abs() < 1e-6, "basis fully absorbed → 0, got {}", total);
}

// ── 3. PFIC §1296 cap-gains-dist routes through is_pfic_mtm flag ─────────────
#[test]
fn cap_gains_dist_routes_to_pfic_ordinary_when_mtm_flagged() {
    let mut a = base_asset("PFICFUND");
    a.pfic_regime = PficRegime::Mtm;
    a.asset_class = AssetClass::MutualFund;
    a.return_profile = Some(DetailedReturnProfile {
        cap_growth: 0.05,
        dividend_yield: 0.02,
        cap_gains_dist: 0.04,  // mutual-fund pass-through
        ..Default::default()
    });
    a.add_lot(iso(2020, 1, 1), 100.0, 10_000.0);

    let acc = taxable_with(a);
    // March is in dividend_months [3,6,9,12].
    let events = collect_distribution_events(&acc, 3);

    let cgd = events.iter()
        .find(|e| e.component == DistributionComponent::CapGainsDist)
        .expect("CapGainsDist event must fire for PFIC fund");
    assert!(cgd.is_pfic_mtm, "PFIC §1296 fund must propagate is_pfic_mtm=true");

    let div = events.iter()
        .find(|e| e.component == DistributionComponent::Dividend)
        .expect("Dividend event must fire alongside CGD");
    assert!(div.is_pfic_mtm,
        "is_pfic_mtm flag is per-asset, so the dividend event also carries it");

    // And the gross math: cgd at 4% / 4 paying months × ($100 × 100sh) = $100.
    assert!((cgd.gross - 100.0).abs() < 1e-6, "cgd gross = $100/quarter, got {}", cgd.gross);
}

// ── 4. Expense-ratio drag compounds monthly via effective_cap_growth ─────────
#[test]
fn expense_ratio_drag_compounds_monthly() {
    // Profile asset: cap_growth=10%, expense_ratio=2% → net = 8%.
    let mut profile_asset = base_asset("PROF");
    profile_asset.return_profile = Some(DetailedReturnProfile {
        cap_growth: 0.10,
        expense_ratio: 0.02,
        ..Default::default()
    });

    // Legacy asset (no profile): growth_rate=10% → unmodified by expense ratio.
    let mut legacy_asset = base_asset("LEG");
    legacy_asset.growth_rate = 0.10;

    assert!((profile_asset.effective_cap_growth() - 0.08).abs() < 1e-9,
        "profile asset effective_cap_growth = 10% - 2% = 8%");
    assert!((legacy_asset.effective_cap_growth() - 0.10).abs() < 1e-9,
        "legacy asset effective_cap_growth = 10% (no profile)");

    // After 12 months of compounding, the profile asset must lag the legacy asset.
    profile_asset.price = 100.0;
    legacy_asset.price  = 100.0;
    for _ in 0..12 {
        profile_asset.grow();
        legacy_asset.grow();
    }
    // Profile: 100 * 1.08 = 108.00 (within float rounding).
    // Legacy:  100 * 1.10 = 110.00.
    assert!((profile_asset.price - 108.0).abs() < 0.01,
        "profile price after 12 months ≈ 108, got {}", profile_asset.price);
    assert!((legacy_asset.price - 110.0).abs() < 0.01,
        "legacy price after 12 months ≈ 110, got {}", legacy_asset.price);
    assert!(profile_asset.price < legacy_asset.price,
        "expense ratio must drag profile below legacy");
}

// ── 5. Basket-FTC does not leak passive credit to general basket ─────────────
#[test]
fn basket_ftc_does_not_leak_pfic_credit_to_general() {
    let engine = TaxEngine::new(TaxRules::default());
    // FERS (general): $50k. PFIC §1296 ordinary (passive): $30k. No cap gains.
    // Japan tax: $100k all passive (transactional CG-style), $0 general.
    // §904 passive limit will cap the credit far below $100k since the
    // passive share of income is only 30k / 80k ≈ 37.5%.
    use retirement_calculator::engine::tax::us_tax::UsTaxInput;
    let input = UsTaxInput {
        year: 2024,
        gross_ord_general: 50_000.0,   // general_ord (FERS)
        gross_ord_passive: 30_000.0,   // passive_ord (PFIC §1296)
        gross_st_cap: 0.0,              // stcg
        gross_lt_cap: 0.0,              // ltcg
        japan_tax_passive_usd: 100_000.0,  // japan_tax_passive_usd (huge)
        japan_tax_general_usd: 0.0,     // japan_tax_general_usd
    };
    let lib = engine.calculate_liability_with_basket_ftc(&input);

    // Same scenario via the lumped FTC: $100k Japan tax against $80k total income.
    let lumped = engine.calculate_liability_with_ftc(
        2024, 80_000.0, 0.0, 0.0, 100_000.0,
    );

    // The basket-aware result must apply STRICTLY LESS FTC than the lumped
    // version — that's the no-leak guarantee. The lumped version lets passive
    // Japan tax fully cap against the entire federal liability; basket-aware
    // restricts it to the passive share.
    assert!(
        lib.ftc_applied < lumped.ftc_applied - 1.0,
        "basket-FTC must apply strictly less credit than lumped FTC \
         (basket={:.2}, lumped={:.2})",
        lib.ftc_applied, lumped.ftc_applied,
    );

    // And the basket-FTC's `ftc_passive` must equal min(japan_tax_passive, passive_limit).
    // Passive limit = federal_before_ftc × (passive_income / total_income).
    let passive_ftc = lib.breakdown.get("ftc_passive").copied().unwrap_or(0.0);
    assert!(passive_ftc > 0.0, "some passive FTC must apply");
    assert!(passive_ftc < 100_000.0,
        "passive FTC capped below the $100k available; got {}", passive_ftc);

    let general_ftc = lib.breakdown.get("ftc_general").copied().unwrap_or(0.0);
    assert!(general_ftc.abs() < 1e-6,
        "no general FTC since japan_tax_general_usd = 0; got {}", general_ftc);
}

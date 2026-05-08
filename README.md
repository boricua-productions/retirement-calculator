# Retirement Calculator — V7.0 Technical Manual with V7.1 Waterfall Updates

A desktop tool for modeling the financial future of **US expats and retirees living in Japan**.
It is designed for non-SOFA residents under standard Japanese immigration status, such as work,
spouse, long-term resident, or permanent resident visas.

> **Dividend Focus.** The engine is heavily focused on **living off portfolio income before
> selling stock**. In the current default V7.1 waterfall, native JPY income and the JPY war chest
> are used first, USD income and bridge cash are converted with an FX spread penalty, spending may
> drop from base to minimum, and equity liquidation is the last resort. Every snapshot reports a
> Dividend Coverage Ratio so you can tell, year by year, whether your portfolio is self-funding.

The engine assumes ordinary Japan resident tax and National Health Insurance exposure unless a
scenario explicitly changes the tax settings. It is not a SOFA, TRICARE, base-access, or
tax-exempt military model, though optional VA, FERS, and military retired-pay income streams are
available for people who need them.

The model follows a **Japan-first tax flow**: it estimates Japan resident tax first, then uses the
Foreign Tax Credit (FTC) to reduce US federal tax on the same income where the treaty and US tax
rules allow it. In plain terms, the goal is to show how the two tax systems interact without
double-counting the same income.

> **Version:** Cargo package `7.0.0`; README documents the V7.0 engine plus V7.1 waterfall fields
> **Build:** ~8.5 MB release binary (LTO fat, `opt-level = 3`, symbols stripped)  
> **Tests:** 66/66 passing — adds the V7.0 integration suite
> `tests/v7_tax_and_liquidation_test.rs` covering state-tax gross-up,
> highest-JPY-basis-first ordering, and DividendOnly short-circuit behaviour.

---

## Beginner Quick Start

If you are new to the project, start with the app and template before reading the full technical
manual.

1. Install Rust stable, then run `cargo run --release`.
2. Open `input/TEMPLATE_scenario.json` from the toolbar.
3. Edit only the basics first: dates, expenses, USD/JPY, portfolio holdings, pension income, and
   VA/FERS/SS/Nenkin fields that apply to you.
4. Run the baseline simulation.
5. Review the Overview tab first, then the Annual Table and CSV output.

The rest of this README is a technical reference. It explains the tax and cashflow assumptions
behind the model, but you do not need to understand every section to run a first scenario.

---

## How To Use The Software

The normal workflow is:

1. Open a scenario JSON file from the toolbar.
2. Review or edit values in **Input Config**.
3. Save the configuration if you changed anything.
4. Run the baseline simulation.
5. Read the **Overview** tab first, then inspect the **Annual Table**, **Charts**, **RSU Schedule**,
   and **Transition** tabs.
6. Export a text report or audit CSV from the **Overview** tab when you want to keep results.

The Input Config tab is the main control surface. Fields accept plain numbers unless the label
says otherwise. For optional income streams, use `0` when that income does not apply.

### Input Config Field Guide

#### Timing

| Field | What it is for |
|-------|----------------|
| **Start Date** | First month of the simulation. Use the date you want the model to begin tracking assets, income, and expenses. |
| **End Date** | Last month of the simulation. A 40- to 50-year horizon is useful for retirement stress testing. |
| **Retirement Date** | First month with no employment salary. This switches the model into retirement cashflow mode. |
| **Rebalance Date** | Month when the retirement transition event runs. It must be on or after the retirement date. |

#### Economics

| Field | What it is for |
|-------|----------------|
| **USD/JPY Rate** | Starting exchange rate. Use `0` only when you want the built-in fallback rate. |
| **US Inflation / CPI** | Inflates US tax brackets and US COLA-linked income. |
| **Japan Inflation / CPI** | Inflates JPY expenses, NHI per-capita assumptions, and Nenkin income. |
| **Enable FX Drift** | Turns on post-retirement USD/JPY movement. |
| **FX Drift Rate** | Legacy continuous annual drift. Positive values make USD/JPY decline over time. |
| **Cadence Months** | Uses step-based FX drift instead of continuous drift when greater than `0`. |
| **Increase Amount (JPY)** | Amount added to USD/JPY at each cadence. Positive weakens yen; negative strengthens yen. |

#### Monthly Expenses

| Field | What it is for |
|-------|----------------|
| **Base Monthly** | Desired household spending before NHI, resident tax, and Nenkin contribution add-ons. |
| **Minimum Monthly** | Reduced spending floor used when the defensive waterfall cannot fund base spending. |
| **NHI Spike Monthly** | Legacy/manual first-year NHI estimate. The calculated NHI engine handles active NHI scheduling when configured. |
| **Nenkin Monthly Household** | Household pension contribution expense. Only the amount above the embedded baseline is added separately. |

#### NHI Settings

| Field | What it is for |
|-------|----------------|
| **Calculated / Manual Override** | `Calculated` estimates NHI from municipal rate cards and prior-year income. `Manual Override` uses your own annual totals. |
| **Medical rate / Support rate / Nursing care rate** | Income-based NHI rates. Nursing care applies for ages 40-64. |
| **Per-capita medical / support / nursing** | Fixed annual per-person NHI amounts. |
| **Medical / support / nursing annual cap** | Maximum annual charge for each NHI component. |
| **Include US Investment Income in NHI Base** | Adds US dividends to the NHI income basis for global-income resident modeling. |
| **Spike Year Annual Total** | Manual first post-retirement annual NHI total. |
| **Ongoing Annual Total** | Manual annual NHI total after the spike year. |

#### Tax Strategy, Filing, and Location

| Field | What it is for |
|-------|----------------|
| **US Tax Mitigation Strategy** | Choose `FTC Only` or `FEIE + FTC`. FTC-only credits Japan tax against US federal tax. FEIE+FTC excludes eligible work income first. |
| **Filing Status** | Determines US federal brackets, standard deduction, senior add-ons, and capital-gains thresholds. |
| **US State Residency** | Sets the state tax rate used for US state liability and V7.0 state-tax gross-up. |
| **Japan Prefecture / City** | Selects resident-tax rates. Most cities use standard rates; some have special rates. |
| **Global Tax Jurisdiction** | Controls whether the overall scenario applies both tax systems, US only, or Japan only. |

#### Investment Accounts

| Field | What it is for |
|-------|----------------|
| **Account Type** | Classifies an account, such as Taxable Brokerage, Roth IRA, DC Plan, NISA, or iDeCo. |
| **Jurisdiction** | Overrides tax treatment for that account: Both, US Only, Japan Only, or Tax Free. |
| **Ticker** | Asset symbol or fund label. |
| **Units** | Number of shares or fund units held. |
| **Auto-Fetch** | Pulls current price and 10-year CAGR when network data is available; falls back if unavailable. |
| **Price USD / Price JPY** | Current asset price used for market value. DC rows can use JPY pricing. |
| **Cost Basis** | Average USD cost basis per share/unit. Used for US gain calculations and JPY-basis fallback. |
| **Growth %** | Expected annual growth rate for deterministic simulations. |
| **Volatility %** | Expected annual volatility used by Marco Polo mode. |
| **DRIP** | Reinvest dividends instead of routing them to cash. |
| **Dividend Reinvest Target** | Optional ticker to receive reinvested dividends. |
| **Target Alloc %** | Desired allocation for periodic target-state rebalancing. |
| **Rebalance Date** | Optional per-position rebalance date that overrides the global rebalance date. |
| **Accum $/mo** | Scheduled monthly purchase amount during accumulation. |
| **Freq** | How often the scheduled purchase fires: monthly, quarterly, or annually. |
| **Stop at Retirement** | Stops scheduled buys once retirement begins. |

#### DC Plan Configuration

| Field | What it is for |
|-------|----------------|
| **Monthly Contribution (JPY)** | Monthly DC/iDeCo contribution during accumulation. |
| **Contribution Fund** | Fund label used for DC purchases. |
| **Allocation %** | Allocation weight for the DC fund row. |
| **Custom Growth %** | DC fund-specific annual growth assumption. |
| **DC Payout Method** | `LUMP_SUM` moves DC value into taxable at payout; `ANNUITY_20YR` pays monthly over 20 years. |
| **DC Payout Start Age** | Age when DC payout begins. |

#### Family Demographics

| Field | What it is for |
|-------|----------------|
| **User Birthday** | Drives age-based rules: FERS, Social Security, Nenkin, NHI nursing care, and senior deduction. |
| **Married** | Enables spouse demographics and spouse benefit fields. |
| **Spouse Birthday** | Gates spouse Social Security/Nenkin and second senior deduction. |
| **Dependent Child Birthday** | Drives VA child rider eligibility cutoffs. |
| **College Student** | Extends VA child rider eligibility to age 23 when applicable. |

#### VA Disability Profile

| Field | What it is for |
|-------|----------------|
| **Disability Rating** | Selects the official 2026 VA compensation table rate. `0%` disables VA income. |
| **Dependent Status** | Selects vet-only, spouse, or spouse-plus-child VA rate. |
| **Override VA Monthly Amount** | Bypasses the rating table with your own 2026-base monthly amount. |
| **Special Monthly Compensation** | Optional SMC variant added to VA compensation. |
| **Override SMC Monthly Amount** | Bypasses the SMC variant lookup with your own 2026-base amount. |

#### Pension and Benefit Income

| Field | What it is for |
|-------|----------------|
| **FERS Monthly** | Monthly FERS pension estimate in USD. Use `0` if not applicable. |
| **FERS Expected Start Age** | Age when FERS begins. |
| **Military Retired Monthly** | Monthly military retired pay in USD. Use `0` if not applicable. |
| **Social Security Monthly** | Primary US Social Security retirement estimate in USD. |
| **Social Security Start Age** | Age when primary Social Security starts. |
| **Spouse Social Security eligible** | Enables spouse Social Security fields. |
| **Spouse SS Monthly / Start Age** | Spouse Social Security amount and age gate. |
| **SSDI Monthly** | Social Security Disability Insurance monthly benefit. Use `0` if not applicable. |
| **Nenkin Monthly Income** | Japanese pension income received in retirement, separate from contribution expense. |
| **Nenkin Start Age** | Age when primary Nenkin income starts. |
| **Spouse Nenkin eligible** | Enables spouse Nenkin fields. |
| **Spouse Nenkin Monthly / Start Age** | Spouse Nenkin amount and age gate. |
| **Tax Jurisdiction** | Per-source override for FERS, military retired pay, Social Security, and Nenkin. |

#### Financial Buffers

| Field | What it is for |
|-------|----------------|
| **War Chest Target (JPY)** | Target JPY emergency reserve. The defensive waterfall taps this before USD bridge cash. |
| **War Chest Target (USD)** | Legacy USD target retained for compatibility. V7.1 treats the active war chest as JPY. |
| **Bridge Fund Months** | Target months of expenses to keep in USD bridge cash. |
| **Pre-Funded War Chest (JPY)** | Starting JPY reserve balance. |
| **Pre-Funded Bridge (USD/JPY)** | Starting bridge cash balance. USD bridge is the active V7.1 operating reserve. |
| **Pre-Funded Japan Tax / US Tax** | Cash reserved at the start for known tax bills. |

#### Market Simulation

| Field | What it is for |
|-------|----------------|
| **Simulate Recession at Retirement** | Applies a one-time retirement-date portfolio shock. |
| **Recession Severity** | Size of the shock, such as `0.20` for a 20% drawdown. |
| **Scheduled Recessions** | Additional year/severity stress events. |
| **Marco Polo Mode** | Runs Monte Carlo-style portfolio paths and shows P10/P50/P90 results in Compare. |

#### RSU Settings

| Field | What it is for |
|-------|----------------|
| **RSU Tax Handling** | `SALARY` pays tax externally; `SELL_TO_COVER` sells vested shares to cover tax. |
| **Ticker** | Stock symbol for the RSU award. |
| **Grant Date** | Award grant date. |
| **Unvested** | Total shares still scheduled to vest. |
| **Frequency** | Monthly, quarterly, or annual vesting cadence. |
| **Vesting Months** | Optional explicit vesting months in the JSON. Overrides cadence defaults. |

---

## V7.0/V7.1 At A Glance — Cost Basis, Liquidation, and Cashflow

The current code combines the V7.0 liquidation engine with V7.1 cashflow controls.

V7.0 reframes the post-retirement liquidation engine around the **JPY value
paid at purchase**, not the USD basis. Two consequences:

1. **Japan-Resident Cost Basis.** Each position carries an explicit
   `avg_purchase_price_jpy` (¥/share). When that field is absent the runtime
   falls back to `avg_cost × usd_jpy_at_load` so legacy scenarios load
   unchanged. Japan capital-gains tax is then computed against this basis as
   `(price × fx − basis_jpy) × 20.315%`, producing the correct realised gain
   even when the yen has weakened by 50% since purchase.

2. **Highest-Basis-First Liquidation.** When the post-retirement cashflow
   waterfall cannot cover expenses, the engine sorts taxable holdings by JPY
   basis **descending** and sells the highest-basis lots first. The
   lowest-realised-gain shares leave the portfolio first, deferring the tax
   bill to later years and stretching the portfolio's longevity. The V7.0
   liquidation order is:

   ```
   (1) Monthly cash + dividends     ← funded from existing cashflow
   (2) Taxable, highest JPY basis   ← V7.0 minimises early-year realised gains
   (3) Taxable, lowest basis        ← natural fall-through after (2) is empty
   (4) Roth / advantaged            ← last resort, tax-free proceeds
   ```

   `WithdrawalStrategy::DividendOnly` short-circuits (2)–(4) entirely;
   `Hybrid` and `TotalReturn` both follow the full waterfall.

The new `us_state_tax_rate` dial sits **outside** the FTC pipeline. Japan
resident tax + Japan capital-gains tax both credit US federal liability via
the existing FTC pool, but state tax does not. The liquidation engine
therefore grosses up each share sale by the state rate and records it in
`StateCapGainsTax_USD`.

V7.1 adds the default **Defensive** spending waterfall:

1. JPY floor income: Nenkin and DC payout.
2. JPY dividends in the current dividend month.
3. JPY war chest.
4. USD floor income: FERS, VA, Social Security, SSDI, and military retired pay, converted to JPY.
5. USD dividends in the current dividend month, converted to JPY.
6. USD bridge fund, converted to JPY.
7. Belt-tightening from base monthly spending to the configured minimum.
8. Stock liquidation using the V7.0 highest-JPY-basis-first rule.

USD-to-JPY conversions in tiers 4, 5, 6, and 8 apply `fx_spread_penalty`
(`0.005` by default). The legacy V7.0-style `Cautious` waterfall is still
available with `withdrawal_waterfall: "cautious"`.

---

## What's New

| Version | Highlights |
|---------|------------|
| **V7.1** | **Defensive JPY-first spending waterfall** added via `withdrawal_waterfall` (`defensive` default, `cautious` for V7.0-compatible behaviour). USD-to-JPY conversions in the waterfall apply `fx_spread_penalty` (default `0.005`). Dividends are now lumpy by `dividend_months` instead of smoothed every month, and each asset can declare `dividend_currency` (`usd` or `jpy`) so income lands in the correct cash bucket. CSV output adds `FXPenalty_JPY` and `MonthsAtMin`. |
| **V7.0** | 🇯🇵 **Japan-Resident Cost Basis model** — `Position.avg_purchase_price_jpy` carries the JPY paid at purchase; `Asset.avg_jpy_basis_per_share` exposes the same value at runtime with `avg_cost × usd_jpy_at_load` as fallback. ⛏ **Highest-JPY-Basis-First liquidation** replaces the alphabetical sweep in `cashflow_manager::v7_liquidate_for_deficit` — taxable holdings are sorted DESC by JPY basis so the lowest-realised-gain shares leave the portfolio first. 🧾 **Japan capital-gains tax (20.315%)** is settled at sale and folded into the FTC pool via `stats.year_japan_cap_gains_tax_jpy`; the controller's `finalize_year_taxes` now combines resident tax + cap-gains tax into the Japan-side credit. 🇺🇸 **`us_state_tax_rate` dial** added to `Config` (mirrored to `tax_rules.us_state_rate` by the loader); state tax is grossed up at sale and surfaced in `stats.year_state_cap_gains_tax_usd`. 🎚 **`WithdrawalStrategy` enum** (`DividendOnly` / `TotalReturn` / `Hybrid`) — `DividendOnly` disables the waterfall entirely, leaving deficits as solvency warnings. 💬 **Jurisdictional tooltips** on Global Tax Jurisdiction, Source Tax Jurisdiction, and US State Tax explain treaty mechanics directly in the UI. 📊 Two new CSV columns: `JapanCapGainsTax_JPY`, `StateCapGainsTax_USD`. 🧪 New integration suite `tests/v7_tax_and_liquidation_test.rs` (3 tests). |
| **V6.6** | 💴 **Dividend-Focus header** prominently surfaced in the Input Configuration panel; 🇯🇵 **Noto Sans JP font** installed via `FontDefinitions` so Japanese tooltips ("NHI Spike", 公的年金等控除) render natively; ⏱ **Timing defaults** auto-populate to Start = today and End = today + 50 years; tooltips added to every Timing / Economics / Expense / Buffer field; **FX Drift moved under USD/JPY** in the Economics section and now supports `fx_drift_cadence_months` + `fx_drift_increase_amount_jpy` (every N months, FX jumps by a signed JPY amount); 👥 **Marriage toggle** + Spouse Birthday in Family Demographics; **Spouse SS** and **Spouse Nenkin** rendered inside their parent income sections with independent estimate / start age / jurisdiction; **Dependents now use full birth dates** (NaiveDate) for exact 18th/23rd birthday cutoffs; 📊 Position grid Ticker / Units / Price column widths **doubled** (108 / 140 / 164 px); 💴 **DC Plans now support per-fund position rows with JPY-denominated Units & Price** via a `dc_jpy_units` toggle; **per-position rebalance dates** (`Position.rebalance_date: Option<NaiveDate>`) supersede the global rebalance event when set; new `Position.recession_override: Option<f64>` reserved for per-position drawdown sensitivity; 4 new V6.6 inline tests + 3 integration stress tests verifying FX cadence math, spouse SS age gating, and per-position rebalance precedence |
| **V6.5** | 🔒 Mathematical hardening & compliance fixes: recovery boost rate guarded against `severity = 1.0` div-by-zero; `calculate_liability_with_feie_ftc` split into `gross_earned` (FEIE-eligible: salary / RSU vests) and `gross_unearned` (pension, Social Security, SSDI — not FEIE-eligible) per IRC §911; FTC apportionment denominator corrected to total Japan-taxable income (earned + unearned + gains) per IRC §904; 59/59 tests passing |
| **V6.4** | 👨‍👩‍👧 `Dependent` + `FamilyUnit` structs in Config; `ssdi_monthly_usd` field; VA child rider college-student extension to age 23; SSDI Combined Income tax rule (IRS Pub 915: 50%/85% tiers, MFJ thresholds $32K/$44K); IRS senior standard deduction add-on at age 65+ per qualifying person (MFJ: +$1,550, Single/HoH: +$1,950); Japan SSDI routed through 公的年金等控除 pension deduction; new "Family Demographics" and "SSDI" UI sections; 65/65 tests passing |
| **V6.3** | 🏛 Official 2026 VA compensation rates hardcoded (100% Vet Only: $3,938.58; 100% With Spouse: $4,158.17; 100% With Spouse+Child: $4,267.28); all 12 SMC variants updated to 2026 rates (K: $139.87 through R.2: $11,271.67); UI "(Using Official 2026 VA Rates)" label added; all VA/SMC unit tests updated |
| **V6.2** | 🗂 Input Panel refactored to a single unified `egui::Grid` (8 columns: Ticker, Units, ✨ Auto-Fetch, Price USD, Cost Basis, Growth %, ⚙ Management, ☐ Select/Delete); centered headers with `ui.with_layout`; `(opt.)` tags in muted style; per-cell `id_salt(("col", acct_idx, pos_idx))` on every `TextEdit` for strict ID uniqueness; management sub-panels relocated after the unified grid |
| **V6.1** | 🐞 Resolved egui widget ID clashes causing red "Second use of widget ID" artifacts when switching tabs; all `ScrollArea` widgets now carry explicit `id_salt` values; baseline tabs wrapped in `push_id("baseline_view")` namespace; comparison tab wrapped in `push_id("comparison_view")` namespace |
| **V6.0** | ⚙ Per-ticker Management sub-panel (expandable ⚙▾ button): Accumulation rules (monthly buy amount, frequency, stop-at-retirement), DRIP toggle + redirect ticker, Target Allocation %; ⚖ Target-State Rebalancing engine (quarterly/semi-annual/annual, sell overweight → buy underweight, 15% LTCG on taxable sells); 📊 Dividend Coverage Ratio metric (div gross JPY ÷ total expenses, shown per-year in Results Table + Overview aggregate, originally column 32 in the V6.0 CSV); new `accumulation_rules` and `target_allocations` JSON keys |
| **V5.9** | 🎲 Marco Polo (Monte Carlo) engine — 1,000 GBM iterations → P10/P50/P90 trajectories; per-position Volatility % field (replaces Growth % when active); 🔀 Dual-Scenario Comparison — load two JSON scenarios, run both, view side-by-side (Ending Wealth, NHI paid, solvency warnings); UI spacing between account cards; "Growth Rate Data" legacy section removed |
| **V5.8** | ✨ Auto-Calc button on every position ticker (live price + 10y CAGR from Yahoo Finance); "(opt.)" labels on optional position columns; DC Plan dual-currency stability note + configurable growth rate / "Use Market Average (10%)" toggle; `dc_growth_rate` wired through Config → contributions handler; RSU Grant Price field removed (engine computes vest value from current price × CAGR) |
| **V5.7** | Universal Japan NHI engine (`NhiModel` enum); `Calculated` mode with per-component rates + caps; `ManualOverride` mode; UI NHI Settings section; "Load Sagamihara 2026 Defaults" button; US investment income flag; dynamic NHI scheduling in controller (replaces static spike rule) |
| **V5.6** | `calculate_nhi_premiums` 1-year lookback utility; dynamic multi-account/position UI; DC Plan inline fields; multi-tranche RSU grant table |
| **V5.5** | `Vec<Position>` data model; Equity & Vesting UI section (stock table + RSU grant); Account Type selector; VA/SMC override toggles; Python debt purged |
| **V5.4** | `TaxProtocol` enum (`TaxFree` variant); `MilitaryRetiredConfig`; SMC 2026 rate table; FEIE/FTC true pipeline fix; ordinary income brackets before LTCG stacking |
| **V5.3** | Currency Shock Engine; purchasing-power audit; multi-month drawdown; historical 10-year CAGR table; FTC carry-over |
| **V5.2** | Multi-month drawdown engine; reinvestment suppression; FTC carry-over logic |
| **V5.1** | Nationwide Juminzei rates (all 47 prefectures); FEIE/FTC strategy support; VA, Social Security, and Nenkin income streams |
| **V5.0** | Nationwide Japan support; initial Technical Manual |

---

## Table of Contents

0. [How To Use The Software](#how-to-use-the-software)
1. [System Architecture](#1-system-architecture)
2. [Japan-First Tax Priority](#2-japan-first-tax-priority)
3. [US Tax Strategy Engine](#3-us-tax-strategy-engine)
4. [VA Disability Engine](#4-va-disability-engine)
5. [Retirement Income Streams](#5-retirement-income-streams)
6. [Portfolio & Market Logic](#6-portfolio--market-logic)
7. [RSU Vesting Engine](#7-rsu-vesting-engine)
8. [UI & Safety Gate — Marco Polo & Compare](#8-ui--safety-gate)
9. [Input Configuration Reference](#9-input-configuration-reference)
10. [Output & Reporting](#10-output--reporting)
11. [Build & Run](#11-build--run)
12. [Project Structure](#12-project-structure)
13. [Universal Japan NHI Support & Overrides](#13-universal-japan-nhi-support--overrides)
14. [Troubleshooting & UI Architecture](#14-troubleshooting--ui-architecture)
15. [Dependencies](#15-dependencies)
16. [Hardening & Compliance (V6.5)](#16-hardening--compliance-v65)

---

## 1. System Architecture

The app models your finances one month at a time from `start_date` to `end_date`. Every December,
it saves an annual snapshot so you can see income, expenses, taxes, insurance, portfolio value,
and cash reserves year by year. The same scenario file produces the same result every time.

```
JSON Scenario
     │
     ▼
config/loader.rs  ──→  Config + Accounts
     │
     ▼
SimulationController (simulation/controller.rs)
     │
     ├── CashFlowEngine      monthly income / expenses
     ├── TaxEngine (US)      ordinary income, capital gains, NIIT, FTC, FEIE
     ├── JapanTaxEngine      resident tax and income deductions
     ├── NhiEngine           city-rate NHI premiums and manual overrides
     ├── RsuEngine           vesting schedule
     ├── MarketDataService   live CAGR / fallback prices
     │
     ▼
SimResults ──→ reporter.rs ──→ output/
                               ├── Retirement_Summary.txt
                               └── simulation_data.csv
```

The Overview tab can also export a user-selected audit CSV named `simulation_audit.csv`.

### Core data flow per year (December true-up)

```
Japan resident tax (JPY)
        │  converted at current FX
        ▼
japan_tax_paid_usd  ──→  FTC credit against US federal liability
                                    │
                    FEIE path?  ────┤
                    FTC-only?   ────┘
                                    │
                                    ▼
                         year_us_fed_tax_usd  →  AnnualSnapshot
```

---

## 2. Japan-First Tax Priority

For a US resident of Japan, the order matters. Japan usually taxes Japan-resident income first;
then the US return may use the FTC to reduce US tax on income that Japan already taxed. The engine
models that sequence each December.

### Japan resident tax (住民税)

Computed via `JapanTaxEngine::calculate_resident_tax()`, with rates drawn from the
nationwide regional database (`src/engine/tax/japan_regions.rs`):

```
gross_pension_jpy = (FERS_annual_USD × current_fx) + nenkin_income_jpy
net_pension       = gross_pension_jpy − pension_deduction(age)
net_salary        = gross_salary_jpy  − employment_deduction()

taxable_basis     = net_pension + net_salary
                  − basic_deduction (¥430,000)
                  − spouse_deduction (¥330,000 per dependent if income ≤ ¥9M)
                  − social_insurance_paid

resident_tax = floor(taxable_basis / 1,000) × 1,000 × income_rate + per_capita_jpy
```

**Regional rate lookup** — `prefecture` and `city` in the scenario JSON control the Juminzei rate:

| Location | Income Rate | Per-Capita Levy | Notes |
|----------|-------------|-----------------|-------|
| All 47 prefectures (standard) | **10.0%** | ¥6,000/yr | 6% city + 4% prefecture + ¥1,000 forest env (FY2024+) |
| Nagoya City (Aichi) | **9.7%** | ¥6,000/yr | Reduced city portion (5.7%) |

The Input Config panel provides Prefecture and City dropdowns covering all 47 prefectures
and their major cities. Nagoya is annotated with its special rate in the UI.

Japan resident tax is scheduled quarterly in June / August / October / January of the
following year, matching the NTA payment calendar.

### NHI premium

National Health Insurance (NHI) is now calculated from the same kind of city-rate card used by
Japanese municipalities. That matters because your health insurance estimate depends mainly on:

- **Where you live** — each city can have different rates, per-person charges, and annual caps.
- **Your prior-year income** — NHI is assessed using the previous calendar year, so the first year
  after retirement can be unusually high if the prior year included salary or RSU income.
- **Your age** — the nursing-care component applies from ages 40 through 64.

In automatic mode, the engine calculates:

```text
NHI basis = max(0, net_salary + net_pension + optional_investment_income - ¥430,000)
```

Then it applies the municipality's three components:

| Component | Who it applies to | What drives it |
|-----------|-------------------|----------------|
| Medical | Everyone on NHI | Income-based rate + per-person charge, capped annually |
| Support | Everyone on NHI | Income-based rate + per-person charge, capped annually |
| Nursing care | Ages 40-64 only | Income-based rate + per-person charge, capped annually |

Manual mode is also available when you already know the exact annual NHI amounts from your city.

### Japan pension income deductions (公的年金等控除)

| Age | Full deduction threshold | Second-tier flat deduction |
|-----|--------------------------|---------------------------|
| < 65 | < ¥600,000 (full) | ¥600,000 for ¥600k–¥1.3M |
| ≥ 65 | < ¥1,100,000 (full) | ¥1,100,000 for ¥1.1M–¥3.3M |

---

## 3. US Tax Strategy Engine

### Capital gains brackets (2024 MFJ base, inflated annually by `inflation_us_cpi`)

| Bracket | Rate | 2024 MFJ threshold |
|---------|------|---------------------|
| 0% LTCG | 0% | Stacked income ≤ $115,000 |
| 15% LTCG | 15% | ≤ $700,000 |
| 20% LTCG | 20% | > $700,000 |
| NIIT | 3.8% | MAGI > $250,000 |
| State | varies | see `us_state_code` table |

> Ordinary income is used as a **bracket floor** for capital gains stacking: gains are taxed at
> the rate of the first available bracket above the floor. Ordinary income splits into **work
> income** and **retirement / benefit income**. FEIE can reduce work income such as salary and RSU
> vest value, but it does not reduce FERS, Social Security Retirement, taxable SSDI, pensions,
> dividends, or capital gains.

### Strategy toggle: `us_tax_strategy`

| Value | Behaviour |
|-------|-----------|
| `ftc_only` *(default)* | Full Japan resident tax credited against US federal liability. No exclusion applied. |
| `feie_and_ftc` | Applies FEIE to eligible work income first, then applies FTC to the remaining eligible Japan-taxed income. |

**Plain English:** FEIE is for work income earned abroad, such as salary and RSU vesting income.
It is not for pension or benefit income. FERS, Social Security Retirement, and taxable SSDI stay in
the US taxable-income stack, and Japan taxes paid on overlapping income are handled through the FTC.

### FEIE + FTC path (`feie_and_ftc`)

Implemented in `src/engine/tax/us_tax.rs:calculate_liability_with_feie_ftc()`.

```text
# Step 1 — FEIE exclusion (work income only, IRC §911)
feie_exclusion       = min(gross_earned, $126,500)          ← 2026 IRS limit
earned_after_feie    = gross_earned - feie_exclusion
total_ord_after_feie = earned_after_feie + gross_unearned   ← pensions / benefits still taxed

# Step 2 — FTC apportionment (IRC §904 proportioning)
total_japan_taxable  = gross_earned + gross_unearned + gross_st_cap + gross_lt_cap
ftc_ratio            = (total_japan_taxable - feie_exclusion) / total_japan_taxable
ftc_creditable       = japan_tax_paid_usd * ftc_ratio
```

**Earned vs unearned split (V6.5):**

| Income stream | FEIE-eligible | How the engine treats it |
|---------------|:---:|-------|
| Salary and RSU vest value | Yes | Work income passed as `gross_earned` |
| FERS pension | No | Retirement income passed as `gross_unearned` |
| Social Security Retirement (US) | No | Benefit income passed as `gross_unearned` |
| SSDI taxable portion | No | Taxable benefit income passed as `gross_unearned` |
| Dividends and capital gains | No | Not FEIE income, but included in FTC apportionment |
| VA disability | N/A | Excluded from the taxable stack |

The FTC denominator is the total Japan-taxable income pool, not just ordinary income. That keeps
dividends and capital gains from being accidentally stripped of FTC credit simply because work
income was excluded by FEIE.

The configured strategy is returned. The `FEIE_Applied` column in `simulation_audit.csv` records
whether the FEIE path applied a positive work-income exclusion in that simulated year.

### IRS Senior Standard Deduction Add-On (V6.4)

Each December true-up, the engine checks whether the user and/or spouse have reached age 65
during that simulation year. If so, the standard deduction is temporarily elevated before
computing the year's federal tax liability:

| Filing Status | 2026 Add-On per Person |
|---------------|------------------------|
| Married Filing Jointly | +$1,550 |
| Single / Head of Household | +$1,950 |
| Married Filing Separately | +$1,550 |

The add-on is applied transiently (save → compute → restore) so it does not compound into
future years' COLA inflation of the base standard deduction.

### Filing status options

`"Married Filing Jointly"` *(default)*, `"Single"`, `"Married Filing Separately"`,
`"Head of Household"`. Brackets, standard deduction, and LTCG thresholds differ per status.

### State income tax

Automatically derived from `us_state_code`. Zero-tax states: `FL TX WA NV AK NH SD TN WY`.
State tax applies to `(gross_ordinary + total_gains − std_deduction).max(0)`.

---

## 4. VA Disability Engine

VA disability compensation is **0% taxable** in all jurisdictions:
- US federal: excluded per Title 38 USC
- US state: excluded
- Japan resident tax: excluded per **US-Japan Tax Treaty Article 19**

VA income accumulates in `year_va_net` and is never added to `total_ord` (the US bracket floor)
or to `gross_pension` (the Japan tax base).

### 2026 Rate Lookup Table

When `va_disability_rating > 0`, the engine uses the official 2026 VA compensation table and
inflates by `inflation_cola` each year from 2026 onward.

| Rating | Vet Only | With Spouse | With Spouse + Child |
|--------|----------|-------------|---------------------|
| 10% | $175.51 | $175.51 | $175.51 |
| 30% | $537.42 | $601.17 | $641.17 |
| 50% | $1,102.04 | $1,233.13 | $1,291.13 |
| 70% | $1,759.19 | $1,890.69 | $1,964.69 |
| 90% | $2,297.96 | $2,463.46 | $2,553.46 |
| **100%** | **$3,938.58** | **$4,158.17** | **$4,267.28** |

Full table (all ratings 10–100 in steps of 10) lives in `src/engine/va_benefits.rs`.

**Rating 0 = No VA Disability.** Setting `va_disability_rating` to `0` explicitly disables VA
income — the engine returns exactly **$0.00** with no NaN risk. The `va_dependent_status` field
is preserved for future use (e.g., upgrading to a service-connected rating later).

Child add-on is automatically removed after the child's 18th birthday (`va_child_cutoff_date`).
The engine transitions from `WithSpouseAndChild` to `WithSpouse` rates on that date.

**College-student extension (V6.4)**: If a dependent in `FamilyUnit.dependents` has
`is_college_student = true` and is ≤ 23 years old (by simulation year), the child add-on
continues past the 18th birthday. The exact 18th birthday cutoff is preserved via
`va_child_cutoff_date`; the 18–23 extension uses year-based arithmetic from `Dependent.birth_year`.

### Special Monthly Compensation (SMC)

SMC variants are defined in `src/engine/va_benefits.rs` and cover SMC-K through SMC-R.2 plus
SMC-Housebound. All SMC amounts are **tax-free** under the same US-Japan Treaty exclusion as
base VA compensation.

- **SMC-K** is *additive* — added on top of the base VA rate.
- **All other variants** (SMC-L through SMC-R.2, Housebound) *replace* the base rate.

The UI SMC dropdown shows the 2026 monthly rate next to each variant. The selected variant is
written to `va_smc_variant` in the scenario JSON.

### Benefit override toggles (V5.5)

The Input Config panel exposes manual override fields for both VA and SMC:

| Toggle | JSON key | Behaviour |
|--------|----------|-----------|
| Override VA Monthly Amount | `va_monthly_override` | Replaces the 2026 rating-table lookup. Treated as 2026 base; inflated by COLA each year. |
| Override SMC Monthly Amount | `smc_monthly_override` | Additive override (K-style). Bypasses the variant lookup. |

When a toggle is off, the corresponding key is removed from the saved JSON.
When on, the key is persisted and the computed table value is suppressed in the UI summary.

---

## 5. Retirement Income Streams

Retirement income can come from several places, and each source starts at a different age and is
taxed differently. The engine lets you turn each one on or off so the scenario matches your life.

### FERS (Federal Employees Retirement System)

| Parameter | Default | Notes |
|-----------|---------|-------|
| `fers_monthly_payment_usd` | — | Monthly gross in today's USD |
| `fers_start_age` | 62 | Age at which FERS begins; simulation bridges cashflow until then |
| COLA | Diet-COLA | ≤2% → full CPI; ≤3% → capped 2%; >3% → CPI−1% |
| US taxable | Yes | Included in `total_ord` (bracket floor) |
| Japan taxable | Yes | Converted to JPY; included in `gross_pension` for resident tax |

Diet-COLA does not compound until the January after age 62 is reached, matching OPM rules.

### Social Security Retirement (US)

| Parameter | Default | Notes |
|-----------|---------|-------|
| `ss_monthly_usd` | 0 | Monthly Social Security Retirement estimate; set to 0 to disable |
| `ss_start_age` | 67 | Full retirement age |
| US taxable | Yes | Savings Clause — included in `total_ord` alongside FERS |
| Japan taxable | No | Not included in Japan `gross_pension` |
| COLA inflation | Yes | Inflated by `inflation_cola` each year after `ss_start_age` |

### Japanese National Pension (Nenkin)

| Parameter | Default | Notes |
|-----------|---------|-------|
| `nenkin_income_monthly_jpy` | 0 | Monthly pension income in JPY; set to 0 to disable |
| `nenkin_income_start_age` | 65 | Age at which Nenkin income begins |
| Japan taxable | Yes | Added to `gross_pension` for Japan resident tax |
| US taxable | No | Not in `total_ord`; covered by Japan-First FTC pipeline |
| COLA inflation | Yes | Inflated by `inflation_japan` each year after `nenkin_income_start_age` |

Nenkin income is distinct from **Nenkin contribution expenses** (`nenkin_monthly_household_jpy`),
which are household-level pension *payments into* the system. The income stream is modelled as
starting at `nenkin_income_start_age` and represents the pension drawdown phase.

### SSDI (Social Security Disability Insurance)

| Parameter | Default | Notes |
|-----------|---------|-------|
| `ssdi_monthly_usd` | `0` | Monthly SSDI benefit in 2026 USD; `0` = not applicable |
| COLA inflation | Yes | Inflated by `inflation_cola` each year from 2026 |
| US taxable | Partial | IRS Pub 915 combined income rule: 0% / 50% / 85% tiers by provisional income |
| Japan taxable | Yes | Treated as public pension (公的年金); routed through pension deduction (公的年金等控除) |

**SSDI Combined Income rule** (MFJ thresholds):

| Provisional Income (PI = AGI_before_SSDI + 0.5 × SSDI) | Taxable SSDI |
|----------------------------------------------------------|-------------|
| ≤ $32,000 | $0 |
| $32,001 – $44,000 | `min(0.50 × (PI − $32,000), 0.50 × SSDI)` |
| > $44,000 | `min(0.85 × SSDI, $6,000 + 0.85 × (PI − $44,000))` |

Only the taxable portion of SSDI is stacked on top of the ordinary income bracket floor;
the non-taxable remainder is received free of federal tax.

### Military Retired Pay (`MilitaryRetiredConfig`)

Military retired pay is an optional income source, separate from FERS. Most expat scenarios do
not need it; set the monthly amount to `0` when it does not apply. When enabled, the default model
treats it under the US-Japan Tax Treaty savings clause: Japan taxes it first, then the US credits
Japan taxes through FTC where allowed.

| Parameter | JSON key | Default |
|-----------|----------|---------|
| Monthly amount (USD) | `military_retired.monthly_usd` | `0` |
| Tax jurisdiction | `military_retired.jurisdiction` | `both` |

Setting `monthly_usd` to `0` disables this income stream. The `jurisdiction` field accepts the same
`TaxProtocol` values as FERS, Social Security, and Nenkin: `both`, `us_only`, `japan_only`, `tax_free`.

---

## 5a. Optional Income Logic

Any income stream can be individually disabled. The engine always produces clean numeric output
— the CSV never contains `"N/A"` strings in numeric columns.

### Disabling an income stream

| Income stream | How to disable | Engine behaviour |
|--------|---------------|-----------------|
| **VA** | Set `va_disability_rating` to `0` | Returns exactly `$0.00` every month. `va_dependent_status` is retained for future use. |
| **FERS** | Set `fers_monthly_payment_usd` to `0` or `"N/A"` | `calculate_fers_monthly()` returns `0.0 × COLA = 0.0`. No NaN. |
| **Social Security Retirement** | Set `ss_monthly_usd` to `0` or `"N/A"` | Social Security block short-circuits to `0.0`. |
| **SSDI** | Set `ssdi_monthly_usd` to `0` or `"N/A"` | SSDI block short-circuits to `0.0`. |
| **Japanese National Pension (Nenkin)** | Set `nenkin_income_monthly_jpy` to `0` or `"N/A"` | Nenkin income block short-circuits to `0.0`. |

### Input Config UI behaviour

- Entering `0` **or** any form of `N/A` (`"N/A"`, `"na"`, `"disabled"`) in the FERS, Social Security, or Nenkin
  monthly amount fields is **valid** — the Safety Gate does not highlight these red.
- On **Save Configuration**, `"N/A"` is normalised to `0` before writing the JSON file, ensuring
  the loader always sees a clean numeric value.
- The VA dropdown shows **"0% — No VA Disability ($0.00)"** for rating 0, making the behaviour
  explicit and unambiguous.

### CSV output guarantee

Numeric amount columns in the audit CSV stay numeric. A disabled income stream outputs `0.00` in
its column — never `"N/A"` or a blank. Boolean/status columns such as `FEIE_Applied` and
`BridgeExhausted` use `Y` / `N`.

### Test coverage

`test_all_pensions_disabled` (in `engine::cashflow_engine`) verifies:
- All four income streams disabled simultaneously produces no NaN, no panic, and exactly `0.0` for every
  income field.
- VA 0% with the 2026 lookup table: `0 * factor = 0.0` is the only code path — no legacy map
  logic, no divide-by-zero risk.

---

## 6. Portfolio & Market Logic

### `Vec<Position>` data model

Portfolio holdings are represented as `Vec<Position>` — a typed, ordered slice of:

```rust
pub struct Position {
    pub ticker:   String,
    pub quantity: f64,
    pub avg_cost: f64, // cost basis per share
}

impl Position {
    pub fn cost_basis(&self) -> f64 { self.quantity * self.avg_cost }
}
```

`MarketDataService::calculate_account_value(&[Position])` returns `(cost_basis_usd, current_value_usd)`.
The UI stock table and the simulation engine both read from this model; the JSON loader
(`src/config/loader.rs`) hydrates `holdings.taxable` into `Vec<Position>` at startup.

### Three accounts

| Account | Currency | Tax treatment |
|---------|----------|---------------|
| `taxable` | USD | Capital gains + dividends taxed under `tax_jurisdiction` |
| `roth_ira` | USD | US tax-free on gains; Japan resident tax may apply to distributions |
| `japan_dc` | JPY | Japan DC / iDeCo — payout at `dc_payout_start_age` |

Additional named brokerage accounts may be defined under `brokerage_accounts` in the JSON,
each with its own `tax_jurisdiction` and `location`.

### Per-asset growth and dividends

Each holding in `taxable` and `roth_ira` carries:
- **`qty`** — share quantity
- **`avg_cost`** — cost basis per share (critical for FIFO capital gains tax)
- **`drip_enabled`** — whether dividends are reinvested
- **`dividend_reinvest_target`** — ticker to reinvest dividends into
- **`custom_growth_rate`** — per-asset CAGR override

> **Cost basis is the user's responsibility.** The simulation uses FIFO lot accounting for
> capital gains. An incorrect `avg_cost` will produce incorrect tax liability at rebalance
> and during any deficit-driven forced liquidations.

### Live 10-year CAGR (`fetch_live_growth_rates: true`)

When enabled, the app looks up each ticker's 10-year growth history from Yahoo Finance. This lets
the same engine work with VTI, SCHD, RSUs, single stocks, ETFs, or any other ticker you enter.
The CAGR is computed as `(last_adj_close / first_adj_close) ^ (1/N_years) - 1` and clamped to
[-50%, +100%].

On any network error, parse failure, or out-of-range result, the engine uses built-in fallback
values. The named tickers below are examples with custom defaults; every other ticker gets the
generic fallback row.

| Ticker | Fallback CAGR | Fallback Price | Fallback Yield |
|--------|--------------|----------------|----------------|
| VTI | 8% | $280 | 1.5% |
| QQQM | 10% | $195 | 0.6% |
| SCHD | 9% | $83 | 3.4% |
| MSFT | 12% | $430 | 0.8% |
| PANW | 15% | $360 | 0.0% |
| *other* | 7% | $100 | 1.5% |

Manual per-ticker overrides in `growth_rates_annual` take priority over both live data and
fallbacks when `fetch_live_growth_rates` is `false`.

### Retirement rebalance event

Triggered on `rebalance_date`. This legacy transition step is still VTI/SCHD-specific:
1. Liquidates growth holdings (PANW, MSFT, QQQM) using FIFO lot accounting.
2. Computes US capital gains tax (ST + LT + NIIT).
3. Funds the war chest and bridge fund from portfolio proceeds.
4. Buys SCHD / VTI at the `rebalance_target_*_pct` allocation.
5. Estimates Year+1 Japan resident tax on the current year's high income and reserves it.

Optional recession shock: if `simulate_recession_at_retirement = true`, the portfolio drops by
`recession_severity_pct` on `rebalance_date` before the rebalance.

The newer target-state rebalancing engine below is ticker-agnostic and uses your
`target_allocations` map instead of hardcoded VTI/SCHD targets.

---

## 7. RSU Vesting Engine

The `RsuEngine` (`src/engine/rsu_engine.rs`) generates a complete sorted vesting schedule
across an arbitrary number of award tranches.

### Key behaviours

- **Multi-award support** — unlimited entries in `rsu_awards`.
- **Vesting cadences** — `quarterly` (default months 2/5/8/11), `monthly` (12×/year),
  `annually` (1×/year). Overridden by explicit `vesting_months` list.
- **`vesting_start_date`** — optional; shifts the vesting clock origin away from `grant_date`
  (used for cliff-then-vest grants where the anniversary year starts later than the grant).
- **Retirement cutoff** — all unvested shares on or after `retirement_date` are forfeited.
- **RSU tax handling**:
  - `SALARY` — tax is paid from external paycheck; vest value flows to portfolio
  - `SELL_TO_COVER` — shares are sold at vest to cover the tax bill; net shares deposited

---

## 8. UI & Safety Gate

The application has **seven tabs**: Overview, Annual Table, Charts, RSU Schedule, Transition,
Input Config, and **🔀 Compare**.

### V6.0 Active Management & Sustainability

#### ⚙ Per-Ticker Management Sub-Panel

Each position row in the Investment Accounts grid has a **⚙** button. The grid itself stays compact
with 8 columns, and any expanded management panels are shown just below the grid so the account
table remains readable.

| Field | Description |
|-------|-------------|
| **DRIP** | Toggle dividend reinvestment. Blank redirect = reinvest in same ticker; `CASH` = route to cash buffer; any other ticker = redirect dividends to that ticker. |
| **Target Alloc %** | Target portfolio weight as a percentage (e.g. `60` = 60%). Used by the rebalancing engine. Leave blank to exclude this ticker from rebalancing targets. |
| **Accum $/mo** | Monthly scheduled buy amount in USD. Processed by the accumulation rules engine before the VA-surplus contribution each month. |
| **Freq** | Accumulation frequency: Monthly / Quarterly / Annual. |
| **Stop at Retirement** | If checked, the accumulation rule fires only during the pre-retirement phase. |

Management settings are persisted in the saved JSON under `simulation_settings.accumulation_rules` (array) and `simulation_settings.target_allocations` (object).

#### ⚖ Target-State Rebalancing Engine

Toggle the **⚖ Target-State Rebalancing** checkbox and choose a frequency (Monthly / Quarterly / Semi-Annual / Annual). On each scheduled month:

1. Total taxable portfolio value is computed.
2. Per-ticker current weight is compared to `target_allocations`.
3. **Overweight** positions are sold (FIFO) — positive long-term gains are approximated at 15%, and positive short-term gains at 22%.
4. **Underweight** positions are bought with net proceeds, proportional to their shortfall.

Only the primary **Taxable** account is rebalanced. DC and Roth accounts use their own scheduled rebalance triggers.

JSON keys:
```json
"simulation_settings": {
  "rebalance_enabled": true,
  "rebalance_frequency_months": 12,
  "target_allocations": { "VTI": 0.60, "SCHD": 0.40 },
  "accumulation_rules": [
    {
      "ticker": "VTI", "account": "Taxable",
      "monthly_amount": 500, "frequency_months": 1,
      "stop_at_retirement": true
    }
  ]
}
```

`target_allocations` and `accumulation_rules` can use any ticker. The VTI/SCHD values above are
examples, not a required strategy.

#### 📊 Dividend Coverage Ratio

A new sustainability metric computed each December:

```
DivCoverageRatio = (div_gross_usd × usd_jpy) / total_exp_jpy
```

- **> 1.0** — dividends alone cover all annual expenses (green)
- **0.5 – 1.0** — partial coverage; drawdown supplements (yellow)
- **< 0.5** — low dividend income relative to expenses (grey)

Displayed in the **Results Table** (`Div Cover` column), the **Overview** panel (lifetime average), and the **CSV** export (`DivCoverageRatio`).

### V5.9 UI Additions

#### 🎲 Marco Polo Mode (Monte Carlo Simulation)

Toggle **"🎲 Marco Polo Mode (Monte Carlo)"** in the Investment Accounts section of Input Config
before running a simulation.

**How it works:**
- Each position row gains a **Volatility %** field (default 18% for equities, 15% for DC/index funds) that replaces the Growth % input while Marco Polo is active.
- Before spawning the simulation thread, the app computes a **weighted-average expected return** (μ) and **weighted-average volatility** (σ) across all positions by portfolio value.
- The **Marco Polo engine** (`src/simulation/monte_carlo.rs`) runs **1,000 Geometric Brownian Motion iterations**:
  ```
  S(t+1) = S(t) × exp((μ − σ²/2) + σ·Z)  +  net_cashflow
  ```
  where Z ~ N(0,1) is drawn from a seed initialized from system time.
- Results are sorted per simulated year to extract **P10 (worst-case)**, **P50 (median)**, and **P90 (best-case)** net worth trajectories.
- The P10/P50/P90 table is displayed in the **🔀 Compare** tab.

**Interpreting results:**
| Percentile | Interpretation |
|------------|---------------|
| P10 | Worst-case outcome — markets under-perform; plan still solvent? |
| P50 | Median / most-likely path — base planning assumption |
| P90 | Best-case outcome — strong markets; excess wealth available |

**Limitations:** The stochastic model is applied to the aggregate portfolio as a single GBM process using weighted-average parameters. It does not model per-asset correlation, currency shocks, or tax effects stochastically — those remain deterministic.

#### 🔀 Dual-Scenario Comparison

Load two distinct JSON scenario files and run them side-by-side:

1. **Open Scenario** (toolbar) — loads the Baseline scenario as usual.
2. **Open Comparison** (toolbar) — loads a second JSON scenario (different retirement date, different withdrawal strategy, etc.).
3. **▶ Run Baseline** — executes baseline, results appear in Overview/Table/Charts.
4. **▶ Run Comparison** — executes comparison scenario in a background thread; navigates to **🔀 Compare** tab on completion.
5. The **Compare** tab shows:
   - Marco Polo P10/P50/P90 table (when Marco Polo was enabled for baseline run)
   - Side-by-side grid: Simulation Years, Final Year, Ending Taxable Portfolio, Roth IRA, DC Plan, FX Rate, Ending Wealth (USD), Total NHI Paid, Solvency Warnings

The Comparison scenario is executed from a separate JSON file (no shared state with baseline). To compare two scenarios that differ only in one parameter, save a copy of the baseline JSON, modify that parameter, and load the copy as the Comparison.

### V5.5 Input Config additions

| Feature | Location in panel | Description |
|---------|------------------|-------------|
| **🗑 New Scenario** (Clear) | Header button row | Calls `Default::default()` on the state — resets all fields without requiring a JSON file to be loaded |
| **Account Type** selector | Tax Jurisdiction section | Dropdown replacing the old "Investment Location" string field. Options: Taxable Brokerage, IRA (Traditional), Roth IRA, 401(k)/DC Plan, NISA, iDeCo |
| **Equity & Vesting** section | Below RSU & DC Settings | Two sub-sections: (1) Stock Holdings table (Ticker, Qty, Avg Cost) with live cost-basis summary; (2) RSU Grant (Ticker, Unvested Shares, Grant Price, Vesting Frequency) |
| **VA Override** toggle | VA Disability Profile | Checkbox + amount field. Bypasses the 2026 rating table; value inflated by COLA each year |
| **SMC Override** toggle | VA Disability Profile / SMC block | Checkbox + amount field. Additive K-style override; bypasses variant lookup |

**USD/JPY spot rate** (`usd_jpy_rate`) is the primary bridge for all JPY-denominated assets. Enter `0`
to use the live-fetch fallback (¥145.00/$). The engine converts JPY portfolio values, expenses,
and income to USD using this rate at each simulation step.

### V5.8 Input Config additions

#### ✨ Auto-Calc button (position rows)

Every ticker cell in a position row has a ✨ button. Clicking it calls the Yahoo Finance v8
chart API synchronously for that ticker and populates:

- **Price USD** — most recent adjusted close (5-day daily window)
- **Growth %** — 10-year annualised CAGR (monthly interval, 10-year range)

The fields remain freely editable after the fill, allowing custom "What-if" overrides.
Falls back to built-in defaults on any network or parse error.

#### DC Plan — dual-currency stability

The DC Plan's **Monthly Contribution** is always stored in Japanese Yen (`dc_monthly_jpy`).
When the global USD/JPY rate changes, the Yen contribution remains constant while its USD
equivalent updates automatically in the simulation — no drift, no re-entry required.

**Growth rate options** (new `dc_growth_rate` field in `simulation_settings`):

| Mode | Behaviour |
|------|-----------|
| *Use Market Average (10%)* checkbox on | Saves `dc_growth_rate: 0.10` to JSON; used by contributions handler |
| *Use Market Average (10%)* checkbox off | Shows a custom % field; saves the entered value |
| Legacy (`dc_info["growth_rate"]`) | Falls back to value in `holdings.japan_dc.growth_rate` if `dc_growth_rate` absent in settings |

#### RSU Awards — Grant Price removed

The **Grant Price** column has been removed from the RSU award table in the UI.
Vest value is computed as `shares × ticker_price_at_vest_date` using the ticker's
projected price (current price grown at CAGR from the grant date to the vest date).
Existing JSON files that contain `grant_price` continue to round-trip safely — the
loader preserves the field but the engine ignores it.

### Input Config — validation safety gate

Before a simulation can run, the Input Config panel validates every required field. Invalid
entries are wrapped in a **red-tinted frame with a red border**. A banner at the top of the
panel displays the count of failing fields.

```
⛔ 3 field(s) require correction before simulation can run (highlighted in red).
```

The **▶ Run Simulation** button in the toolbar is **locked** (`add_enabled(false, ...)`) until
`validation_errors()` returns an empty set.

### Validation rules per field category

| Category | Rules enforced |
|----------|---------------|
| Dates (`start_date`, `retirement_date`, etc.) | Must parse as `YYYY-MM-DD` |
| Expenses (`base_expense_jpy`, `min_expense_jpy`) | Must be a positive number (> 0) |
| Inflation rates | Must be a valid `f64` |
| FERS monthly | Valid `f64`; start age valid `u32` |
| VA rating | Must be `0` or a multiple of 10 in [10, 100] |
| Social Security / Nenkin | `f64` parseable; start age validated only when amount > 0 |
| Buffers | `bridge_months` valid `u32`; `war_chest_target_jpy` valid `f64` |

### Input Config — edit & reload workflow

1. Open a scenario JSON via **📂 Open Scenario** (toolbar) or **📂 Reload Scenario** (panel).
2. Edit any field in the Input Config tab.
3. Click **💾 Save Configuration** to write the updated JSON back to disk.
4. Click **📂 Reload Scenario** and confirm the file. The scenario is now hot-reloaded.
5. Click **▶ Run Simulation** once all fields are valid (no red highlights).

---

## 9. Input Configuration Reference

Scenarios are JSON files with optional `//` and `#` line comments. The loader strips comments
before parsing. Four top-level keys: `simulation_settings`, `rsu_awards`, `holdings`,
`market_prices_usd`.

### 9.1 Personal / Timing

| JSON key | UI label | Type | Default | Description |
|----------|----------|------|---------|-------------|
| `start_date` | Start Date | `YYYY-MM-DD` | `2025-12-31` | Simulation start — first month processed |
| `end_date` | End Date | `YYYY-MM-DD` | `2080-12-31` | Simulation end |
| `retirement_date` | Retirement Date | `YYYY-MM-DD` | `2031-01-01` | Last day of employment; income / expense regime switches here |
| `rebalance_date` | Rebalance Date | `YYYY-MM-DD` | `2031-02-01` | Portfolio rebalance event; must be ≥ `retirement_date` |
| `birth_date` | User Birthday | `YYYY-MM-DD` | `1900-01-01` | Primary retiree's full birth date — drives FERS, SS, Nenkin start-age math, IRS senior add-on at 65, COLA thresholds |
| `is_married` | Married | `bool` | derived | **V6.6.** When `true`, spouse demographics + Spouse SS / Spouse Nenkin participate. Defaults to `true` when `spouse_birth_date` is present, else `false` |
| `spouse_birth_date` | Spouse Birthday | `YYYY-MM-DD` | `1900-01-01` | Spouse's full birth date — second senior std-deduction add-on at 65, gates Spouse SS / Nenkin start ages |
| `child_birth_date` | — | `YYYY-MM-DD` | `1950-01-01` | Legacy single-child VA cutoff. Prefer the `dependents` array (§9.3) for V6.6 full-date precision |

### 9.2 Assets

| JSON key | Location | Type | Description |
|----------|----------|------|-------------|
| `holdings.taxable` | JSON | Object | Taxable brokerage. Keys are ticker symbols; each entry has `qty`, `avg_cost`, and optional `avg_purchase_price_jpy`, `drip_enabled`, `dividend_reinvest_target`, `custom_growth_rate`, `category`, `dividend_months`, `dividend_currency` |
| `holdings.roth_ira` | JSON | Object | Roth IRA. Same per-asset schema as `taxable` |
| `holdings.japan_dc` | JSON | Object | Japan DC / iDeCo. Fields: `qty` (units), `nav_jpy_per_10k`, `growth_rate` |
| `market_prices_usd` | JSON | Object | Manual price override per ticker. Set to `0` to use fallback price |
| `growth_rates_annual` | JSON | Object | Per-ticker annual CAGR. Ignored for any ticker when `fetch_live_growth_rates: true` |
| `fetch_live_growth_rates` | Input Config | `bool` | `false` | When `true`, fetches 10-year CAGR from Yahoo Finance; falls back to 7% on failure |
| `rsu_awards` | JSON | Array | One entry per RSU grant. See §9.5 |
| `brokerage_accounts` | JSON | Array | Optional additional named brokerage accounts |

### 9.3 Income Streams

| JSON key | UI label | Type | Default | Description |
|----------|----------|------|---------|-------------|
| `va_disability_rating` | Disability Rating | `u32` (0, 10–100) | `100` | `0` = no VA disability payment; 10–100 = 2026 rate table lookup |
| `va_dependent_status` | Dependent Status | enum | `with_spouse_and_child` | `vet_only` / `with_spouse` / `with_spouse_and_child` |
| `va_disability_rates` | — | Object | see template | Legacy scenario field retained in templates; current VA monthly income uses the 2026 lookup table, VA override fields, or `$0` when `va_disability_rating = 0` |
| `va_contribution_buffer_usd` | — | `f64` | `800.0` | Monthly VA amount withheld from investment contributions during accumulation phase |
| `fers_monthly_payment_usd` | FERS Monthly (USD) | `f64` | `794.55` | Monthly gross FERS pension in today's USD |
| `fers_start_age` | FERS Expected Start Age | `u32` | `62` | Age at which FERS begins; cashflow bridged from war chest / bridge fund until then |
| `ss_monthly_usd` | Social Security Retirement Monthly (USD) | `f64` | `0` | Monthly Social Security Retirement benefit estimate; `0` = not applicable |
| `ss_start_age` | Social Security Start Age | `u32` | `67` | Age at which Social Security payments begin |
| `ssdi_monthly_usd` | SSDI Monthly (USD) | `f64` | `0` | Monthly SSDI benefit in 2026 USD; `0` = not applicable. Inflated by `inflation_cola` from 2026. |
| `dependents` | Family Demographics | Array | `[]` | Array of `{"birth_date": "YYYY-MM-DD", "birth_year": YYYY, "is_college_student": bool}`. **V6.6:** `birth_date` is the precise field; `birth_year` retained for backward compat and auto-derived when only `birth_date` is supplied. Drives VA child rider cutoff at the exact 18th / 23rd birthday. |
| `spouse_ss_monthly_usd` | Spouse SS Monthly (USD) | `f64` | `0` | **V6.6.** Spouse Social Security monthly estimate; `0` = not applicable. Active only when `is_married` AND spouse age ≥ `spouse_ss_start_age` |
| `spouse_ss_start_age` | Spouse SS Start Age | `u32` | `67` | Age (in spouse years) at which Spouse SS begins |
| `spouse_ss_jurisdiction` | Spouse SS Tax Jurisdiction | enum | `both` | `both` / `us_only` / `japan_only` / `tax_free` |
| `spouse_nenkin_monthly_jpy` | Spouse Nenkin Monthly (JPY) | `f64` | `0` | **V6.6.** Spouse Nenkin monthly estimate; `0` = not applicable |
| `spouse_nenkin_start_age` | Spouse Nenkin Start Age | `u32` | `65` | Age (in spouse years) at which Spouse Nenkin begins |
| `spouse_nenkin_jurisdiction` | Spouse Nenkin Tax Jurisdiction | enum | `both` | Same enum as above |
| `nenkin_income_monthly_jpy` | Nenkin Monthly Income (JPY) | `f64` | `0` | Monthly Japanese National Pension income once drawdown begins; `0` = not applicable. Separate from Nenkin contribution expense |
| `nenkin_income_start_age` | Nenkin Start Age | `u32` | `65` | Age at which Nenkin income begins |
| `total_annual_compensation_usd` | — | `f64` | `0` | Pre-retirement gross salary used for RSU marginal tax estimation |
| `retirement_year_gross_income_jpy` | — | `f64` | `40,000,000` | Gross JPY income in the retirement calendar year; drives Year+1 Japan resident tax |

### 9.4 Strategy & Tax

| JSON key | UI label | Type | Default | Description |
|----------|----------|------|---------|-------------|
| `us_tax_strategy` | US Tax Mitigation Strategy | enum | `ftc_only` | `ftc_only` or `feie_and_ftc`. See §3 |
| `tax_jurisdiction` | Tax Jurisdiction | enum | `both` | `both` / `us_only` / `japan_only`. `us_only` bypasses all Japan calculations |
| `account_type` | Account Type | string | `Taxable Brokerage` | Primary account classification for tax routing. One of: `Taxable Brokerage`, `IRA (Traditional)`, `Roth IRA`, `401(k) / DC Plan`, `NISA`, `iDeCo`. On save, `investment_location` is auto-derived (`japan` for NISA/iDeCo, `us` otherwise) |
| `investment_location` | — (legacy) | enum | `us` | Auto-derived from `account_type`; preserved for backward compatibility |
| `us_filing_status` | Filing Status | string | `Married Filing Jointly` | IRS filing status; determines brackets and std deduction |
| `us_state_code` | US State Residency | string | `FL` | Two-letter postal code; `FL` / `TX` / `WA` and others = no state income tax |
| `us_state_tax_rate` | — | `f64` | `0.0` | Auto-derived from `us_state_code`; manual override possible |
| `inflation_us_cpi` | US Inflation (CPI) | `f64` | `0.028` | Annual US CPI; inflates tax brackets, FERS COLA, SS COLA, RSU bracket floor |
| `inflation_japan_cpi` | Japan Inflation (CPI) | `f64` | `0.028` | Annual Japan CPI; inflates JPY expenses and Nenkin income |
| `simulate_yen_strengthening` | Enable FX Drift | `bool` | `false` | Enables FX trajectory post-retirement. Choose either the legacy continuous rate or the V6.6 cadence-based step below |
| `fx_drift_rate_annual` | FX Drift Rate (annual, legacy) | `f64` | `0.02` | Fraction by which USD/JPY rate declines per year. Engine guards `r.is_finite() && r < 1.0 && r > -1.0` to prevent NaN. Ignored when `fx_drift_cadence_months > 0` |
| `fx_drift_cadence_months` | Cadence Months | `u32` | `0` | **V6.6.** Every N months after retirement, FX jumps by `fx_drift_increase_amount_jpy`. `0` = use legacy continuous rate above |
| `fx_drift_increase_amount_jpy` | Increase Amount (JPY) | `f64` | `0.0` | **V6.6.** Signed JPY step per cadence (positive = yen weakens, negative = yen strengthens) |
| `usd_jpy_rate` | USD/JPY Rate | `f64` | `0` | Initial exchange rate. `0` = use hardcoded fallback (¥145.00/$) |
| `simulate_recession_at_retirement` | Simulate Recession at Retirement | `bool` | `false` | Applies a one-time portfolio shock at `rebalance_date` |
| `recession_severity_pct` | Recession Severity | `f64` | `0.20` | Fraction of portfolio dropped on the retirement-date shock (e.g. `0.20` = -20%) |
| `simulated_recessions` | Dynamic Recession Events | Array | `[]` | Additional market shocks, e.g. `[{"year": 2028, "severity": 0.15, "duration_months": 6, "recovery_months": 18}]` |
| `rsu_tax_handling` | — | string | `SALARY` | `SALARY` or `SELL_TO_COVER` |
| `withdrawal_strategy` | Withdrawal Strategy | enum | `total_return` | `dividend_only`, `total_return`, or `hybrid`; `dividend_only` prevents forced stock sales |
| `withdrawal_waterfall` | Spending Waterfall | enum | `defensive` | `defensive` = V7.1 JPY-first waterfall; `cautious` = legacy V7.0-compatible cashflow behaviour |
| `fx_spread_penalty` | FX Spread Penalty | `f64` | `0.005` | Flat spread cost applied to USD-to-JPY conversions in the defensive waterfall |

Scheduled recession events can be instant shocks or multi-month drawdowns. If `duration_months`
is greater than 1, the engine spreads the loss across that many months and can then model a
V-shaped recovery over `recovery_months`.

$$
\text{monthly recovery boost} =
\left(\frac{1}{\max(1 - \text{severity}, 0.001)}\right)^{1 / \text{recovery months}} - 1
$$

Plain English: if you model a 30% crash with an 18-month recovery, the portfolio falls over the
drawdown period and then receives a steady monthly recovery boost until the simulated market has
bounced back. The `0.001` floor prevents impossible 100% crash scenarios from producing infinite
values.

### 9.5 Expenses & Buffers

| JSON key | UI label | Type | Default | Description |
|----------|----------|------|---------|-------------|
| `base_monthly_expenses_jpy` | Base Monthly | `f64` | `1,016,744` | Target monthly living expenses (JPY); inflated by Japan CPI annually |
| `min_monthly_expenses_jpy` | Minimum Monthly | `f64` | `600,000` | Floor spending if income is insufficient to cover desired |
| `nhi_spike_monthly_jpy` | Legacy NHI Spike Monthly | `f64` | `73,333` | Backward-compatible legacy field. V6.5 uses `nhi_model` for active NHI calculation. |
| `nenkin_monthly_household_jpy` | — | `f64` | `35,020` | Household Nenkin *contribution* expense embedded in base; only the excess over `nenkin_baseline_annual_jpy` is a separate charge |
| `bridge_fund_months_target` | Bridge Fund Months | `u32` | `12` | Target months of base expenses kept liquid in the bridge fund |
| `war_chest_target_jpy` | War Chest Target (JPY) | `f64` | `7,000,000` | Emergency reserve target in JPY (when `war_chest_currency = "JPY"`) |
| `war_chest_target_usd` | — | `f64` | `50,000` | Emergency reserve target in USD (when `war_chest_currency = "USD"`) |
| `pre_funded_war_chest_jpy` | — | `f64` | `0` | War chest balance at simulation start |
| `pre_funded_bridge_usd` | — | `f64` | `0` | Bridge fund balance at simulation start (USD) |
| `pre_funded_japan_tax_jpy` | — | `f64` | `0` | Pre-reserved Japan tax cash at simulation start |
| `pre_funded_us_tax_usd` | — | `f64` | `0` | Pre-reserved US tax cash at simulation start |
| `dc_payout_method` | — | string | `ANNUITY_20YR` | `LUMP_SUM` (invested to Taxable) or `ANNUITY_20YR` (240 monthly draws) |
| `dc_payout_start_age` | — | `u32` | `60` | Age at which DC payout begins |

### 9.6 Rebalancing

The retirement transition and Roth rebalance fields below are legacy VTI/SCHD-specific controls.
For ticker-agnostic periodic rebalancing, use `target_allocations` and `accumulation_rules` in
`simulation_settings`.

| JSON key | Type | Default | Description |
|----------|------|---------|-------------|
| `rebalance_target_vti_pct` | `f64` | `0.20` | VTI allocation % at retirement rebalance |
| `rebalance_target_schd_pct` | `f64` | `0.80` | SCHD allocation % at retirement rebalance |
| `enable_roth_rebalance_at_59` | `bool` | `false` | Triggers a Roth IRA rebalance at age 59.5 |
| `roth_rebalance_target_vti_pct` | `f64` | `0.50` | Roth VTI target if Roth rebalance enabled |
| `roth_rebalance_target_schd_pct` | `f64` | `0.50` | Roth SCHD target if Roth rebalance enabled |
| `buy_schd_last_year` | `bool` | `true` | Redirect contributions to SCHD in the 12 months before retirement |

**V6.6 — Per-position rebalance overrides.** Each entry in `holdings.<account>` may carry an
optional `rebalance_date: "YYYY-MM-DD"` field. When set on a position, it supersedes the global
`rebalance_date` for *that ticker only*. A reserved `recession_override: f64` (0.0–1.0) field
lets a position opt into a different drawdown severity than the global recession event when
needed by future strategies.

### 9.7 RSU Award schema (`rsu_awards` array)

The UI RSU table no longer shows a Grant Price column — vest value is computed from the
ticker's current price × projected CAGR at each vest date. Any `grant_price` already in
the JSON is preserved on round-trip but has no effect on the simulation.

```jsonc
{
  "grant_date":         "2024-11-01",       // YYYY-MM-DD
  "vesting_start_date": null,               // Optional; overrides clock origin
  "ticker":             "PANW",
  "total_shares":       400,
  "vesting_years":      4,
  "vesting_cadence":    "quarterly",        // "quarterly" | "monthly" | "annually"
  "vesting_months":     [2, 5, 8, 11]       // Explicit months; overrides cadence default
}
```

---

## 10. Output & Reporting

Reports are written to `output/` automatically after every successful simulation run and are
excluded from version control via `.gitignore`. Automatic runs write
`output/Retirement_Summary.txt` and `output/simulation_data.csv`; the Overview tab can also export
a user-selected audit CSV, defaulting to the filename `simulation_audit.csv`.

### Manual export buttons (Overview tab → Reporting & Export)

| Button | File | Description |
|--------|------|-------------|
| 💾 Export Text Report | `output/Retirement_Summary.txt` | Full human-readable report: portfolio totals, solvency, retirement income, dual-jurisdiction tax table, RSU vesting, FY2026 VIP bonus, transition event |
| 📊 Export Audit CSV | user-selected path, default `simulation_audit.csv` | Machine-readable annual table (see below) |
| 📋 Copy to Clipboard | system clipboard | Condensed plain-text summary |

Each button displays a **green** confirmation or **red** error (e.g., file locked in Excel)
that auto-clears after 5 seconds.

### Audit CSV column reference (36 columns)

| Column | Unit | Description |
|--------|------|-------------|
| `Year` | — | Calendar year |
| `FX_JPY_per_USD` | ¥/$ | USD/JPY exchange rate at Dec 31 |
| `Brokerage_USD` | USD | Taxable brokerage portfolio value |
| `Roth_USD` | USD | Roth IRA portfolio value |
| `DC_JPY` | JPY | Japan DC portfolio value |
| `DivGross_USD` | USD | Gross dividend income |
| `DivNet_USD` | USD | Dividend income after withholding |
| `FERSNet_USD` | USD | FERS pension net of US withholding |
| `VA_Benefit_USD` | USD | VA disability compensation (always 0% tax) |
| `RSUVest_USD` | USD | RSU vest value at vest date market price |
| `SS_Payout_USD` | USD | US Social Security Retirement received this year |
| `Nenkin_Income_JPY` | JPY | Japanese National Pension (Nenkin) received this year |
| `TotalIncNet_USD` | USD | Total net income (all streams) in USD |
| `TotalIncNet_JPY` | JPY | Total net income converted at year-end FX |
| `BaseExp_JPY` | JPY | Base living expenses |
| `NHI_JPY` | JPY | National Health Insurance obligation |
| `Nenkin_JPY` | JPY | Nenkin contribution expense |
| `ResTax_JPY` | JPY | Japan resident tax installments paid |
| `TotalExp_JPY` | JPY | Total expenses |
| `Gap_JPY` | JPY | Income − Expenses (negative = deficit) |
| `USTaxCharged_USD` | USD | US federal + state tax for the year |
| `JapanTaxCharged_JPY` | JPY | Japan resident tax for the year |
| `FEIE_Applied` | Y/N | Whether the configured FEIE+FTC path applied a positive work-income exclusion |
| `BridgeFund_USD` | USD | Bridge fund balance at Dec 31 |
| `WarChest_JPY` | JPY | War chest balance at Dec 31 |
| `WarChestUsed_JPY` | JPY | War chest drawdown this year |
| `ExtTaxPaid_USD` | USD | External tax paid (pre-retirement SALARY mode RSU) |
| `BridgeExhausted` | Y/N | Whether bridge fund was exhausted at any point in the year |
| `ForcedLiquidations_USD` | USD | Taxable portfolio forced-sells due to insolvency |
| `FTC_Carryover_USD` | USD | Unused Japan FTC carried forward to next year (IRC §904) |
| `Purchasing_Power_USD` | USD | `min_expense_jpy / fx` — real-USD cost of minimum lifestyle |
| `DivCoverageRatio` | ratio | `div_gross_usd × fx / total_exp_jpy` |
| `JapanCapGainsTax_JPY` | JPY | Japan capital-gains tax realised during forced liquidations |
| `StateCapGainsTax_USD` | USD | US state capital-gains tax reserved during forced liquidations |
| `FXPenalty_JPY` | JPY | Cumulative FX spread penalty from USD-to-JPY conversions in the defensive waterfall |
| `MonthsAtMin` | months | Number of months where spending was reduced from base to minimum target |

### `Retirement_Summary.txt` — section layout

```
0. Simulation Configuration
1. Final Portfolio Values
2. Solvency Analysis
3. Retirement Income Summary     ← VA, Social Security, Nenkin lifetime totals; FEIE years count
4. Dual-Jurisdiction Tax Summary ← [US Tax Charged] | [Japan Tax Charged] by year
5. RSU Vesting Summary           ← by ticker; FY2026 VIP Bonus detail
6. Retirement Transition Event   ← sell/buy ledger, tax bill, war chest funding
7. Annual Income vs Expense Summary
```

---

## 11. Build & Run

### Prerequisites

- [Rust toolchain](https://rustup.rs/) — stable channel, edition 2024
- Network access is optional. Live Yahoo market-data fetching is best-effort and falls back to
  hardcoded values when unavailable.

### Build

```bash
# Debug (fast compile, slow simulation)
cargo build

# Release (recommended — LTO fat, opt-level 3, ~8.1 MB binary)
cargo build --release
```

### Run

```bash
cargo run --release
```

The window opens immediately. Use **📂 Open Scenario** to load a JSON file. The Input Config
tab becomes populated. Correct any red-highlighted fields, then click **▶ Run Simulation**.
Results appear across all tabs once the background thread completes. Reports are auto-saved to
`output/`.

### Test suite

```bash
cargo test
```

**59/59 tests** across all modules:

| Module | Tests | Coverage |
|--------|-------|----------|
| `engine::tax::us_tax` | 14 | LTCG brackets, NIIT, FTC, state tax, filing status, bracket inflation, ordinary income at brackets, SSDI combined income (5 tests: zero, below $32K, 50% tier, 85% taxable, 85% cap) |
| `engine::tax::japan_tax` | 10 | Employment deduction tiers, pension deduction age thresholds, resident tax formula, legacy NHI compatibility tests |
| `engine::tax::japan_regions` | 3 | Nagoya 9.7% < Tokyo 10.0%, rate delta = 0.3% of taxable base, city standard rates |
| `engine::cashflow_engine` | 7 | FERS COLA tiers, FERS start gate, VA inflation, VA child cutoff, college-student extension |
| `engine::rsu_engine` | 7 | Date alignment, monthly cadence, retirement cutoff, vesting_start_date, share accounting |
| `engine::va_benefits` | 4 | 100% VetOnly ($3,938.58), 100% WithSpouseAndChild ($4,267.28), 50% WithSpouse ($1,233.13), SMC-K through R.2 2026 rates, override logic |
| `engine::tax::nhi` | 10 | Medical/support/nursing components, caps, ManualOverride dispatch, investment income flag |
| `simulation::controller` | 2 | crash_2030 stress scenario, FX shock 2032 |
| *(other)* | 2 | — |

---

## 12. Project Structure

```
retirement-calculator-v2/
├── Cargo.toml
├── input/
│   └── TEMPLATE_scenario.json      ← Canonical annotated scenario file
├── output/                          ← Generated reports (gitignored)
│   ├── Retirement_Summary.txt
│   └── simulation_data.csv
└── src/
    ├── main.rs
    ├── config/
    │   └── loader.rs               ← JSON parser (strips // and # comments)
    ├── engine/
    │   ├── mod.rs
    │   ├── cashflow_engine.rs      ← Monthly income/expense calculations
    │   ├── market_data.rs          ← Fallback prices, yields, FX, Yahoo Finance CAGR; calculate_account_value
    │   ├── rsu_engine.rs           ← Multi-award vesting schedule generator
    │   ├── va_benefits.rs          ← 2026 VA/SMC rate tables (K through R.2 + Housebound)
    │   └── tax/
    │       ├── japan_regions.rs    ← All 47 prefectures + cities; Juminzei rate lookup
    │       ├── japan_tax.rs        ← JapanTaxEngine: resident tax and pension deduction
    │       ├── nhi.rs              ← NhiEngine: municipal-rate NHI, caps, manual override
    │       └── us_tax.rs           ← TaxEngine: LTCG, NIIT, FTC, FEIE pipeline, state tax
    ├── handlers/
    │   ├── cashflow_manager.rs     ← Post-retirement monthly cash-flow orchestration
    │   ├── contributions.rs        ← Pre-retirement Roth / DC / VA-buffer contributions
    │   ├── dividends.rs            ← Dividend DRIP and withholding
    │   ├── retirement_transition.rs ← Rebalance event, war chest, bridge fund, transition tax
    │   ├── roth_rebalancer.rs      ← Optional Roth rebalance at age 59.5
    │   └── rsu_vesting.rs          ← RSU vest events, SELL_TO_COVER / SALARY logic
    ├── models/
    │   ├── assets.rs               ← Account, Holding, FIFO lot tracking
    │   ├── config.rs               ← Config; TaxProtocol; Position; MilitaryRetiredConfig; all enums
    │   ├── constants.rs            ← SimConstants (legacy NHI compatibility, embedded NHI baseline, etc.)
    │   ├── expense.rs              ← ExpenseRule (NHI, Nenkin, ResTax installments)
    │   ├── rsu.rs                  ← RsuAward schema
    │   └── snapshot.rs             ← AnnualSnapshot (32 CSV columns), SimResults, TransitionReport
    ├── reporter.rs                 ← Text report, CSV, and clipboard formatters
    ├── simulation/
    │   ├── controller.rs           ← SimulationController: month loop, tax true-up
    │   ├── state.rs                ← SimState: all mutable simulation state
    │   └── stats.rs                ← AnnualStats: year-to-date accumulators
    └── ui/
        ├── app.rs                  ← eframe App, toolbar, tab routing, background thread
        └── panels/
            ├── chart_panel.rs
            ├── config_panel.rs
            ├── input_panel.rs      ← Input Config tab; Vec<StockRow>; Equity & Vesting; VA/SMC overrides
            ├── overview_panel.rs
            ├── results_table.rs
            ├── rsu_panel.rs
            └── transition_panel.rs
```

---

## 13. Universal Japan NHI Support & Overrides

Japan's National Health Insurance (NHI / 国民健康保険) is assessed annually in June based on
the prior calendar year's income. This creates a well-known **spike** in the first
post-retirement year: the assessment uses peak employment income rather than the lower
pension-level income that follows in steady state.

This section describes the active V6.5 NHI engine. Older flat-rate or discount-tier examples are
legacy documentation and should not be used for current estimates.

### `NhiModel` enum (`src/models/config.rs`)

```
NhiModel::Calculated(NhiCalculatedRates)   — full rate-schedule calculation
NhiModel::ManualOverride { spike, ongoing } — static annual totals entered by the user
```

**`NhiCalculatedRates`** stores the city-specific rates, per-person charges, and annual caps from
a municipality's NHI rate card. In everyday terms, this is what lets the app estimate NHI for
where you actually live instead of using a rough national shortcut.

| Field | Sagamihara 2026 | Description |
|-------|----------------|-------------|
| `medical_rate` | 8.46% | Income levy for medical component (医療分) |
| `elderly_support_rate` | 2.04% | Income levy for support component (支援分) |
| `nursing_care_rate` | 2.02% | Income levy for nursing care, ages 40–64 (介護分) |
| `per_capita_medical` | ¥33,600 | Equal-share per insured (医療分均等割) |
| `per_capita_support` | ¥11,400 | Equal-share per insured (支援分均等割) |
| `per_capita_nursing` | ¥12,600 | Equal-share per insured (介護分均等割) |
| `cap_medical` | ¥650,000 | Annual ceiling for medical component |
| `cap_support` | ¥240,000 | Annual ceiling for support component |
| `cap_nursing` | ¥170,000 | Annual ceiling for nursing care component |
| `include_us_investment_income` | false | Add US dividends (JPY-converted) to income basis |

### `NhiEngine` (`src/engine/tax/nhi.rs`)

`NhiEngine::compute_annual` dispatches between the two modes:

- **Calculated**: Applies NTA deduction tables (same as resident tax) to derive the
  *income basis* (`max(0, net_salary + net_pension [+ investment_income] − ¥430,000)`),
  then computes each component with `min(basis × rate + per_capita × n, cap)`.
- **ManualOverride**: Returns `spike_year_total_jpy` in the first post-retirement year
  and `ongoing_annual_total_jpy` in all subsequent years.

The **1-year lookback** is the key real-world behavior: NHI uses last year's income. If last year
was a high-income work year, the first post-retirement NHI bill can feel surprisingly high. Once
the prior year reflects pension-level income, the estimate usually normalizes.

### Dynamic scheduling in the controller

`SimulationController::schedule_annual_nhi` runs each January for post-retirement years.
It replaces the old static `"NHI Spike"` loader rule with per-year `"NHI YYYY"` expense
rules. The prior-year dividend history (`div_income_history`) is also tracked so that
the US investment income flag feeds the income basis accurately.

### UI — NHI Settings section

The **NHI Settings** section in the Input Configuration tab provides:

- **Mode toggle**: Automatic (Municipal Rates) ↔ Manual (Fixed Amounts)
- **Load Sagamihara 2026 Defaults** button — pre-fills all rate fields with official 2026 values
- **Rate grid** (Automatic mode): all nine rate/per-capita/cap fields, editable per municipality
- **Include US Investment Income** checkbox — activates the global-earnings NHI base
- **Manual fields** (Manual mode): spike-year annual total and ongoing annual total (JPY)

### JSON schema

The `nhi_model` key is written to `simulation_settings` on save:

```json
// Calculated mode (default)
"nhi_model": {
  "mode": "calculated",
  "medical_rate": 0.0846, "per_capita_medical": 33600, "cap_medical": 650000,
  "elderly_support_rate": 0.0204, "per_capita_support": 11400, "cap_support": 240000,
  "nursing_care_rate": 0.0202, "per_capita_nursing": 12600, "cap_nursing": 170000,
  "include_us_investment_income": false
}

// Manual override mode
"nhi_model": {
  "mode": "manual_override",
  "spike_year_total_jpy": 880000,
  "ongoing_annual_total_jpy": 540000
}
```

Old JSON files without an `nhi_model` key automatically get Calculated mode
with Sagamihara 2026 defaults.

---

## 14. Troubleshooting & UI Architecture

### egui Widget ID Clashes

egui assigns each widget a stable ID derived from its parent context and an explicit or
auto-generated key. When the same ID string is used by two widgets in the same frame —
even across different tabs that share a `ScrollArea` ancestor — egui logs a red
"Second use of widget ID" box at the collision site (IDs like `E2B5`, `3D09`, `C6B2`).

**Root cause in this app**: the central panel's `ScrollArea::vertical()` is a shared
ancestor for every tab. A `ScrollArea` or `Grid` inside one tab panel can produce the same
auto-generated hash as a widget in another tab if neither has an explicit salt.

**Fix applied in V6.1**: two-level namespacing strategy:

1. **Outer ScrollArea salt** — `egui::ScrollArea::vertical().id_salt("central_panel_scroll")` 
   in `app.rs` anchors the root ID context so child auto-IDs are stable and distinct from
   other panels.

2. **`push_id` namespaces** — all baseline tab panels are rendered inside
   `ui.push_id("baseline_view", ...)` and the comparison tab is rendered inside the
   `comparison_view` namespace via `ui.push_id("comparison_view", ...)` in
   `comparison_panel::show()`. This ensures that a `Grid::new("my_grid")` in the Overview
   tab and a `Grid::new("my_grid")` in the Comparison tab produce different egui IDs.

3. **Explicit `id_salt` on all `ScrollArea` calls** — every nested `ScrollArea` now carries
   a unique salt so egui never has to fall back to a call-site hash:

   | Panel | Salt |
   |-------|------|
   | `results_table.rs` — main table | `"annual_table_scroll"` |
   | `results_table.rs` — gap warnings | `"gap_warnings_scroll"` |
   | `rsu_panel.rs` — vesting schedule | `"rsu_schedule_scroll"` |
   | `transition_panel.rs` — sells log | `"sells_scroll"` (pre-existing) |

**Adding new panels**: always pass an `id_salt` to any `ScrollArea::new()` or
`ScrollArea::vertical/horizontal/both()` call, and use distinct string keys for
`egui::Grid::new()`. If a panel may be rendered from multiple sites (e.g. inside both the
baseline and comparison branches), wrap it with `ui.push_id("unique_scope", ...)`.

---

## 15. Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `eframe` | 0.29 | Native desktop application framework |
| `egui` | 0.29 | Immediate-mode GUI |
| `egui_plot` | 0.29 | Portfolio chart panel |
| `egui_extras` | 0.29 | Additional egui utilities |
| `chrono` | 0.4 | Date arithmetic (`NaiveDate`, `Datelike`, `Months`) |
| `serde` + `serde_json` | 1 | JSON scenario serialisation / deserialisation |
| `rfd` | 0.15 | Native OS file-open dialog |
| `ureq` | 2.10 | HTTP client for Yahoo Finance CAGR fetch |
| `log` + `env_logger` | 0.4 / 0.11 | Structured simulation trace logging |
| `rust_decimal` | 1.41 | Exact decimal arithmetic for monetary rounding |

### Release profile

```toml
[profile.release]
opt-level     = 3
lto           = "fat"
codegen-units = 1
strip         = true
```

LTO fat produces a single optimised codegen unit with full cross-crate inlining, at the cost of
longer compile times. The resulting binary is ~8.1 MB with all debug symbols stripped.

---

## 16. Hardening & Compliance (V6.5)

V6.5 resolves three mathematical fragilities identified in the V6.5 audit
(`audit_findings_V6.5.md`). All 59 pre-existing tests continue to pass unchanged.

### Fix 1 — Recession recovery guard (`controller.rs`)

Recession events can now model both the crash and the bounce-back period:

- `duration_months <= 1` applies an instant shock in January.
- `duration_months > 1` spreads the drawdown across multiple months.
- `recovery_months > 0` adds a V-shaped recovery after the drawdown.
- New investing is suppressed while the recession event is active.

$$
\text{monthly recovery boost} =
\left(\frac{1}{\max(1 - \text{severity}, 0.001)}\right)^{1 / \text{recovery months}} - 1
$$

Plain English: the engine can model a market that falls over several months and then recovers over
a chosen period. The `0.001` floor keeps even an extreme 100% crash scenario finite instead of
creating infinite or undefined portfolio values.

### Fix 2 — FEIE applied to non-earned pension income (`controller.rs` + `us_tax.rs`)

**Root cause:** `calculate_liability_with_feie_ftc` received a single `gross_ord` that combined
FERS pension, Social Security, and SSDI — none of which are "earned income" under IRC §911. The
FEIE exclusion was therefore applied to income it cannot legally exclude, producing
artificially low US tax for retirees.

**Fix:** The function signature is split into `gross_earned` (salary and RSU vests — FEIE-eligible)
and `gross_unearned` (FERS, Social Security, SSDI — not eligible). In `finalize_year_taxes`, only
`year_rsu_vest_usd` is passed as `gross_earned`; the full pension total remains in
`gross_unearned` and flows through ordinary brackets without exclusion. In post-retirement
operation (where RSU vests = $0), FEIE naturally evaluates to zero — the function degrades to
a plain FTC calculation, which is correct.

### Fix 3 — FTC apportionment denominator (`us_tax.rs`)

**Root cause:** The anti-double-dip ratio used `gross_ord` as its denominator. In years with
large capital gains and fully FEIE-excluded earned income, `ftc_ratio = 0 / gross_ord = 0`,
granting zero FTC on Japan tax actually paid on dividends and gains — punitive double-taxation.

**Fix:** The denominator is now `total_japan_taxable = gross_earned + gross_unearned +
gross_st_cap + gross_lt_cap`. Only the excluded share is disallowed; capital gains and pension
income remain fully creditable per IRC §904.

---

*Private — all rights reserved.*

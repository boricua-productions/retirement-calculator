# Rearchitecture Instructions — Overview/Transition/Tabs Clarity Pass

**Audience:** Sonnet executor. Self-contained spec — do not assume prior conversation context.
**Author intent:** Make the UI more legible to a non-investor reader. All tax/income/asset numbers must always disclose **which country** they refer to and **which year range** they cover. Add a non-investor "does this work for retirement?" verdict. Reorder tabs so input comes first.
**Repo:** `D:\github-repos\retirement-calculator` (Rust + eframe/egui).
**Version target:** bump to **V8.2.0 — UX Clarity Pass**.

---

## 0. Ground rules

- Do **not** change any simulation/engine behavior. This is a UI-only rearchitecture plus one small additive data plumbing change (per-account snapshots).
- Keep all 236 existing tests passing. Add new tests where noted.
- No new dependencies.
- Style: match existing panel code (egui::Grid, RichText, Color32 palette already in `overview_panel.rs`).
- Country labels must use the existing enums (`AccountLocation`, `AccountJurisdiction`, `TaxProtocol`) — do **not** stringify with hard-coded names where an enum already exists.
- When you write country annotations, use compact `(JP)` / `(US)` / `(JP+US)` suffixes for column labels and the full word ("Japan", "United States") in headings or explanatory text.
- Year range format: `(YYYY–YYYY)` using en-dash, derived from `res.annual_summary.first().year` and `res.annual_summary.last().year`.

---

## 1. Files you will edit

| File | What changes |
|---|---|
| `src/ui/app.rs` | Reorder `Tab` enum + tab strip; change default tab to `InputConfig`; update `poll_simulation_result` to switch to `Overview` post-run (already does, keep). |
| `src/ui/panels/overview_panel.rs` | Major rework — sections b–g below. |
| `src/ui/panels/transition_panel.rs` | Add per-row country/account/tax-country labels — section h. |
| `src/models/snapshot.rs` | Add `AccountSnapshotRow` + `account_snapshots: Vec<AccountSnapshotRow>` on `SimResults`. |
| `src/simulation/controller.rs` | Populate `account_snapshots` at two events: retirement date and each rebalance date. |
| `src/reporter.rs` | If text/CSV reports surface these fields, mirror the new country/range annotations (only if trivial — otherwise leave a `// TODO V8.2` comment). |
| `tests/` | Add a snapshot-test for the new tab order and a unit test for `account_snapshots` population. |

---

## 2. Task (a) — Reorder tabs: Input Config first, then Overview

**File:** `src/ui/app.rs`

### 2.1 Reorder the `Tab` enum and change the default

```rust
#[derive(Default, PartialEq, Eq)]
enum Tab {
    #[default]
    InputConfig,
    Overview,
    Transition,
    Table,
    Charts,
    Rsu,
    Comparison,
}
```

Note the order change: **Transition** now sits immediately after **Overview** (task h).

### 2.2 Update the tab strip in `update()`

Find the `ui.horizontal(|ui| { ui.selectable_value(...) ... })` block (currently lines ~194–202) and replace with:

```rust
ui.selectable_value(&mut self.active_tab, Tab::InputConfig, "⚙ Input Config");
ui.selectable_value(&mut self.active_tab, Tab::Overview,    "📊 Overview");
ui.selectable_value(&mut self.active_tab, Tab::Transition,  "🔄 Transition");
ui.selectable_value(&mut self.active_tab, Tab::Table,       "📋 Annual Table");
ui.selectable_value(&mut self.active_tab, Tab::Charts,      "📈 Charts");
ui.selectable_value(&mut self.active_tab, Tab::Rsu,         "🗓 RSU Schedule");
ui.selectable_value(&mut self.active_tab, Tab::Comparison,  "🔀 Compare");
```

### 2.3 Keep auto-switch to Overview after a successful run

In `poll_simulation_result()` the line `self.active_tab = Tab::Overview;` stays — after a sim completes, jump to Overview. Initial app launch shows InputConfig.

### 2.4 Acceptance

- Cold-start the app: InputConfig tab is selected.
- Click **▶ Run Baseline**: completes, active tab becomes Overview.
- Tab order left-to-right: Input Config | Overview | Transition | Annual Table | Charts | RSU Schedule | Compare.

---

## 3. Task (b) — Overview explains "Surplus" and "Deficit" years

**File:** `src/ui/panels/overview_panel.rs`

Replace the current Surplus/Deficit horizontal label row (currently lines ~163–169) with a small section that **defines the terms inline** so a non-investor understands without leaving the page.

```rust
ui.label(RichText::new("Cash-flow Summary").strong().size(15.0));
ui.add_space(2.0);
ui.label(
    RichText::new(
        "A surplus year is one where total net income (after tax) exceeded total expenses — \
         the portfolio grew or stayed flat. A deficit year is one where expenses exceeded \
         income, meaning savings/buffers had to absorb the gap."
    ).small().color(Color32::from_rgb(180, 180, 180))
);
ui.add_space(4.0);
ui.horizontal(|ui| {
    ui.label(RichText::new(format!("✅ Surplus years: {}", positive_gaps)).color(Color32::GREEN));
    ui.add_space(16.0);
    ui.label(RichText::new(format!("❌ Deficit years: {}", negative_gaps)).color(Color32::RED));
    ui.add_space(16.0);
    ui.label(format!("Range: {}–{}", first_year, last_year));
});
```

Compute `first_year` / `last_year` once near the top of `show()`:

```rust
let first_year = res.annual_summary.first().map(|s| s.year).unwrap_or(0);
let last_year  = res.annual_summary.last().map(|s| s.year).unwrap_or(0);
```

Use these for every "year range" annotation below.

---

## 4. Task (c) — Effective Filing Status: disclose country

The `effective_filing_status` field is a **US** filing-status string (it derives from `SpouseProfile` which is a US-tax construct — see `src/models/config.rs:417–425` and `src/simulation/controller.rs` where it is set). The Japan side has no analogous filing-status switch.

**Change** the Overview grid row (currently `overview_panel.rs:66–69`) to:

```rust
ui.label(RichText::new("Effective US Filing Status:").strong());
ui.label(
    RichText::new(format!("🇺🇸 {}", res.effective_filing_status))
        .color(Color32::from_rgb(180, 220, 255))
);
ui.end_row();

ui.label(RichText::new("Japan Tax Profile:").strong());
ui.label(
    RichText::new(format!("🇯🇵 Permanent resident — Prefecture: {} / City: {}",
        res.prefecture, res.city))
        .color(Color32::from_rgb(180, 255, 200))
);
ui.end_row();
```

Reader now sees both countries explicitly. Use the existing flag glyphs (eframe renders them with NotoSans + system fallback; if rendering is missing, drop the emoji and use "US: …" / "Japan: …").

---

## 5. Task (d) — Investment Location broken down by account

Currently `overview_panel.rs:139–141` shows a single `res.investment_location` value (US/Japan/International). Replace with a per-account breakdown table.

### 5.1 Data: use existing `Account.location`

`Account` already carries `location: AccountLocation` (`src/models/assets.rs:485`). The simulation owns `accounts: HashMap<String, Account>` (see `controller.rs`). We need account names + location + final value.

You must add per-account snapshot data via section 7 below (task f) — once that lands, you can reuse the **retirement-date** rows here.

### 5.2 Render

Replace the single "Investment Location" row with:

```rust
ui.add_space(8.0);
ui.label(RichText::new("Investment Accounts by Location").strong().size(15.0));
ui.add_space(2.0);
egui::Grid::new("inv_location_by_account")
    .num_columns(3)
    .striped(true)
    .spacing([20.0, 4.0])
    .show(ui, |ui| {
        ui.label(RichText::new("Account").strong());
        ui.label(RichText::new("Country").strong());
        ui.label(RichText::new(format!("Final Value ({})", last_year)).strong());
        ui.end_row();

        for row in res.account_snapshots.iter()
            .filter(|r| r.event == AccountSnapshotEvent::FinalYear)
        {
            ui.label(&row.account_name);
            ui.label(country_label(row.location)); // helper: "🇯🇵 Japan", "🇺🇸 US", etc.
            match row.currency.as_str() {
                "JPY" => ui.label(fmt_jpy(row.total_value_native)),
                _     => ui.label(fmt_usd(row.total_value_native)),
            };
            ui.end_row();
        }
    });
```

Add a private helper at the bottom of `overview_panel.rs`:

```rust
fn country_label(loc: crate::models::assets::AccountLocation) -> String {
    use crate::models::assets::AccountLocation::*;
    match loc {
        Us    => "🇺🇸 United States".into(),
        Japan => "🇯🇵 Japan".into(),
        Both  => "🇺🇸 + 🇯🇵 (both)".into(),
        None  => "—".into(),
    }
}
```

---

## 6. Task (e) — Tax / NHI / LTC / RSU / DCR rows: country + year range

Inside the `overview_tax_grid` block (`overview_panel.rs:184–234`), rewrite each label so it states country and the year range the total covers.

Use the already-computed `first_year` / `last_year`:

```rust
let range = format!("{}–{}", first_year, last_year);

ui.label(RichText::new(format!("🇯🇵 Japan — Total Resident Tax Paid ({})", range)).strong());
ui.label(fmt_jpy(total_restax));
ui.end_row();

ui.label(RichText::new(format!("🇯🇵 Japan — Total NHI Paid ({})", range)).strong());
ui.label(fmt_jpy(total_nhi));
ui.end_row();

if total_kaigo > 0.0 {
    ui.label(RichText::new(format!(
        "🇯🇵 Japan — Long-Term Care Cost 介護保険 ({})", range
    )).strong());
    // existing body unchanged
    ui.end_row();
}

ui.label(RichText::new(format!("🇺🇸 US — Total RSU Income ({})", range)).strong());
ui.label(fmt_usd(total_rsu));
ui.end_row();

// Dividend coverage block:
ui.label(RichText::new(format!(
    "Avg Dividend Coverage Ratio (post-retirement, {})",
    // post-retirement subrange
    post_ret_year_range(&res.annual_summary)
)).strong());
ui.label(RichText::new(format!("{:.2}×", avg_dcr)).color(dcr_color));
ui.end_row();
```

Add the helper:

```rust
/// Returns "YYYY–YYYY" for snapshots where div_coverage_ratio > 0 (post-retirement),
/// or "n/a" if none.
fn post_ret_year_range(snaps: &[crate::models::snapshot::AnnualSnapshot]) -> String {
    let mut iter = snaps.iter().filter(|s| s.div_coverage_ratio > 0.0);
    let first = iter.next();
    let last  = snaps.iter().rev().find(|s| s.div_coverage_ratio > 0.0);
    match (first, last) {
        (Some(a), Some(b)) => format!("{}–{}", a.year, b.year),
        _ => "n/a".into(),
    }
}
```

> The DCR is a ratio (gross dividends ÷ expenses) so it isn't country-bound — but the range still matters and the label drop of "Japan/US" is intentional. If you want to be explicit, say "Dividends are USD; expenses are JPY — ratio is currency-normalized to JPY."

---

## 7. Task (f) — Per-account view at Retirement and Rebalance dates

This is the only **non-UI** change in this rearchitecture. The current `SimResults` exposes only aggregated portfolio rollups. We need per-account rows at two event types.

### 7.1 Add the data structure — `src/models/snapshot.rs`

Append:

```rust
/// V8.2 — One row per account, captured at a specific event date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSnapshotRow {
    pub event: AccountSnapshotEvent,
    pub date: chrono::NaiveDate,
    pub account_name: String,
    pub location: crate::models::assets::AccountLocation,
    pub tax_jurisdiction: crate::models::assets::AccountJurisdiction,
    pub currency: String,           // "USD" or "JPY"
    pub total_value_native: f64,    // value in account's native currency
    pub total_value_usd: f64,       // value converted to USD at event-date FX
    pub total_value_jpy: f64,       // value converted to JPY at event-date FX
    pub composition: Vec<AccountAssetRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountAssetRow {
    pub ticker: String,
    pub quantity: f64,
    pub price_native: f64,
    pub market_value_native: f64,
    pub pct_of_account: f64,        // 0.0..1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountSnapshotEvent {
    Retirement,
    Rebalance,
    FinalYear,
}
```

Add a new field to `SimResults`:

```rust
/// V8.2 — Per-account snapshots taken at Retirement, every Rebalance, and FinalYear.
pub account_snapshots: Vec<AccountSnapshotRow>,
```

Default-init it to `Vec::new()` everywhere `SimResults` is constructed (search for `SimResults {` to find every site).

### 7.2 Populate in the controller — `src/simulation/controller.rs`

Add a helper method on the controller:

```rust
fn capture_account_snapshots(&mut self, event: AccountSnapshotEvent, date: chrono::NaiveDate) {
    use crate::models::snapshot::{AccountSnapshotRow, AccountAssetRow};
    let fx = self.state.current_fx;
    for (name, acct) in self.state.accounts.iter() {
        let total_native = acct.total_value(fx);
        let (usd, jpy) = match acct.currency {
            crate::models::assets::Currency::Usd => (total_native, total_native * fx),
            crate::models::assets::Currency::Jpy => (total_native / fx, total_native),
        };
        let composition: Vec<AccountAssetRow> = acct.assets.iter().map(|(t, a)| {
            let mv = a.market_value();
            AccountAssetRow {
                ticker: t.clone(),
                quantity: a.quantity,             // confirm field name; otherwise a.units / a.shares
                price_native: a.price,
                market_value_native: mv,
                pct_of_account: if total_native > 0.0 { mv / total_native } else { 0.0 },
            }
        }).collect();

        self.state.account_snapshots.push(AccountSnapshotRow {
            event, date,
            account_name: name.clone(),
            location: acct.location,
            tax_jurisdiction: acct.tax_jurisdiction,
            currency: match acct.currency {
                crate::models::assets::Currency::Usd => "USD".into(),
                crate::models::assets::Currency::Jpy => "JPY".into(),
            },
            total_value_native: total_native,
            total_value_usd: usd,
            total_value_jpy: jpy,
            composition,
        });
    }
}
```

(Confirm the `Asset` field names by reading `src/models/assets.rs` — adjust `a.quantity` / `a.price` if they're named differently. Inspect around `pub struct Asset {` before writing.)

Add `account_snapshots: Vec<AccountSnapshotRow>` to `SimState` (or wherever transient state lives), default `Vec::new()`. Move it into the final `SimResults` at the end of `run()`.

Call `capture_account_snapshots()` at exactly these places:

1. **Retirement date** — locate the existing block that triggers the retirement transition (search for `transition_report` assignment in `controller.rs`). Capture **before** rebalance executes and **after** rebalance completes, both tagged `AccountSnapshotEvent::Retirement`. Use the retirement date as `date`.
   - Actually take **one** snapshot at `AccountSnapshotEvent::Retirement` — the post-rebalance state. That matches the user's request: "every investment account size, composition, and country at retirement."
2. **Each rebalance event** — find the per-account rebalancer (search for `AccountRebalanceStrategy` usage). After each fire, call `capture_account_snapshots(AccountSnapshotEvent::Rebalance, fire_date)`.
3. **Final year** — at the end of `record_annual_snapshot()` for the last simulated year, call with `AccountSnapshotEvent::FinalYear` and `date = NaiveDate::from_ymd_opt(yr, 12, 31)`. (Easiest: detect "last year" by comparing against `self.cfg.end_date.year()`.)

### 7.3 Render in the Overview — `src/ui/panels/overview_panel.rs`

Below the per-account location table from section 5, add an expandable detail section:

```rust
ui.add_space(10.0);
ui.label(RichText::new("Account Snapshots — Retirement and Rebalance").strong().size(15.0));
ui.add_space(2.0);
ui.label(RichText::new(
    "Composition of every investment account at the retirement date and at each \
     rebalance date. Country and tax jurisdiction shown per account."
).small().color(Color32::from_rgb(180, 180, 180)));
ui.add_space(4.0);

let events: Vec<_> = res.account_snapshots.iter()
    .map(|r| (r.event, r.date))
    .collect::<std::collections::BTreeSet<_>>() // dedup + sort
    .into_iter().collect();

for (event, date) in events {
    let title = match event {
        AccountSnapshotEvent::Retirement => format!("🏁 Retirement — {}", date),
        AccountSnapshotEvent::Rebalance  => format!("⚖ Rebalance — {}", date),
        AccountSnapshotEvent::FinalYear  => format!("📅 Final Year — {}", date),
    };
    egui::CollapsingHeader::new(title)
        .default_open(event == AccountSnapshotEvent::Retirement)
        .show(ui, |ui| {
            for row in res.account_snapshots.iter()
                .filter(|r| r.event == event && r.date == date)
            {
                ui.label(RichText::new(format!(
                    "{} — {} ({})",
                    row.account_name,
                    country_label(row.location),
                    row.tax_jurisdiction,
                )).strong());
                ui.label(format!(
                    "  Total: {} (≈ {} / {})",
                    if row.currency == "JPY" {
                        fmt_jpy(row.total_value_native)
                    } else {
                        fmt_usd(row.total_value_native)
                    },
                    fmt_usd(row.total_value_usd),
                    fmt_jpy(row.total_value_jpy),
                ));
                if !row.composition.is_empty() {
                    egui::Grid::new(format!("comp_{}_{}_{}", row.account_name, event as u8, date))
                        .num_columns(4).striped(true).spacing([16.0, 2.0])
                        .show(ui, |ui| {
                            for h in &["Ticker", "Qty", "Price", "% of acct"] {
                                ui.label(RichText::new(*h).small().strong());
                            }
                            ui.end_row();
                            for a in &row.composition {
                                ui.label(&a.ticker);
                                ui.label(format!("{:.3}", a.quantity));
                                ui.label(format!("${:.2}", a.price_native));
                                ui.label(format!("{:.1}%", a.pct_of_account * 100.0));
                                ui.end_row();
                            }
                        });
                }
                ui.add_space(4.0);
            }
        });
}
```

---

## 8. Task (g) — "Does this scenario work?" verdict for non-investors

Add a **prominent** panel at the **top** of the Overview (above the existing grid), color-coded and plain-English.

### 8.1 Verdict logic

Place this helper at the bottom of `overview_panel.rs`:

```rust
#[derive(Clone)]
struct ScenarioVerdict {
    works: bool,
    summary: String,           // one short sentence
    reasons: Vec<String>,      // why or why not
    recommendations: Vec<String>, // empty if works
}

fn evaluate_scenario(res: &crate::models::snapshot::SimResults) -> ScenarioVerdict {
    let total_years = res.annual_summary.len();
    let deficit_years = res.annual_summary.iter().filter(|s| s.gap_jpy < 0.0).count();
    let warnings = res.gap_warnings.len();
    let unpaid_rsu = res.annual_summary.last()
        .map(|s| s.unpaid_rsu_tax_liability_usd).unwrap_or(0.0);
    let exit_tax_hit = res.annual_summary.iter().any(|s| s.exit_tax_triggered);
    let bridge_exhausted_years = res.annual_summary.iter()
        .filter(|s| s.bridge_exhausted).count();

    // Post-retirement only — strip pre-retirement years where coverage is N/A.
    let dcr_post: Vec<f64> = res.annual_summary.iter()
        .filter(|s| s.div_coverage_ratio > 0.0)
        .map(|s| s.div_coverage_ratio).collect();
    let avg_dcr = if dcr_post.is_empty() { 0.0 }
                  else { dcr_post.iter().sum::<f64>() / dcr_post.len() as f64 };

    // Final portfolio value (USD-equivalent)
    let final_value_usd = res.annual_summary.last().map(|s| {
        s.brokerage_usd + s.roth_usd + (s.dc_jpy / s.usd_jpy)
    }).unwrap_or(0.0);

    let mut reasons = Vec::new();
    let mut recs = Vec::new();

    let deficit_ratio = if total_years > 0 {
        deficit_years as f64 / total_years as f64
    } else { 0.0 };

    let works = warnings == 0
        && unpaid_rsu < 1_000.0
        && bridge_exhausted_years == 0
        && deficit_ratio < 0.20
        && final_value_usd > 0.0;

    if works {
        reasons.push(format!(
            "✅ {} of {} years had no solvency warning.", total_years, total_years));
        reasons.push(format!(
            "✅ Final portfolio (USD-equiv): ${:.0} — still positive at end of horizon.",
            final_value_usd));
        if avg_dcr >= 1.0 {
            reasons.push(format!(
                "✅ Dividends alone covered expenses on average ({:.2}× coverage).", avg_dcr));
        } else if avg_dcr >= 0.5 {
            reasons.push(format!(
                "ℹ Dividends covered {:.0}% of expenses — the rest came from drawdowns.",
                avg_dcr * 100.0));
        }
        return ScenarioVerdict {
            works: true,
            summary: "This scenario supports your retirement.".into(),
            reasons, recommendations: vec![],
        };
    }

    // Failure path — explain and recommend.
    if warnings > 0 {
        reasons.push(format!(
            "❌ {} quarter(s) ran negative — income did not cover expenses.", warnings));
        recs.push("Increase the bridge fund target or war-chest target before retirement.".into());
        recs.push("Delay retirement by 1–2 years to accumulate more buffer capital.".into());
    }
    if bridge_exhausted_years > 0 {
        reasons.push(format!(
            "❌ Bridge fund was exhausted in {} year(s) — forced portfolio sells occurred.",
            bridge_exhausted_years));
        recs.push("Raise the bridge fund cap so it lasts through deficit years.".into());
    }
    if unpaid_rsu >= 1_000.0 {
        reasons.push(format!(
            "❌ ${:.0} in RSU-vest IRS liability remained unpaid (sell-to-cover deficit).",
            unpaid_rsu));
        recs.push("Withhold a higher % at vest, or set aside cash before RSU vest dates.".into());
    }
    if deficit_ratio >= 0.20 {
        reasons.push(format!(
            "❌ {:.0}% of simulated years were deficit years (income < expenses).",
            deficit_ratio * 100.0));
        recs.push("Reduce planned base expenses or boost expected dividend yield.".into());
    }
    if exit_tax_hit {
        reasons.push(
            "⚠ Japan Exit Tax (Article 60-2) would trigger if you leave Japan with current assets.".into());
        recs.push("Consult a Japan-licensed tax advisor before any departure planning.".into());
    }
    if final_value_usd <= 0.0 {
        reasons.push("❌ Portfolio reaches zero before the end of the simulation horizon.".into());
        recs.push("Lower withdrawal rate or extend earning years.".into());
    }

    ScenarioVerdict {
        works: false,
        summary: "This scenario does NOT support your retirement as configured.".into(),
        reasons, recommendations: recs,
    }
}
```

### 8.2 Render at the very top of Overview

Insert this block immediately after `ui.heading("Simulation Overview");` and **before** the existing `egui::Grid::new("overview_grid")`:

```rust
let verdict = evaluate_scenario(res);
let (bg, fg) = if verdict.works {
    (Color32::from_rgb(20, 60, 30), Color32::from_rgb(170, 255, 180))
} else {
    (Color32::from_rgb(70, 25, 25), Color32::from_rgb(255, 180, 180))
};
egui::Frame::none()
    .fill(bg)
    .inner_margin(egui::Margin::symmetric(12.0, 10.0))
    .rounding(egui::Rounding::same(6.0))
    .show(ui, |ui| {
        ui.label(RichText::new(if verdict.works { "✅ Retirement Verdict" } else { "❌ Retirement Verdict" })
            .strong().size(18.0).color(fg));
        ui.label(RichText::new(&verdict.summary).size(15.0).color(fg));
        ui.add_space(6.0);
        ui.label(RichText::new("Why:").strong().color(fg));
        for r in &verdict.reasons {
            ui.label(RichText::new(format!("  • {}", r)).color(fg));
        }
        if !verdict.recommendations.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new("Recommendations:").strong().color(fg));
            for rec in &verdict.recommendations {
                ui.label(RichText::new(format!("  → {}", rec)).color(fg));
            }
        }
    });
ui.add_space(12.0);
```

Reader-friendly. No jargon (no "DCR", no "§904(c)"). Color-coded green/red.

### 8.3 Acceptance for (g)

- A clean scenario shows **green verdict** with 2–3 positive bullets.
- A failing scenario shows **red verdict** with reasons + at least one recommendation.
- No occurrence of acronyms `FTC`, `NIIT`, `PFIC`, `MAGI`, `§…` inside the verdict text.

---

## 9. Task (h) — Transition tab: clearer account + country, country on taxes

**File:** `src/ui/panels/transition_panel.rs`

The tab is already moved (section 2). Now make every transaction row state which account and which country, and every tax line state which country.

### 9.1 Heading: state the account whose transition is being reported

The retirement rebalance currently happens on the `"Taxable"` account (see `TransitionReport`). State this explicitly:

```rust
ui.heading("Retirement Transition Report");
ui.label(RichText::new("Account: 🇺🇸 Taxable Brokerage (US-domiciled)")
    .strong().color(Color32::from_rgb(180, 220, 255)));
ui.add_space(8.0);
```

> If the system later supports per-account transitions, replace `"Taxable Brokerage"` with the account name pulled from the report — but currently `TransitionReport` does not carry an account name, so the literal is correct.

### 9.2 Source & Use of Funds: annotate taxes by country

Rewrite the `transition_funds_grid` block (currently ~lines 60–97):

```rust
ui.label(RichText::new("SOURCE: Portfolio liquidation (🇺🇸 Taxable Brokerage)").strong());
ui.label(format!("${}", c2(total_source)));
ui.end_row();

ui.label("USE: 🇺🇸 US Capital Gains Tax");
ui.label(format!("-${}  (Total: ${} | Pre-funded: ${})",
    c2(alloc.us_tax_paid_from_portfolio), c2(alloc.us_tax_bill), c2(alloc.us_tax_pre)));
ui.end_row();
```

And replace the War-Chest / Bridge rows with annotated versions:

```rust
let wc_country = if alloc.wc_currency == "USD" { "🇺🇸 US" } else { "🇯🇵 Japan" };
ui.label(format!("USE: War Chest Fill ({})", wc_country));
// ...body unchanged...

let bridge_country = if alloc.bridge_fund_currency == "USD" { "🇺🇸 US" } else { "🇯🇵 Japan" };
ui.label(format!("USE: Bridge Fund Fill ({})", bridge_country));
// ...body unchanged...
```

### 9.3 Transaction log: state the account on each side

Above the Sold/Bought tables:

```rust
ui.label(RichText::new("C. Transaction Log — all entries occur in the 🇺🇸 Taxable Brokerage account").strong());
ui.label("Sold (assets liquidated from Taxable Brokerage):");
// ...existing sells grid...
ui.label("Bought (assets purchased into Taxable Brokerage):");
// ...existing buys grid...
```

### 9.4 Tax breakdown: country on every line

Rewrite the `tax_breakdown_grid` heading and rows:

```rust
ui.label(RichText::new("D. Estimated Tax Bills — by Country").strong());
egui::Grid::new("tax_breakdown_grid").num_columns(2).spacing([20.0, 4.0]).show(ui, |ui| {
    ui.label("🇺🇸 US — Gains @ 0%:");  ui.label(format!("${} → Tax: $0.00", c(_g0))); ui.end_row();
    ui.label("🇺🇸 US — Gains @ 15%:"); ui.label(format!("${} → Tax: ${}", c(g15), c2(g15 * 0.15))); ui.end_row();
    ui.label("🇺🇸 US — Gains @ 20%:"); ui.label(format!("${} → Tax: ${}", c(g20), c2(g20 * 0.20))); ui.end_row();
    ui.label("🇺🇸 US — NIIT (3.8% surtax):"); ui.label(format!("${}", c2(niit))); ui.end_row();
    ui.label(RichText::new("🇺🇸 US — Total Federal Tax Due:").strong());
    ui.label(RichText::new(format!("${}", c2(alloc.us_tax_bill))).strong());
    ui.end_row();
    ui.label(format!("🇯🇵 Japan — Resident Tax ({}+1):", t.date.year()));
    ui.label(format!("¥{}", c(alloc.jp_tax_bill)));
    ui.end_row();
});
```

### 9.5 Acceptance for (h)

- Heading line names the account and its country.
- Every Source/Use row mentions either an account or a country.
- The transaction-log preamble explicitly names the account both sides of trades occur in.
- Every line in the tax-breakdown grid begins with `🇺🇸 US —` or `🇯🇵 Japan —`.

---

## 10. Tests

Add to `tests/` a new file `ui_tab_order_test.rs`:

```rust
// Confirms the tab order and default tab without launching egui.
// The Tab enum is private — bring it out via a small helper or make it pub(crate)
// only if it isn't already, then revert. Simpler approach: integration test that
// just builds the app and asserts the default tab variant matches `InputConfig`.
```

If `Tab` is private and you don't want to widen it, drop this unit test and add a one-line comment in `app.rs` pointing at the spec.

Add to the existing controller integration tests a new test (e.g., in `tests/transition_test.rs` or a new file) that asserts:

```rust
#[test]
fn account_snapshots_contain_retirement_event() {
    // load a known fixture, run, assert at least one row with event == Retirement
    // assert composition is non-empty
}

#[test]
fn account_snapshots_contain_final_year_event() {
    // similar, event == FinalYear
}
```

If a fixture scenario JSON already exists under `input/` (e.g. `input/sample_baseline.json`), use it.

Run `cargo test` — all 236 prior tests must still pass.

---

## 11. Version & docs

- Update the version in `Cargo.toml` to `8.2.0`.
- Update `README.md` if it has a version banner.
- Add one line to README's changelog if one exists: `V8.2.0 — UX Clarity Pass: tab reorder, per-account snapshots, plain-English retirement verdict, country/year labels.`

---

## 12. Out of scope (do NOT change)

- Tax engines, FTC carryover logic, PFIC MTM, estate computations.
- Chart panel, RSU panel, Comparison panel.
- The Input Config panel's layout (no UX changes inside it).
- The reporter's text/CSV output (only mirror new annotations if trivial — otherwise leave a `// TODO V8.2` and continue).

---

## 13. Final acceptance checklist

- [ ] Tab order: Input Config | Overview | Transition | Annual Table | Charts | RSU Schedule | Compare.
- [ ] App opens to Input Config; after a run, jumps to Overview.
- [ ] Overview has a top-of-page green/red verdict box with plain-English reasons & recommendations.
- [ ] Overview defines "Surplus year" and "Deficit year" inline.
- [ ] Effective filing status row prefixed with US flag; new Japan tax-profile row.
- [ ] Per-account Investment Location table shows account, country, final value.
- [ ] Per-account snapshots show composition at Retirement, each Rebalance, and Final Year — with country + tax jurisdiction per account.
- [ ] Resident Tax / NHI / LTC / RSU rows all show country flag + year range.
- [ ] Transition tab: account named, every Source/Use line annotated, transaction log preamble explicit, every tax line prefixed by country.
- [ ] `cargo test` passes; new tests for `account_snapshots` added.
- [ ] Version bumped to 8.2.0.

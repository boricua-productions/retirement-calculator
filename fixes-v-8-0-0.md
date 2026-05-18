# Fixes V8.0.0 — Implementation Instructions for Sonnet

**Source spec:** `docs/corrections-app-v-8-0-0.md`
**Target branch:** `main` (V7.11.1 baseline → V8.0.0-alpha)
**Author:** Opus 4.7 (analysis) → Sonnet (execution)
**Status:** Approved for staged implementation

---

## 0. Executive Summary — Applicability Verdict

| # | Spec Section | Applicable? | Effort | Notes |
|---|--------------|-------------|--------|-------|
| 1.1 | Resident Tax Cadence (Tokubetsu vs Futsu) | **YES** | Medium | Currently no active-phase scheduling; legacy 4-quarter logic exists |
| 1.2 | 2026 Baseline Parameter Sync | **YES** | Small | Constants only |
| 2.1 | 2026 Std Deduction / Filing Status | **YES** | Small | `for_filing_status` already takes `&str`; just refresh numbers |
| 2.2 | §70103 Enhanced Senior Deduction | **YES** | Medium | New layer on top of existing `senior_addon_per_person` |
| 2.3 | SSDI Combined Income Multi-Bracket | **YES** | Medium | Function signature change — threads filing status |
| 2.4 | §904(c) FTC Carryover Queue (FIFO) | **YES** | Medium | Replaces scalar fields in `SimState` |
| 3.1 | Japan Total Moving Average (総平均法) | **YES** | Large | `avg_jpy_basis_per_share` is currently load-time only |
| 3.2 | US State Tax → Japan FTC Pool | **PARTIAL** | Large | No Japan-side FTC engine exists yet — would need to add one |
| 3.3 | Japan 3-Year Capital Loss Ledger | **YES** | Small | Replace scalar `japan_loss_carryforward_jpy` with 3-slot ledger |
| 3.4 | Reconstruction Surcharge Sunset (2038) | **YES** | Small | Pass `year` into `calculate_income_tax` |
| 3.5 | Visa Classification Exit Tax Filter | **YES** | Small | New `VisaType` enum + early return |
| 4.1 | Japan Spousal Mitigation (Art. 19-2) | **YES** | Medium | Replace 0.5 shortcut in `project_at_death` |
| 5 | Order-of-Execution Invariants | **YES (audit)** | Small | Mostly already correct; add asserts + a doc comment block |

**Recommended phasing for Sonnet:**
- **Phase A (constants & enums):** 1.2, 2.1, 3.4 — pure value/signature changes, fast tests
- **Phase B (controller wiring):** 1.1, 2.2, 2.3, 3.5 — additions to existing engines
- **Phase C (state-shape changes):** 2.4, 3.3 — `SimState` field migration; touches serialization
- **Phase D (asset-model changes):** 3.1 — touches lot acquisition flow
- **Phase E (estate engine):** 4.1 — isolated to `estate_tax.rs`
- **Phase F (deferred):** 3.2 — leave as TODO; document the gap

**Defer 3.2** unless the user explicitly requests it. The current code has no Japan-side FTC routine (it only models Japan-First FTC from the US side). Building Japan FTC infrastructure is a separate Stage 14 initiative and is out of scope for this remediation pass.

---

## 1. Pre-flight Checklist (run before any edits)

```powershell
cargo build --release
cargo test --no-fail-fast 2>&1 | Tee-Object -FilePath baseline_test_output.txt
```

Record the test count and any pre-existing failures in `baseline_test_output.txt`. After each phase below, re-run `cargo test` and confirm only the explicitly-changed assertions break.

**Memory**: per `memory/project_retirement_calc.md` the project should be at 31/31 tests passing. Confirm this baseline first; if the count differs, **stop and report** rather than proceeding.

---

## Phase A — Constants & Pure-Value Fixes

### A.1 — 2026 Federal Tax Constants (Spec §1.2 + §2.1)

**File:** `src/engine/tax/us_tax.rs`

1. Update the FEIE constant:
   ```rust
   /// 2026 Foreign Earned Income Exclusion annual limit (USD), IRC §911.
   pub const FEIE_LIMIT_2026: f64 = 132_900.0;
   ```

2. Replace `TaxRules::for_filing_status` body with 2026 statutory values:
   ```rust
   pub fn for_filing_status(status: &str) -> TaxRules {
       match status {
           "Single" | "Married Filing Separately" => TaxRules {
               filing_status: status.into(),
               std_deduction: 16_100.0,
               ltcg_0_limit:  47_025.0,       // Keep — 2026 indexed value
               ltcg_15_limit: 518_900.0,      // Keep
               niit_threshold: 200_000.0,
               senior_addon_per_person: 1_950.0,
               brackets: vec![
                   (11_925.0, 0.10),          // 2026 indexed
                   (48_475.0, 0.12),
                   (103_350.0, 0.22),
                   (197_300.0, 0.24),
                   (250_525.0, 0.32),
                   (626_350.0, 0.35),
                   (f64::INFINITY, 0.37),
               ],
               ..TaxRules::default()
           },
           "Head of Household" => TaxRules {
               filing_status: status.into(),
               std_deduction: 24_150.0,
               ltcg_0_limit:  63_000.0,
               ltcg_15_limit: 551_350.0,
               niit_threshold: 200_000.0,
               senior_addon_per_person: 1_950.0,
               brackets: vec![
                   (17_000.0, 0.10),
                   (64_850.0, 0.12),
                   (103_350.0, 0.22),
                   (197_300.0, 0.24),
                   (250_500.0, 0.32),
                   (626_350.0, 0.35),
                   (f64::INFINITY, 0.37),
               ],
               ..TaxRules::default()
           },
           _ => TaxRules::default(),  // MFJ — see #3 below
       }
   }
   ```

3. Update `TaxRules::default()` (the MFJ baseline) in `src/models/config.rs`:
   ```rust
   std_deduction: 32_200.0,
   ltcg_0_limit:  115_000.0,    // Keep
   ltcg_15_limit: 700_000.0,    // Keep
   niit_threshold: 250_000.0,   // MFJ (already correct)
   senior_addon_per_person: 1_550.0,
   brackets: vec![
       (23_850.0, 0.10),         // 2026 indexed MFJ brackets
       (96_950.0, 0.12),
       (206_700.0, 0.22),
       (394_600.0, 0.24),
       (501_050.0, 0.32),
       (752_700.0, 0.35),
       (f64::INFINITY, 0.37),
   ],
   ```

4. Update the existing `test_ltcg_*` tests' expected values to reflect the new MFJ std_deduction ($35K → $32.2K). Recompute by hand and patch each assertion. Expect ~3 test failures here that need numeric updates.

### A.2 — US Estate Exclusion Floor (Spec §1.2)

**File:** `src/engine/tax/estate_tax.rs`, function `us_estate_exclusion`

Replace the post-2026 branch:
```rust
fn us_estate_exclusion(year: i32) -> f64 {
    if year < 2026 {
        13_610_000.0 * (1.028_f64).powi((year - 2024).max(0))
    } else {
        // 2026 Unified Credit Guidelines (OBBBA permanent extension).
        15_000_000.0 * (1.028_f64).powi((year - 2026).max(0))
    }
}
```

Update the test `us_estate_post_sunset`:
```rust
#[test]
fn us_estate_post_sunset() {
    let tax = compute_us_estate_tax(20_000_000.0, 2026);
    let expected = (20_000_000.0 - 15_000_000.0) * 0.40;
    assert!((tax - expected).abs() < 1.0);
}
```

### A.3 — Reconstruction Surcharge Sunset (Spec §3.4)

**File:** `src/engine/tax/japan_tax.rs`

1. Add a new function signature that takes the year:
   ```rust
   pub fn calculate_income_tax_for_year(
       gross_salary_jpy: f64,
       gross_pension_jpy: f64,
       social_insurance_paid_jpy: f64,
       age: i32,
       num_dependents: u32,
       current_year: i32,
   ) -> f64 {
       // ... existing body up through `let base_tax = ...;` ...
       let surcharge = if current_year <= 2037 { 1.021 } else { 1.000 };
       base_tax * surcharge
   }
   ```

2. Keep the old `calculate_income_tax` as a thin wrapper that defaults to year 2026 (or the year embedded in any caller) **only if** doing so doesn't break callers. Better: change the signature directly and update the one caller in `controller.rs::compute_working_year_japan_income_tax` to pass `yr`.

3. Add a unit test:
   ```rust
   #[test]
   fn reconstruction_surcharge_sunsets_after_2037() {
       let pre  = JapanTaxEngine::calculate_income_tax_for_year(
           10_000_000.0, 0.0, 0.0, 50, 0, 2037);
       let post = JapanTaxEngine::calculate_income_tax_for_year(
           10_000_000.0, 0.0, 0.0, 50, 0, 2038);
       // post should be slightly lower (1.000 vs 1.021)
       assert!(post < pre);
       assert!((pre / post - 1.021).abs() < 0.001);
   }
   ```

**Checkpoint:** Run `cargo test` after Phase A. Net change should be: ~3 LTCG tests updated, 1 estate test updated, 1 new surcharge test passing. Total still 31+ tests.

---

## Phase B — Engine Additions

### B.1 — Resident Tax Cadence (Spec §1.1)

**File:** `src/simulation/controller.rs`

The current `schedule_annual_resident_tax` runs only after retirement and always uses the 4-quarter cycle. The spec demands the 12-month Tokubetsu Choushuu cadence during active employment (when the employer would normally withhold).

**Important caveat:** Pre-retirement in this model, the user's salary is recorded gross (`total_annual_compensation_usd`). Resident tax is not currently deducted as a separate cashflow line because employer withholding is implicitly netted. Adding Tokubetsu Choushuu scheduling here will create a NEW expense pipeline that didn't exist before. This is a **behavioral change** — discuss with the user before merging.

**Recommended approach:**
1. Add a config flag (default `false` for back-compat):
   ```rust
   /// V8.0 — When true, model active-phase resident tax as a 12-month Tokubetsu
   /// Choushuu (Special Collection) deduction. When false (default), assume
   /// employer-withheld and net (legacy V7 behaviour).
   #[serde(default)]
   pub model_active_phase_resident_tax: bool,
   ```

2. In `schedule_annual_resident_tax`, gate the cadence selection by `state.is_retired`:
   ```rust
   let is_retired = self.state.date >= self.cfg.retirement_date;
   if is_retired {
       // EXISTING 4-quarter (Futsu Choushuu) logic — unchanged.
   } else if self.cfg.model_active_phase_resident_tax {
       // NEW 12-month (Tokubetsu Choushuu) cadence: June Y through May Y+1.
       let monthly = tax_bill / 12.0;
       let mut rules = Vec::with_capacity(12);
       for m_offset in 0..12 {
           let abs_month = 6 + m_offset;
           let (yr_, mo_) = if abs_month <= 12 {
               (current_year, abs_month)
           } else {
               (current_year + 1, abs_month - 12)
           };
           let start = NaiveDate::from_ymd_opt(yr_, mo_ as u32, 1).unwrap();
           let end = start.with_day(28).unwrap();  // safe end-of-window stub
           rules.push(ExpenseRule::new(
               format!("ResTax {} M{:02}", current_year, m_offset + 1),
               monthly, start, end,
           ));
       }
       self.cfg.expense_rules.extend(rules.clone());
       self.cf_engine.add_expense_rules(&rules);
   }
   ```

3. Remove the `if self.state.date >= self.cfg.retirement_date` gate in `handle_new_year` around `self.schedule_annual_resident_tax(yr);` so it can fire for active years when the flag is set.

4. Add an integration test that toggles the flag and asserts that pre-retirement years now have 12 ResTax expense rules instead of 0.

### B.2 — §70103 Enhanced Senior Deduction (Spec §2.2)

**File:** `src/engine/tax/us_tax.rs`

Add a new helper function:
```rust
/// OBBBA §70103 — Temporary Enhanced Senior Deduction (TY 2025–2028).
/// Returns the additive deduction pool in USD after the 6% MAGI phase-out.
///
/// * `eligible_seniors` — count of taxpayer + spouse who reach age ≥ 65 by Dec 31.
/// * `magi` — Modified Adjusted Gross Income (USD).
/// * `filing_status` — "Married Filing Jointly" or other.
pub fn enhanced_senior_deduction_2026(
    eligible_seniors: u32,
    magi: f64,
    filing_status: &str,
) -> f64 {
    if eligible_seniors == 0 { return 0.0; }
    let max_pool = 6_000.0 * eligible_seniors as f64;
    let (threshold, floor_at) = if filing_status == "Married Filing Jointly" {
        (150_000.0, 350_000.0)
    } else {
        (75_000.0, 175_000.0)
    };
    if magi <= threshold {
        max_pool
    } else if magi >= floor_at {
        0.0
    } else {
        (max_pool - (magi - threshold) * 0.06).max(0.0)
    }
}
```

**Wire it into the year-end true-up.** In `src/simulation/controller.rs::finalize_year_taxes`, AFTER the existing `senior_bonus` block (~line 880), add:

```rust
// V8.0 — §70103 Enhanced Senior Deduction (temporary, TY 2025–2028).
if yr <= 2028 {
    let eligible: u32 =
        (if user_age >= 65 { 1 } else { 0 })
        + (if spouse_is_senior { 1 } else { 0 });
    let magi_est = total_ord + total_cap;
    let extra = crate::engine::tax::us_tax::enhanced_senior_deduction_2026(
        eligible, magi_est, &self.tax_engine.rules.filing_status,
    );
    self.tax_engine.rules.std_deduction += extra;
}
```

Make sure the existing `saved_std_deduction` restore at the end of `finalize_year_taxes` still restores correctly (it does — it saves before the bonus block).

Add unit tests for the function: zero seniors → 0; one senior, MFJ, MAGI=$200K → $3,000 ((6,000 − (200K−150K)×0.06)); MFJ MAGI=$350K → 0; etc.

### B.3 — SSDI Multi-Bracket Selector (Spec §2.3)

**File:** `src/engine/tax/us_tax.rs`

1. Replace `ssdi_combined_income_taxable_portion` with a filing-aware variant:
   ```rust
   pub fn ssdi_combined_income_taxable_portion(
       provisional_income: f64,
       annual_ssdi: f64,
       filing_status: &str,
       lived_with_spouse_during_year: bool,
   ) -> f64 {
       if annual_ssdi <= 0.0 { return 0.0; }
       let (low, high) = match filing_status {
           "Married Filing Jointly"  => (32_000.0, 44_000.0),
           "Single" | "Head of Household" => (25_000.0, 34_000.0),
           "Married Filing Separately" => {
               if lived_with_spouse_during_year { (0.0, 0.0) }
               else { (25_000.0, 34_000.0) }
           }
           _ => (32_000.0, 44_000.0),  // safe MFJ default
       };
       if provisional_income <= low {
           0.0
       } else if provisional_income <= high {
           ((provisional_income - low) * 0.5).min(annual_ssdi * 0.5)
       } else {
           let tier1 = (high - low) * 0.5;
           let tier2 = (provisional_income - high) * 0.85;
           (tier1 + tier2).min(annual_ssdi * 0.85)
       }
   }
   ```

2. Update the call site in `controller.rs::finalize_year_taxes` (~line 820):
   ```rust
   let ssdi_taxable = if annual_ssdi > 0.0 {
       let provisional_income = base_ord + 0.5 * annual_ssdi;
       ssdi_combined_income_taxable_portion(
           provisional_income,
           annual_ssdi,
           &self.tax_engine.rules.filing_status,
           true,  // assume MFS pairs live together unless tracked elsewhere
       )
   } else {
       0.0
   };
   ```

3. Update all existing `ssdi_combined_income_taxable_portion` tests (~5 tests in `us_tax.rs`) to pass the new arguments. The MFJ default tests will still pass with `"Married Filing Jointly"`.

4. Add new tests for Single ($25K/$34K) and MFS-with-spouse ($0/$0) boundary cases.

### B.4 — Visa Classification Exit Tax Filter (Spec §3.5)

**File:** `src/models/config.rs`

1. Add the enum:
   ```rust
   /// V8.0 — Japan visa classification for Exit Tax eligibility.
   /// Per IT Act Art. 60-2, only Table 2 visa holders are subject to the
   /// 5-of-10-year residency test.
   #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
   #[serde(rename_all = "snake_case")]
   pub enum VisaType {
       /// Table 1 (work visas: engineer, intra-company transferee, etc.).
       /// EXEMPT from Exit Tax regardless of years of residence.
       #[default]
       Table1,
       /// Table 2 (Permanent Resident, Spouse of Japanese National, Long-Term Resident).
       /// Subject to Exit Tax once 5-of-10-year residency test is met.
       Table2,
   }
   ```

2. Add the field to `Config`:
   ```rust
   /// V8.0 — Visa type for Exit Tax evaluation. Defaults to Table1 (exempt).
   #[serde(default)]
   pub primary_taxpayer_visa: VisaType,
   ```

3. **File:** `src/simulation/controller.rs::evaluate_exit_tax_trigger` — add an early return at the top:
   ```rust
   fn evaluate_exit_tax_trigger(&self, yr: i32) -> (bool, f64) {
       use crate::models::config::VisaType;
       if self.cfg.primary_taxpayer_visa == VisaType::Table1 {
           return (false, 0.0);  // Table 1 visas exempt per IT Act Art. 60-2.
       }
       // ... existing logic unchanged.
   }
   ```

4. Add a test toggling the visa type and asserting that Table1 never triggers regardless of asset size or residency duration.

**Checkpoint:** Run `cargo test` after Phase B. Expect new tests to pass and ~5 SSDI tests to need argument-list updates.

---

## Phase C — State-Shape Migrations

### C.1 — Japan 3-Year Capital Loss Ledger (Spec §3.3)

**File:** `src/simulation/state.rs`

1. Replace the scalar field:
   ```rust
   // REMOVE:
   // pub japan_loss_carryforward_jpy: f64,

   // ADD:
   /// V8.0 — Rolling 3-year Japan capital-loss carry-forward ledger per
   /// 租税特別措置法 第37条の12の2. Each slot holds losses realized in that
   /// many years prior; the oldest slot is discarded at year-rollover.
   pub japan_loss_ledger: JapanLossLedger,
   ```

2. Add the struct in the same file (or `src/models/snapshot.rs` if you prefer it serializable):
   ```rust
   #[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
   pub struct JapanLossLedger {
       pub year_minus_1: f64,
       pub year_minus_2: f64,
       pub year_minus_3: f64,
   }

   impl JapanLossLedger {
       /// Total carry-forward available against this year's gains (JPY).
       pub fn total(&self) -> f64 {
           self.year_minus_1 + self.year_minus_2 + self.year_minus_3
       }
       /// Advance the ledger one year. Discard year_minus_3, shift others down,
       /// load the most recent year's loss into year_minus_1.
       pub fn roll(&mut self, current_year_loss: f64) {
           self.year_minus_3 = self.year_minus_2;
           self.year_minus_2 = self.year_minus_1;
           self.year_minus_1 = current_year_loss.max(0.0);
       }
   }
   ```

3. Update `SimState::new` initializer: remove the scalar init, add `japan_loss_ledger: JapanLossLedger::default(),`.

4. **File:** `src/simulation/controller.rs::handle_new_year` — replace the existing block (~lines 381–384):
   ```rust
   // V8.0 — Roll the 3-year capital-loss ledger per 租特法 37条の12の2.
   let new_loss = self.state.stats.year_japan_cap_loss_jpy;
   self.state.japan_loss_ledger.roll(new_loss);
   ```

5. Grep for any other reads of `japan_loss_carryforward_jpy` (use Grep tool). Replace each with `self.state.japan_loss_ledger.total()`. Likely candidates: NHI scheduler, resident-tax scheduler, dividend handler — at this writing only the init/decay site exists, so this may be a single-site change.

6. Add a unit test for `JapanLossLedger::roll` covering: 3-year decay, max(0) on negative input, total() across all slots.

### C.2 — IRC §904(c) FIFO Carryover Queue (Spec §2.4)

**File:** `src/models/snapshot.rs` (so it's serializable) — add at module level:

```rust
/// V8.0 — A single FTC lot in a §904(c) carryover queue.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FtcLot {
    pub origin_year: u16,
    pub remaining_credit_usd: f64,
}

/// V8.0 — Per-basket FIFO queue tracking §904(c) 10-year carryover lifetime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FtcCarryoverQueue {
    pub passive_basket: Vec<FtcLot>,
    pub general_basket: Vec<FtcLot>,
}

impl FtcCarryoverQueue {
    /// Sum remaining credit across all non-expired lots in a basket.
    pub fn passive_total(&self) -> f64 {
        self.passive_basket.iter().map(|l| l.remaining_credit_usd).sum()
    }
    pub fn general_total(&self) -> f64 {
        self.general_basket.iter().map(|l| l.remaining_credit_usd).sum()
    }
    /// Add a new origin-year lot to a basket (skip zero/negative).
    pub fn add_passive(&mut self, year: u16, credit: f64) {
        if credit > 0.0 {
            self.passive_basket.push(FtcLot { origin_year: year, remaining_credit_usd: credit });
        }
    }
    pub fn add_general(&mut self, year: u16, credit: f64) {
        if credit > 0.0 {
            self.general_basket.push(FtcLot { origin_year: year, remaining_credit_usd: credit });
        }
    }
    /// FIFO consume up to `amount` from a basket; returns the amount actually used.
    pub fn consume_passive(&mut self, mut amount: f64) -> f64 {
        let mut used = 0.0;
        for lot in self.passive_basket.iter_mut() {
            if amount <= 0.0 { break; }
            let take = lot.remaining_credit_usd.min(amount);
            lot.remaining_credit_usd -= take;
            amount -= take;
            used += take;
        }
        self.passive_basket.retain(|l| l.remaining_credit_usd > 1e-9);
        used
    }
    pub fn consume_general(&mut self, mut amount: f64) -> f64 {
        let mut used = 0.0;
        for lot in self.general_basket.iter_mut() {
            if amount <= 0.0 { break; }
            let take = lot.remaining_credit_usd.min(amount);
            lot.remaining_credit_usd -= take;
            amount -= take;
            used += take;
        }
        self.general_basket.retain(|l| l.remaining_credit_usd > 1e-9);
        used
    }
    /// Evict any lot older than 10 years per IRC §904(c).
    pub fn evict_expired(&mut self, current_year: u16) {
        self.passive_basket.retain(|l| current_year.saturating_sub(l.origin_year) <= 10);
        self.general_basket.retain(|l| current_year.saturating_sub(l.origin_year) <= 10);
    }
}
```

**File:** `src/simulation/state.rs`

1. Add the new field:
   ```rust
   /// V8.0 — IRC §904(c) FIFO carryover queue per basket (10-year lifetime).
   pub ftc_queue: FtcCarryoverQueue,
   ```
2. Keep `ftc_carryover_passive_usd` / `ftc_carryover_general_usd` / `ftc_carryover_usd` AS DERIVED FIELDS but recompute them from the queue at snapshot time. (Easier than removing — preserves the reporter/snapshot interface.) Set them in `SimState::new` to 0.0 and update them after each annual true-up.
3. Initialize `ftc_queue: FtcCarryoverQueue::default()` in `SimState::new`.

**File:** `src/simulation/controller.rs::finalize_year_taxes`

Replace the carryover-tracking block (~lines 923–980) with FIFO logic:

```rust
// V8.0 — IRC §904(c) FIFO carryover queue.
// 1. Evict lots older than 10 years.
self.state.ftc_queue.evict_expired(yr as u16);
// 2. Build the effective credit pool for THIS year: queue carry + new credits.
let effective_passive_usd = self.state.ftc_queue.passive_total() + japan_tax_passive_usd;
let effective_general_usd = self.state.ftc_queue.general_total() + japan_tax_general_usd;
let effective_japan_tax_usd = effective_passive_usd + effective_general_usd;

// ... existing FEIE-vs-basket call site uses these effective_* values, unchanged ...

// 3. AFTER `liability` is computed, FIFO-consume by basket (oldest lots first).
let passive_used = liability.breakdown.get("ftc_passive").copied().unwrap_or(0.0);
let general_used = liability.breakdown.get("ftc_general").copied().unwrap_or(0.0);
// Consume from old lots first, then current-year residual.
let p_consumed_from_queue = self.state.ftc_queue.consume_passive(passive_used);
let g_consumed_from_queue = self.state.ftc_queue.consume_general(general_used);
// Whatever wasn't consumed from current-year credits becomes a new lot.
let p_new_credit = japan_tax_passive_usd - (passive_used - p_consumed_from_queue).max(0.0);
let g_new_credit = japan_tax_general_usd - (general_used - g_consumed_from_queue).max(0.0);
self.state.ftc_queue.add_passive(yr as u16, p_new_credit.max(0.0));
self.state.ftc_queue.add_general(yr as u16, g_new_credit.max(0.0));

// 4. Sync legacy scalar fields so existing snapshots / reporter still work.
self.state.ftc_carryover_passive_usd = self.state.ftc_queue.passive_total();
self.state.ftc_carryover_general_usd = self.state.ftc_queue.general_total();
self.state.ftc_carryover_usd = self.state.ftc_carryover_passive_usd
    + self.state.ftc_carryover_general_usd;
```

5. Add tests:
   - 10-year eviction: insert a lot at year 2026, query at year 2037 → present; at 2038 → evicted.
   - FIFO consumption: insert lots at 2026 and 2027, consume amount equal to half the 2026 lot → 2026 partially drained, 2027 untouched.
   - `passive_total` / `general_total` honest after consume + evict.

**Checkpoint:** Run `cargo test`. Existing FTC carryover test (`test_crash_2030_stress_scenario`) should still pass since the totals are preserved; new FIFO unit tests should pass.

---

## Phase D — Asset Model Change

### D.1 — Japan Total Moving Average (Spec §3.1)

**File:** `src/models/assets.rs`

This is the largest change. Currently `Asset.avg_jpy_basis_per_share` is set at load time from `Position.avg_purchase_price_jpy` and never updated when new lots are added.

1. Modify `Asset::add_lot` to take the FX rate at acquisition and update the running JPY average:
   ```rust
   /// V8.0 — Add a new lot AND update the JPY weighted-average basis per
   /// 総平均法に準ずる方法 (Japan Income Tax Order Art. 119-2).
   ///   weighted_avg = Σ(purchase_usd × fx_at_purchase) / total_shares
   pub fn add_lot_with_fx(
       &mut self, purchase_date: NaiveDate, qty: f64, basis: f64, fx_at_purchase: f64,
   ) {
       let prior_qty = self.qty();
       let prior_jpy_total = prior_qty * self.avg_jpy_basis_per_share;
       let new_jpy_total = basis * fx_at_purchase;
       let new_total_qty = prior_qty + qty;
       if new_total_qty > 0.0 {
           self.avg_jpy_basis_per_share = (prior_jpy_total + new_jpy_total) / new_total_qty;
       }
       // Then delegate to the FIFO USD-lot insert.
       self.add_lot(purchase_date, qty, basis);
   }
   ```

2. **Important:** Keep the legacy `add_lot(date, qty, basis)` as a thin wrapper that takes a FX-less path — it will leave `avg_jpy_basis_per_share` at its existing value. Any code that needs total-moving-average must migrate to `add_lot_with_fx`.

3. Find all callers of `add_lot` (use Grep):
   ```
   Grep pattern: \.add_lot\(
   ```
   Expect hits in: `loader.rs` (initial load), `dividends.rs` (DRIP reinvest), `cashflow_manager.rs` (rebalance buys), `contributions.rs`, `roth_rebalancer.rs`, `rsu_vesting.rs`. For each, decide:
   - Initial-load lots in `loader.rs`: do NOT migrate — load-time JPY basis is authoritative.
   - DRIP and rebalance buys: MIGRATE — pass `state.current_fx`.
   - RSU vests: MIGRATE — pass `state.current_fx` at vest date.

4. **File:** `src/handlers/cashflow_manager.rs::v7_liquidate_for_deficit` (~line 866)

   The current liquidation reads `asset.jpy_basis_per_share(fx)`. After D.1, this still works because `avg_jpy_basis_per_share` is now the proper weighted average. No code change needed at the liquidation site — but **add a code comment** documenting that the basis read here is the Japan 総平均法 weighted average, not USD-derived.

5. Add a unit test:
   ```rust
   #[test]
   fn moving_average_jpy_basis_weighted_correctly() {
       let mut a = Asset::new("VTI", 250.0, 0.015, 0.07);
       // Tranche 1: 100 sh @ $200 basis, fx=¥150 → JPY basis = $200 × 150 = ¥30,000/sh
       a.add_lot_with_fx(NaiveDate::from_ymd_opt(2020,1,1).unwrap(), 100.0, 20_000.0, 150.0);
       assert!((a.avg_jpy_basis_per_share - 30_000.0).abs() < 0.01);
       // Tranche 2: 100 sh @ $250 basis, fx=¥160 → JPY basis = $250 × 160 = ¥40,000/sh
       // Weighted avg = (100×30,000 + 100×40,000) / 200 = ¥35,000/sh
       a.add_lot_with_fx(NaiveDate::from_ymd_opt(2021,1,1).unwrap(), 100.0, 25_000.0, 160.0);
       assert!((a.avg_jpy_basis_per_share - 35_000.0).abs() < 0.01);
   }
   ```

6. **Edge case to watch:** The Stage-05 PFIC drift checker (`src/engine/tax/pfic.rs`) cross-validates USD-derived vs JPY-tracked basis. After D.1, this comparison becomes meaningful — currently it can flag false drift because the two were diverging by design. Re-read pfic.rs and either:
   - Disable drift warnings when `avg_jpy_basis_per_share > 0.0` (legitimately diverged), OR
   - Tighten the drift tolerance now that the JPY side is properly tracked.

**Checkpoint:** Run `cargo test`. The moving-average test should pass; the crash-stress integration test may produce slightly different `forced_liquidations_usd` totals — that's expected, since liquidation tax now uses a properly-weighted JPY basis. **Re-baseline the assertion** to `> 0.0` if necessary, not to a specific dollar value.

---

## Phase E — Estate Engine

### E.1 — Japan Spousal Mitigation (Spec §4.1)

**File:** `src/engine/tax/estate_tax.rs::EstatePlanningEngine::project_at_death`

Replace the simplistic `japan_tax_jpy *= 0.5;` shortcut (~line 249) with a per-heir computation following Article 19-2:

```rust
// V8.0 — Article 19-2 (配偶者の税額軽減) full implementation.
// Spouse's tax exemption credit = tax due on max(statutory share, ¥160M).
//
// Algorithm:
// 1. Compute total tentative inheritance tax across heirs (already done above).
// 2. Identify spouse's actual share fraction.
// 3. Compute the credit = tax on the larger of (statutory share value, ¥160M).
// 4. Subtract credit from spouse's personal liability (cannot go below ¥0).
let japan_tax_jpy = if has_spouse {
    // Recompute by walking each heir individually.
    let basic_exclusion = 30_000_000.0 + 6_000_000.0 * heir_count as f64;
    let taxable = (total_jpy - basic_exclusion).max(0.0);
    let mut total_after_credit = 0.0_f64;
    for (i, heir) in cfg.heirs.iter().enumerate() {
        let share = heir_shares[i];
        let share_value = taxable * share;
        let heir_tax = japan_bracket_tax(share_value);
        if heir.relationship == HeirRelationship::Spouse {
            // Article 19-2 ceiling: tax on max(statutory_share_value, ¥160M).
            let ceiling_value = share_value.max(160_000_000.0);
            let credit_basis = japan_bracket_tax(ceiling_value.min(taxable));
            let credit = credit_basis.min(heir_tax);
            total_after_credit += (heir_tax - credit).max(0.0);
        } else {
            total_after_credit += heir_tax;
        }
    }
    total_after_credit
} else {
    compute_japan_sozoku_zei(total_jpy, heir_count, &heir_shares)
};
```

Note: `japan_bracket_tax` is currently private to the module — that's fine because this code is in the same file.

Add tests:
- A pure-spouse heir under ¥160M → spouse credit fully absorbs liability → total = ¥0.
- A spouse + 1 child, estate ¥300M → verify spouse's allocation is reduced by the ¥160M-or-statutory-share-tax credit while the child's liability is untouched.

**Checkpoint:** Run `cargo test`. Existing estate tests should still pass except for any that hard-coded the 0.5 shortcut result. Update those expected values.

---

## Phase F — Documentation & Audits (No Code Changes)

### F.1 — Year-End Execution Order Invariants (Spec §5)

**File:** `src/simulation/controller.rs::process_month`

Add a comment block above the December branch describing the canonical year-end order. The current code already does:
1. Monthly distributions / cap-gains events (in `handle_dividends`, monthly)
2. TLH (pre-waterfall, December via `tlh_active_months`)
3. Year-end true-up (`finalize_year_taxes`)
4. Cashflow waterfall

This matches the spec. Add only:
```rust
// V8.0 Invariant per Spec §5:
//   1. Distributions & realized cap-gains complete before TLH.
//   2. TLH (§1091 wash-sale aware, skips crypto per Notice 2014-21) runs pre-waterfall.
//   3. Year-end tax true-up (basket FTC + §70103 senior dedn) computes before T4.
//   4. True-up liabilities feed the waterfall as expenses, draining buffers first.
```

No runtime change. Just a doc anchor.

### F.2 — Document the Deferred Item (Spec §3.2)

**File:** `docs/corrections-app-v-8-0-0.md` — append a note at the bottom:

```markdown
## Implementation Status (V8.0.0)

- §3.2 (US State Tax → Japan FTC Pool) is **DEFERRED**.
  Rationale: the current engine models only US-side FTC (Japan→US credit).
  Building Japan-side FTC infrastructure requires a separate Stage 14 design;
  tracked as TODO in `fixes-v-8-0-0.md`.
```

---

## Validation Plan

After all phases:

1. `cargo build --release` — no warnings (treat `dead_code` warnings as info).
2. `cargo test --no-fail-fast` — expect (baseline_count + ~12 new tests). Document any baseline regression.
3. Manual scenario re-run: `cargo run --release -- input/scenario_2026_mfj_balanced.json` (or whichever golden scenario you use). Diff the output CSV against the V7.11.1 baseline. Expected meaningful diffs:
   - FEIE-affected years: marginally different US tax (132,900 vs 126,500 limit).
   - Senior-deduction years: lower US tax 2026–2028.
   - Estate-summary year: different `japan_sozoku_zei_jpy` if spouse heir present.
   - Post-2037 working years: marginally lower Japan income tax (no 復興 surcharge).
4. UI smoke test: launch the egui app (`cargo run --release`), load a scenario, confirm the V8.0 config fields (`primary_taxpayer_visa`, `model_active_phase_resident_tax`) show up in the input panel or are at least loadable from JSON without parse errors.

## What to NOT Do

- Do NOT touch the Python codebase (if any remains) — this is the Rust port.
- Do NOT add a §3.2 Japan-FTC engine speculatively. Leave the TODO.
- Do NOT amend prior commits. Each phase = new commit, conventional title (`feat(v8): Phase A — 2026 tax constants`).
- Do NOT use `--no-verify` to skip pre-commit hooks.
- Do NOT delete the legacy `ftc_carryover_passive_usd` / `_general_usd` / `_usd` scalar fields. They are read by the reporter and snapshot serializers; keep them as derived mirrors of the new queue totals.

## Commit Plan

| Commit | Scope |
|---|---|
| `feat(v8): Phase A — 2026 tax constants & sunset` | §1.2, §2.1, §3.4 |
| `feat(v8): Phase B.1 — resident tax Tokubetsu Choushuu cadence` | §1.1 |
| `feat(v8): Phase B.2 — §70103 enhanced senior deduction` | §2.2 |
| `feat(v8): Phase B.3 — SSDI multi-bracket selector` | §2.3 |
| `feat(v8): Phase B.4 — visa-aware exit tax filter` | §3.5 |
| `feat(v8): Phase C.1 — Japan 3-year capital-loss ledger` | §3.3 |
| `feat(v8): Phase C.2 — §904(c) FIFO carryover queue` | §2.4 |
| `feat(v8): Phase D.1 — Japan total moving average basis` | §3.1 |
| `feat(v8): Phase E.1 — Article 19-2 spousal mitigation` | §4.1 |
| `docs(v8): order-of-execution invariants & deferred §3.2 note` | §5, §3.2 |

---

## Sonnet-Specific Notes

- This codebase has the user's auto-memory loaded; per `memory/project_retirement_calc.md` the Rust engine has 31/31 tests passing as of V7.7.1 (target version may have advanced). Verify the actual count in Phase 0.
- The user prefers terse, surgical changes. Do not refactor surrounding code while implementing a fix. Do not add comments that explain what well-named code already shows.
- When you encounter pre-existing comments in the form `// V7.X — ...`, keep them. Add your own `// V8.0 — ...` tags so future readers can see when each layer arrived.
- The PowerShell-on-Windows environment is the active shell. Use `cargo` directly (no `./` prefix). For multi-line files, prefer `Write` tool over here-strings.
- If a phase blows past 4 file edits, stop, summarize progress, and ask the user before continuing. Do not try to land all 10 commits in one sweep.

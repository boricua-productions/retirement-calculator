# V7.5 Strategic Optimization — Audit & Required Changes

Second-opinion audit of the six proposed V7.5 features against the V7.4 hardened
codebase, with concrete file-level changes required to land each one safely.

This document is structured in three sections:

1. **Pre-existing logic defects** uncovered during the audit that must be fixed
   before (or alongside) the V7.5 features that depend on them.
2. **Per-feature changes** — for each proposed feature, what needs to change,
   where, and what false positives in the original proposal must be corrected.
3. **Implementation order** and dependency graph.

---

## Section 1 — Pre-Existing Logic Defects

These are defects in the V7.4 engine itself, uncovered by reading the code with
fresh eyes during the V7.5 audit. Each is a blocker for at least one V7.5
feature.

### Defect 1.1 — Japan-side capital losses are silently discarded

**Severity: HIGH. Blocks Feature 6 (Tax-Loss Harvesting) entirely.**

**Location:** `src/handlers/cashflow_manager.rs:726, 753`

```rust
// line 726
let jpy_gain_per_share = (jpy_proceeds_per_share - jpy_basis_per_share).max(0.0);
// line 753
let jpy_gain = (jpy_proceeds - jpy_basis_sold).max(0.0);
```

Both call sites in `v7_liquidate_for_deficit` clamp Japan-side gains to ≥ 0.
A sale at a loss in JPY terms (which is independent of the USD-side gain
because of FX movement between purchase and sale) generates zero Japan capital
gains tax — correct — but the loss is **never recorded** for Japan's 3-year
capital loss carry-forward (損失の繰越控除, Income Tax Act Art. 37-12-2).

**Consequence:**
- TLH cannot deliver value through this engine until losses are tracked.
- Even without TLH, the engine over-reports Japan tax in any year where a forced
  liquidation realizes a JPY loss followed by a JPY gain in a subsequent year
  (the gain is taxed fully; the prior loss is invisible).

**Fix:**
1. Add `year_japan_cap_loss_jpy: f64` to `AnnualStats` (`src/simulation/stats.rs`).
2. Add `japan_loss_carryforward_jpy: f64` to `SimState` (`src/simulation/state.rs`).
3. Remove the `.max(0.0)` clamps. Accumulate signed values:
   ```rust
   let jpy_gain_per_share = jpy_proceeds_per_share - jpy_basis_per_share;
   // ...
   if jpy_gain_per_share >= 0.0 {
       japan_tax_per_share_jpy = jpy_gain_per_share * JAPAN_CAPITAL_GAINS_RATE;
   } else {
       japan_tax_per_share_jpy = 0.0;
       // record absolute loss for carry-forward
   }
   ```
4. At year-end in `controller.rs::handle_new_year`, decay carry-forward by
   1 year and drop entries older than 3 years (or model as a single rolling
   sum if 3-year FIFO bucketing is out-of-scope).
5. In `schedule_annual_nhi` and `schedule_annual_resident_tax`, subtract
   `state.japan_loss_carryforward_jpy` from the investment-income basis before
   computing the bill. Cap at zero (a loss cannot turn taxes negative).

### Defect 1.2 — FEIE+FTC apportionment uses §911(d)(6) haircut, not §904 limitation

**Severity: MEDIUM. Blocks the FTC basket separation that Feature 1 (PFIC) requires.**

**Location:** `src/engine/tax/us_tax.rs:175-184`

```rust
let total_japan_taxable = gross_earned + gross_unearned + gross_st_cap + gross_lt_cap;
let ftc_ratio = if total_japan_taxable > 0.0 {
    (total_japan_taxable - feie_exclusion) / total_japan_taxable
} else {
    1.0
};
let ftc_for_path = japan_tax_paid_usd * ftc_ratio;
```

This formula correctly enforces the §911(d)(6) anti-double-dip rule (you can't
credit Japan tax that fell on FEIE'd income), but it does **not** enforce the
IRC §904 limitation:

```
FTC_limit = US_tax × (foreign_source_taxable_income / total_taxable_income)
```

And it does not separate the FTC into baskets (passive, general, etc.). PFIC
MTM income (Feature 1) is **passive basket** income under §904(d)(1)(B). If
the engine doesn't distinguish baskets, PFIC integration can spuriously
absorb Japan tax credit that legally belongs to a different basket.

**Fix (minimum viable for V7.5):**
1. Introduce a `FtcBasket` enum: `Passive | General`.
2. Tag income streams: dividends + cap gains + PFIC MTM → `Passive`; FERS, SS,
   SSDI, RSU → `General`.
3. Compute `ftc_limit_per_basket = US_tax × (basket_foreign_income / total_taxable_income)`.
4. Apply Japan tax to each basket separately, cap at limit, carry forward
   the unused portion per basket.

The current single-pool FTC works only because, today, all material foreign-
sourced income for this taxpayer is in one basket. PFIC will break that
assumption — see Feature 1.

### Defect 1.3 — DC lump-sum payout bypasses Japan 退職所得 tax

**Severity: LOW. Not a V7.5 blocker but worth flagging.**

**Location:** `src/handlers/cashflow_manager.rs:598-604` (and the USD twin at 638-644)

The DC `LUMP_SUM` branch calls `dc_acc.liquidate_all(current_date)` and routes
the gross proceeds straight into the Tier-0 cash inflow. Japan's retirement
income deduction (退職所得控除) and the 1/2 reduction rule for 退職所得 are not
applied; the entire lump sum lands as untaxed JPY in the same month.

In practice the year-end true-up does not pull a Japan retirement income tax
line, so the lump sum effectively becomes tax-free in the simulation. For a
20-year DC balance this can understate Japan tax by ¥1–2M in the payout year.

**Fix:** Out of scope for V7.5; tracked here for a future tax-engine pass.

### Defect 1.4 — Mode B preempt oracle is not aware of any non-spend draws

**Severity: MEDIUM. Will be aggravated by Feature 4 (Gift Sink) and Feature 6 (TLH).**

**Location:** `src/handlers/cashflow_manager.rs:1039-1073`

`project_buffer_minimums` projects 4 months of war-chest / bridge balances
assuming only dividends in, base-spend out. It is blind to:
- The Tier-2.5 education savings skim (`edu_savings_jpy_monthly`).
- The Tier-2.5 education draw (`exp.education > 0.0`).
- Any future Tier-9 gift draw.
- Any future TLH-driven sale that lands JPY in the war chest.

Today the education effect is small and same-signed, so the oracle is roughly
correct. With Tier 9 diverting ~¥1.1M/yr/recipient out of the war chest the
oracle will systematically over-project and under-trigger preemptive restock.

**Fix:** Generalize the oracle to accept a `monthly_non_spend_jpy_drain` term
and pass `(cfg.edu_savings_jpy_monthly + annual_gift_jpy_total / 12.0)`.

---

## Section 2 — Per-Feature Audit

### Feature 1 — PFIC Tax Drag

**Verdict: Accept WITH MAJOR REWORK. Two structural false positives.**

#### False Positive 1.A — Sec 1291 and Sec 1296 are different math, not one flag

The proposal asks for a single `is_pfic: bool` that "overrides LTCG rates with
punitive MTM or Excess Distribution logic." These are mutually exclusive
elections with radically different tax characters:

| Regime | Election | Tax Character | Per-Year Math |
|--------|----------|---------------|---------------|
| §1296 MTM | Annual | **Ordinary** income on FMV − basis | Single line |
| §1291 Excess Distribution | Default (no election) | Spread-back at top rate per year + §6621 interest | Multi-year reconstruction |

A boolean cannot disambiguate these. The engine must know which regime
applies per asset.

#### False Positive 1.B — MTM is ordinary income, not a "rate override on LTCG"

`finalize_year_taxes` (controller.rs:484-504) routes dividends and gains into
`year_div_gross` and `year_cap_gains`, both flowing as `gross_lt_cap` into
the LTCG bracket-stack at us_tax.rs:91-105. PFIC MTM gains are **ordinary
income** under §1296(a) — they belong in `gross_ord`, not in the LTCG track.

A naive "override LTCG rate to 37%" would also fail the NIIT calculation,
which is computed only on gains (`total_gains = gross_st_cap + gross_lt_cap`,
us_tax.rs:108) — PFIC MTM is still investment income for NIIT purposes per
§1411 even though it's ordinary for §1296.

#### Required changes

**`src/models/assets.rs`**

Add at the top of the file:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PficRegime {
    #[default]
    NotPfic,
    /// IRC §1296 — annual mark-to-market, FMV − basis taxed as ordinary income.
    Mtm,
    /// IRC §1291 — default treatment. Out-of-scope for V7.5 (flag and warn).
    ExcessDistribution,
}
```

Add to `struct Asset`:

```rust
#[serde(default)]
pub pfic_regime: PficRegime,
/// Prior-year FMV per share, used by MTM to compute (FMV_t − FMV_{t−1})
/// at each year-end. Initialized to the basis at first MTM mark.
#[serde(default)]
pub pfic_prior_year_fmv_per_share: f64,
```

**`src/simulation/stats.rs`**

```rust
/// V7.5 — PFIC §1296 mark-to-market gain for the year (USD).
/// Taxed as ordinary income; NOT FEIE-eligible (passive income, §911(d)(2)).
pub year_pfic_mtm_income_usd: f64,
```

**New file: `src/engine/tax/pfic.rs`**

```rust
use crate::models::assets::{Asset, PficRegime};

/// Compute the annual §1296 MTM gain for a single asset.
/// Returns (gain_usd, new_basis_for_next_year). MTM losses are limited to
/// prior MTM-included income (§1296(d)) — for a long-only retail portfolio
/// this is effectively zero in early years; we floor at 0 for V7.5 and emit
/// a warning when a real loss is discarded so the user knows §1296(d)
/// carryforward is needed.
pub fn compute_annual_mtm_gain_usd(asset: &Asset) -> Option<f64> {
    if asset.pfic_regime != PficRegime::Mtm { return None; }
    let qty = asset.qty();
    if qty <= 0.0 { return Some(0.0); }
    let prior = if asset.pfic_prior_year_fmv_per_share > 0.0 {
        asset.pfic_prior_year_fmv_per_share
    } else {
        asset.basis() / qty
    };
    let delta_per_share = asset.price - prior;
    Some((delta_per_share * qty).max(0.0))  // §1296(d) limit, V7.5 simplification
}
```

**`src/simulation/controller.rs::finalize_year_taxes`** — between lines 500 and 504:

```rust
// V7.5 — Aggregate §1296 MTM gains as ordinary income (not LTCG).
let pfic_mtm_usd = sum_pfic_mtm_across_accounts(&mut self.state);
self.state.stats.year_pfic_mtm_income_usd = pfic_mtm_usd;

// Pension + SSDI + PFIC MTM all go to unearned ordinary.
let unearned_ord = total_ord + pfic_mtm_usd;
```

**`src/simulation/controller.rs::finalize_year_taxes`** — after `tax_engine` invocation,
mark each MTM asset's `pfic_prior_year_fmv_per_share = asset.price` for next year.

#### Treaty/Savings-Clause notes baked into comments

In `pfic.rs`, document:

```rust
// US-Japan Treaty (Article 1(5) Savings Clause): The US retains the right to
// tax its citizens as if the Treaty did not exist. Japan-side tax-free
// status (NISA, iDeCo) does NOT shelter from §1296 MTM. However, the
// FTC pool does not grow from these assets — Japan collects no offsetting
// tax — so the PFIC drag is fully unhedged.
```

#### Tests required

- `tests/v75_pfic_mtm_increases_ord_income.rs`: confirm a Japan-domiciled fund
  flagged `Mtm` produces non-zero `year_pfic_mtm_income_usd` and that the
  US ordinary-income tax rises by approximately `mtm_gain × marginal_rate`.
- Confirm MTM gain is **not** routed through the LTCG brackets (assert
  `breakdown["gains_at_15_pct"]` is unchanged when only PFIC assets exist).

---

### Feature 2 — Gokusen NHI Bridge (24-month cap)

**Verdict: REJECT AS PROPOSED. Implement as third NhiModel variant.**

#### False Positive 2.A — 任意継続 is a different insurance, not a cap on NHI

任意継続 (Voluntary Continuation, Health Insurance Act Art. 37) keeps an
employee on the **employer's Shakai Hoken (社会保険)** plan for up to 24
months after separation. It is NOT NHI (国民健康保険). The proposal's
"clamp NHI premiums" framing is legally incorrect.

Eligibility for 任意継続 requires:
- ≥ 2 months of continuous Shakai Hoken enrollment immediately before
  separation.
- Application within 20 days of separation.
- Premium fully self-paid (employer no longer covers half).

A US expat who was on NHI during employment (e.g., remote worker for a
US employer not enrolled in Japan Shakai Hoken) does **not** qualify and
the V7.5 model should not pretend they do.

#### False Positive 2.B — "¥30k/month cap" is not a statutory cap

The ¥30k figure approximates the Ninki Keizoku monthly premium for a
specific final-salary bracket and one health insurance society. The actual
premium is `min(standard_monthly_remuneration × (rate ÷ 2 × 2), society_cap)`
where the society cap varies (Kyokai Kenpo cap ≈ ¥35-40k in 2026; many
society plans cap higher). Hardcoding ¥30k will systematically understate
the premium for higher-income retirees.

#### Architectural collision the proposal would create

Today, `is_spike_year` in `compute_annual` (nhi.rs:38-39) handles the
year-1 NHI spike correctly — see `test_nhi_engine_transition_year_spike`
producing ¥767,484 on an ¥8M prior-year salary. A flat ¥30k×12 = ¥360k
cap would **understate the spike year by more than 2×** for that scenario.
Year 2 NHI naturally drops because the income basis is the low first
post-retirement year.

#### Required changes

**`src/models/config.rs`** — add to `NhiModel`:

```rust
/// V7.5 — Voluntary Continuation (任意継続) of employer Shakai Hoken
/// for `duration_months` (max 24 per HIA Art. 37). Replaces NHI for that
/// window; falls back to the `fallback` model thereafter.
NinkiKeizoku {
    monthly_premium_jpy: f64,
    duration_months: u32,
    fallback: Box<NhiModel>,
},
```

**`src/engine/tax/nhi.rs::compute_annual`** — extend the match:

```rust
NhiModel::NinkiKeizoku { monthly_premium_jpy, duration_months, fallback } => {
    // The caller passes is_spike_year=true for the first post-retirement
    // year; subsequent years pass false. duration_months is consumed by
    // the scheduler, not by this stateless function — see schedule_annual_nhi.
    if is_spike_year {
        monthly_premium_jpy * 12.0
    } else {
        Self::compute_annual(
            fallback, prev_year_gross_salary_jpy, prev_year_gross_pension_jpy,
            prev_year_investment_income_jpy, num_insured, age, is_spike_year,
        )
    }
}
```

**`src/simulation/controller.rs::schedule_annual_nhi`** — add state-tracked
duration counter:
- Add `nhi_ninki_keizoku_months_remaining: u32` to `SimState`.
- Decrement at each scheduling tick; switch to `fallback` once depleted.

#### Tests required

- `test_ninki_keizoku_active_in_year_1`: scheduled NHI = `monthly × 12`.
- `test_ninki_keizoku_falls_back_after_duration`: year 3 uses fallback model.
- `test_ninki_keizoku_does_not_override_spike_for_ineligible_user`: when
  the config field is absent / model is `Calculated`, no change occurs.

---

### Feature 3 — Exit Tax Monitor

**Verdict: Accept. One numerical correction.**

#### False Positive 3.A — "5 out of 10 years," not ">10 years"

Income Tax Act Art. 60-2 triggers when:
- (a) Total Japan-subject financial assets ≥ ¥100M; **AND**
- (b) The person was a Japanese tax resident for **5 or more years out of
  the preceding 10 years** (residency on or after the date the exit notice
  was filed counts).

The proposal's ">10 years" misses anyone in years 5-10 of residency who is
already subject to the tax. This is the single most common implementation
error in the wild.

#### Asset basis subtlety

The ¥100M threshold counts:
- Listed securities (foreign and domestic).
- Bonds, derivatives, structured products.
- **NISA assets** (yes, NISA tax-exemption is on the gain, not on the
  asset-value threshold).
- iDeCo / DC plans count if vested.

It excludes:
- Real estate.
- Bank cash deposits (these count toward gift/inheritance tax bases but
  not Exit Tax).

#### Treaty/FTC framing the proposal got wrong

Japan Exit Tax is on **unrealized** gains. The US has no corresponding
recognition event (IRC §877A applies only on citizenship renunciation).
Routing Japan exit tax through `effective_japan_tax_usd` in
`finalize_year_taxes` would credit it against current-year US tax on
realized income — which is not legally permissible. Treat exit tax purely
as an alerting signal; never feed it into the FTC pool.

#### Required changes

**`src/models/config.rs`** — add:

```rust
/// V7.5 — Japan residency start (used for Exit Tax 5-of-10 test).
/// None disables the Exit Tax monitor.
#[serde(default)]
pub japan_residency_start_date: Option<NaiveDate>,
/// V7.5 — Whether to include NISA/iDeCo asset values in the ¥100M
/// threshold. Per Art. 60-2 the answer is yes; flag retained for "what
/// if" analysis.
#[serde(default = "default_true")]
pub exit_tax_include_tax_advantaged: bool,
```

**`src/models/snapshot.rs::AnnualSnapshot`** — add:

```rust
/// V7.5 — true when the year-end position triggers Japan Exit Tax exposure.
pub exit_tax_triggered: bool,
/// V7.5 — global Japan-subject financial assets at year-end (¥).
pub exit_tax_asset_value_jpy: f64,
```

**`src/simulation/controller.rs::record_annual_snapshot`** — compute after the
existing `val` closure:

```rust
let exit_tax_triggered = self.evaluate_exit_tax_trigger(yr);
```

with the helper:

```rust
fn evaluate_exit_tax_trigger(&self, yr: i32) -> bool {
    const THRESHOLD_JPY: f64 = 100_000_000.0;
    let start = match self.cfg.japan_residency_start_date {
        Some(d) => d,
        None => return false,
    };
    // 5-of-10 residency test
    let years_resident = (yr - start.year()).min(10).max(0) as i32;
    if years_resident < 5 { return false; }
    // Asset basis
    let mut assets_jpy = 0.0;
    for (name, acc) in &self.state.accounts {
        let val_jpy = acc.total_value(self.state.current_fx);
        let include = if self.cfg.exit_tax_include_tax_advantaged {
            true
        } else {
            !matches!(name.as_str(), "Roth" | "NISA" | "iDeCo")
        };
        if include { assets_jpy += val_jpy; }
    }
    assets_jpy >= THRESHOLD_JPY
}
```

#### Tests required

- `test_exit_tax_not_triggered_below_threshold`: ¥99M assets, 6 years → false.
- `test_exit_tax_not_triggered_below_residency`: ¥150M assets, 4 years → false.
- `test_exit_tax_triggered_at_5_of_10`: ¥150M assets, 5 years → true.

---

### Feature 4 — Tier 9 Estate Planning (Gift Sink)

**Verdict: Accept WITH PER-RECIPIENT MODEL. One Mode B collision to fix.**

#### False Positive 4.A — ¥1.1M is per-recipient, not per-donor

Japan's 暦年贈与 ¥1.1M annual exclusion (Inheritance Tax Act Art. 21-5) is
the **recipient's** annual exclusion. A donor can give ¥1.1M to each of
N recipients tax-free. The proposal's "¥1.1M annual JPY surplus" implicitly
treats it as a per-donor cap, which would block legitimate multi-child
planning.

US side: IRC §2503(b) annual exclusion is also per donor-recipient pair
($19,000 in 2026, inflation-indexed). The check the proposal calls out
must be evaluated **per recipient**:

```
for each recipient:
    if (annual_gift_jpy_per_recipient / fx) > US_2503_exclusion_usd:
        warn — donor must file Form 709 for this recipient
```

#### Sensitivity table (illustrative)

| Annual gift (¥/recipient) | At ¥80/$ | At ¥145/$ | At ¥200/$ |
|--------------------------|----------|-----------|-----------|
| ¥1,100,000 | $13,750 | $7,586 | $5,500 |
| ¥2,500,000 | $31,250 | $17,241 | $12,500 |

The US ceiling becomes the binding constraint when the FX rate is very
strong (low ¥/$). Above ~¥2.76M/recipient at ¥145/$ → triggers US Form 709.

#### Architectural collision — Mode B oracle blindness

`project_buffer_minimums` (cashflow_manager.rs:1039-1073) does not know
about T9 draws. If T9 diverts `N × ¥1.1M` annually from the war chest, the
preemptive restock under-fires. See Defect 1.4 above for the underlying fix.

#### Required changes

**`src/models/config.rs`**:

```rust
/// V7.5 — Tier 9 Gift Sink configuration.
#[serde(default)]
pub annual_gift_jpy_per_recipient: f64,   // typically 1_100_000
#[serde(default)]
pub gift_recipient_count: u32,             // typically 1-4 (children/grandchildren)
/// US §2503(b) annual gift exclusion per donor-recipient pair (2026 = 19_000).
#[serde(default = "default_us_gift_exclusion")]
pub us_gift_exclusion_usd: f64,
```

**`src/simulation/stats.rs`**:

```rust
/// V7.5 — JPY diverted into the Tier 9 Gift Sink this year.
pub year_gift_sink_jpy: f64,
/// V7.5 — true if any per-recipient gift exceeded the US $19k exclusion
/// (flagged for Form 709 filing in the audit report).
pub year_form_709_required: bool,
```

**`src/simulation/state.rs`**:

```rust
/// V7.5 — Cumulative JPY routed to the Gift Sink (held outside the
/// waterfall; never drawn back in).
pub gift_sink_jpy: f64,
```

**`src/handlers/cashflow_manager.rs::manage_monthly_cashflow_defensive`** —
insert immediately AFTER line 335 (`let jpy_surplus_raw = ...`):

```rust
// ── V7.5 — Tier 9: Estate Planning Gift Sink ─────────────────────────────
// Diverts surplus into a recipient-scoped gift bucket once per year
// (December) so the bucket models legal-year donation semantics rather
// than monthly drips. Per-recipient evaluation against IRC §2503(b)
// flags Form 709 obligation for the year.
let t9_jpy_drawn = if state.date.month() == 12 {
    process_tier9_gift_sink(state, cfg, jpy_surplus_raw)
} else {
    0.0
};
let jpy_surplus_raw = jpy_surplus_raw - t9_jpy_drawn;
```

Helper at file bottom:

```rust
fn process_tier9_gift_sink(state: &mut SimState, cfg: &Config, surplus_jpy: f64) -> f64 {
    if cfg.gift_recipient_count == 0 || cfg.annual_gift_jpy_per_recipient <= 0.0 {
        return 0.0;
    }
    let annual_total = cfg.annual_gift_jpy_per_recipient
        * cfg.gift_recipient_count as f64;
    let drawn = annual_total.min(surplus_jpy.max(0.0));
    state.gift_sink_jpy += drawn;
    state.stats.year_gift_sink_jpy += drawn;

    // §2503(b) per-recipient check (USD).
    let per_recipient_usd = cfg.annual_gift_jpy_per_recipient / state.current_fx;
    if per_recipient_usd > cfg.us_gift_exclusion_usd {
        state.stats.year_form_709_required = true;
    }
    drawn
}
```

**`src/handlers/cashflow_manager.rs::project_buffer_minimums`** — add a
parameter `monthly_non_spend_drain_jpy: f64` and subtract it from
`proj_wc_jpy` each iteration. Caller passes
`(annual_gift_jpy_per_recipient × recipient_count + edu_savings_jpy_monthly × 12) / 12.0`.

#### Tests required

- `test_t9_fires_only_in_december`: months 1-11 → 0 drawn; month 12 → annual total.
- `test_t9_respects_surplus_ceiling`: surplus_jpy=¥500k, recipient_count=2,
  per-recipient=¥1.1M → drawn=¥500k (not ¥2.2M).
- `test_t9_form_709_flagged_at_low_fx`: fx=80, per-recipient=¥1.6M → $20k > $19k → flag set.
- `test_t9_form_709_clear_at_high_fx`: fx=145, per-recipient=¥1.1M → $7.5k < $19k → flag clear.
- `test_mode_b_oracle_subtracts_t9_drain`: with T9 active, projected WC
  minimum is `prior_projection − (annual_gift_total / 12) × lookahead_months`.

---

### Feature 5 — Stochastic FX Stress / IRC §988

**Verdict: ACCEPT THE FX STOCHASTIC PART. REJECT THE §988 OVERLAY.**

#### False Positive 5.A — §988 does not apply to USD-denominated stock sales

IRC §988 covers "section 988 transactions" — foreign-currency-denominated
debt instruments, forward contracts, options, and acquisitions/dispositions
of nonfunctional currency. A US person resident in Japan selling VTI/SCHD
(USD-denominated securities) is **not** engaged in a §988 transaction with
respect to that sale; the gain is a §1001 capital gain on USD.

For this taxpayer the only true §988 exposure is:
1. Conversion of USD cash → JPY cash for spending (held inventory).
2. Any USD-denominated borrowing repaid as the dollar strengthens.

Per Treas. Reg. §1.988-2(a)(2)(iii) (de minimis personal-use exception),
personal-use FX conversions for living expenses are not §988 gains. The
simulation does not model debt. Net: there is **no §988 phantom-gain
exposure** in this engine.

The proposal's "Phantom Currency Gains under IRC §988 during Tier 8 stock
sales" is a textbook misapplication of §988 to a non-§988 transaction.

#### What to keep: stochastic FX in Monte Carlo

The current Monte Carlo engine (`src/simulation/monte_carlo.rs`) runs a GBM
on the USD portfolio total with no FX coupling. Every iteration shares the
same deterministic FX path — every percentile in the output assumes
identical USD/JPY history. This understates JPY-resident tail risk.

#### Required changes

**`src/simulation/monte_carlo.rs`** — extend `MarcoPoloInput`:

```rust
/// V7.5 — Stochastic FX parameters. None disables (deterministic FX = initial).
pub fx_stochastic: Option<FxStochasticParams>,
}

pub struct FxStochasticParams {
    pub initial_fx: f64,          // e.g. 145.0
    pub annual_mean_drift: f64,   // e.g. 0.02
    pub annual_volatility: f64,   // e.g. 0.10 (10% σ for USD/JPY)
}
```

**Critical:** correlate the FX path against the asset return path. The simple
implementation is to draw one z-vector for asset returns and an independent
z-vector for FX (low realized correlation), which already lifts P10 worse
than the deterministic case for JPY-cost-base scenarios.

Inside `run_marco_polo`, alongside the existing GBM step:

```rust
let mut fx = params.initial_fx;
let fx_drift = params.annual_mean_drift - 0.5 * params.annual_volatility.powi(2);
// ... per year:
let z_fx = rng.normal();
fx *= (fx_drift + params.annual_volatility * z_fx).exp();
// portfolio_jpy = portfolio_usd * fx — output P10/P50/P90 in BOTH currencies
```

#### Tests required

- `test_marco_polo_with_stochastic_fx_widens_jpy_band`: same asset params,
  same iterations; P90-P10 spread in JPY terms is strictly wider with
  stochastic FX enabled than without.
- `test_marco_polo_deterministic_fx_preserves_legacy`: `fx_stochastic = None`
  produces identical USD percentiles to the V7.4 output.

---

### Feature 6 — Tax-Loss Harvesting

**Verdict: ACCEPT BUT BLOCKED ON DEFECT 1.1.**

#### False Positive 6.A — A "Tier" is the wrong architectural slot

The proposal describes TLH as a "proactive liquidation tier for underwater
lots." The waterfall (T0 → T8) is a **cash-shortfall coverage system** — its
contract is "make this month's gap zero." TLH is not gap-driven; it is
calendar-driven (typically year-end or quarter-end) and its purpose is to
realize losses, not to raise cash.

Slotting TLH as "T8.5" or "T9" muddies the abstraction. The correct slot
is a **pre-waterfall handler** in `process_month`, parallel to
`handle_dividends` and `handle_rsu_vesting`, called only in months where
TLH is configured to fire (typically November-December).

#### False Positive 6.B — 31-day wash sale needs to be 30-day in §1091 terms

IRC §1091 disallows a loss when the taxpayer acquires substantially
identical stock or securities **within 30 days before or after** the sale.
The total window is 61 days (30 before + sale day + 30 after). The
proposal's "31-day wash-sale logic" is correct in spirit but loose: the
test is "no purchase within the 30 calendar days surrounding the sale," not
"wait 31 days before repurchase."

Japan has no equivalent rule. A Japan-tax-resident US citizen still faces
US §1091 because the Savings Clause applies.

#### Hard dependency on Defect 1.1

Until `v7_liquidate_for_deficit` stops clamping JPY losses to zero,
harvested losses are invisible in Japan. TLH would fire sales, eat T8
capacity, and deliver no Japan-side benefit. **Do not ship Feature 6
without fixing Defect 1.1.**

#### Required changes

**`src/models/assets.rs::AssetLot`** — add:

```rust
/// V7.5 — §1091 wash-sale taint: if Some, the lot's loss is disallowed and
/// the basis is adjusted upward by `disallowed_loss_usd`. Set when a
/// replacement security is acquired within the 30-day window.
#[serde(default)]
pub disallowed_loss_usd: f64,
/// V7.5 — Date after which this lot is no longer wash-sale tainted.
#[serde(default)]
pub wash_sale_clean_after: Option<NaiveDate>,
```

**New file: `src/handlers/tax_loss_harvesting.rs`**

```rust
//! V7.5 — Tax-Loss Harvesting (IRC §1091 wash-sale aware).
//!
//! Fires in months listed in `cfg.tlh_active_months` (typically [11, 12]).
//! For each Taxable asset with at least one lot at a loss, computes the
//! harvestable loss and either:
//!   - Records it for current-year US capital loss offset, OR
//!   - Marks lots as wash-sale tainted if a replacement was acquired in
//!     the 61-day window (and adjusts basis accordingly).
//!
//! The corresponding Japan loss is recorded into
//! `state.japan_loss_carryforward_jpy` (3-year carry under IT Act Art. 37-12-2).

pub fn harvest_losses(state: &mut SimState, cfg: &Config) {
    if !cfg.tlh_enabled { return; }
    let mo = state.date.month();
    if !cfg.tlh_active_months.contains(&mo) { return; }
    // ... per-asset loss harvesting with §1091 check ...
}
```

**`src/simulation/controller.rs::process_month`** — call before
`handle_dividends`:

```rust
if self.state.date >= self.cfg.retirement_date && self.cfg.tlh_enabled {
    crate::handlers::tax_loss_harvesting::harvest_losses(
        &mut self.state, &self.cfg,
    );
}
```

**`src/models/config.rs`** — add:

```rust
#[serde(default)]
pub tlh_enabled: bool,
#[serde(default = "default_tlh_months")]
pub tlh_active_months: Vec<u32>,   // default vec![11, 12]
/// Threshold below which losses are not worth harvesting (transaction
/// costs + tax filing complexity dominate).
#[serde(default = "default_tlh_threshold")]
pub tlh_min_loss_usd: f64,         // default 500.0
```

#### Tests required

- `test_tlh_skips_lots_at_gain`: only loss lots are touched.
- `test_tlh_respects_min_loss_threshold`: $400 loss skipped at $500 threshold.
- `test_tlh_japan_loss_carryforward_recorded`: after harvesting a ¥80k JPY
  loss, `state.japan_loss_carryforward_jpy == 80_000.0`.
- `test_tlh_wash_sale_disallowed`: harvest sale, then buy same ticker 5
  days later → loss is disallowed, basis adjustment applied.
- `test_tlh_wash_sale_clean_after_30_days`: harvest sale, repurchase 35
  days later → loss is recognized, no basis adjustment.

---

## Section 3 — Implementation Order & Dependency Graph

```
┌─────────────────────────┐
│ Defect 1.1              │  ← MUST land first
│ (Japan loss tracking)   │
└───────────┬─────────────┘
            │
            ├──→ Feature 6 (TLH) — blocked until 1.1
            │
┌───────────▼─────────────┐
│ Defect 1.2              │  ← Needed before Feature 1
│ (§904 FTC baskets)      │
└───────────┬─────────────┘
            │
            ├──→ Feature 1 (PFIC) — blocked until 1.2
            │
┌───────────▼─────────────┐
│ Defect 1.4              │  ← Needed before Feature 4
│ (Mode B oracle drain)   │
└───────────┬─────────────┘
            │
            ├──→ Feature 4 (Gift Sink)
            │
            ├──→ Feature 3 (Exit Tax Monitor)   ← independent
            ├──→ Feature 5 (Stochastic FX)      ← independent, MC-only
            └──→ Feature 2 (Ninki Keizoku)      ← independent, NHI-only
```

### Recommended sequence

1. **Defect 1.1** — Japan loss tracking. Smallest change; unblocks Feature 6.
2. **Defect 1.4** — Generalize Mode B oracle. Smallest change; unblocks Feature 4.
3. **Feature 3** — Exit Tax Monitor. Pure read-only addition; lowest risk.
4. **Feature 5** — Stochastic FX. Isolated to `monte_carlo.rs`; no waterfall impact.
5. **Feature 2** — Ninki Keizoku as third NhiModel variant.
6. **Feature 4** — Tier 9 Gift Sink. Depends on Defect 1.4.
7. **Defect 1.2** — FTC basket split. Larger refactor; needed before PFIC.
8. **Feature 1** — PFIC §1296 MTM. Most complex; goes last.
9. **Feature 6** — TLH. Most complex behavior; benefits from all prior work.

### Cross-cutting concern: `tests/logic_audit.rs`

The CLAUDE.md "Logic Integrity" constraint requires zero ¥ drift on Jido
Teate. Every defect fix and every feature above introduces new state and
new tax flows. Each change must:

- Run `tests/logic_audit.rs` and confirm ¥0 drift on Jido Teate accrual.
- Add at least one regression test asserting V7.4 baseline behavior is
  preserved when the new feature is disabled (`Default::default()` config).

### Out-of-scope (explicit non-goals for V7.5)

- IRC §1291 Excess Distribution multi-year reconstruction (flag and warn only).
- §1296(d) MTM loss carryforward (floor losses at zero for V7.5).
- Japan retirement income tax (退職所得) on DC lump sums (Defect 1.3).
- Per-state US state-tax rules beyond the existing flat rate.
- Inheritance Tax (相続税) modeling — distinct from Exit Tax and Gift Tax.

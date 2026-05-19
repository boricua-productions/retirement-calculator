# V8.4 — Conservative-default Cashflow Waterfall

Handoff document. Captures the diagnosis of the "war chest empty" bug, the
redesigned waterfall, all design decisions reached via Q&A, and the
implementation plan. Pick up here in a fresh environment — everything needed
to start writing code is in this file.

**Status:** plan approved, not yet implemented. V8.3 is uncommitted (per
prior memory); do not commit anything until the user explicitly approves.

---

## 1. Original problem

The user observed: "during execution of a simulation the war chest shows as
empty for all years" using two scenarios:

- `20260519_NO_RECESSION_50K_REFRESH_RETIRE_MAY_2029.json`
- `20260519_NO_RECESSION_50K_REFRESH_RETIRE_MAY_2030.json`

Both have:

| Field | Value |
|---|---|
| `war_chest_enabled` | true |
| `war_chest_currency` | JPY |
| `war_chest_target_jpy` | 3,140,000 |
| `pre_funded_war_chest_jpy` | 3,140,000 |
| `war_chest_funding_timing` | AtRetirement |
| `withdrawal_regime` | (unset → defaults to Shielded / Mode A) |
| JPY-side income | ¥0 until DC lump-sum at age 60, no Nenkin until ~2045 |

A diagnostic test (`tests/diag_war_chest.rs`, kept in the repo) reproduces the
issue. Year-end `war_chest_jpy = 0` for every simulated year.

### Root causes (three stacked effects)

**Cause 1 — Pre-retirement years (2026–2028).** `pre_funded_war_chest_jpy` is
a *portfolio-offset only*, not an opening balance.

- `simulation/state.rs:203` initializes `state.war_chest_jpy = 0.0`.
- `handlers/retirement_transition.rs:49-58` consults `pre_funded_war_chest_jpy`
  only to decide how much extra cash to pull from Taxable at the rebalance
  event. `state.war_chest_jpy` is set to the target only on the rebalance
  date.

**Cause 2 — Retirement year drains it in ~3 months, never refills.**
The current Shielded waterfall in `handlers/cashflow_manager.rs` is:

```
Tier 0  JPY floor income (Nenkin, DC payout, JPY rental)
Tier 1  JPY dividends
Tier 3  JPY WAR CHEST          ← drained here, before USD is touched
Tier 4  USD floor income (FERS, VA, SS, SSDI)
Tier 5  USD dividends
Tier 6  USD Bridge Fund
Tier 7/8  belt-tightening / liquidation
```

T0+T1 = ¥0/month for the user's scenarios. Gap of ~¥1.0M lands on T3 every
month. ¥3.14M ÷ ¥1.0M ≈ 3 months → war chest empty by ~Aug of the retirement
year.

The Shielded refill path (`cashflow_manager.rs:638 deposit_jpy_surplus`)
only refills from `jpy_surplus_raw = t0_surplus_jpy + t1_surplus_jpy`. There
is no USD→JPY refill in Shielded mode (that exists only in
`WithdrawalRegime::Dynamic`). Once JPY-side income is zero, the war chest
can never refill.

**Cause 3 — Mid-retirement: war chest acts as a JPY transit account.**
When DC lump-sum (age 60) and Nenkin (~2045) arrive, they flow into Tier 0,
surplus refills war chest, next month's gap drains it. December snapshot
always reads ¥0.

By the code's existing intent these are not bugs — they match the design
comments in `models/config.rs:238` and the waterfall description in
`models/snapshot.rs`. But the result is not what the user expects.

---

## 2. New design — Conservative default waterfall (replaces Shielded)

### User's stated requirements (verbatim)

> "funding should be used in this order: 1. income from FERS, VA, SS, SSDI,
> Nenkin. 2. bridge fund. 3. war chest. Any dividends generated go toward
> refilling the bridge fund and the war chest. If this cannot be accomplished,
> then we start looking at belt-tightening and finally liquidation. This
> should be the default approach and the only time this changes is if the
> user selects to use both stocks and dividends, in which case belt
> tightening would only happen if there is not enough stock to survive the
> entire simulation."

> "for USD dividends, if the bridge fund is full, also refill the war chest
> with them. for JPY dividends, if the war chest is full, it can be used to
> fill the bridge fund."

### Step-by-step monthly waterfall (default)

| Step | Source | Notes |
|---|---|---|
| 1 | **Floor income** | FERS, VA, SS, SSDI, Nenkin, DC payout, rental, military pension. JPY streams cover gap directly; USD streams with FX penalty. |
| 2 | **Bridge Fund (USD)** | Drained with FX penalty. |
| 3 | **War Chest (JPY)** | Drained directly, no FX. |
| 4 | **Belt-tightening** | Drop spend target from base → minimum. Default path. |
| 5 | **HELOC** | Existing T7.5 logic, unchanged. |
| 6 | **Liquidation** | Sell from Taxable to cover residual gap (against minimum target since belt-tighten already fired). |

**Dividends never directly cover expenses.** Both JPY and USD dividends flow
into the deposit helpers below.

### Surplus routing (cross-currency)

**USD dividends + USD floor-income surplus** (updated `deposit_usd_surplus`):

1. Fill `bridge_fund_usd` up to bridge target.
2. If bridge is full → convert remainder to JPY (apply `cfg.fx_penalty`) and
   fill `war_chest_jpy` up to war chest target.
3. If both buffers full → reinvest into VTI/SCHD per target allocations
   (existing behavior).

**JPY dividends + JPY floor-income surplus** (updated `deposit_jpy_surplus`):

1. Fill `war_chest_jpy` up to war chest target.
2. If war chest is full → convert remainder to USD (apply `cfg.fx_penalty`)
   and fill `bridge_fund_usd` up to bridge target.
3. If both buffers full → remainder stays in `war_chest_jpy` (no JPY equity
   path; same as today).

FX conversion in both directions uses `cfg.fx_penalty`. The JPY-equivalent
penalty is added to `state.stats.year_fx_penalty_jpy` for both directions so
reporting still surfaces FX drag.

### Alt mode — "use both stocks and dividends"

A new boolean config flag on `Config`, default `false`. When `true`, Step 4
(belt-tightening) becomes conditional on a stock-survival projection:

```
remaining_months = months_between(state.date, cfg.end_date)
projection = state.accounts["Taxable"].total_value(fx)
             >= target_min_jpy * remaining_months
```

- If projection passes → skip Step 4, jump to Step 6 (liquidate stock to
  cover full gap, not just minimum).
- If projection fails → belt-tighten as in default mode.

Coarse projection — no growth or inflation adjustment. Code comment must
explain this trade-off.

Dividends still refill buffers in alt mode (same routing as default — no
change there). Alt mode only changes belt-tighten vs liquidate ordering.

---

## 3. Design decisions (from Q&A)

| Question | User's answer |
|---|---|
| Replace Shielded or add new mode? | **Replace Shielded** (becomes new default) |
| In alt mode, where do dividends go? | **Still refill buffers first** (stock liquidation covers residual gap) |
| Stock-survival projection basis? | **Minimum spend** (the floor) |
| Plan first or build straight? | **Plan first, wait for approval** |
| Cross-currency dividend routing? | **Yes — USD divs refill war chest if bridge full; JPY divs refill bridge if war chest full** |

### Assumptions confirmed implicitly (no objection raised)

- DC payout, rental, military pension treated as floor income (Step 1).
- Dividends and floor-income surplus follow the same routing rule (no split).
- `cfg.fx_penalty` used for cross-currency refills.
- `Dynamic` (Mode B) and `Cautious` modes untouched.
- `retirement_transition.rs` untouched. `pre_funded_war_chest_jpy` remains
  a portfolio-offset (not an opening balance). If the user wants to also
  make it an opening balance, that's a **separate** decision they have
  not yet authorized.
- Surplus deposit helpers (`deposit_jpy_surplus`, `deposit_usd_surplus`) get
  the cross-currency logic added; signatures may need adjusting (war chest
  needs to know about bridge fund's target and vice versa).

---

## 4. Files to modify

| File | Change |
|---|---|
| `src/handlers/cashflow_manager.rs` | Rewrite the `WithdrawalRegime::Shielded` arm of `manage_monthly_cashflow`. Reorder T0–T6, remove T1/T5 direct-funding paths, route all dividends through deposit helpers. Add `stock_can_sustain_minimum_for_remaining()` helper. Extend `deposit_usd_surplus` / `deposit_jpy_surplus` with cross-currency overflow. |
| `src/models/config.rs` | Add `prefer_liquidation_over_belt_tightening: bool` (default `false`). Pick a better name if one comes to mind — `prefer_stock_liquidation` is an option. |
| `src/config/loader.rs` | Load the new flag from JSON (`get_bool` with `false` default). |
| `src/ui/panels/input_panel.rs` | Add a checkbox in the Withdrawal section with a tooltip explaining the trade-off (belt-tighten earlier vs. liquidate stock first). Persist via `set_bool!` in Save. |
| `src/engine/cashflow_engine.rs`, `src/engine/tax/estate_tax.rs` | Test-fixture Config literals — add the new field with `false`. |
| `tests/v7_tax_and_liquidation_test.rs`, `tests/stage_11a_buffer_selection.rs`, `tests/v8_2_account_snapshots.rs` and any other test that asserts on the Shielded ordering | Update expectations to match new waterfall. Likely will need to re-derive expected `wc_used`, `bridge_fund_usd`, etc. |
| `tests/diag_war_chest.rs` | Keep — should now show non-zero `war_chest_jpy` throughout retirement for the two scenarios. |

---

## 5. Test strategy

1. Run `cargo test` after the rewrite. Expect breakage in tests that
   hard-coded Shielded's old tier order.
2. For each broken test: read the original assertion, decide whether the new
   behavior is still correct semantically, update the expected number. Do
   not weaken assertions without understanding why they changed.
3. Re-run `tests/diag_war_chest.rs -- --nocapture`. Acceptance criteria:
   - `war_chest_jpy` is non-zero in most retirement years (not literally
     every year — it can still hit zero in periods of high JPY drain).
   - `wc_used_jpy` shows steady flow.
   - `bridge_fund_usd` and `war_chest_jpy` both stay near target when
     dividends are flowing.
4. Manually verify both scenarios in the GUI before declaring done. Per
   project guidance: "type checking and test suites verify code correctness,
   not feature correctness — if you can't test the UI, say so explicitly."

---

## 6. Diagnostic helpers already in place

- `tests/diag_war_chest.rs` — runs both scenarios end-to-end and prints
  year-by-year `war_chest_jpy`, `wc_used_jpy`, `bridge_fund_usd`,
  `nenkin_jpy`, `div_net_usd`. Useful for before/after comparison.

Run with:

```
cargo test --test diag_war_chest -- --nocapture
```

---

## 7. Open items / explicit non-decisions

- `pre_funded_war_chest_jpy` as opening balance — **not authorized**. Was
  raised in the diagnosis but the user did not approve changing it. Do not
  modify retirement_transition.rs as part of V8.4 unless the user explicitly
  asks.
- Whether to rename Shielded → "Conservative" in the enum / UI — open. The
  enum variant name is internal; user-facing label in `input_panel.rs` and
  `Display` impl in `models/config.rs:258` can be changed. Probably worth
  doing for clarity.
- V8.3 is uncommitted. Do **not** commit V8.4 work alongside V8.3 without
  asking the user. Recommend: get V8.3 committed first, then V8.4 as its
  own commit / PR.

---

## 8. Memory notes (for the next session)

If continuing in a new environment without access to the prior session's
memory:

- Project: Rust retirement simulation (egui GUI app).
- Codebase root: `D:\github-repos\retirement-calculator` (on Windows).
- Repo had V8.3 (Input UX & Diagnostics Polish) uncommitted at the time
  this plan was written; user reviews diffs before commit.
- Tests can be run with `cargo test`. The Cargo project builds cleanly on
  Windows. First-time build pulls many deps; subsequent runs are fast.
- The user prefers terse responses, no trailing summaries, and "plan first,
  build after approval" for non-trivial changes.

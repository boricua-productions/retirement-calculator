# Fix Instructions for Sonnet

Two issues need to be resolved in this Rust/egui retirement calculator application.

---

## Issue 1: Remove "Rebalance Date" from the Timing Section

### Problem

The Timing section in `src/ui/panels/input_panel.rs` (line 1957) contains a "Rebalance Date" text field. This is incorrect. All rebalancing timing should be configured at the **investment account level**, not as a global timing field.

The global `rebalance_date` in the timing panel currently serves as the trigger date for the one-time retirement transition event (`handle_transition` in `src/handlers/retirement_transition.rs`). However, this belongs at the account level since V7.7 introduced per-account rebalance strategies.

### What to Change

#### A. Remove the Rebalance Date text field from the Timing section

**File:** `src/ui/panels/input_panel.rs`  
**Line 1957-1958:** Delete these two lines:
```rust
vfield_tt(ui, "Rebalance Date",  &mut state.rebalance_date,  "YYYY-MM-DD", !errors.contains("rebalance_date"),
    "Major portfolio rebalance event. Per-position overrides (V6.6) supersede this.");
```

#### B. Replace with a read-only summary + warning display

In its place (still within the Timing section), add a **read-only display** that shows:
1. A summary of all per-account rebalance configurations (account name, frequency or one-time date, enabled/disabled).
2. A **warning** (yellow/orange text) if any account's `rebalance_strategy` has a `trigger_year_month` that resolves to a date **before** the configured `retirement_date`. The warning should say something like:  
   `"⚠ Account '{name}' rebalance triggers before retirement date ({trigger_date} < {retirement_date})"`

The summary should be compact, e.g.:
```
Rebalance Schedule:
  Taxable: Quarterly (starts 2030-04)
  Roth IRA: One-time (2029-06)
  ⚠ Roth IRA rebalance triggers before retirement (2029-06 < 2030-01)
```

**Implementation approach:**
- After the `vfield_tt` calls for Start/End/Retirement dates (and closing the grid), add a new block.
- Iterate over `state.accounts` to collect rebalance info. Each `InvestmentAccountRow` does NOT currently have a `rebalance_strategy` field in the UI state — it uses the per-position `rebalance_date` and the global `rebalance_enabled`/`rebalance_frequency`. You need to check:
  - The global `state.rebalance_enabled` and `state.rebalance_frequency` (applies to Taxable account).
  - Each position's `pos.rebalance_date` field (V6.6 per-position override).
- Display these as a compact label list.
- Parse `state.retirement_date` and compare against each configured rebalance date/trigger. If any rebalance date is earlier, show a warning in `Color32::from_rgb(255, 180, 50)` (amber/orange).

#### C. Keep the underlying `rebalance_date` field in state but use retirement_date as fallback

The `rebalance_date` field in `InputState` (line 223) and `Config` (line 601 of `src/models/config.rs`) is still used by `handle_transition` in the simulation controller (line 191 of `src/simulation/controller.rs`). Two options:

**Option 1 (simpler):** Keep `rebalance_date` in the config but auto-set it to equal `retirement_date` during config build (in the `build_config` function). Remove the user-editable field. The transition event fires at retirement.

**Option 2 (cleaner long-term):** Refactor `handle_transition` to trigger based on `retirement_date` directly. Change line 191 in `src/simulation/controller.rs` from:
```rust
if self.state.date.year() == self.cfg.rebalance_date.year()
    && self.state.date.month() == self.cfg.rebalance_date.month()
```
to:
```rust
if self.state.date.year() == self.cfg.retirement_date.year()
    && self.state.date.month() == self.cfg.retirement_date.month()
```

**Choose Option 1** for minimal blast radius. In the `build_config` or equivalent function that converts `InputState` to `Config`, set:
```rust
rebalance_date: parse_date(&self.retirement_date).unwrap_or(defaults.retirement_date),
```

#### D. Remove the validation for rebalance_date

**File:** `src/ui/panels/input_panel.rs`  
**Line 1099:** Remove or comment out:
```rust
if bad_date(&self.rebalance_date)  { bad.insert("rebalance_date"); }
```

#### E. Remove rebalance_date from serialization (optional cleanup)

**File:** `src/ui/panels/input_panel.rs`  
**Line 932:** Remove:
```rust
rebalance_date:  str_val("rebalance_date",   ""),
```
**Line 1190:** Remove:
```rust
set_str!("rebalance_date",  self.rebalance_date);
```

Or keep them for backward-compat with saved files but simply ignore the loaded value.

---

## Issue 2: Auto-Fetch Freezes the UI

### Problem

All market data fetching uses `ureq` (a synchronous HTTP client) and executes directly on the main egui render thread. When the user clicks the "✨" auto-fetch button, the UI completely freezes for 2-30 seconds while 2-3 sequential HTTP requests complete. There is no visual feedback.

**Root cause files:**
- `src/ui/panels/input_panel.rs` lines 2976-3053 (fetch execution in the UI frame)
- `src/engine/market_data/mod.rs` lines 143-234 (blocking `ureq::get().call()`)

### What to Change

#### A. Add a background thread for fetching

Use `std::thread::spawn` to move network calls off the main thread. Communicate results back via a shared channel or `Arc<Mutex<>>`.

**Recommended pattern using `std::sync::mpsc`:**

1. Add to `InputState` (or a sibling struct that lives alongside it):
```rust
use std::sync::mpsc::{Receiver, Sender};

pub struct FetchState {
    /// Channel receiver for completed fetch results
    pub rx: Receiver<FetchResult>,
    /// Channel sender (cloned into spawned threads)
    pub tx: Sender<FetchResult>,
    /// Currently pending fetches (account_idx, position_idx) -> start_time
    pub pending: HashMap<(usize, usize), std::time::Instant>,
}

pub enum FetchResult {
    PriceAndCagr {
        account_idx: usize,
        position_idx: usize,
        price: f64,
        cagr: f64,
    },
    DetailedProfile {
        account_idx: usize,
        position_idx: usize,
        profile: DetailedMarketProfile,
        show_cap: bool,
        show_nav: bool,
        show_cg: bool,
        show_er: bool,
    },
    DcFund {
        account_idx: usize,
        fund_idx: usize,
        price: f64,
        cagr: f64,
    },
    Error {
        account_idx: usize,
        position_idx: usize,
        message: String,
    },
}
```

2. Initialize in the app constructor:
```rust
let (tx, rx) = std::sync::mpsc::channel();
let fetch_state = FetchState {
    rx,
    tx,
    pending: HashMap::new(),
};
```

#### B. Spawn threads instead of blocking

Replace the blocking fetch blocks (lines 2976-3053 in `input_panel.rs`) with thread spawns:

**Before (line 2976-2987):**
```rust
if let Some((ai, pi)) = auto_fill {
    if ai < state.accounts.len() && pi < state.accounts[ai].positions.len() {
        let ticker = state.accounts[ai].positions[pi].ticker.clone();
        if !ticker.is_empty() {
            let price = crate::engine::market_data::MarketDataService::fetch_current_price(&ticker);
            let cagr  = crate::engine::market_data::MarketDataService::fetch_10y_cagr(&ticker);
            let pos = &mut state.accounts[ai].positions[pi];
            pos.unit_value = format!("{:.2}", price);
            pos.growth_pct = format!("{:.1}", cagr * 100.0);
        }
    }
}
```

**After:**
```rust
if let Some((ai, pi)) = auto_fill {
    if ai < state.accounts.len() && pi < state.accounts[ai].positions.len() {
        let ticker = state.accounts[ai].positions[pi].ticker.clone();
        if !ticker.is_empty() {
            let tx = fetch_state.tx.clone();
            fetch_state.pending.insert((ai, pi), std::time::Instant::now());
            std::thread::spawn(move || {
                let price = crate::engine::market_data::MarketDataService::fetch_current_price(&ticker);
                let cagr  = crate::engine::market_data::MarketDataService::fetch_10y_cagr(&ticker);
                let _ = tx.send(FetchResult::PriceAndCagr {
                    account_idx: ai,
                    position_idx: pi,
                    price,
                    cagr,
                });
            });
        }
    }
}
```

Do the same for `dc_auto_fill` and `auto_fill_profile` blocks.

#### C. Poll for results each frame

At the **top** of the `input_panel` function (before any UI drawing), drain the channel:

```rust
// Poll for completed fetch results
while let Ok(result) = fetch_state.rx.try_recv() {
    match result {
        FetchResult::PriceAndCagr { account_idx, position_idx, price, cagr } => {
            fetch_state.pending.remove(&(account_idx, position_idx));
            if account_idx < state.accounts.len() && position_idx < state.accounts[account_idx].positions.len() {
                let pos = &mut state.accounts[account_idx].positions[position_idx];
                pos.unit_value = format!("{:.2}", price);
                pos.growth_pct = format!("{:.1}", cagr * 100.0);
            }
        }
        FetchResult::DetailedProfile { account_idx, position_idx, profile, show_cap, show_nav, show_cg, show_er } => {
            fetch_state.pending.remove(&(account_idx, position_idx));
            if account_idx < state.accounts.len() && position_idx < state.accounts[account_idx].positions.len() {
                let pos = &mut state.accounts[account_idx].positions[position_idx];
                pos.dividend_yield_pct = format!("{:.3}", profile.dividend_yield * 100.0);
                if show_cap { pos.cap_growth_pct     = format!("{:.3}", profile.cap_growth * 100.0); }
                if show_nav { pos.nav_growth_pct     = format!("{:.3}", profile.nav_growth * 100.0); }
                if show_cg  { pos.cap_gains_dist_pct = format!("{:.3}", profile.cap_gains_dist * 100.0); }
                if show_er  { pos.expense_ratio_pct  = format!("{:.3}", profile.expense_ratio * 100.0); }
                pos.use_detailed_profile = true;
            }
        }
        FetchResult::DcFund { account_idx, fund_idx, price, cagr } => {
            fetch_state.pending.remove(&(account_idx, fund_idx));
            if account_idx < state.accounts.len() && fund_idx < state.accounts[account_idx].dc_funds.len() {
                let fund = &mut state.accounts[account_idx].dc_funds[fund_idx];
                fund.price_per_10k = format!("{:.0}", price);
                fund.growth_pct    = format!("{:.1}", cagr * 100.0);
            }
        }
        FetchResult::Error { account_idx, position_idx, message } => {
            fetch_state.pending.remove(&(account_idx, position_idx));
            log::warn!("[AutoFetch] Error for ({}, {}): {}", account_idx, position_idx, message);
        }
    }
}
```

#### D. Show a loading indicator with elapsed timer

Next to each position's "✨" button, check if a fetch is pending for that `(acct_idx, pos_idx)`. If so, show a spinner/timer instead of (or next to) the button:

```rust
if fetch_state.pending.contains_key(&(acct_idx, pos_idx)) {
    let elapsed = fetch_state.pending[&(acct_idx, pos_idx)].elapsed();
    ui.label(
        RichText::new(format!("⏳ Fetching... {:.0}s", elapsed.as_secs_f32()))
            .small()
            .color(Color32::from_rgb(255, 200, 80))
    );
    // Request repaint so timer updates
    ui.ctx().request_repaint();
} else {
    // Show the normal ✨ button
    if ui.small_button("✨").on_hover_text("Auto-fill...").clicked() {
        auto_fill = Some((acct_idx, pos_idx));
    }
}
```

**Important:** Call `ui.ctx().request_repaint()` when there are pending fetches so egui redraws each frame and the timer updates smoothly. You can also add this as a global check at the top:

```rust
if !fetch_state.pending.is_empty() {
    ui.ctx().request_repaint();
}
```

#### E. Add a timeout to ureq calls

**File:** `src/engine/market_data/mod.rs`

Add timeouts to prevent indefinite hangs. Change all `ureq::get(&url)` calls to include a timeout:

```rust
let resp = ureq::AgentBuilder::new()
    .timeout_connect(std::time::Duration::from_secs(10))
    .timeout_read(std::time::Duration::from_secs(15))
    .build()
    .get(&url)
    .set("User-Agent", "Mozilla/5.0 retirement-calculator/1.0")
    .call()?;
```

Or create a shared agent once and reuse it:

```rust
use std::sync::LazyLock;

static HTTP_AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(15))
        .build()
});
```

Then replace `ureq::get(&url)` with `HTTP_AGENT.get(&url)` throughout.

#### F. Disable the fetch button while pending

To prevent double-clicks, disable the button when a fetch is already in progress for that position:

```rust
let is_fetching = fetch_state.pending.contains_key(&(acct_idx, pos_idx));
ui.add_enabled(!is_fetching, egui::Button::new("✨").small())
```

---

## Summary of Files to Modify

| File | Changes |
|------|---------|
| `src/ui/panels/input_panel.rs` | Remove Rebalance Date field from Timing; add rebalance summary + warning; replace blocking fetches with thread spawns; add loading indicators |
| `src/engine/market_data/mod.rs` | Add HTTP timeouts via shared `ureq::Agent` |
| `src/simulation/controller.rs` | (Optional) Change transition trigger from `rebalance_date` to `retirement_date` |
| `src/models/config.rs` | (Optional) Remove or deprecate `rebalance_date` field |

## Dependencies

No new crate dependencies are required. `std::thread`, `std::sync::mpsc`, `std::time::Instant`, and `std::sync::LazyLock` are all in the standard library. `ureq::AgentBuilder` is already available in `ureq = "2.10"`.

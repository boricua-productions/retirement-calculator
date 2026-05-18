use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

fn default_apply_to_floor() -> bool { true }
fn default_frequency_months() -> u32 { 1 }

/// Represents a one-time or temporary expense/income event during the simulation.
/// A negative `amount_jpy` models an income event (e.g., a bonus).
/// Mirrors Python's `ExpenseRule` dataclass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpenseRule {
    pub name: String,
    pub amount_jpy: f64,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,

    /// V8.1 — When false, the rule's amount is added to `base_desired` only,
    /// NOT to `base_floor`. Default true preserves legacy behavior.
    #[serde(default = "default_apply_to_floor")]
    pub apply_to_floor: bool,

    /// V8.1 — When true, the rule's amount is multiplied by the same
    /// Japan-CPI inflation factor that scales the user's base/min scalars
    /// before being added. Used by detailed-mode synthetic stop-rules so they
    /// keep pace with the inflated base. Default false preserves legacy behavior.
    #[serde(default)]
    pub inflate: bool,
}

impl ExpenseRule {
    pub fn new(name: impl Into<String>, amount_jpy: f64, start_date: NaiveDate, end_date: NaiveDate) -> Self {
        Self {
            name: name.into(),
            amount_jpy,
            start_date,
            end_date,
            apply_to_floor: true,
            inflate: false,
        }
    }

    /// Returns true if this rule is active on the given date.
    pub fn is_active_on(&self, date: NaiveDate) -> bool {
        date >= self.start_date && date <= self.end_date
    }
}

// ─── V8.1 Detailed Expense Categories ────────────────────────────────────────

/// V8.1 — Whether a detailed-expense category counts toward the minimum floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CategoryKind {
    /// Counts toward both `base_expense_jpy` AND `min_expense_jpy`.
    #[default]
    Essential,
    /// Counts toward `base_expense_jpy` ONLY.
    Discretional,
}

/// V8.1 — A single user-defined expense category for the detailed-entry mode.
///
/// `amount_jpy` is the amount per billing period; `frequency_months` is the
/// billing cadence (1 = monthly, 12 = annual, 24 = car shaken every 2 years,
/// 60 = home insurance every 5 years, etc.). Effective monthly burn is
/// `amount_jpy / frequency_months` and is what the UI/save path sums into
/// `base_expense_jpy` / `min_expense_jpy`.
///
/// `end_date`, when set, makes the category stop contributing to expenses on
/// the first of the month after that date. The loader emits a synthetic
/// negative `ExpenseRule` to implement this dynamically during the simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpenseCategory {
    pub name: String,
    #[serde(default)]
    pub kind: CategoryKind,
    pub amount_jpy: f64,
    /// 1 = monthly (default). 12 = annual. 24 = every 2 years. 60 = every 5 years. Must be ≥ 1.
    #[serde(default = "default_frequency_months")]
    pub frequency_months: u32,
    /// None = ongoing for life of simulation. Some(d) = stops contributing on the
    /// first of the month after `d`.
    #[serde(default)]
    pub end_date: Option<chrono::NaiveDate>,
    /// Optional free-form note (e.g. "rent + management fee", "billed Apr").
    #[serde(default)]
    pub note: String,
}

impl Default for ExpenseCategory {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: CategoryKind::Essential,
            amount_jpy: 0.0,
            frequency_months: 1,
            end_date: None,
            note: String::new(),
        }
    }
}

impl ExpenseCategory {
    /// Amortized monthly burn for this category, regardless of cadence.
    pub fn effective_monthly_jpy(&self) -> f64 {
        let f = self.frequency_months.max(1) as f64;
        self.amount_jpy / f
    }
}

/// V8.1 — Returns true if a category name overlaps with an engine-computed
/// expense stream (NHI or Japan resident tax). Case-insensitive.
///
/// Accepted false-positive risk: substring `"nhi"` will match innocuous names
/// like "NHILINGSWORTH". Users can rename to work around this.
pub fn looks_like_reserved_category(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.contains("nhi")
        || lower.contains("national health")
        || lower.contains("kokumin kenko")
        || lower.contains("kokumin-kenko")
        || lower.contains("juminzei")
        || lower.contains("jumin-zei")
        || lower.contains("resident tax")
    {
        return true;
    }
    // Kanji forms
    name.contains('\u{56fd}') && name.contains('\u{6c11}') && name.contains('\u{5065}')  // 国民健
        || name.contains('\u{4f4f}') && name.contains('\u{6c11}') && name.contains('\u{7a0e}')  // 住民税
}

/// V8.1 — Default seeded expense categories shown when the user first enables
/// detailed mode. Amounts are zero except Home Fire & Earthquake Insurance.
pub fn default_expense_categories() -> Vec<ExpenseCategory> {
    let mk = |name: &str, kind: CategoryKind, amt: f64, freq: u32, note: &str| ExpenseCategory {
        name: name.into(),
        kind,
        amount_jpy: amt,
        frequency_months: freq,
        end_date: None,
        note: note.into(),
    };
    vec![
        // ── Essential ─────────────────────────────────────────────────────────
        mk("House Loan Repayment",              CategoryKind::Essential,    0.0,      1,  ""),
        mk("Land Loan Repayment",               CategoryKind::Essential,    0.0,      1,  ""),
        mk("Land & House Taxes",                CategoryKind::Essential,    0.0,     12,  "Fixed asset tax — billed annually"),
        mk("Home Fire & Earthquake Insurance",  CategoryKind::Essential,    321530.0, 60, "5-year premium"),
        mk("Monthly Electric Bill",             CategoryKind::Essential,    0.0,      1,  ""),
        mk("Monthly Water & Sewage Bill",       CategoryKind::Essential,    0.0,      1,  ""),
        mk("Monthly Phone Bill (3 lines)",      CategoryKind::Essential,    0.0,      1,  ""),
        mk("Monthly Internet Bill",             CategoryKind::Essential,    0.0,      1,  ""),
        mk("Monthly Car Insurance",             CategoryKind::Essential,    0.0,      1,  ""),
        mk("Car Shaken",                        CategoryKind::Essential,    0.0,     24,  "Biennial mandatory inspection"),
        mk("Monthly Groceries",                 CategoryKind::Essential,    0.0,      1,  ""),
        mk("Personal & Home Care",              CategoryKind::Essential,    0.0,      1,  ""),
        mk("Baseline Pet Care",                 CategoryKind::Essential,    0.0,      1,  ""),
        mk("Monthly Car Gas",                   CategoryKind::Essential,    0.0,      1,  ""),
        mk("Monthly Car Maintenance",           CategoryKind::Essential,    0.0,      1,  ""),
        // ── Discretional ──────────────────────────────────────────────────────
        mk("Dining Out & Take-Away",                         CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Clothing & Footwear",                            CategoryKind::Discretional, 0.0, 1,  ""),
        mk("General Retail & Hobbies",                       CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Child Extracurricular School & Activities",      CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Other Child Costs",                              CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Digital Subscriptions & Apps",                   CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Haircut / Hair Salon",                           CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Social Gift Giving & Events",                    CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Monthly Fun Fund (JPY)",                         CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Medical Emergency Buffer",                       CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Senior Pet Vet Fund",                            CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Monthly Car Replacement Fund",                   CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Monthly Home Maintenance Fund",                  CategoryKind::Discretional, 0.0, 1,  ""),
        mk("Annual Travel Fund",                             CategoryKind::Discretional, 0.0, 12, ""),
    ]
}

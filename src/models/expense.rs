use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Represents a one-time or temporary expense/income event during the simulation.
/// A negative `amount_jpy` models an income event (e.g., a bonus).
/// Mirrors Python's `ExpenseRule` dataclass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpenseRule {
    pub name: String,
    pub amount_jpy: f64,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
}

impl ExpenseRule {
    pub fn new(name: impl Into<String>, amount_jpy: f64, start_date: NaiveDate, end_date: NaiveDate) -> Self {
        Self {
            name: name.into(),
            amount_jpy,
            start_date,
            end_date,
        }
    }

    /// Returns true if this rule is active on the given date.
    pub fn is_active_on(&self, date: NaiveDate) -> bool {
        date >= self.start_date && date <= self.end_date
    }
}

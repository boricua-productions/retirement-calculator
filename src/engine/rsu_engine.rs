use chrono::{Datelike, NaiveDate};
use std::collections::HashMap;

use crate::models::rsu::{RsuAward, VestingEvent, VestStatus};

/// Manages the full lifecycle of RSU awards:
/// generating the vesting schedule and answering queries against it.
/// Mirrors Python's `RSUEngine` class in `engine.py`.
pub struct RsuEngine {
    awards: Vec<RsuAward>,
    retirement_date: Option<NaiveDate>,
    /// Sorted list of all vesting events across all awards (ascending date).
    pub vesting_schedule: Vec<VestingEvent>,
}

impl RsuEngine {
    pub fn new(awards: Vec<RsuAward>, retirement_date: Option<NaiveDate>) -> Self {
        let schedule = Self::generate_vesting_schedule(&awards, retirement_date);
        Self { awards, retirement_date, vesting_schedule: schedule }
    }

    /// Creates the full sorted vesting schedule from all awards.
    ///
    /// Supports `vesting_months` list, `VestingCadence`, `vesting_start_date`,
    /// `vesting_months_total`, and `cliff_vest_months`. When `vesting_start_date`
    /// is set it is used as the clock origin instead of `grant_date`. When
    /// `cliff_vest_months > 0`, events before the cliff end date are accumulated
    /// and delivered all at once on the first post-cliff event.
    fn generate_vesting_schedule(
        awards: &[RsuAward],
        retirement_date: Option<NaiveDate>,
    ) -> Vec<VestingEvent> {
        let mut events: Vec<VestingEvent> = Vec::new();

        for award in awards {
            let effective_months = award.effective_vesting_months();

            // vesting_months_total (in months) takes precedence over vesting_years.
            let effective_years: u32 = award.vesting_months_total
                .map(|m| (m + 11) / 12)
                .unwrap_or(award.vesting_years);

            if effective_years == 0
                || effective_months.is_empty()
                || award.total_shares <= 0.0
            {
                continue;
            }

            let clock_origin = award.effective_start_date();

            let total_events = effective_years as usize * effective_months.len();
            if total_events == 0 {
                continue;
            }
            let shares_per_event = award.total_shares / total_events as f64;

            let end_date = add_years(clock_origin, effective_years);

            // Cliff: accumulate events before cliff_end_date into the first real event.
            let cliff_end_opt = if award.cliff_vest_months > 0 {
                Some(add_months(clock_origin, award.cliff_vest_months))
            } else {
                None
            };
            let mut cliff_triggered = cliff_end_opt.is_none();
            let mut cliff_skipped: usize = 0;

            for year_offset in 0..effective_years {
                let base_date = add_years(clock_origin, year_offset);

                for &month in &effective_months {
                    let mut vesting_date = NaiveDate::from_ymd_opt(base_date.year(), month, 1)
                        .expect("invalid month in vesting schedule");

                    if vesting_date < base_date {
                        vesting_date = NaiveDate::from_ymd_opt(base_date.year() + 1, month, 1)
                            .expect("invalid month in vesting schedule");
                    }

                    if vesting_date >= end_date {
                        continue;
                    }

                    if let Some(ret) = retirement_date {
                        if vesting_date >= ret {
                            continue;
                        }
                    }

                    // Cliff accumulation: skip events before the cliff end date.
                    if let Some(cliff_end) = cliff_end_opt {
                        if !cliff_triggered && vesting_date < cliff_end {
                            cliff_skipped += 1;
                            continue;
                        }
                    }

                    let shares = if !cliff_triggered {
                        cliff_triggered = true;
                        shares_per_event * (cliff_skipped as f64 + 1.0)
                    } else {
                        shares_per_event
                    };

                    events.push(VestingEvent {
                        date: vesting_date,
                        shares,
                        ticker: award.ticker.clone(),
                    });
                }
            }
        }

        events.sort_by_key(|e| e.date);
        events
    }

    /// Returns all vesting events that fall within the given month.
    pub fn events_for_month(&self, date: NaiveDate) -> Vec<&VestingEvent> {
        self.vesting_schedule
            .iter()
            .filter(|e| e.date.year() == date.year() && e.date.month() == date.month())
            .collect()
    }

    /// Calculates total vested and unvested shares for all tickers as of `as_of`.
    pub fn vested_and_unvested(&self, as_of: NaiveDate) -> HashMap<String, VestStatus> {
        let mut summary: HashMap<String, VestStatus> = HashMap::new();

        // Seed totals from the original awards.
        for award in &self.awards {
            let entry = summary.entry(award.ticker.clone()).or_default();
            entry.unvested += award.total_shares;
        }

        // Move shares from unvested → vested as events occur.
        for event in &self.vesting_schedule {
            if event.date <= as_of {
                if let Some(status) = summary.get_mut(&event.ticker) {
                    status.vested += event.shares;
                    status.unvested -= event.shares;
                }
            }
        }

        // After retirement, all remaining unvested shares are forfeited.
        if let Some(ret) = self.retirement_date {
            if as_of >= ret {
                for status in summary.values_mut() {
                    status.unvested = 0.0;
                }
            }
        }

        summary
    }
}

/// Add `months` to a date. Always returns the 1st of the resulting month.
fn add_months(date: NaiveDate, months: u32) -> NaiveDate {
    let total = date.year() as i64 * 12 + date.month() as i64 - 1 + months as i64;
    let y = (total / 12) as i32;
    let m = (total % 12 + 1) as u32;
    NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(date)
}

/// Add `years` to a date, handling month/day clamping (e.g., Feb 29 → Feb 28).
/// Mirrors Python's `date + relativedelta(years=n)`.
pub fn add_years(date: NaiveDate, years: u32) -> NaiveDate {
    let target_year = date.year() + years as i32;
    NaiveDate::from_ymd_opt(target_year, date.month(), date.day())
        .or_else(|| NaiveDate::from_ymd_opt(target_year, date.month(), date.day() - 1))
        .unwrap_or(date)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::rsu::VestingCadence;

    fn make_award(grant: &str, months: Vec<u32>, years: u32, shares: f64) -> RsuAward {
        RsuAward {
            grant_date: NaiveDate::parse_from_str(grant, "%Y-%m-%d").unwrap(),
            vesting_start_date: None,
            ticker: "TEST".into(),
            total_shares: shares,
            vesting_years: years,
            vesting_months_total: None,
            vesting_months: months,
            vesting_cadence: VestingCadence::Quarterly,
            cliff_vest_months: 0,
        }
    }

    #[test]
    fn test_first_event_aug_2025_for_nov_2024_grant() {
        let award = make_award("2024-11-01", vec![2, 5, 8, 11], 4, 400.0);
        let engine = RsuEngine::new(vec![award], None);
        assert_eq!(engine.vesting_schedule[0].date, NaiveDate::from_ymd_opt(2024, 11, 1).unwrap());
    }

    #[test]
    fn test_vesting_cutoff_at_retirement() {
        let award = make_award("2024-01-01", vec![1, 7], 4, 400.0);
        let retirement = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let engine = RsuEngine::new(vec![award], Some(retirement));
        for event in &engine.vesting_schedule {
            assert!(event.date < retirement, "Event {:?} should be before retirement", event.date);
        }
    }

    #[test]
    fn test_vested_plus_unvested_equals_total_shares() {
        let total = 120.0;
        let award = make_award("2023-01-01", vec![3, 9], 4, total);
        let engine = RsuEngine::new(vec![award], None);
        let as_of = NaiveDate::from_ymd_opt(2025, 6, 1).unwrap();
        let summary = engine.vested_and_unvested(as_of);
        let status = summary.get("TEST").unwrap();
        let sum = status.vested + status.unvested;
        assert!((sum - total).abs() < 1e-9, "vested+unvested={} != total={}", sum, total);
    }

    #[test]
    fn test_events_for_month() {
        let award = make_award("2023-01-01", vec![3, 9], 2, 100.0);
        let engine = RsuEngine::new(vec![award], None);
        let march_2023 = NaiveDate::from_ymd_opt(2023, 3, 15).unwrap();
        let events = engine.events_for_month(march_2023);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].date.month(), 3);
    }

    #[test]
    fn test_monthly_cadence_generates_12_events_per_year() {
        let award = RsuAward {
            grant_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            vesting_start_date: None,
            ticker: "TEST".into(),
            total_shares: 120.0,
            vesting_years: 1,
            vesting_months_total: None,
            vesting_months: vec![],
            vesting_cadence: VestingCadence::Monthly,
            cliff_vest_months: 0,
        };
        let engine = RsuEngine::new(vec![award], None);
        assert_eq!(engine.vesting_schedule.len(), 12);
    }

    #[test]
    fn test_annual_cadence_generates_1_event_per_year() {
        let award = RsuAward {
            grant_date: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
            vesting_start_date: None,
            ticker: "TEST".into(),
            total_shares: 40.0,
            vesting_years: 4,
            vesting_months_total: None,
            vesting_months: vec![],
            vesting_cadence: VestingCadence::Annually,
            cliff_vest_months: 0,
        };
        let engine = RsuEngine::new(vec![award], None);
        assert_eq!(engine.vesting_schedule.len(), 4);
    }

    #[test]
    fn test_vesting_start_date_shifts_clock_origin() {
        let award = RsuAward {
            grant_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            vesting_start_date: Some(NaiveDate::from_ymd_opt(2025, 1, 1).unwrap()),
            ticker: "TEST".into(),
            total_shares: 100.0,
            vesting_years: 1,
            vesting_months_total: None,
            vesting_months: vec![1],
            vesting_cadence: VestingCadence::Annually,
            cliff_vest_months: 0,
        };
        let engine = RsuEngine::new(vec![award], None);
        assert_eq!(engine.vesting_schedule[0].date.year(), 2025);
    }

    #[test]
    fn test_cliff_zero_is_normal_vesting() {
        // cliff=0 must produce an identical schedule to an award with no cliff field.
        let base  = make_award("2024-01-01", vec![2, 5, 8, 11], 2, 200.0);
        let award = RsuAward { cliff_vest_months: 0, ..base.clone() };
        let e_base  = RsuEngine::new(vec![base],  None).vesting_schedule;
        let e_cliff = RsuEngine::new(vec![award], None).vesting_schedule;
        assert_eq!(e_base.len(), e_cliff.len());
        for (a, b) in e_base.iter().zip(e_cliff.iter()) {
            assert_eq!(a.date, b.date);
            assert!((a.shares - b.shares).abs() < 1e-9);
        }
    }

    #[test]
    fn test_cliff_3_month_quarterly_doubles_first_event() {
        // Grant Jan 2024, quarterly [2,5,8,11], 4 years, 400 shares, 3-month cliff.
        // cliff_end = 2024-04-01. Feb-24 is before the cliff → skipped (1 event).
        // First real event: May-24 → 2× shares_per_event.
        let award = RsuAward {
            cliff_vest_months: 3,
            ..make_award("2024-01-01", vec![2, 5, 8, 11], 4, 400.0)
        };
        let engine = RsuEngine::new(vec![award], None);
        let sched = &engine.vesting_schedule;
        // One event skipped → 15 emitted instead of 16.
        assert_eq!(sched.len(), 15);
        // First event is May-24 with 2× the normal share count.
        assert_eq!(sched[0].date, NaiveDate::from_ymd_opt(2024, 5, 1).unwrap());
        let shares_per = 400.0 / 16.0;
        assert!((sched[0].shares - shares_per * 2.0).abs() < 1e-9,
            "first event shares={}", sched[0].shares);
        // Total shares preserved.
        let total: f64 = sched.iter().map(|e| e.shares).sum();
        assert!((total - 400.0).abs() < 1e-9, "total={}", total);
    }

    #[test]
    fn test_cliff_14_month_quarterly_total_preserved() {
        // Grant Jan 2024, quarterly [2,5,8,11], 4 years, 400 shares, 14-month cliff.
        // cliff_end = add_months(2024-01-01, 14) = 2025-03-01.
        // Events before 2025-03-01: Feb24, May24, Aug24, Nov24, Feb25 → 5 skipped.
        // First real event: May-25, shares = 6× normal.
        let award = RsuAward {
            cliff_vest_months: 14,
            ..make_award("2024-01-01", vec![2, 5, 8, 11], 4, 400.0)
        };
        let engine = RsuEngine::new(vec![award], None);
        let sched = &engine.vesting_schedule;
        assert_eq!(sched.len(), 11, "expected 16-5=11 events, got {}", sched.len());
        let shares_per = 400.0 / 16.0;
        assert!((sched[0].shares - shares_per * 6.0).abs() < 1e-9,
            "first event shares={} (expected {})", sched[0].shares, shares_per * 6.0);
        let total: f64 = sched.iter().map(|e| e.shares).sum();
        assert!((total - 400.0).abs() < 1e-9, "total shares={}", total);
    }
}

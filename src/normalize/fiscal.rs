//! Fiscal calendar alignment.
//!
//! Two traps this module exists to avoid:
//!
//! 1. **`fy`/`fp` on a fact describe the filing, not the fact.** A 10-K filed
//!    for FY2020 carries `fy=2020, fp=FY` on its FY2018 comparatives too.
//!    Using those fields as period identity misfiles years of history. Period
//!    identity is derived from `period_start` and `period_end` instead.
//!
//! 2. **Not every registrant ends its year in December.** Apple ends in late
//!    September, and 52/53-week filers move their year-end by a few days
//!    annually. Fiscal periods are resolved against each company's own
//!    year-end with a tolerance, never assumed.

use chrono::{Datelike, Duration, NaiveDate};

/// Day counts by period shape. Ranges are wide because 52/53-week fiscal
/// calendars and transition periods both wander.
const QUARTER_DAYS: (i64, i64) = (80, 100);
const HALF_DAYS: (i64, i64) = (170, 195);
const NINE_MONTH_DAYS: (i64, i64) = (260, 285);
const ANNUAL_DAYS: (i64, i64) = (340, 380);

/// How far a period end may sit from the nominal fiscal year end and still
/// belong to it.
const FYE_TOLERANCE_DAYS: i64 = 20;

const DAYS_PER_QUARTER: f64 = 91.31;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Instant,
    Quarter,
    Half,
    NineMonth,
    Annual,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FiscalPeriod {
    /// SEC convention: a fiscal year is labelled by the calendar year in
    /// which it ends.
    pub fiscal_year: i32,
    /// `Q1`..`Q4`, `FY`, or a cumulative stub (`half`, `nine_month`).
    pub fiscal_period: Period,
    pub kind: Kind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Q1,
    Q2,
    Q3,
    Q4,
    Fy,
    Half,
    NineMonth,
}

impl Period {
    pub fn as_str(&self) -> &'static str {
        match self {
            Period::Q1 => "Q1",
            Period::Q2 => "Q2",
            Period::Q3 => "Q3",
            Period::Q4 => "Q4",
            Period::Fy => "FY",
            Period::Half => "H1",
            Period::NineMonth => "9M",
        }
    }
}

/// What shape of period is this — a quarter, a year, or a cumulative stub?
///
/// XBRL reports year-to-date figures alongside quarterly ones, so half-year
/// and nine-month durations are common and must be recognised rather than
/// mistaken for quarters.
pub fn classify(period_start: NaiveDate, period_end: NaiveDate, is_instant: bool) -> Kind {
    if is_instant || period_start == period_end {
        return Kind::Instant;
    }
    let days = (period_end - period_start).num_days() + 1;
    for (kind, (low, high)) in [
        (Kind::Quarter, QUARTER_DAYS),
        (Kind::Half, HALF_DAYS),
        (Kind::NineMonth, NINE_MONTH_DAYS),
        (Kind::Annual, ANNUAL_DAYS),
    ] {
        if (low..=high).contains(&days) {
            return kind;
        }
    }
    Kind::Other
}

/// Parse the submissions API's `fiscalYearEnd`, e.g. `"0928"` or `"--12-31"`.
pub fn parse_fye(fiscal_year_end: Option<&str>) -> Option<(u32, u32)> {
    let digits: String = fiscal_year_end?
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    if digits.len() != 4 {
        return None;
    }
    let month: u32 = digits[..2].parse().ok()?;
    let day: u32 = digits[2..].parse().ok()?;
    if (1..=12).contains(&month) && (1..=31).contains(&day) {
        Some((month, day))
    } else {
        None
    }
}

/// Fiscal year end in a given year, clamped for short months and leap years.
fn fye_date(year: i32, month: u32, day: u32) -> NaiveDate {
    let last = last_day_of_month(year, month);
    NaiveDate::from_ymd_opt(year, month, day.min(last)).expect("clamped date is valid")
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (next_y, next_m) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    (NaiveDate::from_ymd_opt(next_y, next_m, 1).expect("first of month") - Duration::days(1)).day()
}

/// Map a period end onto the company's fiscal calendar.
///
/// Returns `None` for shapes that do not correspond to a reportable period
/// (cumulative stubs of unknown shape, transition periods, malformed year
/// ends). Callers treat `None` as "not part of the panel" rather than as an
/// error.
pub fn resolve(
    period_end: NaiveDate,
    fiscal_year_end: Option<&str>,
    kind: Kind,
) -> Option<FiscalPeriod> {
    let (month, day) = parse_fye(fiscal_year_end)?;
    if kind == Kind::Other || kind == Kind::Instant {
        return None;
    }

    // The fiscal year that `period_end` falls within is the first whose year
    // end lands on or after it, allowing for 52/53-week drift.
    let mut year_end = None;
    for candidate_year in [
        period_end.year() - 1,
        period_end.year(),
        period_end.year() + 1,
    ] {
        let candidate = fye_date(candidate_year, month, day);
        if period_end <= candidate + Duration::days(FYE_TOLERANCE_DAYS) {
            year_end = Some(candidate);
            break;
        }
    }
    let year_end = year_end?;
    let fiscal_year = year_end.year();

    match kind {
        Kind::Annual => Some(FiscalPeriod {
            fiscal_year,
            fiscal_period: Period::Fy,
            kind,
        }),
        // Cumulative periods are keyed to the year; only the annual one is
        // retained downstream, but all need a year for Q4 derivation.
        Kind::Half => Some(FiscalPeriod {
            fiscal_year,
            fiscal_period: Period::Half,
            kind,
        }),
        Kind::NineMonth => Some(FiscalPeriod {
            fiscal_year,
            fiscal_period: Period::NineMonth,
            kind,
        }),
        Kind::Quarter => {
            let quarters_before_year_end =
                ((year_end - period_end).num_days() as f64 / DAYS_PER_QUARTER).round() as i64;
            let quarter = 4 - quarters_before_year_end;
            let fiscal_period = match quarter {
                1 => Period::Q1,
                2 => Period::Q2,
                3 => Period::Q3,
                4 => Period::Q4,
                _ => return None,
            };
            Some(FiscalPeriod {
                fiscal_year,
                fiscal_period,
                kind,
            })
        }
        Kind::Instant | Kind::Other => None,
    }
}

/// Balance-sheet dates map to the quarter they close.
pub fn resolve_instant(
    period_end: NaiveDate,
    fiscal_year_end: Option<&str>,
) -> Option<FiscalPeriod> {
    resolve(period_end, fiscal_year_end, Kind::Quarter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn classify_recognises_period_shapes() {
        assert_eq!(
            classify(d("2020-01-01"), d("2020-01-01"), false),
            Kind::Instant
        );
        assert_eq!(
            classify(d("2020-01-01"), d("2020-03-31"), false),
            Kind::Quarter
        );
        assert_eq!(
            classify(d("2020-01-01"), d("2020-06-30"), false),
            Kind::Half
        );
        assert_eq!(
            classify(d("2020-01-01"), d("2020-09-30"), false),
            Kind::NineMonth
        );
        assert_eq!(
            classify(d("2020-01-01"), d("2020-12-31"), false),
            Kind::Annual
        );
        assert_eq!(
            classify(d("2020-01-01"), d("2021-06-30"), false),
            Kind::Other
        );
    }

    #[test]
    fn parse_fye_accepts_both_edgar_formats() {
        assert_eq!(parse_fye(Some("0928")), Some((9, 28)));
        assert_eq!(parse_fye(Some("--12-31")), Some((12, 31)));
        assert_eq!(parse_fye(Some("1331")), None); // month 13
        assert_eq!(parse_fye(Some("")), None);
        assert_eq!(parse_fye(None), None);
    }

    /// Apple: fiscal year ends the last Saturday of September, so FY2019
    /// ended 2019-09-28 — a 52/53-week drift the tolerance must absorb.
    #[test]
    fn resolves_apple_like_september_year_end() {
        let fp = resolve(d("2019-09-28"), Some("0928"), Kind::Annual).unwrap();
        assert_eq!(fp.fiscal_year, 2019);
        assert_eq!(fp.fiscal_period, Period::Fy);

        // Q1 FY2020 ended 2019-12-28.
        let q1 = resolve(d("2019-12-28"), Some("0928"), Kind::Quarter).unwrap();
        assert_eq!(q1.fiscal_year, 2020);
        assert_eq!(q1.fiscal_period, Period::Q1);
    }

    /// NVIDIA: January year-end — the fiscal year labelled by the calendar
    /// year it ends in, so the year ending 2020-01-26 is FY2020.
    #[test]
    fn resolves_january_year_end() {
        let fp = resolve(d("2020-01-26"), Some("0131"), Kind::Annual).unwrap();
        assert_eq!(fp.fiscal_year, 2020);
        assert_eq!(fp.fiscal_period, Period::Fy);
    }

    #[test]
    fn calendar_year_filer_quarters() {
        let fye = Some("1231");
        for (end, expect) in [
            ("2020-03-31", Period::Q1),
            ("2020-06-30", Period::Q2),
            ("2020-09-30", Period::Q3),
            ("2020-12-31", Period::Q4),
        ] {
            let fp = resolve(d(end), fye, Kind::Quarter).unwrap();
            assert_eq!(fp.fiscal_period, expect, "period end {end}");
            assert_eq!(fp.fiscal_year, 2020);
        }
    }

    #[test]
    fn instants_map_to_their_closing_quarter() {
        let fp = resolve_instant(d("2020-06-30"), Some("1231")).unwrap();
        assert_eq!(fp.fiscal_period, Period::Q2);
        assert_eq!(fp.fiscal_year, 2020);
    }

    #[test]
    fn malformed_year_end_is_none_not_error() {
        assert!(resolve(d("2020-12-31"), None, Kind::Annual).is_none());
        assert!(resolve(d("2020-12-31"), Some("bogus"), Kind::Annual).is_none());
    }

    #[test]
    fn leap_and_short_months_clamp() {
        // FYE "0229" in a non-leap year clamps to Feb 28.
        let fp = resolve(d("2021-02-28"), Some("0229"), Kind::Annual).unwrap();
        assert_eq!(fp.fiscal_year, 2021);
    }
}

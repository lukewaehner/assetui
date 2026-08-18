//! Pure earnings analysis: EPS surprise and next-report-date selection.

use chrono::{DateTime, NaiveDate, Utc};

/// One quarter's EPS estimate against what the company actually reported.
#[derive(Debug, Clone, PartialEq)]
pub struct QuarterEps {
    /// Reporting period as upstream labels it, e.g. `2026Q2`.
    pub period: String,
    /// Consensus EPS estimate, absent when upstream has none.
    pub estimate: Option<f64>,
    /// Reported EPS, absent for a quarter that hasn't been reported yet.
    pub actual: Option<f64>,
}

/// One quarter's revenue, for the trend bars under the EPS table.
#[derive(Debug, Clone, PartialEq)]
pub struct QuarterRevenue {
    /// Reporting period as upstream labels it, e.g. `2026Q2`.
    pub period: String,
    /// Revenue for the period in the filer's reporting currency.
    pub revenue: Option<f64>,
}

/// Everything the earnings modal shows for one ticker.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TickerEarnings {
    /// The ticker this data belongs to.
    pub ticker: String,
    /// Quarterly EPS rows, newest first.
    pub eps: Vec<QuarterEps>,
    /// Quarterly revenue rows, newest first.
    pub revenue: Vec<QuarterRevenue>,
    /// The next scheduled report date, if one is in the future.
    pub next_report: Option<NaiveDate>,
}

pub fn surprise_pct(estimate: Option<f64>, actual: Option<f64>) -> Option<f64> {
    let (estimate, actual) = (estimate?, actual?);
    if estimate == 0.0 || !estimate.is_finite() || !actual.is_finite() {
        return None;
    }
    Some((actual - estimate) / estimate.abs() * 100.0)
}

pub fn next_report_date(dates: &[DateTime<Utc>], now: DateTime<Utc>) -> Option<NaiveDate> {
    dates
        .iter()
        .filter(|d| **d >= now)
        .min()
        .map(|d| d.date_naive())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    #[test]
    fn surprise_pct_is_positive_when_actual_beats_estimate() {
        assert_eq!(surprise_pct(Some(2.0), Some(2.5)), Some(25.0));
    }

    #[test]
    fn surprise_pct_is_negative_when_actual_misses_estimate() {
        assert_eq!(surprise_pct(Some(2.0), Some(1.5)), Some(-25.0));
    }

    #[test]
    fn surprise_pct_treats_a_narrowed_loss_as_a_beat() {
        assert_eq!(surprise_pct(Some(-0.50), Some(-0.25)), Some(50.0));
    }

    #[test]
    fn surprise_pct_is_none_for_a_zero_estimate() {
        assert_eq!(surprise_pct(Some(0.0), Some(1.20)), None);
    }

    #[test]
    fn surprise_pct_is_none_when_either_side_is_missing() {
        assert_eq!(surprise_pct(None, Some(1.20)), None);
        assert_eq!(surprise_pct(Some(1.20), None), None);
    }

    #[test]
    fn next_report_date_picks_the_earliest_future_date() {
        // arrange
        let dates = [dt(2026, 11, 5), dt(2026, 2, 1), dt(2026, 8, 30)];

        // act
        let next = next_report_date(&dates, dt(2026, 8, 18));

        // assert
        assert_eq!(next, Some(NaiveDate::from_ymd_opt(2026, 8, 30).unwrap()));
    }

    #[test]
    fn next_report_date_is_none_when_every_date_is_past() {
        let dates = [dt(2026, 2, 1), dt(2026, 5, 2)];

        assert_eq!(next_report_date(&dates, dt(2026, 8, 18)), None);
    }

    #[test]
    fn next_report_date_includes_a_date_equal_to_now() {
        let dates = [dt(2026, 8, 18)];

        assert_eq!(
            next_report_date(&dates, dt(2026, 8, 18)),
            Some(NaiveDate::from_ymd_opt(2026, 8, 18).unwrap())
        );
    }

    #[test]
    fn next_report_date_is_none_for_an_empty_calendar() {
        assert_eq!(next_report_date(&[], dt(2026, 8, 18)), None);
    }
}

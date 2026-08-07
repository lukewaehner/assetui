//! Portfolio optimization: efficient-frontier and correlation math.

use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet};

pub const TRADING_DAYS: f64 = 252.0;
/// Minimum aligned daily returns required to include a holding.
pub const MIN_RETURN_SAMPLES: usize = 30;

/// Log daily returns of a close series: `ln(c[i]/c[i-1])`.
pub fn daily_returns(closes: &[f64]) -> Vec<f64> {
    if closes.len() < 2 {
        return Vec::new();
    }
    closes
        .windows(2)
        .map(|w| if w[0] > 0.0 && w[1] > 0.0 { (w[1] / w[0]).ln() } else { 0.0 })
        .collect()
}

/// Per-asset mean daily return, annualized by `TRADING_DAYS`.
pub fn mean_returns(returns: &[Vec<f64>]) -> Vec<f64> {
    returns
        .iter()
        .map(|r| {
            if r.is_empty() {
                0.0
            } else {
                r.iter().sum::<f64>() / r.len() as f64 * TRADING_DAYS
            }
        })
        .collect()
}

/// Sample covariance matrix (divide by `n-1`), annualized by `TRADING_DAYS`.
pub fn covariance_matrix(returns: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let k = returns.len();
    let n = returns.first().map_or(0, |r| r.len());
    let mut cov = vec![vec![0.0; k]; k];
    if n < 2 {
        return cov;
    }
    let means: Vec<f64> = returns.iter().map(|r| r.iter().sum::<f64>() / n as f64).collect();
    for i in 0..k {
        for j in 0..k {
            let s: f64 = (0..n)
                .map(|t| (returns[i][t] - means[i]) * (returns[j][t] - means[j]))
                .sum();
            cov[i][j] = s / (n as f64 - 1.0) * TRADING_DAYS;
        }
    }
    cov
}

pub fn correlation_matrix(cov: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let k = cov.len();
    let sd: Vec<f64> = (0..k).map(|i| cov[i][i].max(0.0).sqrt()).collect();
    let mut corr = vec![vec![0.0; k]; k];
    for i in 0..k {
        for j in 0..k {
            corr[i][j] = if sd[i] > 0.0 && sd[j] > 0.0 {
                (cov[i][j] / (sd[i] * sd[j])).clamp(-1.0, 1.0)
            } else if i == j {
                1.0
            } else {
                0.0
            };
        }
    }
    corr
}

pub fn aligned_returns(
    histories: &[(String, Vec<(NaiveDate, f64)>)],
) -> (Vec<String>, Vec<Vec<f64>>) {
    let usable: Vec<(String, BTreeMap<NaiveDate, f64>)> = histories
        .iter()
        .filter(|(_, closes)| closes.len() > MIN_RETURN_SAMPLES)
        .map(|(t, closes)| (t.clone(), closes.iter().copied().collect()))
        .collect();
    if usable.len() < 2 {
        return (Vec::new(), Vec::new());
    }

    let mut common: BTreeSet<NaiveDate> = usable[0].1.keys().copied().collect();
    for (_, m) in &usable[1..] {
        let ks: BTreeSet<NaiveDate> = m.keys().copied().collect();
        common = common.intersection(&ks).copied().collect();
    }
    let dates: Vec<NaiveDate> = common.into_iter().collect();
    if dates.len() <= MIN_RETURN_SAMPLES {
        return (Vec::new(), Vec::new());
    }

    let tickers: Vec<String> = usable.iter().map(|(t, _)| t.clone()).collect();
    let returns: Vec<Vec<f64>> = usable
        .iter()
        .map(|(_, m)| {
            let closes: Vec<f64> = dates.iter().map(|d| m[d]).collect();
            daily_returns(&closes)
        })
        .collect();
    (tickers, returns)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    #[test]
    fn daily_returns_are_log_ratios_of_consecutive_closes() {
        // arrange
        let closes = [100.0, 110.0, 99.0];
        // act
        let r = daily_returns(&closes);
        // assert
        assert_eq!(r.len(), 2);
        approx(r[0], 0.09531017980432486);
        approx(r[1], -0.10536051565782628);
    }

    #[test]
    fn daily_returns_short_input_is_empty() {
        assert!(daily_returns(&[]).is_empty());
        assert!(daily_returns(&[100.0]).is_empty());
    }

    #[test]
    fn mean_returns_annualizes_the_daily_mean() {
        let r = vec![vec![0.0, 0.02, 0.01]];
        let m = mean_returns(&r);
        approx(m[0], 2.52);
    }

    #[test]
    fn correlation_diagonal_is_one_and_offdiagonal_in_range() {
        let cov = vec![vec![0.04, 0.012], vec![0.012, 0.09]];
        let corr = correlation_matrix(&cov);
        approx(corr[0][0], 1.0);
        approx(corr[1][1], 1.0);
        approx(corr[0][1], 0.2);
        assert!(corr[0][1] >= -1.0 && corr[0][1] <= 1.0);
    }

    #[test]
    fn aligned_returns_intersects_dates_and_drops_thin_holdings() {
        let d = |n: u32| NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(n as u64);
        let series_a: Vec<(NaiveDate, f64)> = (0..40).map(|n| (d(n), 100.0 + n as f64)).collect();
        let series_b: Vec<(NaiveDate, f64)> = (0..40).map(|n| (d(n), 50.0 + n as f64)).collect();
        let series_c: Vec<(NaiveDate, f64)> = (0..5).map(|n| (d(n), 10.0 + n as f64)).collect();
        let (tickers, returns) = aligned_returns(&[
            ("A".into(), series_a),
            ("B".into(), series_b),
            ("C".into(), series_c),
        ]);
        assert_eq!(tickers, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(returns.len(), 2);
        assert_eq!(returns[0].len(), 39);
        assert_eq!(returns[1].len(), 39);
    }

    #[test]
    fn aligned_returns_fewer_than_two_usable_is_empty() {
        let d = |n: u32| NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(n as u64);
        let only: Vec<(NaiveDate, f64)> = (0..40).map(|n| (d(n), 100.0 + n as f64)).collect();
        let (tickers, returns) = aligned_returns(&[("A".into(), only)]);
        assert!(tickers.is_empty());
        assert!(returns.is_empty());
    }

    #[test]
    fn aligned_returns_rejects_exactly_min_aligned_dates() {
        let d = |n: u32| NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(n as u64);
        let series_a: Vec<(NaiveDate, f64)> = (0..40).map(|n| (d(n), 100.0 + n as f64)).collect();
        let series_b: Vec<(NaiveDate, f64)> = (10..50)
            .map(|n| (d(n), 200.0 + n as f64))
            .collect();
        let (tickers, returns) = aligned_returns(&[("A".into(), series_a), ("B".into(), series_b)]);
        assert!(tickers.is_empty());
        assert!(returns.is_empty());
    }

    #[test]
    fn aligned_returns_accepts_one_past_min_aligned_dates() {
        let d = |n: u32| NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(n as u64);
        let series_a: Vec<(NaiveDate, f64)> = (0..40).map(|n| (d(n), 100.0 + n as f64)).collect();
        let series_b: Vec<(NaiveDate, f64)> = (9..49)
            .map(|n| (d(n), 200.0 + n as f64))
            .collect();
        let (tickers, returns) = aligned_returns(&[("A".into(), series_a), ("B".into(), series_b)]);
        assert_eq!(tickers.len(), 2);
        assert_eq!(tickers, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(returns.len(), 2);
        assert_eq!(returns[0].len(), 30);
        assert_eq!(returns[1].len(), 30);
    }
}

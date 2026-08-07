//! Portfolio optimization: efficient-frontier and correlation math.

use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub const TRADING_DAYS: f64 = 252.0;
/// Minimum aligned daily returns required to include a holding.
pub const MIN_RETURN_SAMPLES: usize = 30;
/// Annual risk-free rate used for Sharpe ratios and the tangency portfolio.
pub const RISK_FREE_ANNUAL: f64 = 0.04;
/// Number of random portfolios sampled for the Monte Carlo cloud.
pub const MC_SAMPLES: usize = 5_000;
/// Fixed PRNG seed so the sampled cloud is reproducible across runs.
pub const MC_SEED: u64 = 0x5EED_1234_ABCD_0001;

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

/// A point in risk/return space with its Sharpe ratio.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortfolioPoint {
    /// Annualized standard deviation (`sqrt(wᵀΣw)`).
    pub risk: f64,
    /// Annualized expected return (`wᵀμ`).
    pub ret: f64,
    /// `(ret - rf) / risk`.
    pub sharpe: f64,
}

/// A weighted portfolio and where it lands in risk/return space.
#[derive(Debug, Clone, PartialEq)]
pub struct Portfolio {
    /// Per-asset weights, index-aligned with the analysis' ticker list.
    pub weights: Vec<f64>,
    /// The portfolio's risk/return/Sharpe.
    pub point: PortfolioPoint,
}

/// Risk, return, and Sharpe for `weights` given annualized `mean`/`cov` and a risk-free rate `rf`.
pub fn portfolio_stats(weights: &[f64], mean: &[f64], cov: &[Vec<f64>], rf: f64) -> PortfolioPoint {
    let ret: f64 = weights.iter().zip(mean).map(|(w, m)| w * m).sum();
    let var: f64 = (0..weights.len())
        .map(|i| (0..weights.len()).map(|j| weights[i] * weights[j] * cov[i][j]).sum::<f64>())
        .sum();
    let risk = var.max(0.0).sqrt();
    let sharpe = if risk > 0.0 { (ret - rf) / risk } else { 0.0 };
    PortfolioPoint { risk, ret, sharpe }
}

/// Inverse of a square matrix via Gauss-Jordan elimination with partial pivoting.
fn invert(mat: &[Vec<f64>]) -> Option<Vec<Vec<f64>>> {
    let n = mat.len();
    let mut a: Vec<Vec<f64>> = mat
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut r = row.clone();
            r.extend((0..n).map(|j| if i == j { 1.0 } else { 0.0 }));
            r
        })
        .collect();
    for col in 0..n {
        let mut pivot = col;
        for r in (col + 1)..n {
            if a[r][col].abs() > a[pivot][col].abs() {
                pivot = r;
            }
        }
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        let d = a[col][col];
        for j in 0..2 * n {
            a[col][j] /= d;
        }
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a[r][col];
            if f != 0.0 {
                for j in 0..2 * n {
                    a[r][j] -= f * a[col][j];
                }
            }
        }
    }
    Some(a.iter().map(|row| row[n..].to_vec()).collect())
}

/// Matrix-vector product `mat · v`.
fn mat_vec(mat: &[Vec<f64>], v: &[f64]) -> Vec<f64> {
    mat.iter().map(|row| row.iter().zip(v).map(|(a, b)| a * b).sum()).collect()
}

/// Analytical global minimum-variance portfolio: `w = Σ⁻¹·1 / (1ᵀΣ⁻¹1)`.
pub fn min_variance(cov: &[Vec<f64>], mean: &[f64], rf: f64) -> Option<Portfolio> {
    let inv = invert(cov)?;
    let ones = vec![1.0; cov.len()];
    let z = mat_vec(&inv, &ones);
    let denom: f64 = z.iter().sum();
    if denom.abs() < 1e-12 {
        return None;
    }
    let weights: Vec<f64> = z.iter().map(|x| x / denom).collect();
    let point = portfolio_stats(&weights, mean, cov, rf);
    Some(Portfolio { weights, point })
}

pub fn tangency(mean: &[f64], cov: &[Vec<f64>], rf: f64) -> Option<Portfolio> {
    let inv = invert(cov)?;
    let excess: Vec<f64> = mean.iter().map(|m| m - rf).collect();
    let z = mat_vec(&inv, &excess);
    let denom: f64 = z.iter().sum();
    if denom.abs() < 1e-12 {
        return None;
    }
    let weights: Vec<f64> = z.iter().map(|x| x / denom).collect();
    let point = portfolio_stats(&weights, mean, cov, rf);
    Some(Portfolio { weights, point })
}

/// A tiny deterministic xorshift64* PRNG.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn next_f64(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 1.0) / ((1u64 << 53) as f64 + 1.0)
    }
}

fn random_simplex(rng: &mut Rng, k: usize) -> Vec<f64> {
    let raw: Vec<f64> = (0..k).map(|_| -rng.next_f64().ln()).collect();
    let sum: f64 = raw.iter().sum();
    if sum > 0.0 {
        raw.iter().map(|x| x / sum).collect()
    } else {
        vec![1.0 / k as f64; k]
    }
}

pub fn sample_frontier(
    mean: &[f64],
    cov: &[Vec<f64>],
    rf: f64,
    n: usize,
    seed: u64,
) -> Vec<PortfolioPoint> {
    let k = mean.len();
    if k == 0 {
        return Vec::new();
    }
    let mut rng = Rng::new(seed);
    (0..n)
        .map(|_| {
            let w = random_simplex(&mut rng, k);
            portfolio_stats(&w, mean, cov, rf)
        })
        .collect()
}

/// The full optimization result the Optimizer page renders.
#[derive(Debug, Clone, PartialEq)]
pub struct OptimizerResult {
    /// Included tickers, index-aligned with `correlation` and every weight vec.
    pub tickers: Vec<String>,
    /// The Monte Carlo long-only cloud.
    pub cloud: Vec<PortfolioPoint>,
    /// Analytical tangency portfolio.
    pub tangency: Option<Portfolio>,
    /// Analytical minimum-variance portfolio.
    pub min_variance: Option<Portfolio>,
    /// The current allocation (weights from live market values).
    pub current: Portfolio,
    /// Correlation matrix over the included tickers.
    pub correlation: Vec<Vec<f64>>,
    /// Holdings included after dropping thin/misaligned series.
    pub included: usize,
    /// Holdings supplied (for the "N of M included" note).
    pub total: usize,
}

pub fn compute_optimization(
    histories: &[(String, Vec<(NaiveDate, f64)>)],
    market_values: &HashMap<String, f64>,
    rf: f64,
    samples: usize,
    seed: u64,
) -> Option<OptimizerResult> {
    let total = histories.len();
    let (tickers, returns) = aligned_returns(histories);
    if tickers.len() < 2 {
        return None;
    }
    let included = tickers.len();
    let mean = mean_returns(&returns);
    let cov = covariance_matrix(&returns);
    let correlation = correlation_matrix(&cov);
    let cloud = sample_frontier(&mean, &cov, rf, samples, seed);
    let tangency = tangency(&mean, &cov, rf);
    let min_variance = min_variance(&cov, &mean, rf);

    let mvs: Vec<f64> = tickers.iter().map(|t| *market_values.get(t).unwrap_or(&0.0)).collect();
    let total_mv: f64 = mvs.iter().sum();
    let current_weights: Vec<f64> = if total_mv > 0.0 {
        mvs.iter().map(|v| v / total_mv).collect()
    } else {
        vec![1.0 / included as f64; included]
    };
    let current = Portfolio {
        point: portfolio_stats(&current_weights, &mean, &cov, rf),
        weights: current_weights,
    };

    Some(OptimizerResult {
        tickers,
        cloud,
        tangency,
        min_variance,
        current,
        correlation,
        included,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "expected {b}, got {a}");
    }

    #[test]
    fn random_simplex_weights_are_nonnegative_and_sum_to_one() {
        let mut rng = Rng::new(42);
        for _ in 0..100 {
            let w = random_simplex(&mut rng, 4);
            assert_eq!(w.len(), 4);
            assert!(w.iter().all(|&x| x >= 0.0));
            approx(w.iter().sum::<f64>(), 1.0);
        }
    }

    #[test]
    fn sample_frontier_is_deterministic_for_a_seed() {
        let cov = vec![vec![0.04, 0.0], vec![0.0, 0.09]];
        let a = sample_frontier(&[0.10, 0.15], &cov, 0.0, 500, 7);
        let b = sample_frontier(&[0.10, 0.15], &cov, 0.0, 500, 7);
        assert_eq!(a.len(), 500);
        assert_eq!(a, b);
    }

    #[test]
    fn sample_frontier_points_are_finite() {
        let cov = vec![vec![0.04, 0.01], vec![0.01, 0.09]];
        let cloud = sample_frontier(&[0.10, 0.15], &cov, 0.0, 200, 1);
        assert!(cloud.iter().all(|p| p.risk.is_finite() && p.ret.is_finite() && p.sharpe.is_finite()));
    }

    fn d(n: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap() + chrono::Days::new(n as u64)
    }

    #[test]
    fn compute_optimization_two_holdings_produces_a_result() {
        let a: Vec<(NaiveDate, f64)> = (0..40).map(|n| (d(n), 100.0 + n as f64)).collect();
        let b: Vec<(NaiveDate, f64)> = (0..40).map(|n| (d(n), 50.0 + (n as f64) * 0.5)).collect();
        let mut mv = std::collections::HashMap::new();
        mv.insert("A".to_string(), 6000.0);
        mv.insert("B".to_string(), 4000.0);
        let res = compute_optimization(
            &[("A".into(), a), ("B".into(), b)],
            &mv,
            0.0,
            500,
            7,
        )
        .expect("two priced holdings optimize");
        assert_eq!(res.tickers, vec!["A".to_string(), "B".to_string()]);
        assert_eq!(res.included, 2);
        assert_eq!(res.total, 2);
        assert_eq!(res.cloud.len(), 500);
        assert_eq!(res.correlation.len(), 2);
        approx(res.current.weights[0], 0.6);
        approx(res.current.weights[1], 0.4);
    }

    #[test]
    fn compute_optimization_fewer_than_two_usable_is_none() {
        let a: Vec<(NaiveDate, f64)> = (0..40).map(|n| (d(n), 100.0 + n as f64)).collect();
        let mv = std::collections::HashMap::new();
        assert!(compute_optimization(&[("A".into(), a)], &mv, 0.0, 100, 7).is_none());
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

    #[test]
    fn invert_2x2_diagonal() {
        let inv = invert(&[vec![4.0, 0.0], vec![0.0, 9.0]]).unwrap();
        approx(inv[0][0], 0.25);
        approx(inv[1][1], 1.0 / 9.0);
        approx(inv[0][1], 0.0);
    }

    #[test]
    fn invert_singular_matrix_is_none() {
        assert!(invert(&[vec![1.0, 1.0], vec![1.0, 1.0]]).is_none());
    }

    #[test]
    fn portfolio_stats_known_two_asset_case() {
        let cov = vec![vec![0.04, 0.0], vec![0.0, 0.09]];
        let p = portfolio_stats(&[0.6, 0.4], &[0.10, 0.15], &cov, 0.0);
        approx(p.ret, 0.12);
        approx(p.risk, 0.16970562748477142);
        approx(p.sharpe, 0.7071067811865475);
    }

    #[test]
    fn portfolio_stats_zero_risk_gives_zero_sharpe_not_nan() {
        let cov = vec![vec![0.0, 0.0], vec![0.0, 0.0]];
        let p = portfolio_stats(&[0.5, 0.5], &[0.10, 0.10], &cov, 0.02);
        assert_eq!(p.risk, 0.0);
        assert_eq!(p.sharpe, 0.0);
    }

    #[test]
    fn min_variance_weights_are_inverse_variance_normalized() {
        let cov = vec![vec![0.04, 0.0], vec![0.0, 0.09]];
        let mv = min_variance(&cov, &[0.10, 0.15], 0.0).unwrap();
        approx(mv.weights[0], 0.6923076923076923);
        approx(mv.weights[1], 0.3076923076923077);
    }

    #[test]
    fn tangency_weights_known_two_asset_case() {
        let cov = vec![vec![0.04, 0.0], vec![0.0, 0.09]];
        let t = tangency(&[0.10, 0.15], &cov, 0.0).unwrap();
        approx(t.weights[0], 0.6);
        approx(t.weights[1], 0.4);
    }

    #[test]
    fn min_variance_and_tangency_none_on_singular_covariance() {
        let cov = vec![vec![1.0, 1.0], vec![1.0, 1.0]];
        assert!(min_variance(&cov, &[0.1, 0.1], 0.0).is_none());
        assert!(tangency(&[0.1, 0.1], &cov, 0.0).is_none());
    }
}

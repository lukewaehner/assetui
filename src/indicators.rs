//! Pure technical-indicator math over close-price series.

/// Fast/slow simple-moving-average periods and the RSI period the chart uses.
pub const SMA_FAST: usize = 20;
pub const SMA_SLOW: usize = 50;
pub const RSI_PERIOD: usize = 14;

/// Simple moving average of `values` over `period`, output aligned 1:1 with the input.
pub fn sma(values: &[f64], period: usize) -> Vec<Option<f64>> {
    if period == 0 {
        return vec![None; values.len()];
    }
    let mut out = Vec::with_capacity(values.len());
    let mut running = 0.0;
    for i in 0..values.len() {
        running += values[i];
        if i >= period {
            running -= values[i - period];
        }
        out.push(if i + 1 >= period {
            Some(running / period as f64)
        } else {
            None
        });
    }
    out
}

/// Wilder's Relative Strength Index of `values` over `period`, aligned 1:1 with the input.
pub fn rsi(values: &[f64], period: usize) -> Vec<Option<f64>> {
    let n = values.len();
    let mut out = vec![None; n];
    if period == 0 || n <= period {
        return out;
    }

    let mut gain_sum = 0.0;
    let mut loss_sum = 0.0;
    for i in 1..=period {
        let delta = values[i] - values[i - 1];
        if delta >= 0.0 {
            gain_sum += delta;
        } else {
            loss_sum -= delta;
        }
    }
    let mut avg_gain = gain_sum / period as f64;
    let mut avg_loss = loss_sum / period as f64;
    out[period] = Some(rsi_from(avg_gain, avg_loss));

    for i in (period + 1)..n {
        let delta = values[i] - values[i - 1];
        let (gain, loss) = if delta >= 0.0 { (delta, 0.0) } else { (0.0, -delta) };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
        out[i] = Some(rsi_from(avg_gain, avg_loss));
    }
    out
}

fn rsi_from(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss == 0.0 {
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - 100.0 / (1.0 + rs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sma_is_none_until_period_samples_then_averages() {
        let out = sma(&[1.0, 2.0, 3.0, 4.0, 5.0], 3);
        assert_eq!(out, vec![None, None, Some(2.0), Some(3.0), Some(4.0)]);
    }

    #[test]
    fn sma_period_larger_than_input_is_all_none() {
        assert_eq!(sma(&[1.0, 2.0], 5), vec![None, None]);
    }

    #[test]
    fn sma_period_zero_is_all_none() {
        assert_eq!(sma(&[1.0, 2.0, 3.0], 0), vec![None, None, None]);
    }

    #[test]
    fn sma_empty_input_is_empty() {
        assert!(sma(&[], 3).is_empty());
    }

    #[test]
    fn rsi_is_none_until_period_plus_one_samples() {
        let closes: Vec<f64> = (1..=14).map(|n| n as f64).collect();
        let out = rsi(&closes, 14);
        assert_eq!(out.len(), 14);
        assert!(out.iter().all(|v| v.is_none()));
    }

    #[test]
    fn rsi_all_gains_is_one_hundred() {
        let closes: Vec<f64> = (1..=20).map(|n| n as f64).collect();
        let out = rsi(&closes, 14);
        assert_eq!(out[19], Some(100.0));
    }

    #[test]
    fn rsi_known_series_matches_wilder() {
        let closes = vec![
            44.34, 44.09, 44.15, 43.61, 44.33, 44.83, 45.10, 45.42, 45.84, 46.08,
            45.89, 46.03, 45.61, 46.28, 46.28,
        ];
        let out = rsi(&closes, 14);
        let v = out[14].expect("defined at index 14");
        assert!((v - 70.46).abs() < 0.01, "expected ~70.46, got {v}");
    }

    #[test]
    fn rsi_period_zero_or_empty_is_all_none_or_empty() {
        assert_eq!(rsi(&[1.0, 2.0, 3.0], 0), vec![None, None, None]);
        assert!(rsi(&[], 14).is_empty());
    }
}

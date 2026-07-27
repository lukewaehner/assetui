//! Portfolio holdings and the pure P&L compute core.

#[derive(Debug, Clone, PartialEq)]
pub struct Holding {
    /// Ticker symbol (stored uppercased at the db boundary).
    pub ticker: String,
    /// Number of shares held (average-cost aggregate).
    pub shares: f64,
    /// Average cost basis per share.
    pub avg_cost: f64,
}

/// A [`Holding`] enriched with valuation against a current price.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub holding: Holding,
    /// Current price used for valuation.
    pub price: Option<f64>,
    /// `shares * price`.
    pub market_value: Option<f64>,
    /// `shares * (price - avg_cost)`.
    pub unrealized_pnl: Option<f64>,
    /// `(price - avg_cost) / avg_cost * 100`.
    pub unrealized_pnl_pct: Option<f64>,
    /// `shares * (price - previous_close)`.
    pub day_change: Option<f64>,
}

/// Portfolio-wide totals.
#[derive(Debug, Clone, PartialEq)]
pub struct PortfolioSummary {
    /// Sum of `shares * avg_cost` across all positions (price-independent).
    pub total_cost: f64,
    /// Sum of each position's market value (positions without a price omitted).
    pub total_market_value: f64,
    /// Sum of each position's unrealized P&L.
    pub total_pnl: f64,
    pub total_pnl_pct: Option<f64>,
    /// Sum of each position's day-change.
    pub total_day_change: f64,
}

/// Values a holding against a `price` and `previous_close`, returning a [`Position`].
pub fn value_position(
    holding: Holding,
    price: Option<f64>,
    previous_close: Option<f64>,
) -> Position {
    let market_value = price.map(|p| holding.shares * p);
    let unrealized_pnl = price.map(|p| holding.shares * (p - holding.avg_cost));
    let unrealized_pnl_pct = match price {
        Some(p) if holding.avg_cost != 0.0 => Some((p - holding.avg_cost) / holding.avg_cost * 100.0),
        _ => None,
    };
    let day_change = match (price, previous_close) {
        (Some(p), Some(pc)) => Some(holding.shares * (p - pc)),
        _ => None,
    };
    Position {
        holding,
        price,
        market_value,
        unrealized_pnl,
        unrealized_pnl_pct,
        day_change,
    }
}

/// Sums a set of valued positions into a [`PortfolioSummary`].
pub fn summarize(positions: &[Position]) -> PortfolioSummary {
    let total_cost: f64 = positions
        .iter()
        .map(|p| p.holding.shares * p.holding.avg_cost)
        .sum();
    let total_market_value: f64 = positions.iter().filter_map(|p| p.market_value).sum();
    let total_pnl: f64 = positions.iter().filter_map(|p| p.unrealized_pnl).sum();
    let total_day_change: f64 = positions.iter().filter_map(|p| p.day_change).sum();
    let total_pnl_pct = if total_cost != 0.0 {
        Some(total_pnl / total_cost * 100.0)
    } else {
        None
    };
    PortfolioSummary {
        total_cost,
        total_market_value,
        total_pnl,
        total_pnl_pct,
        total_day_change,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(shares: f64, avg_cost: f64) -> Holding {
        Holding {
            ticker: "AAPL".to_string(),
            shares,
            avg_cost,
        }
    }

    #[test]
    fn value_position_long_at_a_gain() {
        // arrange
        let h = holding(10.0, 100.0);

        // act
        let p = value_position(h, Some(150.0), Some(140.0));

        // assert
        assert_eq!(p.market_value, Some(1500.0));
        assert_eq!(p.unrealized_pnl, Some(500.0));
        assert_eq!(p.unrealized_pnl_pct, Some(50.0));
        assert_eq!(p.day_change, Some(100.0));
    }

    #[test]
    fn value_position_long_at_a_loss() {
        let h = holding(10.0, 100.0);

        let p = value_position(h, Some(80.0), Some(90.0));

        assert_eq!(p.market_value, Some(800.0));
        assert_eq!(p.unrealized_pnl, Some(-200.0));
        assert_eq!(p.unrealized_pnl_pct, Some(-20.0));
        assert_eq!(p.day_change, Some(-100.0));
    }

    #[test]
    fn value_position_returns_none_pct_when_avg_cost_is_zero() {
        let h = holding(5.0, 0.0);

        let p = value_position(h, Some(10.0), Some(9.0));

        assert_eq!(p.market_value, Some(50.0));
        assert_eq!(p.unrealized_pnl, Some(50.0));
        assert_eq!(p.unrealized_pnl_pct, None);
        assert_eq!(p.day_change, Some(5.0));
    }

    #[test]
    fn value_position_without_price_is_all_none() {
        let h = holding(10.0, 100.0);

        let p = value_position(h, None, Some(140.0));

        assert_eq!(p.market_value, None);
        assert_eq!(p.unrealized_pnl, None);
        assert_eq!(p.unrealized_pnl_pct, None);
        assert_eq!(p.day_change, None);
    }

    #[test]
    fn value_position_without_previous_close_omits_only_day_change() {
        let h = holding(10.0, 100.0);

        let p = value_position(h, Some(150.0), None);

        assert_eq!(p.market_value, Some(1500.0));
        assert_eq!(p.unrealized_pnl, Some(500.0));
        assert_eq!(p.unrealized_pnl_pct, Some(50.0));
        assert_eq!(p.day_change, None);
    }

    #[test]
    fn summarize_sums_a_mix_of_positions() {
        // arrange
        let gain = value_position(holding(10.0, 100.0), Some(150.0), Some(140.0));
        let loss = value_position(holding(10.0, 100.0), Some(80.0), Some(90.0));

        // act
        let s = summarize(&[gain, loss]);

        // assert
        assert_eq!(s.total_cost, 2000.0);
        assert_eq!(s.total_market_value, 2300.0);
        assert_eq!(s.total_pnl, 300.0);
        assert_eq!(s.total_pnl_pct, Some(15.0));
        assert_eq!(s.total_day_change, 0.0);
    }

    #[test]
    fn summarize_empty_is_zeroed_with_none_pct() {
        let s = summarize(&[]);

        assert_eq!(s.total_cost, 0.0);
        assert_eq!(s.total_market_value, 0.0);
        assert_eq!(s.total_pnl, 0.0);
        assert_eq!(s.total_pnl_pct, None);
        assert_eq!(s.total_day_change, 0.0);
    }
}

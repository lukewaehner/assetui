//! Portfolio holdings and the pure P&L compute core.

use crate::models::QuoteRecord;
use crate::models::ticker::split_ticker_entry;
use chrono::NaiveDate;
use std::collections::BTreeMap;

/// Why a user's holding spec (`shares avg_cost`, e.g. `10 150.5`) could not be parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum HoldingParseError {
    /// Not exactly two whitespace-separated fields.
    WrongFieldCount,
    /// A field wasn't a number.
    InvalidNumber(String),
    /// Share count wasn't finite and strictly positive.
    NonPositiveShares(f64),
    NegativeCost(f64),
}

impl std::fmt::Display for HoldingParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HoldingParseError::WrongFieldCount => {
                write!(f, "enter shares and average cost, e.g. 10 150.50")
            }
            HoldingParseError::InvalidNumber(s) => write!(f, "not a number: {s}"),
            HoldingParseError::NonPositiveShares(v) => write!(f, "shares must be positive (got {v})"),
            HoldingParseError::NegativeCost(v) => write!(f, "cost can't be negative (got {v})"),
        }
    }
}

impl std::error::Error for HoldingParseError {}

/// Parses a `"shares avg_cost"` spec (two whitespace-separated numbers) into validated values.
pub fn parse_holding_input(input: &str) -> Result<(f64, f64), HoldingParseError> {
    let parts: Vec<&str> = input.split_whitespace().collect();
    if parts.len() != 2 {
        return Err(HoldingParseError::WrongFieldCount);
    }
    let shares: f64 = parts[0]
        .parse()
        .map_err(|_| HoldingParseError::InvalidNumber(parts[0].to_string()))?;
    let avg_cost: f64 = parts[1]
        .parse()
        .map_err(|_| HoldingParseError::InvalidNumber(parts[1].to_string()))?;
    if !shares.is_finite() || shares <= 0.0 {
        return Err(HoldingParseError::NonPositiveShares(shares));
    }
    if !avg_cost.is_finite() || avg_cost < 0.0 {
        return Err(HoldingParseError::NegativeCost(avg_cost));
    }
    Ok((shares, avg_cost))
}

pub fn parse_holding_entry(input: &str) -> Result<Holding, HoldingParseError> {
    let (ticker, rest) =
        split_ticker_entry(input).ok_or(HoldingParseError::WrongFieldCount)?;
    let (shares, avg_cost) = parse_holding_input(rest)?;
    Ok(Holding {
        ticker,
        shares,
        avg_cost,
    })
}

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

pub(crate) fn resolve_price(ticker: &str, rows: &[QuoteRecord]) -> (Option<f64>, Option<f64>) {
    rows.iter()
        .find(|r| r.ticker.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(ticker)))
        .map_or((None, None), |r| (r.price, r.previous_close))
}

pub(crate) fn portfolio_positions(holdings: &[Holding], rows: &[QuoteRecord]) -> Vec<Position> {
    holdings
        .iter()
        .map(|h| {
            let (price, previous_close) = resolve_price(&h.ticker, rows);
            value_position(h.clone(), price, previous_close)
        })
        .collect()
}

/// A portfolio value series' return vs a flat cost basis.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesReturn {
    /// Most recent total value (0.0 when the series is empty).
    pub latest_value: f64,
    /// `latest_value - cost_basis`.
    pub total_pnl: f64,
    pub total_pnl_pct: Option<f64>,
}

/// Reconstructs a portfolio value series from each holding's dated closes.
pub fn reconstruct_value_series(
    holdings_closes: &[(f64, Vec<(NaiveDate, f64)>)],
) -> Vec<(NaiveDate, f64)> {
    let mut dates: BTreeMap<NaiveDate, ()> = BTreeMap::new();
    for (_, closes) in holdings_closes {
        for (date, _) in closes {
            dates.insert(*date, ());
        }
    }
    if dates.is_empty() {
        return Vec::new();
    }

    let sorted: Vec<(f64, BTreeMap<NaiveDate, f64>)> = holdings_closes
        .iter()
        .map(|(shares, closes)| (*shares, closes.iter().map(|(d, c)| (*d, *c)).collect()))
        .collect();

    dates
        .keys()
        .map(|date| {
            let total: f64 = sorted
                .iter()
                .filter_map(|(shares, closes)| {
                    closes
                        .range(..=*date)
                        .next_back()
                        .map(|(_, close)| shares * close)
                })
                .sum();
            (*date, total)
        })
        .collect()
}

pub fn persistable_totals(positions: &[Position]) -> Option<(f64, f64)> {
    if positions.is_empty() || positions.iter().any(|p| p.market_value.is_none()) {
        return None;
    }
    let summary = summarize(positions);
    Some((summary.total_market_value, summary.total_cost))
}

/// Start→end return of `series` against a flat `cost_basis`.
pub fn series_return(series: &[(NaiveDate, f64)], cost_basis: f64) -> SeriesReturn {
    let latest_value = series.last().map_or(0.0, |(_, v)| *v);
    let total_pnl = latest_value - cost_basis;
    let total_pnl_pct = if cost_basis != 0.0 {
        Some(total_pnl / cost_basis * 100.0)
    } else {
        None
    };
    SeriesReturn { latest_value, total_pnl, total_pnl_pct }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, day).unwrap()
    }

    #[test]
    fn reconstruct_single_holding_is_shares_times_closes() {
        let closes = vec![(d(1), 10.0), (d(2), 11.0), (d(3), 12.0)];
        let series = reconstruct_value_series(&[(2.0, closes)]);
        assert_eq!(series, vec![(d(1), 20.0), (d(2), 22.0), (d(3), 24.0)]);
    }

    #[test]
    fn reconstruct_sums_two_aligned_holdings() {
        let a = vec![(d(1), 10.0), (d(2), 12.0)];
        let b = vec![(d(1), 5.0), (d(2), 6.0)];
        let series = reconstruct_value_series(&[(1.0, a), (3.0, b)]);
        assert_eq!(series, vec![(d(1), 25.0), (d(2), 30.0)]);
    }

    #[test]
    fn reconstruct_forward_fills_and_delays_a_late_holding() {
        let a = vec![(d(1), 10.0), (d(2), 10.0), (d(3), 10.0)];
        let b = vec![(d(2), 4.0)];
        let series = reconstruct_value_series(&[(1.0, a), (2.0, b)]);
        assert_eq!(series, vec![(d(1), 10.0), (d(2), 18.0), (d(3), 18.0)]);
    }

    #[test]
    fn reconstruct_empty_inputs_are_empty() {
        assert!(reconstruct_value_series(&[]).is_empty());
        assert!(reconstruct_value_series(&[(1.0, Vec::new())]).is_empty());
    }

    #[test]
    fn series_return_computes_pnl_vs_cost_basis() {
        let series = vec![(d(1), 100.0), (d(2), 130.0)];
        let r = series_return(&series, 120.0);
        assert_eq!(r.latest_value, 130.0);
        assert_eq!(r.total_pnl, 10.0);
        assert_eq!(r.total_pnl_pct, Some(8.333333333333332));
    }

    #[test]
    fn series_return_zero_cost_basis_yields_none_pct() {
        assert_eq!(series_return(&[(d(1), 5.0)], 0.0).total_pnl_pct, None);
    }

    #[test]
    fn series_return_empty_series_has_zero_value_and_full_loss_pct() {
        let r = series_return(&[], 100.0);
        assert_eq!(r.latest_value, 0.0);
        assert_eq!(r.total_pnl_pct, Some(-100.0));
    }

    fn holding(shares: f64, avg_cost: f64) -> Holding {
        Holding {
            ticker: "AAPL".to_string(),
            shares,
            avg_cost,
        }
    }

    #[test]
    fn parse_holding_input_valid() {
        assert_eq!(parse_holding_input("10 150.5"), Ok((10.0, 150.5)));
    }

    #[test]
    fn parse_holding_input_allows_zero_cost() {
        assert_eq!(parse_holding_input("5 0"), Ok((5.0, 0.0)));
    }

    #[test]
    fn parse_holding_input_wrong_field_count_is_error() {
        assert_eq!(parse_holding_input("10"), Err(HoldingParseError::WrongFieldCount));
        assert_eq!(parse_holding_input("10 150 3"), Err(HoldingParseError::WrongFieldCount));
    }

    #[test]
    fn parse_holding_input_non_numeric_is_error() {
        assert_eq!(
            parse_holding_input("abc 150"),
            Err(HoldingParseError::InvalidNumber("abc".to_string()))
        );
    }

    #[test]
    fn parse_holding_input_non_positive_shares_is_error() {
        assert_eq!(parse_holding_input("0 150"), Err(HoldingParseError::NonPositiveShares(0.0)));
        assert_eq!(parse_holding_input("-5 150"), Err(HoldingParseError::NonPositiveShares(-5.0)));
    }

    #[test]
    fn parse_holding_input_negative_cost_is_error() {
        assert_eq!(parse_holding_input("10 -3"), Err(HoldingParseError::NegativeCost(-3.0)));
    }

    #[test]
    fn parse_holding_entry_valid() {
        assert_eq!(
            parse_holding_entry("aapl 10 150.5"),
            Ok(Holding { ticker: "AAPL".to_string(), shares: 10.0, avg_cost: 150.5 })
        );
    }

    #[test]
    fn parse_holding_entry_missing_fields_is_error() {
        assert_eq!(parse_holding_entry("AAPL"), Err(HoldingParseError::WrongFieldCount));
        assert_eq!(parse_holding_entry("AAPL 10"), Err(HoldingParseError::WrongFieldCount));
        assert_eq!(parse_holding_entry(""), Err(HoldingParseError::WrongFieldCount));
    }

    #[test]
    fn parse_holding_entry_bad_numbers_propagate() {
        assert_eq!(
            parse_holding_entry("AAPL x 150"),
            Err(HoldingParseError::InvalidNumber("x".to_string()))
        );
        assert_eq!(
            parse_holding_entry("AAPL -1 150"),
            Err(HoldingParseError::NonPositiveShares(-1.0))
        );
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

    fn quote(ticker: &str, price: Option<f64>, prev: Option<f64>) -> QuoteRecord {
        QuoteRecord {
            ticker: Some(ticker.to_string()),
            price,
            previous_close: prev,
            ..Default::default()
        }
    }

    #[test]
    fn resolve_price_finds_row_case_insensitively() {
        let rows = vec![quote("AAPL", Some(150.0), Some(148.0))];
        assert_eq!(resolve_price("aapl", &rows), (Some(150.0), Some(148.0)));
    }

    #[test]
    fn resolve_price_absent_ticker_is_none() {
        let rows = vec![quote("AAPL", Some(150.0), Some(148.0))];
        assert_eq!(resolve_price("TSLA", &rows), (None, None));
    }

    #[test]
    fn persistable_totals_returns_value_and_cost_when_every_holding_is_priced() {
        // arrange
        let positions = vec![
            value_position(Holding { ticker: "AAPL".into(), shares: 10.0, avg_cost: 100.0 }, Some(120.0), None),
            value_position(Holding { ticker: "TSLA".into(), shares: 5.0, avg_cost: 200.0 }, Some(220.0), None),
        ];

        // act
        let totals = persistable_totals(&positions);

        // assert
        assert_eq!(totals, Some((2300.0, 2000.0)));
    }

    #[test]
    fn persistable_totals_is_none_when_any_holding_is_unpriced() {
        let positions = vec![
            value_position(Holding { ticker: "AAPL".into(), shares: 10.0, avg_cost: 100.0 }, Some(120.0), None),
            value_position(Holding { ticker: "TSLA".into(), shares: 5.0, avg_cost: 200.0 }, None, None),
        ];

        assert_eq!(persistable_totals(&positions), None);
    }

    #[test]
    fn persistable_totals_is_none_for_an_empty_portfolio() {
        assert_eq!(persistable_totals(&[]), None);
    }
}

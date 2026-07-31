use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use yfinance_rs::{PriceTarget, QuoteUpdate, RecommendationSummary};

use crate::black_scholes::{
    BlackScholesInputs, Greeks, OptionKind, greeks as bs_greeks, implied_volatility,
    price as bs_price,
};

/// A single stock quote snapshot, shared across the fetch layer, database operations, and CSV exports.
#[derive(sqlx::FromRow, serde::Serialize, Clone, Default, Debug)]
pub struct QuoteRecord {
    /// Database-assigned row ID.
    pub id: Option<i32>,
    /// Ticker symbol in uppercase (e.g. `"AAPL"`).
    pub ticker: Option<String>,
    /// Company display name returned by Yahoo Finance (e.g. `"Apple Inc."`).
    pub name: Option<String>,
    /// Most recent trade price.
    pub price: Option<f64>,
    /// Closing price from the prior trading session.
    pub previous_close: Option<f64>,
    /// Number of shares traded during the current session.
    pub day_volume: Option<f64>,
    /// Timestamp at which Yahoo Finance reported the quote.
    pub as_of: Option<DateTime<Utc>>,
}

/// Analyst-consensus data for a single ticker, fetched separately from the real-time quote.
#[derive(Default, Debug)]
pub struct QuoteRecordAnalysis {
    /// Ticker this analysis belongs to.
    pub ticker: Option<String>,
    /// Aggregated analyst recommendation (buy / hold / sell breakdown).
    pub recommendation_summary: Option<RecommendationSummary>,
    /// Analyst price-target consensus (mean, low, high).
    pub price_target: Option<PriceTarget>,
    pub coverage_not_applicable: bool,
}

/// Slow-moving key statistics for a single ticker, fetched separately from the real-time quote.
#[derive(Default, Debug)]
pub struct QuoteRecordStatistics {
    /// Ticker these statistics belong to.
    pub ticker: Option<String>,
    /// Market capitalisation (price × shares outstanding).
    pub market_cap: Option<f64>,
    /// Price-to-earnings ratio against trailing-twelve-month EPS.
    pub pe_ratio: Option<f64>,
    /// Earnings per share over the trailing twelve months.
    pub eps: Option<f64>,
    pub dividend_yield: Option<f64>,
    pub fifty_two_week_high: Option<f64>,
    pub fifty_two_week_low: Option<f64>,
    /// Market beta (period/frequency vary by provider).
    pub beta: Option<f64>,
}

#[derive(Default, Debug, Clone)]
pub struct NewsItem {
    /// The headline text.
    pub title: String,
    /// Publisher name (e.g. `"Reuters"`).
    pub publisher: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    /// Direct link to the article.
    pub link: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub struct StockNews {
    /// Ticker these headlines belong to.
    pub ticker: Option<String>,
    /// Recent articles, most-recent first (as returned by the provider).
    pub items: Vec<NewsItem>,
}

/// Which side of the threshold a price alert fires on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertDirection {
    /// Fire when the price rises to or above the threshold.
    Above,
    /// Fire when the price falls to or below the threshold.
    Below,
}

impl AlertDirection {
    pub fn label(&self) -> &'static str {
        match self {
            AlertDirection::Above => "above",
            AlertDirection::Below => "below",
        }
    }
}

/// Why a user's alert spec (`>150` / `<100`) could not be parsed.
#[derive(Debug, Clone, PartialEq)]
pub enum AlertParseError {
    /// No leading `>` or `<`, so the direction is ambiguous.
    MissingOperator,
    /// The text after the operator wasn't a number.
    InvalidNumber(String),
    /// The threshold parsed but wasn't a finite, strictly-positive price.
    NonPositive(f64),
}

impl std::fmt::Display for AlertParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AlertParseError::MissingOperator => {
                write!(f, "start with > (above) or < (below), e.g. >150")
            }
            AlertParseError::InvalidNumber(s) => write!(f, "not a number: {s}"),
            AlertParseError::NonPositive(v) => write!(f, "threshold must be positive (got {v})"),
        }
    }
}

impl std::error::Error for AlertParseError {}

/// Parses a compact alert spec into a direction and threshold.
pub fn parse_alert_input(input: &str) -> Result<(AlertDirection, f64), AlertParseError> {
    let s = input.trim();
    let (direction, rest) = if let Some(rest) = s.strip_prefix('>') {
        (AlertDirection::Above, rest)
    } else if let Some(rest) = s.strip_prefix('<') {
        (AlertDirection::Below, rest)
    } else {
        return Err(AlertParseError::MissingOperator);
    };

    let rest = rest.trim();
    let value: f64 = rest
        .parse()
        .map_err(|_| AlertParseError::InvalidNumber(rest.to_string()))?;
    if !value.is_finite() || value <= 0.0 {
        return Err(AlertParseError::NonPositive(value));
    }
    Ok((direction, value))
}

pub fn parse_alert_entry(
    input: &str,
) -> Result<(String, AlertDirection, f64), AlertParseError> {
    let (ticker, rest) = input
        .trim()
        .split_once(char::is_whitespace)
        .ok_or(AlertParseError::MissingOperator)?;
    let (direction, threshold) = parse_alert_input(rest)?;
    Ok((ticker.to_uppercase(), direction, threshold))
}

pub fn crosses(direction: AlertDirection, threshold: f64, price: f64) -> bool {
    match direction {
        AlertDirection::Above => price >= threshold,
        AlertDirection::Below => price <= threshold,
    }
}

/// A persisted price alert: fire once when `ticker` crosses `threshold` in `direction`.
#[derive(Debug, Clone)]
pub struct Alert {
    /// Database-assigned row id.
    pub id: i32,
    /// Ticker the alert watches (stored uppercased).
    pub ticker: String,
    /// Which side of `threshold` triggers the alert.
    pub direction: AlertDirection,
    /// Absolute price threshold.
    pub threshold: f64,
    pub created_at: DateTime<Utc>,
    /// When the alert fired.
    pub triggered_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct OptionContract {
    /// Call or put.
    pub kind: OptionKind,
    pub expiry: DateTime<Utc>,
    pub strike: f64,
    /// Best bid premium.
    pub bid: Option<f64>,
    /// Best ask premium.
    pub ask: Option<f64>,
    /// Last traded premium.
    pub last: Option<f64>,
    /// Provider's implied volatility as a fraction (`0.25` = 25%).
    pub implied_volatility: Option<f64>,
    pub volume: Option<u64>,
    pub open_interest: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct OptionChain {
    /// The underlying ticker symbol.
    pub underlying: String,
    pub underlying_price: Option<f64>,
    /// All expirations offered for this underlying, most-recent-first as the provider returns them.
    pub expirations: Vec<DateTime<Utc>>,
    /// Contracts (calls and puts) for the currently-fetched expiry.
    pub contracts: Vec<OptionContract>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContractAnalytics {
    /// Theoretical (fair) premium from Black-Scholes-Merton.
    pub theo_price: f64,
    pub iv: f64,
    /// First-order Greeks in the core's conventions (theta per year, vega/rho per 1.00 vol/rate move).
    pub greeks: Greeks,
}

fn year_fraction(expiry: DateTime<Utc>, now: DateTime<Utc>) -> f64 {
    (expiry - now).num_seconds() as f64 / (365.0 * 86_400.0)
}

fn contract_market_price(contract: &OptionContract) -> Option<f64> {
    match (contract.bid, contract.ask) {
        (Some(bid), Some(ask)) => Some((bid + ask) / 2.0),
        _ => contract.last,
    }
}

impl OptionContract {
    /// Computes this contract's theoretical price and Greeks as of `now`.
    pub fn analytics(
        &self,
        spot: Option<f64>,
        risk_free_rate: f64,
        dividend_yield: f64,
        now: DateTime<Utc>,
    ) -> Option<ContractAnalytics> {
        let spot = spot?;
        let t = year_fraction(self.expiry, now);

        let iv = match self.implied_volatility {
            Some(iv) if iv > 0.0 => iv,
            _ => {
                let market = contract_market_price(self)?;
                let seed =
                    BlackScholesInputs::new(spot, self.strike, t, risk_free_rate, 0.5, dividend_yield)
                        .ok()?;
                implied_volatility(market, &seed, self.kind).ok()?
            }
        };

        let inputs =
            BlackScholesInputs::new(spot, self.strike, t, risk_free_rate, iv, dividend_yield).ok()?;
        Some(ContractAnalytics {
            theo_price: bs_price(&inputs, self.kind),
            iv,
            greeks: bs_greeks(&inputs, self.kind),
        })
    }
}

impl QuoteRecord {
    pub fn matches_query(&self, query: &str) -> bool {
        let query = query.trim();
        if query.is_empty() {
            return true;
        }
        [self.ticker.as_deref(), self.name.as_deref()]
            .into_iter()
            .flatten()
            .any(|field| crate::search::subseq_match_ci(field, query).is_some())
    }
}

pub fn day_change_pct(price: Option<f64>, previous_close: Option<f64>) -> Option<f64> {
    let (price, previous_close) = (price?, previous_close?);
    if previous_close == 0.0 {
        return None;
    }
    Some((price - previous_close) / previous_close * 100.0)
}

/// How long a price-tick flash stays coloured before reverting to the default day-change styling.
pub const FLASH_TTL: Duration = Duration::from_millis(500);

pub struct QuoteTick {
    pub ticker: Option<String>,
    pub price: Option<f64>,
    pub previous_close: Option<f64>,
    pub day_volume: Option<f64>,
    pub as_of: Option<DateTime<Utc>>,
}

/// Treats a non-positive volume as absent.
fn positive_volume(volume: Option<f64>) -> Option<f64> {
    volume.filter(|v| *v > 0.0)
}

impl From<QuoteUpdate> for QuoteTick {
    fn from(update: QuoteUpdate) -> Self {
        Self {
            ticker: Some(update.instrument.symbol.to_string()),
            price: update.price.map(|p| p.as_decimal().round_dp(2).as_f64()),
            previous_close: update
                .previous_close
                .map(|p| p.as_decimal().round_dp(2).as_f64()),
            day_volume: positive_volume(
                update.volume.map(|v| v.into_inner().as_decimal().as_f64()),
            ),
            as_of: Some(update.ts),
        }
    }
}

impl QuoteTick {
    pub fn apply(
        &self,
        rows: &mut [QuoteRecord],
        flash_map: &mut HashMap<String, (f64, Instant)>,
    ) -> Option<usize> {
        let ticker = self.ticker.as_deref()?;
        let i = rows.iter().position(|r| {
            r.ticker
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(ticker))
        })?;
        self.record_flash(flash_map, &rows[i]);
        let row = &mut rows[i];
        row.price = self.price.or(row.price);
        row.previous_close = self.previous_close.or(row.previous_close);
        row.day_volume = self.day_volume.or(row.day_volume);
        row.as_of = self.as_of.or(row.as_of);
        Some(i)
    }

    pub fn record_flash(&self, map: &mut HashMap<String, (f64, Instant)>, record: &QuoteRecord) {
        map.retain(|_, (_, ts)| ts.elapsed() < FLASH_TTL);

        let (Some(new), Some(old)) = (self.price, record.price) else {
            return;
        };
        if let Some(ticker) = self.ticker.as_deref() {
            map.insert(ticker.to_ascii_uppercase(), (new - old, Instant::now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::black_scholes::{BlackScholesInputs, DEFAULT_RISK_FREE_RATE, OptionKind, price};
    use chrono::Utc;
    use chrono::Duration as ChronoDuration;

    fn one_year_contract(strike: f64) -> (OptionContract, chrono::DateTime<Utc>) {
        let now = Utc::now();
        let contract = OptionContract {
            kind: OptionKind::Call,
            expiry: now + ChronoDuration::days(365),
            strike,
            bid: None,
            ask: None,
            last: None,
            implied_volatility: None,
            volume: None,
            open_interest: None,
        };
        (contract, now)
    }

    #[test]
    fn parse_alert_input_above() {
        assert_eq!(parse_alert_input(">150"), Ok((AlertDirection::Above, 150.0)));
    }

    #[test]
    fn parse_alert_input_below() {
        assert_eq!(parse_alert_input("<100"), Ok((AlertDirection::Below, 100.0)));
    }

    #[test]
    fn parse_alert_input_tolerates_whitespace() {
        assert_eq!(parse_alert_input("  > 150.5 "), Ok((AlertDirection::Above, 150.5)));
    }

    #[test]
    fn parse_alert_input_missing_operator_is_error() {
        assert_eq!(parse_alert_input("150"), Err(AlertParseError::MissingOperator));
    }

    #[test]
    fn parse_alert_input_empty_is_error() {
        assert_eq!(parse_alert_input("   "), Err(AlertParseError::MissingOperator));
    }

    #[test]
    fn parse_alert_input_non_numeric_is_error() {
        assert_eq!(
            parse_alert_input(">abc"),
            Err(AlertParseError::InvalidNumber("abc".to_string()))
        );
    }

    #[test]
    fn parse_alert_input_negative_is_error() {
        assert_eq!(parse_alert_input(">-5"), Err(AlertParseError::NonPositive(-5.0)));
    }

    #[test]
    fn parse_alert_input_zero_is_error() {
        assert_eq!(parse_alert_input("<0"), Err(AlertParseError::NonPositive(0.0)));
    }

    #[test]
    fn parse_alert_entry_valid() {
        assert_eq!(parse_alert_entry("aapl >150"), Ok(("AAPL".to_string(), AlertDirection::Above, 150.0)));
        assert_eq!(parse_alert_entry("TSLA <100"), Ok(("TSLA".to_string(), AlertDirection::Below, 100.0)));
    }

    #[test]
    fn parse_alert_entry_missing_spec_is_error() {
        assert_eq!(parse_alert_entry("AAPL"), Err(AlertParseError::MissingOperator));
    }

    #[test]
    fn parse_alert_entry_propagates_spec_errors() {
        assert_eq!(parse_alert_entry("AAPL 150"), Err(AlertParseError::MissingOperator));
        assert_eq!(parse_alert_entry("AAPL >-5"), Err(AlertParseError::NonPositive(-5.0)));
    }

    #[test]
    fn crosses_above_fires_at_or_past_threshold() {
        assert!(crosses(AlertDirection::Above, 150.0, 151.0), "just above");
        assert!(crosses(AlertDirection::Above, 150.0, 150.0), "exactly at");
        assert!(!crosses(AlertDirection::Above, 150.0, 149.99), "just below");
    }

    #[test]
    fn crosses_below_fires_at_or_past_threshold() {
        assert!(crosses(AlertDirection::Below, 100.0, 99.0), "just below");
        assert!(crosses(AlertDirection::Below, 100.0, 100.0), "exactly at");
        assert!(!crosses(AlertDirection::Below, 100.0, 100.01), "just above");
    }

    #[test]
    fn alert_direction_label_maps_each_variant() {
        assert_eq!(AlertDirection::Above.label(), "above");
        assert_eq!(AlertDirection::Below.label(), "below");
    }

    #[test]
    fn year_fraction_one_year_out_is_one() {
        let now = Utc::now();
        let expiry = now + ChronoDuration::days(365);
        assert!((year_fraction(expiry, now) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn analytics_uses_provider_iv_when_present() {
        // arrange
        let (mut contract, now) = one_year_contract(100.0);
        contract.implied_volatility = Some(0.25);

        // act
        let a = contract
            .analytics(Some(100.0), DEFAULT_RISK_FREE_RATE, 0.0, now)
            .unwrap();

        // assert
        assert!((a.iv - 0.25).abs() < 1e-12);
        let expected = price(
            &BlackScholesInputs::new(100.0, 100.0, 1.0, DEFAULT_RISK_FREE_RATE, 0.25, 0.0).unwrap(),
            OptionKind::Call,
        );
        assert!((a.theo_price - expected).abs() < 1e-9);
    }

    #[test]
    fn analytics_solves_iv_from_mid_when_provider_iv_absent() {
        // arrange
        let (mut contract, now) = one_year_contract(100.0);
        let true_vol = 0.30;
        let market = price(
            &BlackScholesInputs::new(100.0, 100.0, 1.0, DEFAULT_RISK_FREE_RATE, true_vol, 0.0)
                .unwrap(),
            OptionKind::Call,
        );
        contract.bid = Some(market);
        contract.ask = Some(market);

        // act
        let a = contract
            .analytics(Some(100.0), DEFAULT_RISK_FREE_RATE, 0.0, now)
            .unwrap();

        // assert
        assert!((a.iv - true_vol).abs() < 1e-4, "solved iv {}", a.iv);
    }

    #[test]
    fn analytics_none_without_spot() {
        let (mut contract, now) = one_year_contract(100.0);
        contract.implied_volatility = Some(0.25);
        assert!(contract.analytics(None, DEFAULT_RISK_FREE_RATE, 0.0, now).is_none());
    }

    #[test]
    fn analytics_none_for_expired_contract() {
        let now = Utc::now();
        let contract = OptionContract {
            kind: OptionKind::Call,
            expiry: now - ChronoDuration::days(1),
            strike: 100.0,
            bid: None,
            ask: None,
            last: None,
            implied_volatility: Some(0.25),
            volume: None,
            open_interest: None,
        };
        assert!(contract.analytics(Some(100.0), DEFAULT_RISK_FREE_RATE, 0.0, now).is_none());
    }

    #[test]
    fn analytics_none_when_no_iv_and_no_market_price() {
        let (contract, now) = one_year_contract(100.0);
        assert!(contract.analytics(Some(100.0), DEFAULT_RISK_FREE_RATE, 0.0, now).is_none());
    }

    #[test]
    fn test_quote_record_default_has_all_none() {
        let record = QuoteRecord::default();
        assert!(record.id.is_none());
        assert!(record.ticker.is_none());
        assert!(record.name.is_none());
        assert!(record.price.is_none());
        assert!(record.previous_close.is_none());
        assert!(record.day_volume.is_none());
        assert!(record.as_of.is_none());
    }

    #[test]
    fn test_quote_record_construction() {
        let now = Utc::now();
        let record = QuoteRecord {
            id: Some(42),
            ticker: Some("AAPL".to_string()),
            name: Some("Apple Inc.".to_string()),
            price: Some(189.50),
            previous_close: Some(187.25),
            day_volume: Some(55_000_000.0),
            as_of: Some(now),
        };
        assert_eq!(record.id, Some(42));
        assert_eq!(record.ticker.as_deref(), Some("AAPL"));
        assert_eq!(record.name.as_deref(), Some("Apple Inc."));
        assert_eq!(record.price, Some(189.50));
        assert_eq!(record.previous_close, Some(187.25));
        assert_eq!(record.day_volume, Some(55_000_000.0));
        assert!(record.as_of.is_some());
    }

    #[test]
    fn test_quote_record_analysis_default() {
        let analysis = QuoteRecordAnalysis::default();
        assert!(analysis.ticker.is_none());
        assert!(analysis.recommendation_summary.is_none());
        assert!(analysis.price_target.is_none());
        assert!(!analysis.coverage_not_applicable);
    }

    #[test]
    fn test_quote_record_debug_contains_ticker() {
        let record = QuoteRecord {
            ticker: Some("TSLA".to_string()),
            as_of: Some(Utc::now()),
            ..Default::default()
        };
        let debug_str = format!("{:?}", record);
        assert!(debug_str.contains("TSLA"));
    }

    #[test]
    fn test_apply_updates_matching_row() {
        let mut rows = vec![
            QuoteRecord {
                ticker: Some("AAPL".to_string()),
                price: Some(100.0),
                ..Default::default()
            },
            QuoteRecord {
                ticker: Some("TSLA".to_string()),
                price: Some(200.0),
                ..Default::default()
            },
        ];
        let now = Utc::now();
        let tick = QuoteTick {
            ticker: Some("aapl".to_string()),
            price: Some(150.0),
            previous_close: Some(140.0),
            day_volume: Some(1_000.0),
            as_of: Some(now),
        };

        let mut flash_map = HashMap::new();
        assert_eq!(tick.apply(&mut rows, &mut flash_map), Some(0));
        assert_eq!(rows[0].price, Some(150.0));
        assert_eq!(rows[0].previous_close, Some(140.0));
        assert_eq!(rows[0].day_volume, Some(1_000.0));
        assert_eq!(rows[0].as_of, Some(now));
        assert_eq!(rows[1].price, Some(200.0));
        let (diff, _) = flash_map.get("AAPL").expect("flash recorded by apply");
        assert!((diff - 50.0).abs() < 1e-9);
    }

    #[test]
    fn test_apply_unknown_ticker_is_noop() {
        let mut rows = vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: Some(100.0),
            ..Default::default()
        }];
        let tick = QuoteTick {
            ticker: Some("NVDA".to_string()),
            price: Some(999.0),
            previous_close: None,
            day_volume: None,
            as_of: Some(Utc::now()),
        };

        let mut flash_map = HashMap::new();
        assert_eq!(tick.apply(&mut rows, &mut flash_map), None);
        assert_eq!(rows[0].price, Some(100.0));
        assert!(flash_map.is_empty(), "no flash for an unmatched ticker");
    }

    /// The fuzzy query matches on ticker or company name, case-insensitively and with gaps.
    #[test]
    fn test_matches_query() {
        let record = QuoteRecord {
            ticker: Some("AAPL".to_string()),
            name: Some("Apple Inc.".to_string()),
            ..Default::default()
        };
        assert!(record.matches_query("aapl"), "ticker, case-insensitive");
        assert!(record.matches_query("apl"), "ticker subsequence");
        assert!(record.matches_query("apple"), "company name");
        assert!(record.matches_query("apn"), "name subsequence with gaps");
        assert!(record.matches_query(""), "blank matches everything");
        assert!(record.matches_query("   "), "whitespace-only is blank");
        assert!(!record.matches_query("tsla"), "no match anywhere");
    }

    #[test]
    fn test_matches_query_empty_record() {
        let record = QuoteRecord::default();
        assert!(record.matches_query(""));
        assert!(!record.matches_query("a"));
    }

    #[test]
    fn test_day_change_pct_positive() {
        assert_eq!(day_change_pct(Some(110.0), Some(100.0)), Some(10.0));
    }

    #[test]
    fn test_day_change_pct_negative() {
        assert_eq!(day_change_pct(Some(90.0), Some(100.0)), Some(-10.0));
    }

    #[test]
    fn test_day_change_pct_unchanged_is_zero() {
        assert_eq!(day_change_pct(Some(100.0), Some(100.0)), Some(0.0));
    }

    #[test]
    fn test_day_change_pct_zero_previous_close_is_none() {
        assert_eq!(day_change_pct(Some(100.0), Some(0.0)), None);
    }

    #[test]
    fn test_day_change_pct_missing_price_is_none() {
        assert_eq!(day_change_pct(None, Some(100.0)), None);
    }

    #[test]
    fn test_day_change_pct_missing_previous_close_is_none() {
        assert_eq!(day_change_pct(Some(100.0), None), None);
    }

    #[test]
    fn test_positive_volume_filters_non_positive() {
        assert_eq!(positive_volume(Some(0.0)), None);
        assert_eq!(positive_volume(Some(-1.0)), None);
        assert_eq!(positive_volume(None), None);
        assert_eq!(positive_volume(Some(63_825_743.0)), Some(63_825_743.0));
    }

    #[test]
    fn test_apply_diff_only_preserves_existing_fields() {
        let mut rows = vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: Some(100.0),
            previous_close: Some(99.0),
            day_volume: Some(5_000.0),
            ..Default::default()
        }];
        let tick = QuoteTick {
            ticker: Some("AAPL".to_string()),
            price: Some(101.0),
            previous_close: None,
            day_volume: None,
            as_of: Some(Utc::now()),
        };

        assert_eq!(tick.apply(&mut rows, &mut HashMap::new()), Some(0));
        assert_eq!(rows[0].price, Some(101.0));
        assert_eq!(rows[0].previous_close, Some(99.0));
        assert_eq!(rows[0].day_volume, Some(5_000.0));
    }

    #[test]
    fn test_record_flash_records_uppercase_keyed_delta() {
        let mut map = HashMap::new();
        let record = QuoteRecord {
            ticker: Some("aapl".to_string()),
            price: Some(100.0),
            ..Default::default()
        };
        let tick = QuoteTick {
            ticker: Some("aapl".to_string()),
            price: Some(102.5),
            previous_close: None,
            day_volume: None,
            as_of: None,
        };
        tick.record_flash(&mut map, &record);
        let (diff, _) = map.get("AAPL").expect("flash keyed by uppercase ticker");
        assert!((diff - 2.5).abs() < 1e-9);
    }

    #[test]
    fn test_record_flash_noop_without_prices() {
        let mut map = HashMap::new();
        let record = QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: None,
            ..Default::default()
        };
        let tick = QuoteTick {
            ticker: Some("AAPL".to_string()),
            price: Some(100.0),
            previous_close: None,
            day_volume: None,
            as_of: None,
        };
        tick.record_flash(&mut map, &record);
        assert!(map.is_empty());
    }

    #[test]
    fn test_record_flash_evicts_expired() {
        let mut map = HashMap::new();
        let expired = Instant::now()
            .checked_sub(FLASH_TTL + Duration::from_millis(100))
            .expect("test clock underflow");
        map.insert("OLD".to_string(), (1.0, expired));

        let record = QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: Some(100.0),
            ..Default::default()
        };
        let tick = QuoteTick {
            ticker: Some("AAPL".to_string()),
            price: Some(101.0),
            previous_close: None,
            day_volume: None,
            as_of: None,
        };
        tick.record_flash(&mut map, &record);

        assert!(!map.contains_key("OLD"), "expired entry should be evicted");
        assert!(map.contains_key("AAPL"));
    }
}

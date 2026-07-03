use chrono::{DateTime, Utc};
use yfinance_rs::{PriceTarget, QuoteUpdate, RecommendationSummary};

/// A single stock quote snapshot, shared across the fetch layer, database
/// operations, and CSV exports.
///
/// All fields are `Option` because the Yahoo Finance payload is partially
/// populated depending on market hours and symbol availability.  The `id`
/// field is `None` until a row is inserted and the database assigns one.
///
/// Derives `sqlx::FromRow` for automatic mapping from Postgres result rows,
/// and `serde::Serialize` so rows can be written directly to CSV.
#[derive(sqlx::FromRow, serde::Serialize, Clone, Default, Debug)]
pub struct QuoteRecord {
    /// Database-assigned row ID; `None` before the record has been stored.
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

/// Analyst-consensus data for a single ticker, fetched separately from the
/// real-time quote.
///
/// Both inner fields are `Option` because the Yahoo Finance API may not have
/// coverage for every symbol.
#[derive(Default, Debug)]
pub struct QuoteRecordAnalysis {
    /// Ticker this analysis belongs to.
    pub ticker: Option<String>,
    /// Aggregated analyst recommendation (buy / hold / sell breakdown).
    pub recommendation_summary: Option<RecommendationSummary>,
    /// Analyst price-target consensus (mean, low, high).
    pub price_target: Option<PriceTarget>,
}

pub struct QuoteTick {
    pub ticker: Option<String>,
    pub price: Option<f64>,
    pub previous_close: Option<f64>,
    pub day_volume: Option<f64>,
    pub as_of: Option<DateTime<Utc>>,
}

impl From<QuoteUpdate> for QuoteTick {
    fn from(update: QuoteUpdate) -> Self {
        Self {
            ticker: Some(update.instrument.symbol.to_string()),
            price: update.price.map(|p| p.as_decimal().round_dp(2).as_f64()),
            previous_close: update
                .previous_close
                .map(|p| p.as_decimal().round_dp(2).as_f64()),
            day_volume: update.volume.map(|v| v.into_inner().as_decimal().as_f64()),
            as_of: Some(update.ts),
        }
    }
}

impl QuoteTick {
    pub fn apply(&self, rows: &mut [QuoteRecord]) -> Option<usize> {
        let ticker = self.ticker.as_deref()?;
        let i = rows.iter().position(|r| {
            r.ticker
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(ticker))
        })?;
        let row = &mut rows[i];
        row.price = self.price.or(row.price);
        row.previous_close = self.previous_close.or(row.previous_close);
        row.day_volume = self.day_volume.or(row.day_volume);
        row.as_of = self.as_of.or(row.as_of);
        Some(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

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

    /// A tick updates the matching row in place, matches the ticker
    /// case-insensitively, and leaves every other row untouched.
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
        // Lowercase ticker exercises the case-insensitive match.
        let tick = QuoteTick {
            ticker: Some("aapl".to_string()),
            price: Some(150.0),
            previous_close: Some(140.0),
            day_volume: Some(1_000.0),
            as_of: Some(now),
        };

        assert_eq!(tick.apply(&mut rows), Some(0));
        assert_eq!(rows[0].price, Some(150.0));
        assert_eq!(rows[0].previous_close, Some(140.0));
        assert_eq!(rows[0].day_volume, Some(1_000.0));
        assert_eq!(rows[0].as_of, Some(now));
        // The non-matching row is left alone.
        assert_eq!(rows[1].price, Some(200.0));
    }

    /// A tick for a ticker not present in `rows` returns `None` and mutates
    /// nothing (display-only no-op, no insert, no panic).
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

        assert_eq!(tick.apply(&mut rows), None);
        assert_eq!(rows[0].price, Some(100.0));
    }

    /// A diff-only tick carries only the fields that changed; the rest arrive
    /// as `None` and must preserve the row's existing values rather than blank
    /// them out (regression test for the earlier `.unwrap()` panic).
    #[test]
    fn test_apply_diff_only_preserves_existing_fields() {
        let mut rows = vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: Some(100.0),
            previous_close: Some(99.0),
            day_volume: Some(5_000.0),
            ..Default::default()
        }];
        // Only the price moved; every other field is absent.
        let tick = QuoteTick {
            ticker: Some("AAPL".to_string()),
            price: Some(101.0),
            previous_close: None,
            day_volume: None,
            as_of: Some(Utc::now()),
        };

        assert_eq!(tick.apply(&mut rows), Some(0));
        assert_eq!(rows[0].price, Some(101.0)); // updated
        assert_eq!(rows[0].previous_close, Some(99.0)); // preserved
        assert_eq!(rows[0].day_volume, Some(5_000.0)); // preserved
    }
}

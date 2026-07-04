use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

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

impl QuoteRecord {
    /// Returns `true` when `query` fuzzy-matches this record's ticker or
    /// company name (case-insensitive subsequence, see
    /// [`crate::search::subseq_match_ci`]).  A blank query matches everything
    /// so an empty filter shows the full table.
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

/// How long a price-tick flash stays coloured before reverting to the default
/// day-change styling. Shared by [`QuoteTick::record_flash`] (eviction) and the
/// TUI's `price_change_color` (render check) so the two never drift.
pub const FLASH_TTL: Duration = Duration::from_millis(500);

pub struct QuoteTick {
    pub ticker: Option<String>,
    pub price: Option<f64>,
    pub previous_close: Option<f64>,
    pub day_volume: Option<f64>,
    pub as_of: Option<DateTime<Utc>>,
}

/// Treats a non-positive volume as absent.
///
/// Yahoo's websocket protobuf defaults `day_volume` to `0` when a tick
/// doesn't carry volume (proto3 semantics), and `yfinance-rs` passes that
/// through as `Some(0)`.  Prices get the equivalent guard upstream
/// (`ws_price_from_f32` maps `<= 0` to `None`) but volume does not, so
/// without this filter nearly every price tick would wipe the stored volume
/// of the row it updates.
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
    /// Applies this tick to the matching row in `rows` (matched by ticker,
    /// case-insensitively), recording a price-flash from the row's pre-update
    /// price into `flash_map` before the fields are overwritten.  Fields the
    /// tick doesn't carry (diff-only streams) keep their existing values.
    ///
    /// Returns the index of the updated row, or `None` when no row matches -
    /// a display-only no-op, never a panic.
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

    /// Records a price-flash for this tick's ticker: the signed delta from the
    /// row's pre-update price and the current time. Expired entries are evicted
    /// first so the map stays bounded. A no-op when either price is absent.
    pub fn record_flash(&self, map: &mut HashMap<String, (f64, Instant)>, record: &QuoteRecord) {
        // Drop flashes that have already expired so the map doesn't grow unbounded.
        map.retain(|_, (_, ts)| ts.elapsed() < FLASH_TTL);

        let (Some(new), Some(old)) = (self.price, record.price) else {
            return;
        };
        if let Some(ticker) = self.ticker.as_deref() {
            // Key on the uppercase ticker so the render-side lookup (which uses
            // the stored row's ticker) matches regardless of source casing.
            map.insert(ticker.to_ascii_uppercase(), (new - old, Instant::now()));
        }
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
    /// case-insensitively, records a flash from the pre-update price, and
    /// leaves every other row untouched.
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

        let mut flash_map = HashMap::new();
        assert_eq!(tick.apply(&mut rows, &mut flash_map), Some(0));
        assert_eq!(rows[0].price, Some(150.0));
        assert_eq!(rows[0].previous_close, Some(140.0));
        assert_eq!(rows[0].day_volume, Some(1_000.0));
        assert_eq!(rows[0].as_of, Some(now));
        // The non-matching row is left alone.
        assert_eq!(rows[1].price, Some(200.0));
        // The flash was recorded against the pre-update price (150 - 100).
        let (diff, _) = flash_map.get("AAPL").expect("flash recorded by apply");
        assert!((diff - 50.0).abs() < 1e-9);
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

        let mut flash_map = HashMap::new();
        assert_eq!(tick.apply(&mut rows, &mut flash_map), None);
        assert_eq!(rows[0].price, Some(100.0));
        assert!(flash_map.is_empty(), "no flash for an unmatched ticker");
    }

    /// The fuzzy query matches on ticker or company name, case-insensitively
    /// and with gaps; a blank query matches everything.
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

    /// A record with no ticker and no name only matches a blank query.
    #[test]
    fn test_matches_query_empty_record() {
        let record = QuoteRecord::default();
        assert!(record.matches_query(""));
        assert!(!record.matches_query("a"));
    }

    /// Yahoo's websocket stream sends `day_volume = 0` (the proto3 default)
    /// on ticks that don't carry volume; the conversion must map that to
    /// `None` so `apply` preserves the stored volume instead of zeroing it.
    #[test]
    fn test_positive_volume_filters_non_positive() {
        assert_eq!(positive_volume(Some(0.0)), None);
        assert_eq!(positive_volume(Some(-1.0)), None);
        assert_eq!(positive_volume(None), None);
        assert_eq!(positive_volume(Some(63_825_743.0)), Some(63_825_743.0));
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

        assert_eq!(tick.apply(&mut rows, &mut HashMap::new()), Some(0));
        assert_eq!(rows[0].price, Some(101.0)); // updated
        assert_eq!(rows[0].previous_close, Some(99.0)); // preserved
        assert_eq!(rows[0].day_volume, Some(5_000.0)); // preserved
    }

    /// A flash records the signed delta from the row's old price, keyed by the
    /// uppercase ticker regardless of the source casing.
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

    /// A tick with no price (or against a row with no price) records nothing.
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

    /// Expired entries are evicted when a new flash is recorded, so the map
    /// cannot grow without bound.
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

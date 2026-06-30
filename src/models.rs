use chrono::{DateTime, Utc};
use yfinance_rs::{PriceTarget, RecommendationSummary};

/// A single stock quote snapshot, shared across the fetch layer, database
/// operations, and CSV exports.
///
/// All fields are `Option` because the Yahoo Finance payload is partially
/// populated depending on market hours and symbol availability.  The `id`
/// field is `None` until a row is inserted and the database assigns one.
///
/// Derives `sqlx::FromRow` for automatic mapping from Postgres result rows,
/// and `serde::Serialize` so rows can be written directly to CSV.
#[derive(sqlx::FromRow, serde::Serialize, Default, Debug)]
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

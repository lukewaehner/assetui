use chrono::{DateTime, Utc};
use yfinance_rs::{PriceTarget, RecommendationSummary};

// Shared record type used across fetching, DB writes, and CSV dumps.
// Kept separate so `sqlx::FromRow` and `serde::Serialize` can both derive on it.
#[derive(sqlx::FromRow, serde::Serialize, Default, Debug)]
pub struct QuoteRecord {
    pub id: Option<i32>,
    pub ticker: Option<String>,
    pub name: Option<String>,
    pub price: Option<f64>,
    pub previous_close: Option<f64>,
    pub day_volume: Option<f64>,
    pub as_of: Option<DateTime<Utc>>,
}

#[derive(Default, Debug)]
pub struct QuoteRecordAnalysis {
    pub ticker: Option<String>,
    pub recommendation_summary: Option<RecommendationSummary>,
    pub price_target: Option<PriceTarget>,
}

use chrono::{DateTime, Utc};

// Shared record type used across fetching, DB writes, and CSV dumps.
// Kept separate so `sqlx::FromRow` and `serde::Serialize` can both derive on it.
#[derive(sqlx::FromRow, serde::Serialize)]
pub struct QuoteRecord {
    pub id: Option<i32>,
    pub name: Option<String>,
    pub price: Option<f64>,
    pub previous_close: Option<f64>,
    pub day_volume: Option<f64>,
    pub as_of: Option<DateTime<Utc>>,
}

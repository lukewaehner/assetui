use chrono::{DateTime, Utc};

#[derive(sqlx::FromRow, serde::Serialize, Default, Debug)]
pub struct QuoteRecord {
    pub id: Option<i32>,
    pub ticker: Option<String>,
    pub price: Option<f64>,
    pub previous_close: Option<f64>,
    pub day_volume: Option<f64>,
    pub as_of: Option<DateTime<Utc>>,
}

use tracing::{debug, instrument};
use yfinance_rs::{Ticker, YfClient};

use crate::models::QuoteRecord;

// Core logic: talk to yfinance and translate its payload into our QuoteRecord.
#[instrument(skip(ticker), fields(symbol = %symbol))]
pub async fn fetch_quote(
    symbol: &str,
    ticker: &Ticker,
) -> Result<Option<QuoteRecord>, Box<dyn std::error::Error + Send + Sync>> {
    debug!("requesting quote from yfinance");
    let quote = ticker.quote().await?;
    debug!(
        name = ?quote.name,
        has_price = quote.price.is_some(),
        "received quote payload"
    );
    let quote_record = QuoteRecord {
        id: None, // Set by the database
        name: quote.name.clone(),
        price: quote.price.map(|p| p.into_inner().as_f64()),
        previous_close: quote.previous_close.map(|p| p.into_inner().as_f64()),
        day_volume: quote
            .day_volume
            .as_ref()
            .map(|p| p.clone().into_inner().as_decimal().as_f64()),
        as_of: quote.as_of,
    };
    Ok(Some(quote_record))
}

pub fn prepare_tickers(s: &[String], c: &YfClient) -> Vec<(String, Ticker)> {
    s.iter().map(|t| (t.clone(), Ticker::new(c, t))).collect()
}

pub async fn fetch_recent(pool: &sqlx::PgPool, limit: i64) -> sqlx::Result<Vec<QuoteRecord>> {
    sqlx::query_as::<_, QuoteRecord>(
        "SELECT id, name, price, previous_close, day_volume, as_of
              FROM quotes
              ORDER BY as_of DESC, id DESC
              LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

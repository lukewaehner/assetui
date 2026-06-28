use tracing::{debug, instrument};
use yfinance_rs::{Ticker, YfClient};

use crate::sort::{SortMode, SortOrder};
use crate::{db, models::QuoteRecord};

// Core logic: talk to yfinance and translate its payload into our QuoteRecord.
#[instrument(skip(ticker), fields(symbol = %symbol))]
pub async fn fetch_quote(
    symbol: &str,
    ticker: &Ticker,
) -> Result<Option<QuoteRecord>, Box<dyn std::error::Error + Send + Sync>> {
    debug!("requesting quote from yfinance");
    let quote = ticker.quote().await?;
    debug!(
        ticker = ?quote.name,
        has_price = quote.price.is_some(),
        "received quote payload"
    );
    let quote_record = QuoteRecord {
        id: None, // Set by the database
        ticker: quote.name.clone(),
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

pub async fn fetch_quote_and_store(
    pool: &sqlx::PgPool,
    symbol: &str,
) -> Result<Option<QuoteRecord>, Box<dyn std::error::Error + Send + Sync>> {
    let client = YfClient::default();
    let ticker = Ticker::new(&client, symbol);

    let Some(mut quote_record) = fetch_quote(symbol, &ticker).await? else {
        return Ok(None);
    };

    // Stamp the DB-assigned id onto the record so callers can display it without
    // This allows the tui to show the row's id immediately after storing without th eneed for
    // requery
    quote_record.id = db::quotes::store_quote_to_db(&quote_record, pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(Some(quote_record))
}

pub fn prepare_tickers(s: &[String], c: &YfClient) -> Vec<(String, Ticker)> {
    s.iter().map(|t| (t.clone(), Ticker::new(c, t))).collect()
}

pub async fn fetch_recent(pool: &sqlx::PgPool, limit: i64) -> sqlx::Result<Vec<QuoteRecord>> {
    sqlx::query_as::<_, QuoteRecord>(
        "SELECT id, ticker, price, previous_close, day_volume, as_of
              FROM quotes
              ORDER BY as_of DESC, id DESC
              LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn fetch_sorted(
    pool: &sqlx::PgPool,
    mode: SortMode,
    order: SortOrder,
    limit: i64,
) -> sqlx::Result<Vec<QuoteRecord>> {
    debug!(?mode, ?order, "fetching sorted quotes");
    // `column` and `direction` come from closed enums, never user input, so
    // interpolating them into the query is safe from injection. `limit` is bound.
    let column = match mode {
        SortMode::ById => "id",
        SortMode::ByTicker => "ticker",
        SortMode::ByPrice => "price",
        SortMode::ByPrevClose => "previous_close",
        SortMode::ByVolume => "day_volume",
        SortMode::ByAsOf => "as_of",
    };
    let direction = match order {
        SortOrder::Ascending => "ASC",
        SortOrder::Descending => "DESC",
    };

    let query = format!(
        "SELECT id, ticker, price, previous_close, day_volume, as_of
              FROM quotes
              ORDER BY {column} {direction}, id DESC
              LIMIT $1"
    );

    sqlx::query_as::<_, QuoteRecord>(sqlx::AssertSqlSafe(query))
        .bind(limit)
        .fetch_all(pool)
        .await
}

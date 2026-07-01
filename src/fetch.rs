//! Yahoo Finance API integration.

use tokio::try_join;
use tracing::{debug, instrument};
use yfinance_rs::{AnalysisBuilder, Ticker, YfClient};

use crate::models::QuoteRecordAnalysis;
use crate::sort::{SortMode, SortOrder};
use crate::{db, models::QuoteRecord, AppError};

#[instrument(skip(ticker), fields(symbol = %symbol))]
pub async fn fetch_quote(
    symbol: &str,
    ticker: &Ticker,
) -> Result<Option<QuoteRecord>, AppError> {
    debug!("requesting quote from yfinance");
    let quote = ticker.quote().await?;
    debug!(
        ticker = ?quote.name,
        has_price = quote.price.is_some(),
        "received quote payload"
    );
    let quote_record = QuoteRecord {
        id: None,
        ticker: Some(symbol.to_string()),
        name: quote.name.clone(),
        price: quote.price.map(|p| p.into_inner().as_f64()),
        previous_close: quote.previous_close.map(|p| p.into_inner().as_f64()),
        day_volume: quote.day_volume.map(|p| p.into_inner().as_decimal().as_f64()),
        as_of: quote.as_of,
    };
    Ok(Some(quote_record))
}

/// Fetches a quote for `symbol` and immediately persists it to the database.
#[instrument(skip(pool), fields(symbol = %symbol))]
pub async fn fetch_quote_and_store(
    pool: &sqlx::PgPool,
    symbol: &str,
) -> Result<Option<QuoteRecord>, AppError> {
    let client = YfClient::default();
    let ticker = Ticker::new(&client, symbol);

    let Some(mut quote_record) = fetch_quote(symbol, &ticker).await? else {
        return Ok(None);
    };

    quote_record.id = match db::quotes::store_quote_to_db(&quote_record, pool).await? {
        id @ Some(_) => id,
        None => {
            sqlx::query_scalar(
                "SELECT id FROM quotes \
                 WHERE ticker IS NOT DISTINCT FROM $1 \
                 AND as_of IS NOT DISTINCT FROM $2",
            )
            .bind(quote_record.ticker.as_deref())
            .bind(quote_record.as_of)
            .fetch_optional(pool)
            .await?
        }
    };
    Ok(Some(quote_record))
}

/// Pairs each ticker symbol with an initialised [`Ticker`] client ready for async fetching.
pub fn prepare_tickers(s: &[String], c: &YfClient) -> Vec<(String, Ticker)> {
    s.iter().map(|t| (t.clone(), Ticker::new(c, t))).collect()
}

/// Returns up to `limit` quotes ordered by `as_of DESC, id DESC`.
#[deprecated(note = "use fetch_sorted(pool, SortMode::ByAsOf, SortOrder::Descending, limit) instead")]
pub async fn fetch_recent(pool: &sqlx::PgPool, limit: i64) -> sqlx::Result<Vec<QuoteRecord>> {
    fetch_sorted(pool, SortMode::ByAsOf, SortOrder::Descending, limit).await
}

/// Returns up to `limit` quotes sorted by `mode` in the given `order`.
pub async fn fetch_sorted(
    pool: &sqlx::PgPool,
    mode: SortMode,
    order: SortOrder,
    limit: i64,
) -> sqlx::Result<Vec<QuoteRecord>> {
    debug!(?mode, ?order, "fetching sorted quotes");
    let column = match mode {
        SortMode::ById => "id",
        SortMode::ByTicker => "ticker",
        SortMode::ByPrice => "price",
        SortMode::ByPrevClose => "previous_close",
        SortMode::ByVolume => "day_volume",
        SortMode::ByAsOf => "as_of",
        SortMode::ByName => "name",
    };
    let direction = match order {
        SortOrder::Ascending => "ASC",
        SortOrder::Descending => "DESC",
    };

    let query = format!(
        "SELECT id, ticker, name, price, previous_close, day_volume, as_of
              FROM quotes
              ORDER BY {column} {direction}, id DESC
              LIMIT $1"
    );

    sqlx::query_as::<_, QuoteRecord>(sqlx::AssertSqlSafe(query))
        .bind(limit)
        .fetch_all(pool)
        .await
}

/// Fetches analyst consensus and price targets for `symbol` concurrently.
#[instrument(fields(symbol = %symbol))]
pub async fn fetch_analysis(symbol: &str) -> Result<QuoteRecordAnalysis, AppError> {
    let client = YfClient::default();
    let analysis_builder = AnalysisBuilder::new(&client, symbol);

    let (rec, pt) = try_join!(
        analysis_builder.recommendations_summary(),
        analysis_builder.analyst_price_target(None),
    )?;

    Ok(QuoteRecordAnalysis {
        ticker: Some(symbol.to_string()),
        recommendation_summary: Some(rec),
        price_target: Some(pt),
    })
}

#[cfg(test)]
mod tests {
    use super::prepare_tickers;
    use yfinance_rs::YfClient;

    #[test]
    fn test_prepare_tickers_empty() {
        let client = YfClient::default();
        let result = prepare_tickers(&[], &client);
        assert!(result.is_empty());
    }

    #[test]
    fn test_prepare_tickers_count() {
        let client = YfClient::default();
        let symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        let result = prepare_tickers(&symbols, &client);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_prepare_tickers_symbols_preserved() {
        let client = YfClient::default();
        let symbols = vec!["AAPL".to_string(), "MSFT".to_string()];
        let result = prepare_tickers(&symbols, &client);
        assert_eq!(result[0].0, "AAPL");
        assert_eq!(result[1].0, "MSFT");
    }
}

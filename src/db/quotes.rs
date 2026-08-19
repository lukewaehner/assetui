//! Queries against the `quotes` table.

use std::path::{Path, PathBuf};

use futures_util::TryStreamExt;
use sqlx::{Pool, Postgres};
use tracing::{debug, info, instrument};

use crate::AppError;
use crate::models::QuoteRecord;

/// Rows kept per ticker in the `quotes` history.
pub const QUOTE_HISTORY_RETENTION: i64 = 200;

/// Inserts a quote into the database and returns the row's `id`.
#[instrument(skip(quote, p), fields(ticker = ?quote.ticker))]
pub async fn store_quote_to_db(quote: &QuoteRecord, p: &Pool<Postgres>) -> Result<i32, AppError> {
    if quote.ticker.is_none() || quote.as_of.is_none() {
        return Err(AppError::Data(
            "cannot store quote without a ticker and as_of".to_string(),
        ));
    }

    debug!("inserting quote into postgres");
    let id: i32 = sqlx::query_scalar(
        "
        INSERT INTO quotes (ticker, name, price, previous_close, day_volume, as_of)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (ticker, as_of) DO UPDATE SET ticker = EXCLUDED.ticker
        RETURNING id
        ",
    )
    .bind(quote.ticker.as_deref())
    .bind(quote.name.as_deref())
    .bind(quote.price)
    .bind(quote.previous_close)
    .bind(quote.day_volume)
    .bind(quote.as_of)
    .fetch_one(p)
    .await?;
    Ok(id)
}

/// Deletes a ticker's oldest quote rows, retaining the most recent `keep`.
#[instrument(skip(p))]
pub async fn prune_quotes_history(
    p: &Pool<Postgres>,
    ticker: &str,
    keep: i64,
) -> Result<u64, AppError> {
    let result = sqlx::query(
        "
        DELETE FROM quotes
        WHERE ticker = $1
          AND as_of < (
            SELECT as_of FROM quotes
            WHERE ticker = $1
            ORDER BY as_of DESC
            OFFSET $2 - 1 LIMIT 1
          )
        ",
    )
    .bind(ticker)
    .bind(keep)
    .execute(p)
    .await?;
    Ok(result.rows_affected())
}

/// Returns every row in the `quotes` table with no ordering applied.
#[instrument(skip(p))]
pub async fn fetch_all_quotes(p: &Pool<Postgres>) -> Result<Vec<QuoteRecord>, AppError> {
    debug!("pulling quotes from postgres");
    let rows = sqlx::query_as::<_, QuoteRecord>(
        "SELECT id, ticker, name, price, previous_close, day_volume, as_of FROM quotes",
    )
    .fetch_all(p)
    .await?;
    Ok(rows)
}

/// Dumps every quote to the CSV file at `out`, returning that same path.
#[instrument(skip(p, out))]
pub async fn dump_table_to_csv(p: &Pool<Postgres>, out: &Path) -> Result<PathBuf, AppError> {
    debug!("dumping quotes table to csv");
    let mut wtr = csv::Writer::from_path(out)?;
    let mut rows = sqlx::query_as::<_, QuoteRecord>(
        "SELECT id, ticker, name, price, previous_close, day_volume, as_of FROM quotes",
    )
    .fetch(p);
    let mut written = 0u64;
    while let Some(row) = rows.try_next().await? {
        wtr.serialize(row)?;
        written += 1;
    }
    wtr.flush()?;
    info!(
        rows = written,
        "quotes table dumped to csv successfully, written at: {}",
        out.display()
    );
    Ok(out.to_path_buf())
}

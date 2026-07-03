//! Queries against the `quotes` table.

use std::path::{Path, PathBuf};

use sqlx::{Pool, Postgres};
use tracing::{debug, info, instrument};

use crate::AppError;
use crate::models::QuoteRecord;

/// Inserts a quote into the database and returns the row's `id`.
///
/// The table has a `UNIQUE (ticker, as_of)` constraint.  On a duplicate
/// (same ticker and timestamp) the no-op `DO UPDATE` fires so the statement
/// still returns the existing row's `id` in a single round-trip, keeping the
/// call idempotent without a follow-up `SELECT`.
#[instrument(skip(quote, p), fields(ticker = ?quote.ticker))]
pub async fn store_quote_to_db(quote: &QuoteRecord, p: &Pool<Postgres>) -> Result<i32, AppError> {
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

/// Returns every row in the `quotes` table with no ordering applied.
///
/// Primarily used by the CSV dump and the CLI display path, where the caller
/// controls presentation order.
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

/// Dumps every quote to a timestamped CSV file inside `dir`, returning the
/// path of the file that was written.
///
/// The filename is `quotes_dump_YYYYMMDDHHMMSS.csv`.  Rows are serialised
/// using `QuoteRecord`'s `serde::Serialize` implementation.
#[instrument(skip(p, dir))]
pub async fn dump_table_to_csv(p: &Pool<Postgres>, dir: &Path) -> Result<PathBuf, AppError> {
    debug!("dumping quotes table to csv");
    let rows: Vec<QuoteRecord> = fetch_all_quotes(p).await?;
    let path = dir.join(format!(
        "quotes_dump_{}.csv",
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    ));
    let mut wtr = csv::Writer::from_path(&path)?;
    for row in rows {
        wtr.serialize(row)?;
    }
    wtr.flush()?;
    info!(
        "quotes table dumped to csv successfully, written at: {}",
        path.display()
    );
    Ok(path)
}

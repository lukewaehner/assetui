//! Queries against the `quotes` table.

use sqlx::{Pool, Postgres};
use tracing::{debug, info, instrument};

use crate::AppError;
use crate::models::QuoteRecord;

/// Inserts a quote into the database and returns the new row's `id`.
///
/// The table has a `UNIQUE (ticker, as_of)` constraint.  If the same ticker
/// and timestamp already exist the insert is silently skipped and `None` is
/// returned, which lets callers detect duplicates without treating them as
/// errors.
#[instrument(skip(quote, p), fields(ticker = ?quote.ticker))]
pub async fn store_quote_to_db(
    quote: &QuoteRecord,
    p: &Pool<Postgres>,
) -> Result<Option<i32>, AppError> {
    debug!("inserting quote into postgres");
    let id: Option<i32> = sqlx::query_scalar(
        "
        INSERT INTO quotes (ticker, name, price, previous_close, day_volume, as_of)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (ticker, as_of) DO NOTHING
        RETURNING id
        ",
    )
    .bind(quote.ticker.as_deref())
    .bind(quote.name.as_deref())
    .bind(quote.price)
    .bind(quote.previous_close)
    .bind(quote.day_volume)
    .bind(quote.as_of)
    .fetch_optional(p)
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

/// Dumps every quote to a timestamped CSV file in the current directory.
///
/// The filename is `quotes_dump_YYYYMMDDHHMMSS.csv`.  Rows are serialised
/// using `QuoteRecord`'s `serde::Serialize` implementation.
#[instrument(skip(p))]
pub async fn dump_table_to_csv(p: &Pool<Postgres>) -> Result<(), AppError> {
    debug!("dumping quotes table to csv");
    let rows: Vec<QuoteRecord> = fetch_all_quotes(p).await?;
    let path = format!(
        "quotes_dump_{}.csv",
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    );
    let mut wtr = csv::Writer::from_path(&path)?;
    for row in rows {
        wtr.serialize(row)?;
    }
    wtr.flush()?;
    info!(
        "quotes table dumped to csv successfully, written at: {}",
        path
    );
    Ok(())
}

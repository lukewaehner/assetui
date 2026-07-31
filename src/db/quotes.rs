//! Queries against the `quotes` table.

use std::path::{Path, PathBuf};

use sqlx::{Pool, Postgres};
use tracing::{debug, info, instrument};

use crate::AppError;
use crate::models::QuoteRecord;

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
    let rows: Vec<QuoteRecord> = fetch_all_quotes(p).await?;
    let mut wtr = csv::Writer::from_path(out)?;
    for row in rows {
        wtr.serialize(row)?;
    }
    wtr.flush()?;
    info!(
        "quotes table dumped to csv successfully, written at: {}",
        out.display()
    );
    Ok(out.to_path_buf())
}

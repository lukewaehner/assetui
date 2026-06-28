use sqlx::{Pool, Postgres};
use tracing::{debug, info, instrument};

use crate::models::QuoteRecord;

#[instrument(skip(quote, p), fields(ticker = ?quote.ticker))]
pub async fn store_quote_to_db(
    quote: &QuoteRecord,
    p: &Pool<Postgres>,
) -> Result<Option<i32>, Box<dyn std::error::Error>> {
    debug!("inserting quote into postgres");
    let id: Option<i32> = sqlx::query_scalar(
        "
        INSERT INTO quotes (ticker, price, previous_close, day_volume, as_of)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (ticker, as_of) DO NOTHING
        RETURNING id
        ",
    )
    .bind(quote.ticker.clone())
    .bind(quote.price)
    .bind(quote.previous_close)
    .bind(quote.day_volume)
    .bind(quote.as_of)
    .fetch_optional(p)
    .await?;
    Ok(id)
}

#[instrument(skip(p))]
pub async fn fetch_all_quotes(
    p: &Pool<Postgres>,
) -> Result<Vec<QuoteRecord>, Box<dyn std::error::Error>> {
    debug!("pulling quotes from postgres");
    let rows = sqlx::query_as::<_, QuoteRecord>("SELECT * FROM quotes")
        .fetch_all(p)
        .await?;
    Ok(rows)
}

pub async fn dump_table_to_csv(p: &Pool<Postgres>) -> Result<(), Box<dyn std::error::Error>> {
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

//! Watchlist database module

use sqlx::{Pool, Postgres};

use crate::AppError;

/// Tracks a ticker to the watchlist table. IF the ticker is already in the watchlist, we no-op.
pub async fn add_to_watchlist(pool: &Pool<Postgres>, ticker: &str) -> Result<(), AppError> {
    let ticker = ticker.to_uppercase();
    sqlx::query("INSERT INTO watchlist (ticker) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(ticker)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn remove_from_watchlist(pool: &Pool<Postgres>, ticker: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM watchlist WHERE ticker = $1")
        .bind(ticker)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn fetch_watchlist(pool: &Pool<Postgres>) -> Result<Vec<String>, AppError> {
    let rows = sqlx::query!("SELECT ticker FROM watchlist")
        .fetch_all(pool)
        .await?;

    let tickers = rows
        .into_iter()
        .map(|row| row.ticker.to_uppercase())
        .collect();
    Ok(tickers)
}

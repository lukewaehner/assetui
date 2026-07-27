//! Portfolio-holdings database module.

use sqlx::{Pool, Postgres};

use crate::AppError;
use crate::portfolio::Holding;

/// Inserts or updates a holding (add and edit share one path).
pub async fn upsert_holding(pool: &Pool<Postgres>, holding: &Holding) -> Result<(), AppError> {
    let ticker = holding.ticker.to_uppercase();
    sqlx::query(
        "INSERT INTO portfolio_holdings (ticker, shares, avg_cost) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (ticker) DO UPDATE \
         SET shares = EXCLUDED.shares, avg_cost = EXCLUDED.avg_cost, updated_at = now()",
    )
    .bind(ticker)
    .bind(holding.shares)
    .bind(holding.avg_cost)
    .execute(pool)
    .await?;
    Ok(())
}

/// Removes a holding by ticker (no-op if it doesn't exist).
pub async fn remove_holding(pool: &Pool<Postgres>, ticker: &str) -> Result<(), AppError> {
    let ticker = ticker.to_uppercase();
    sqlx::query("DELETE FROM portfolio_holdings WHERE ticker = $1")
        .bind(ticker)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list_holdings(pool: &Pool<Postgres>) -> Result<Vec<Holding>, AppError> {
    let rows = sqlx::query_as::<_, (String, f64, f64)>(
        "SELECT ticker, shares, avg_cost FROM portfolio_holdings ORDER BY ticker",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(ticker, shares, avg_cost)| Holding {
            ticker,
            shares,
            avg_cost,
        })
        .collect())
}

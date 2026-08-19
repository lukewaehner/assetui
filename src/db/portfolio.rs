//! Portfolio-holdings database module.

use rust_decimal::Decimal;
use sqlx::{Pool, Postgres};

use crate::AppError;
use crate::db::numeric::{from_numeric, to_numeric};
use crate::portfolio::Holding;

/// Inserts or updates a holding (add and edit share one path).
pub async fn upsert_holding(pool: &Pool<Postgres>, holding: &Holding) -> Result<(), AppError> {
    let ticker = holding.ticker.to_uppercase();
    let shares = to_numeric(holding.shares, "shares")?;
    let avg_cost = to_numeric(holding.avg_cost, "avg_cost")?;
    sqlx::query(
        "INSERT INTO portfolio_holdings (ticker, shares, avg_cost) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (ticker) DO UPDATE \
         SET shares = EXCLUDED.shares, avg_cost = EXCLUDED.avg_cost, updated_at = now()",
    )
    .bind(ticker)
    .bind(shares)
    .bind(avg_cost)
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
    let rows = sqlx::query_as::<_, (String, Decimal, Decimal)>(
        "SELECT ticker, shares, avg_cost FROM portfolio_holdings ORDER BY ticker",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(ticker, shares, avg_cost)| {
            Ok(Holding {
                ticker,
                shares: from_numeric(shares, "shares")?,
                avg_cost: from_numeric(avg_cost, "avg_cost")?,
            })
        })
        .collect()
}

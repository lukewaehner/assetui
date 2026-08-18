//! Portfolio value-history database module.

use chrono::NaiveDate;
use sqlx::{Pool, Postgres};

use crate::AppError;

/// Records `day`'s totals, replacing any existing row for that day so the last sample of the day wins.
pub async fn upsert_day(
    pool: &Pool<Postgres>,
    day: NaiveDate,
    market_value: f64,
    cost_basis: f64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO portfolio_value_history (day, market_value, cost_basis) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (day) DO UPDATE \
         SET market_value = EXCLUDED.market_value, \
             cost_basis = EXCLUDED.cost_basis, \
             updated_at = now()",
    )
    .bind(day)
    .bind(market_value)
    .bind(cost_basis)
    .execute(pool)
    .await?;
    Ok(())
}

/// Loads the full history as `(day, market_value, cost_basis)`, oldest first.
pub async fn load_history(pool: &Pool<Postgres>) -> Result<Vec<(NaiveDate, f64, f64)>, AppError> {
    let rows = sqlx::query_as::<_, (NaiveDate, f64, f64)>(
        "SELECT day, market_value, cost_basis FROM portfolio_value_history ORDER BY day",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

//! Portfolio value-history database module.

use chrono::NaiveDate;
use rust_decimal::Decimal;
use sqlx::{Pool, Postgres};

use crate::AppError;
use crate::db::numeric::{from_numeric, to_numeric};

/// Records `day`'s totals, replacing any existing row for that day so the last sample of the day wins.
pub async fn upsert_day(
    pool: &Pool<Postgres>,
    day: NaiveDate,
    market_value: f64,
    cost_basis: f64,
) -> Result<(), AppError> {
    let market_value = to_numeric(market_value, "market_value")?;
    let cost_basis = to_numeric(cost_basis, "cost_basis")?;
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
    let rows = sqlx::query_as::<_, (NaiveDate, Decimal, Decimal)>(
        "SELECT day, market_value, cost_basis FROM portfolio_value_history ORDER BY day",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(day, market_value, cost_basis)| {
            Ok((
                day,
                from_numeric(market_value, "market_value")?,
                from_numeric(cost_basis, "cost_basis")?,
            ))
        })
        .collect()
}

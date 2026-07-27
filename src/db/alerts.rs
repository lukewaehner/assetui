//! Price-alerts database module.

use chrono::{DateTime, Utc};
use sqlx::{Pool, Postgres};

use crate::AppError;
use crate::models::{Alert, AlertDirection};

#[derive(sqlx::FromRow)]
struct AlertRow {
    id: i32,
    ticker: String,
    direction: String,
    threshold: f64,
    created_at: DateTime<Utc>,
    triggered_at: Option<DateTime<Utc>>,
}

impl AlertRow {
    fn into_alert(self) -> Result<Alert, AppError> {
        Ok(Alert {
            id: self.id,
            ticker: self.ticker,
            direction: direction_from_db(&self.direction)?,
            threshold: self.threshold,
            created_at: self.created_at,
            triggered_at: self.triggered_at,
        })
    }
}

/// Canonical string stored in the `direction` column.
fn direction_to_db(direction: AlertDirection) -> &'static str {
    match direction {
        AlertDirection::Above => "ABOVE",
        AlertDirection::Below => "BELOW",
    }
}

/// Parses the `direction` column back into [`AlertDirection`].
fn direction_from_db(s: &str) -> Result<AlertDirection, AppError> {
    match s.to_ascii_uppercase().as_str() {
        "ABOVE" => Ok(AlertDirection::Above),
        "BELOW" => Ok(AlertDirection::Below),
        other => Err(AppError::Data(format!("invalid alert direction: {other}"))),
    }
}

/// Inserts an alert and returns the stored row (including its assigned `id`).
pub async fn add_alert(
    pool: &Pool<Postgres>,
    ticker: &str,
    direction: AlertDirection,
    threshold: f64,
) -> Result<Alert, AppError> {
    let ticker = ticker.to_uppercase();
    let row = sqlx::query_as::<_, AlertRow>(
        "INSERT INTO alerts (ticker, direction, threshold) \
         VALUES ($1, $2, $3) \
         RETURNING id, ticker, direction, threshold, created_at, triggered_at",
    )
    .bind(ticker)
    .bind(direction_to_db(direction))
    .bind(threshold)
    .fetch_one(pool)
    .await?;
    row.into_alert()
}

/// Deletes the alert with the given id (no-op if it doesn't exist).
pub async fn remove_alert(pool: &Pool<Postgres>, id: i32) -> Result<(), AppError> {
    sqlx::query("DELETE FROM alerts WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Lists the **active** alerts (`triggered_at IS NULL`), most-recently-created first.
pub async fn list_alerts(pool: &Pool<Postgres>) -> Result<Vec<Alert>, AppError> {
    let rows = sqlx::query_as::<_, AlertRow>(
        "SELECT id, ticker, direction, threshold, created_at, triggered_at \
         FROM alerts \
         WHERE triggered_at IS NULL \
         ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(AlertRow::into_alert).collect()
}

pub async fn mark_alert_triggered(
    pool: &Pool<Postgres>,
    id: i32,
    triggered_at: DateTime<Utc>,
) -> Result<(), AppError> {
    sqlx::query("UPDATE alerts SET triggered_at = $1 WHERE id = $2")
        .bind(triggered_at)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

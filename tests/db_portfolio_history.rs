//! Integration tests for the portfolio value-history db module.

use assetui::db::portfolio_history::{load_history, upsert_day};
use chrono::NaiveDate;

fn day(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// A fresh database has no history.
#[sqlx::test]
async fn test_load_empty_returns_empty(pool: sqlx::PgPool) {
    let history = load_history(&pool)
        .await
        .expect("load should succeed on empty db");
    assert!(history.is_empty(), "expected no history, got {history:?}");
}

#[sqlx::test]
async fn test_upsert_then_load_returns_the_day(pool: sqlx::PgPool) {
    upsert_day(&pool, day(2026, 8, 18), 1100.0, 1000.0)
        .await
        .expect("upsert should succeed");

    let history = load_history(&pool).await.expect("load should succeed");
    assert_eq!(history, vec![(day(2026, 8, 18), 1100.0, 1000.0)]);
}

#[sqlx::test]
async fn test_upsert_same_day_replaces_the_row(pool: sqlx::PgPool) {
    upsert_day(&pool, day(2026, 8, 18), 1100.0, 1000.0)
        .await
        .expect("first upsert should succeed");

    upsert_day(&pool, day(2026, 8, 18), 1250.0, 1000.0)
        .await
        .expect("second upsert should succeed");

    let history = load_history(&pool).await.expect("load should succeed");
    assert_eq!(history, vec![(day(2026, 8, 18), 1250.0, 1000.0)]);
}

#[sqlx::test]
async fn test_load_orders_days_ascending(pool: sqlx::PgPool) {
    upsert_day(&pool, day(2026, 8, 18), 1200.0, 1000.0)
        .await
        .unwrap();
    upsert_day(&pool, day(2026, 8, 16), 1000.0, 1000.0)
        .await
        .unwrap();
    upsert_day(&pool, day(2026, 8, 17), 1100.0, 1000.0)
        .await
        .unwrap();

    let history = load_history(&pool).await.expect("load should succeed");

    let days: Vec<NaiveDate> = history.into_iter().map(|(d, _, _)| d).collect();
    assert_eq!(
        days,
        vec![day(2026, 8, 16), day(2026, 8, 17), day(2026, 8, 18)]
    );
}

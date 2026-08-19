use assetui::db::alerts::{add_alert, list_alerts, mark_alert_triggered, remove_alert};
use assetui::models::AlertDirection;
use chrono::Utc;

/// A fresh database has no active alerts.
#[sqlx::test]
async fn test_list_empty_returns_empty(pool: sqlx::PgPool) {
    let alerts = list_alerts(&pool)
        .await
        .expect("list should succeed on empty db");
    assert!(alerts.is_empty(), "expected no alerts, got {alerts:?}");
}

#[sqlx::test]
async fn test_add_then_list_returns_alert(pool: sqlx::PgPool) {
    let created = add_alert(&pool, "aapl", AlertDirection::Above, 150.0)
        .await
        .expect("add should succeed");

    assert!(created.id > 0, "insert should assign an id");
    assert_eq!(created.ticker, "AAPL", "ticker should be uppercased");
    assert_eq!(created.direction, AlertDirection::Above);
    assert_eq!(created.threshold, 150.0);
    assert!(created.triggered_at.is_none(), "new alert should be active");

    let alerts = list_alerts(&pool).await.expect("list should succeed");
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].direction, AlertDirection::Above);
}

#[sqlx::test]
async fn test_below_direction_round_trips(pool: sqlx::PgPool) {
    add_alert(&pool, "TSLA", AlertDirection::Below, 100.0)
        .await
        .expect("add should succeed");

    let alerts = list_alerts(&pool).await.expect("list should succeed");
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].direction, AlertDirection::Below);
}

#[sqlx::test]
async fn test_list_excludes_triggered_alerts(pool: sqlx::PgPool) {
    let a = add_alert(&pool, "AAPL", AlertDirection::Above, 150.0)
        .await
        .expect("add AAPL failed");
    add_alert(&pool, "TSLA", AlertDirection::Below, 100.0)
        .await
        .expect("add TSLA failed");

    mark_alert_triggered(&pool, a.id, Utc::now())
        .await
        .expect("mark triggered should succeed");

    let alerts = list_alerts(&pool).await.expect("list should succeed");
    assert_eq!(alerts.len(), 1, "triggered alert should be excluded");
    assert_eq!(alerts[0].ticker, "TSLA");
}

#[sqlx::test]
async fn test_remove_deletes_alert(pool: sqlx::PgPool) {
    let a = add_alert(&pool, "AAPL", AlertDirection::Above, 150.0)
        .await
        .expect("add should succeed");

    remove_alert(&pool, a.id)
        .await
        .expect("remove should succeed");

    let alerts = list_alerts(&pool).await.expect("list should succeed");
    assert!(alerts.is_empty(), "removed alert should be gone");
}

#[sqlx::test]
async fn test_multiple_alerts_per_ticker(pool: sqlx::PgPool) {
    add_alert(&pool, "AAPL", AlertDirection::Above, 200.0)
        .await
        .expect("add first failed");
    add_alert(&pool, "AAPL", AlertDirection::Below, 150.0)
        .await
        .expect("add second failed");

    let alerts = list_alerts(&pool).await.expect("list should succeed");
    assert_eq!(
        alerts.len(),
        2,
        "both alerts on the same ticker should persist"
    );
}

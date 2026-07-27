//! Integration tests for the portfolio-holdings db module (PORT-1).

use assetui::db::portfolio::{list_holdings, remove_holding, upsert_holding};
use assetui::portfolio::Holding;

fn holding(ticker: &str, shares: f64, avg_cost: f64) -> Holding {
    Holding {
        ticker: ticker.to_string(),
        shares,
        avg_cost,
    }
}

/// A fresh database has no holdings.
#[sqlx::test]
async fn test_list_empty_returns_empty(pool: sqlx::PgPool) {
    let holdings = list_holdings(&pool).await.expect("list should succeed on empty db");
    assert!(holdings.is_empty(), "expected no holdings, got {holdings:?}");
}

#[sqlx::test]
async fn test_upsert_then_list_returns_holding(pool: sqlx::PgPool) {
    upsert_holding(&pool, &holding("aapl", 10.0, 150.0))
        .await
        .expect("upsert should succeed");

    let holdings = list_holdings(&pool).await.expect("list should succeed");
    assert_eq!(holdings, vec![holding("AAPL", 10.0, 150.0)]);
}

#[sqlx::test]
async fn test_upsert_same_ticker_edits_in_place(pool: sqlx::PgPool) {
    upsert_holding(&pool, &holding("AAPL", 10.0, 150.0))
        .await
        .expect("first upsert failed");
    upsert_holding(&pool, &holding("AAPL", 25.0, 140.0))
        .await
        .expect("second upsert failed");

    let holdings = list_holdings(&pool).await.expect("list should succeed");
    assert_eq!(holdings, vec![holding("AAPL", 25.0, 140.0)], "edit should replace, not duplicate");
}

#[sqlx::test]
async fn test_list_is_ordered_by_ticker(pool: sqlx::PgPool) {
    upsert_holding(&pool, &holding("TSLA", 5.0, 200.0))
        .await
        .expect("upsert TSLA failed");
    upsert_holding(&pool, &holding("AAPL", 10.0, 150.0))
        .await
        .expect("upsert AAPL failed");

    let tickers: Vec<String> = list_holdings(&pool)
        .await
        .expect("list should succeed")
        .into_iter()
        .map(|h| h.ticker)
        .collect();
    assert_eq!(tickers, vec!["AAPL".to_string(), "TSLA".to_string()]);
}

#[sqlx::test]
async fn test_remove_deletes_holding(pool: sqlx::PgPool) {
    upsert_holding(&pool, &holding("AAPL", 10.0, 150.0))
        .await
        .expect("upsert failed");

    remove_holding(&pool, "aapl").await.expect("remove should succeed");

    let holdings = list_holdings(&pool).await.expect("list should succeed");
    assert!(holdings.is_empty(), "lowercase remove should delete the stored holding");
}

/// Removing a holding that isn't present is a no-op, not an error.
#[sqlx::test]
async fn test_remove_absent_is_noop(pool: sqlx::PgPool) {
    remove_holding(&pool, "AAPL")
        .await
        .expect("removing an absent holding should not error");
}

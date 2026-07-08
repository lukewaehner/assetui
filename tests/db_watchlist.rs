//! Integration tests for the watchlist db module (WATCH-2).

use assetui::db::watchlist::{add_to_watchlist, fetch_watchlist, remove_from_watchlist};

#[sqlx::test]
async fn test_fetch_empty_returns_empty(pool: sqlx::PgPool) {
    let tickers = fetch_watchlist(&pool)
        .await
        .expect("fetch_watchlist should succeed on empty db");
    assert!(
        tickers.is_empty(),
        "expected empty watchlist, got {tickers:?}"
    );
}

#[sqlx::test]
async fn test_add_then_fetch_returns_ticker(pool: sqlx::PgPool) {
    add_to_watchlist(&pool, "AAPL")
        .await
        .expect("add should succeed");

    let tickers = fetch_watchlist(&pool).await.expect("fetch should succeed");
    assert_eq!(tickers, vec!["AAPL".to_string()]);
}

#[sqlx::test]
async fn test_add_uppercases_ticker(pool: sqlx::PgPool) {
    add_to_watchlist(&pool, "aapl")
        .await
        .expect("add should succeed");

    let tickers = fetch_watchlist(&pool).await.expect("fetch should succeed");
    assert_eq!(tickers, vec!["AAPL".to_string()]);
}

#[sqlx::test]
async fn test_add_is_idempotent(pool: sqlx::PgPool) {
    add_to_watchlist(&pool, "AAPL")
        .await
        .expect("first add should succeed");
    add_to_watchlist(&pool, "aapl")
        .await
        .expect("duplicate add should not error");

    let tickers = fetch_watchlist(&pool).await.expect("fetch should succeed");
    assert_eq!(
        tickers,
        vec!["AAPL".to_string()],
        "duplicate should not add a second row"
    );
}

#[sqlx::test]
async fn test_remove_deletes_ticker(pool: sqlx::PgPool) {
    add_to_watchlist(&pool, "AAPL")
        .await
        .expect("add AAPL failed");
    add_to_watchlist(&pool, "TSLA")
        .await
        .expect("add TSLA failed");

    remove_from_watchlist(&pool, "AAPL")
        .await
        .expect("remove should succeed");

    let tickers = fetch_watchlist(&pool).await.expect("fetch should succeed");
    assert_eq!(
        tickers,
        vec!["TSLA".to_string()],
        "only AAPL should be removed"
    );
}

#[sqlx::test]
async fn test_remove_is_case_insensitive(pool: sqlx::PgPool) {
    add_to_watchlist(&pool, "AAPL")
        .await
        .expect("add should succeed");

    remove_from_watchlist(&pool, "aapl")
        .await
        .expect("remove should succeed");

    let tickers = fetch_watchlist(&pool).await.expect("fetch should succeed");
    assert!(
        tickers.is_empty(),
        "lowercase remove should delete the stored ticker"
    );
}

/// Removing a ticker that isn't tracked is a no-op, not an error.
#[sqlx::test]
async fn test_remove_absent_is_noop(pool: sqlx::PgPool) {
    remove_from_watchlist(&pool, "AAPL")
        .await
        .expect("removing an absent ticker should not error");

    let tickers = fetch_watchlist(&pool).await.expect("fetch should succeed");
    assert!(tickers.is_empty());
}

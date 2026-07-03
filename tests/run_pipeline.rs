use yfinance::run::fetch_and_store;
use yfinance_rs::YfClient;

/// Passing an empty ticker slice exercises the channel-setup and drain loop
/// without making any network requests.  The channel is created, no tasks are
/// spawned, the sender is dropped, and `rx.recv()` immediately returns `None`,
/// so the function must return `Ok(())`.
#[sqlx::test]
async fn test_fetch_and_store_empty_tickers(pool: sqlx::PgPool) {
    let result = fetch_and_store(&pool, &YfClient::default(), &[]).await;
    assert!(result.is_ok(), "expected Ok(()), got: {:?}", result);
}

/// Fetches a real quote for AAPL from Yahoo Finance and stores it.
///
/// Marked `#[ignore]` because it requires outbound network access; run it
/// explicitly with:
///
/// ```sh
/// cargo test -- --ignored
/// ```
///
/// A missing quote (Yahoo returns nothing for this symbol at this moment) is
/// not an error - the function still returns `Ok(())`.
#[sqlx::test]
#[ignore = "requires network access"]
async fn test_fetch_and_store_real_ticker(pool: sqlx::PgPool) {
    let tickers = vec!["AAPL".to_string()];
    let result = fetch_and_store(&pool, &YfClient::default(), &tickers).await;
    assert!(result.is_ok(), "expected Ok(()), got: {:?}", result);
}

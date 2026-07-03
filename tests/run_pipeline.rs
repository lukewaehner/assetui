use yfinance::run::fetch_and_store;
use yfinance_rs::YfClient;

#[sqlx::test]
async fn test_fetch_and_store_empty_tickers(pool: sqlx::PgPool) {
    let result = fetch_and_store(&pool, &YfClient::default(), &[]).await;
    assert!(result.is_ok(), "expected Ok(()), got: {:?}", result);
}

#[sqlx::test]
#[ignore = "requires network access"]
async fn test_fetch_and_store_real_ticker(pool: sqlx::PgPool) {
    let tickers = vec!["AAPL".to_string()];
    let result = fetch_and_store(&pool, &YfClient::default(), &tickers).await;
    assert!(result.is_ok(), "expected Ok(()), got: {:?}", result);
}

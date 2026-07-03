use chrono::{DateTime, Utc};
use yfinance::db::quotes::{dump_table_to_csv, fetch_all_quotes, store_quote_to_db};
use yfinance::models::QuoteRecord;

fn make_quote(ticker: &str) -> QuoteRecord {
    QuoteRecord {
        id: None,
        ticker: Some(ticker.to_string()),
        name: Some(format!("{ticker} Inc.")),
        price: Some(150.0),
        previous_close: Some(148.0),
        day_volume: Some(1_000_000.0),
        as_of: Some(Utc::now()),
    }
}

/// store_quote_to_db returns Ok(id) where id > 0 for a fresh insert.
#[sqlx::test]
async fn test_store_returns_id(pool: sqlx::PgPool) {
    let quote = make_quote("AAPL");
    let result = store_quote_to_db(&quote, &pool).await;
    assert!(
        result.is_ok(),
        "store_quote_to_db returned an error: {:?}",
        result
    );
    assert!(result.unwrap() > 0, "returned id should be positive");
}

/// Inserting the same (ticker, as_of) twice is idempotent: the duplicate
/// insert succeeds and returns the same id as the first (the ON CONFLICT
/// upsert re-returns the existing row).
#[sqlx::test]
async fn test_store_duplicate_returns_existing_id(pool: sqlx::PgPool) {
    let fixed_ts: DateTime<Utc> = DateTime::from_timestamp(1_000_000, 0).unwrap();
    let quote = QuoteRecord {
        id: None,
        ticker: Some("MSFT".to_string()),
        name: Some("MSFT Inc.".to_string()),
        price: Some(300.0),
        previous_close: Some(298.0),
        day_volume: Some(500_000.0),
        as_of: Some(fixed_ts),
    };

    let first = store_quote_to_db(&quote, &pool)
        .await
        .expect("first insert should succeed");

    let second = store_quote_to_db(&quote, &pool)
        .await
        .expect("duplicate insert should not error");
    assert_eq!(
        first, second,
        "duplicate insert should return the existing row's id"
    );
}

/// fetch_all_quotes on a fresh (empty) database returns an empty vec.
#[sqlx::test]
async fn test_fetch_all_empty(pool: sqlx::PgPool) {
    let rows = fetch_all_quotes(&pool)
        .await
        .expect("fetch_all_quotes should succeed on empty db");
    assert!(
        rows.is_empty(),
        "expected empty vec on a fresh db, got {} rows",
        rows.len()
    );
}

/// Storing two quotes with distinct tickers then fetching returns both.
#[sqlx::test]
async fn test_fetch_all_returns_stored(pool: sqlx::PgPool) {
    let q1 = make_quote("GOOG");
    let q2 = make_quote("TSLA");

    store_quote_to_db(&q1, &pool)
        .await
        .expect("store q1 failed");
    store_quote_to_db(&q2, &pool)
        .await
        .expect("store q2 failed");

    let rows = fetch_all_quotes(&pool)
        .await
        .expect("fetch_all_quotes failed");

    assert_eq!(rows.len(), 2, "expected 2 rows, got {}", rows.len());
    assert!(
        rows.iter().any(|r| r.ticker.as_deref() == Some("GOOG")),
        "GOOG not found in results"
    );
    assert!(
        rows.iter().any(|r| r.ticker.as_deref() == Some("TSLA")),
        "TSLA not found in results"
    );
}

/// dump_table_to_csv succeeds, writes into the requested directory, and
/// returns the path of the created file.
#[sqlx::test]
async fn test_dump_to_csv_creates_file(pool: sqlx::PgPool) {
    // Insert at least one row so the CSV is non-trivial.
    store_quote_to_db(&make_quote("AMZN"), &pool)
        .await
        .expect("store failed");

    let dir = std::env::temp_dir();
    let path = dump_table_to_csv(&pool, &dir)
        .await
        .expect("dump_table_to_csv returned an error");

    assert!(path.starts_with(&dir), "CSV should be written inside `dir`");
    let name = path.file_name().unwrap().to_string_lossy();
    assert!(
        name.starts_with("quotes_dump_") && name.ends_with(".csv"),
        "unexpected CSV filename: {name}"
    );
    assert!(path.exists(), "returned path should exist on disk");

    // Clean up.
    let _ = std::fs::remove_file(&path);
}

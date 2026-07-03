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

/// store_quote_to_db returns Ok(Some(id)) where id > 0 for a fresh insert.
#[sqlx::test]
async fn test_store_returns_id(pool: sqlx::PgPool) {
    let quote = make_quote("AAPL");
    let result = store_quote_to_db(&quote, &pool).await;
    assert!(
        result.is_ok(),
        "store_quote_to_db returned an error: {:?}",
        result
    );
    let id = result.unwrap();
    assert!(
        id.is_some(),
        "expected Some(id) for a fresh insert, got None"
    );
    assert!(id.unwrap() > 0, "returned id should be positive");
}

/// Inserting the same (ticker, as_of) twice: first call returns Some(id),
/// second call returns None (ON CONFLICT DO NOTHING).
#[sqlx::test]
async fn test_store_duplicate_returns_none(pool: sqlx::PgPool) {
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
    assert!(first.is_some(), "first insert should return Some(id)");

    let second = store_quote_to_db(&quote, &pool)
        .await
        .expect("duplicate insert should not error");
    assert!(second.is_none(), "duplicate insert should return None");
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

/// dump_table_to_csv succeeds and creates a file matching quotes_dump_*.csv,
/// which is cleaned up after the assertion.
#[sqlx::test]
async fn test_dump_to_csv_creates_file(pool: sqlx::PgPool) {
    // Insert at least one row so the CSV is non-trivial.
    store_quote_to_db(&make_quote("AMZN"), &pool)
        .await
        .expect("store failed");

    let result = dump_table_to_csv(&pool).await;
    assert!(
        result.is_ok(),
        "dump_table_to_csv returned an error: {:?}",
        result
    );

    // Find the generated CSV file and clean it up.
    let csv_file = std::fs::read_dir(".")
        .expect("could not read current directory")
        .filter_map(|entry| entry.ok())
        .find(|entry| {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            name_str.starts_with("quotes_dump_") && name_str.ends_with(".csv")
        });

    assert!(
        csv_file.is_some(),
        "no quotes_dump_*.csv file found after dump"
    );

    // Clean up.
    if let Some(file) = csv_file {
        let _ = std::fs::remove_file(file.path());
    }
}

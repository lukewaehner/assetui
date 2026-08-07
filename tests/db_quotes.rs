use assetui::db::quotes::{
    dump_table_to_csv, fetch_all_quotes, prune_quotes_history, store_quote_to_db,
};
use assetui::models::QuoteRecord;
use chrono::{DateTime, TimeZone, Utc};

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

fn quote_at(ticker: &str, price: f64, secs: i64) -> QuoteRecord {
    QuoteRecord {
        ticker: Some(ticker.into()),
        price: Some(price),
        as_of: Some(Utc.timestamp_opt(secs, 0).unwrap()),
        ..Default::default()
    }
}

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

#[sqlx::test]
async fn test_store_rejects_missing_ticker(pool: sqlx::PgPool) {
    let quote = QuoteRecord {
        id: None,
        ticker: None,
        name: Some("Nameless Inc.".to_string()),
        price: Some(10.0),
        previous_close: Some(9.0),
        day_volume: Some(1_000.0),
        as_of: Some(Utc::now()),
    };

    let result = store_quote_to_db(&quote, &pool).await;

    assert!(
        result.is_err(),
        "storing a quote without a ticker should return an error, got {result:?}"
    );
}

#[sqlx::test]
async fn test_dump_to_csv_writes_to_given_path(pool: sqlx::PgPool) {
    store_quote_to_db(&make_quote("AMZN"), &pool)
        .await
        .expect("store failed");

    let out = std::env::temp_dir().join(format!("quotes_dump_test_{}.csv", std::process::id()));
    let _ = std::fs::remove_file(&out);

    let written = dump_table_to_csv(&pool, &out)
        .await
        .expect("dump_table_to_csv returned an error");

    assert_eq!(written, out, "should return the file path it was given");
    assert!(out.exists(), "the CSV file should exist on disk");
    let bytes = std::fs::metadata(&out)
        .expect("CSV file should be readable")
        .len();
    assert!(bytes > 0, "the CSV file should be non-empty");

    let _ = std::fs::remove_file(&out);
}

#[sqlx::test]
async fn prune_keeps_only_the_most_recent_rows(pool: sqlx::PgPool) {
    for i in 0..5 {
        store_quote_to_db(&quote_at("AAPL", 100.0 + i as f64, 1_000 + i), &pool)
            .await
            .unwrap();
    }
    let deleted = prune_quotes_history(&pool, "AAPL", 2).await.unwrap();
    assert_eq!(deleted, 3);

    let remaining: Vec<f64> =
        sqlx::query_scalar("SELECT price FROM quotes WHERE ticker = 'AAPL' ORDER BY as_of ASC")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, vec![103.0, 104.0]);
}

#[sqlx::test]
async fn prune_is_a_noop_when_under_the_cap(pool: sqlx::PgPool) {
    store_quote_to_db(&quote_at("MSFT", 400.0, 2_000), &pool).await.unwrap();
    let deleted = prune_quotes_history(&pool, "MSFT", 200).await.unwrap();
    assert_eq!(deleted, 0);
}

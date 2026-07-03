use chrono::{DateTime, TimeZone, Utc};
use yfinance::db::quotes::{dump_table_to_csv, fetch_all_quotes, store_quote_to_db};
use yfinance::fetch::fetch_sorted;
use yfinance::models::QuoteRecord;
use yfinance::sort::{SortMode, SortOrder};

fn full_quote(
    ticker: &str,
    price: f64,
    prev_close: f64,
    volume: f64,
    as_of: DateTime<Utc>,
) -> QuoteRecord {
    QuoteRecord {
        id: None,
        ticker: Some(ticker.to_string()),
        name: Some(format!("{ticker} Corporation")),
        price: Some(price),
        previous_close: Some(prev_close),
        day_volume: Some(volume),
        as_of: Some(as_of),
    }
}

/// Stores a fully-populated QuoteRecord and then reads it back via
/// fetch_all_quotes, asserting that every field round-trips through Postgres
/// without loss.
#[sqlx::test]
async fn test_store_and_retrieve_field_integrity(pool: sqlx::PgPool) {
    let as_of = Utc.with_ymd_and_hms(2024, 6, 1, 15, 0, 0).unwrap();
    let quote = full_quote("INTL", 350.75, 348.0, 2_500_000.0, as_of);

    let result = store_quote_to_db(&quote, &pool).await;
    assert!(
        result.is_ok(),
        "store_quote_to_db returned an error: {:?}",
        result
    );
    assert!(
        result.unwrap() > 0,
        "expected a positive id for fresh insert"
    );

    let rows = fetch_all_quotes(&pool)
        .await
        .expect("fetch_all_quotes failed");
    assert_eq!(rows.len(), 1, "expected exactly 1 row, got {}", rows.len());

    let row = &rows[0];
    assert_eq!(row.ticker.as_deref(), Some("INTL"), "ticker mismatch");
    assert_eq!(
        row.name.as_deref(),
        Some("INTL Corporation"),
        "name mismatch"
    );
    assert_eq!(row.price, Some(350.75), "price mismatch");
    assert_eq!(row.previous_close, Some(348.0), "previous_close mismatch");
    assert_eq!(row.day_volume, Some(2_500_000.0), "day_volume mismatch");
    assert_eq!(row.as_of, Some(as_of), "as_of mismatch");
}

/// Stores two rows for the same ticker at different timestamps (simulating
/// historical snapshots), verifies both are returned by fetch_all_quotes, and
/// checks that fetch_sorted returns the newer row first.
#[sqlx::test]
async fn test_historical_tracking_same_ticker_two_timestamps(pool: sqlx::PgPool) {
    let t1 = Utc.with_ymd_and_hms(2024, 6, 1, 9, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 6, 1, 10, 0, 0).unwrap();

    let q1 = full_quote("HIST", 100.0, 98.0, 1_000_000.0, t1);
    let q2 = full_quote("HIST", 105.0, 100.0, 1_200_000.0, t2);

    let id1 = store_quote_to_db(&q1, &pool)
        .await
        .expect("first store failed");

    let id2 = store_quote_to_db(&q2, &pool)
        .await
        .expect("second store failed");
    assert_ne!(id1, id2, "two distinct inserts must have different ids");

    let all = fetch_all_quotes(&pool)
        .await
        .expect("fetch_all_quotes failed");
    assert_eq!(
        all.len(),
        2,
        "expected 2 rows for same ticker at different timestamps"
    );

    let recent = fetch_sorted(&pool, SortMode::ByAsOf, SortOrder::Descending, 1)
        .await
        .expect("fetch_sorted failed");
    assert_eq!(recent.len(), 1, "limit of 1 should return exactly 1 row");
    assert_eq!(recent[0].as_of, Some(t2), "most-recent row should be t2");
    assert_eq!(
        recent[0].price,
        Some(105.0),
        "most-recent price should be 105.0"
    );
}

/// Stores two quotes, dumps them to CSV, then reads the generated file back
/// and verifies that both tickers and prices are present in the output.
#[sqlx::test]
async fn test_csv_export_content_matches_stored_data(pool: sqlx::PgPool) {
    let t1 = Utc.with_ymd_and_hms(2024, 7, 1, 9, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 7, 1, 9, 0, 1).unwrap();

    store_quote_to_db(&full_quote("CSVTEST1", 111.11, 110.0, 500_000.0, t1), &pool)
        .await
        .expect("store CSVTEST1 failed");
    store_quote_to_db(&full_quote("CSVTEST2", 222.22, 220.0, 600_000.0, t2), &pool)
        .await
        .expect("store CSVTEST2 failed");

    let csv_path = dump_table_to_csv(&pool, &std::env::temp_dir())
        .await
        .expect("dump_table_to_csv failed");

    // Parse the CSV. QuoteRecord serialises as: id,ticker,name,price,...
    // price is at index 3.
    let mut rdr = csv::Reader::from_path(&csv_path).expect("could not open CSV file");
    let records: Vec<csv::StringRecord> = rdr
        .records()
        .collect::<Result<Vec<_>, _>>()
        .expect("CSV parse error");

    assert_eq!(
        records.len(),
        2,
        "expected 2 data rows in CSV, got {}",
        records.len()
    );

    let tickers: Vec<&str> = records
        .iter()
        .map(|r| r.get(1).expect("missing ticker column"))
        .collect();
    assert!(
        tickers.contains(&"CSVTEST1"),
        "CSVTEST1 not found in CSV tickers"
    );
    assert!(
        tickers.contains(&"CSVTEST2"),
        "CSVTEST2 not found in CSV tickers"
    );

    let prices: Vec<&str> = records
        .iter()
        .map(|r| r.get(3).expect("missing price column"))
        .collect();
    assert!(
        prices.contains(&"111.11"),
        "price 111.11 not found in CSV prices"
    );
    assert!(
        prices.contains(&"222.22"),
        "price 222.22 not found in CSV prices"
    );

    // Clean up regardless of outcome.
    let _ = std::fs::remove_file(&csv_path);
}

/// Exercises the remaining SortMode variants (ByName, ByPrevClose, ByVolume,
/// ByAsOf, ById) using three fixed quotes.
#[sqlx::test]
async fn test_fetch_sorted_remaining_modes(pool: sqlx::PgPool) {
    let t1 = Utc.with_ymd_and_hms(2024, 8, 1, 9, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 8, 1, 9, 0, 1).unwrap();
    let t3 = Utc.with_ymd_and_hms(2024, 8, 1, 9, 0, 2).unwrap();

    store_quote_to_db(&full_quote("ALPHA", 50.0, 49.0, 1_000.0, t1), &pool)
        .await
        .expect("store ALPHA failed");
    store_quote_to_db(&full_quote("BETA", 75.0, 80.0, 5_000.0, t2), &pool)
        .await
        .expect("store BETA failed");
    store_quote_to_db(&full_quote("GAMMA", 60.0, 55.0, 3_000.0, t3), &pool)
        .await
        .expect("store GAMMA failed");

    // ByName ASC: "ALPHA Corporation" < "BETA Corporation" < "GAMMA Corporation"
    let by_name_asc = fetch_sorted(&pool, SortMode::ByName, SortOrder::Ascending, 10)
        .await
        .expect("fetch_sorted ByName ASC failed");
    assert_eq!(by_name_asc.len(), 3);
    assert_eq!(
        by_name_asc[0].name.as_deref(),
        Some("ALPHA Corporation"),
        "ByName ASC: first should be ALPHA Corporation"
    );
    assert_eq!(
        by_name_asc[1].name.as_deref(),
        Some("BETA Corporation"),
        "ByName ASC: second should be BETA Corporation"
    );
    assert_eq!(
        by_name_asc[2].name.as_deref(),
        Some("GAMMA Corporation"),
        "ByName ASC: third should be GAMMA Corporation"
    );

    // ByPrevClose ASC: 49.0 (ALPHA), 55.0 (GAMMA), 80.0 (BETA)
    let by_prev_close_asc = fetch_sorted(&pool, SortMode::ByPrevClose, SortOrder::Ascending, 10)
        .await
        .expect("fetch_sorted ByPrevClose ASC failed");
    assert_eq!(by_prev_close_asc.len(), 3);
    assert_eq!(
        by_prev_close_asc[0].previous_close,
        Some(49.0),
        "ByPrevClose ASC: first should be 49.0 (ALPHA)"
    );
    assert_eq!(
        by_prev_close_asc[1].previous_close,
        Some(55.0),
        "ByPrevClose ASC: second should be 55.0 (GAMMA)"
    );
    assert_eq!(
        by_prev_close_asc[2].previous_close,
        Some(80.0),
        "ByPrevClose ASC: third should be 80.0 (BETA)"
    );

    // ByVolume DESC: 5_000.0 (BETA), 3_000.0 (GAMMA), 1_000.0 (ALPHA)
    let by_volume_desc = fetch_sorted(&pool, SortMode::ByVolume, SortOrder::Descending, 10)
        .await
        .expect("fetch_sorted ByVolume DESC failed");
    assert_eq!(by_volume_desc.len(), 3);
    assert_eq!(
        by_volume_desc[0].day_volume,
        Some(5_000.0),
        "ByVolume DESC: first should be 5000.0 (BETA)"
    );
    assert_eq!(
        by_volume_desc[1].day_volume,
        Some(3_000.0),
        "ByVolume DESC: second should be 3000.0 (GAMMA)"
    );
    assert_eq!(
        by_volume_desc[2].day_volume,
        Some(1_000.0),
        "ByVolume DESC: third should be 1000.0 (ALPHA)"
    );

    // ByAsOf DESC: GAMMA (t3), BETA (t2), ALPHA (t1) - most recent first
    let by_as_of_desc = fetch_sorted(&pool, SortMode::ByAsOf, SortOrder::Descending, 10)
        .await
        .expect("fetch_sorted ByAsOf DESC failed");
    assert_eq!(by_as_of_desc.len(), 3);
    assert_eq!(
        by_as_of_desc[0].ticker.as_deref(),
        Some("GAMMA"),
        "ByAsOf DESC: first should be GAMMA"
    );
    assert_eq!(
        by_as_of_desc[1].ticker.as_deref(),
        Some("BETA"),
        "ByAsOf DESC: second should be BETA"
    );
    assert_eq!(
        by_as_of_desc[2].ticker.as_deref(),
        Some("ALPHA"),
        "ByAsOf DESC: third should be ALPHA"
    );

    // ById ASC: IDs are assigned in insert order → ALPHA < BETA < GAMMA
    let by_id_asc = fetch_sorted(&pool, SortMode::ById, SortOrder::Ascending, 10)
        .await
        .expect("fetch_sorted ById ASC failed");
    assert_eq!(by_id_asc.len(), 3);
    assert_eq!(
        by_id_asc[0].ticker.as_deref(),
        Some("ALPHA"),
        "ById ASC: first should be ALPHA (inserted first)"
    );
}

/// Verifies that SortOrder::Descending and Ascending both work correctly for a
/// string column (name), by using two quotes whose names have an obvious
/// alphabetical order.
#[sqlx::test]
async fn test_fetch_sorted_descending_by_name(pool: sqlx::PgPool) {
    let t1 = Utc.with_ymd_and_hms(2024, 9, 1, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 9, 1, 10, 0, 1).unwrap();

    store_quote_to_db(&full_quote("AAAA", 10.0, 9.0, 100.0, t1), &pool)
        .await
        .expect("store AAAA failed");
    store_quote_to_db(&full_quote("ZZZZ", 20.0, 19.0, 200.0, t2), &pool)
        .await
        .expect("store ZZZZ failed");

    // Descending: "ZZZZ Corporation" should come before "AAAA Corporation".
    let desc = fetch_sorted(&pool, SortMode::ByName, SortOrder::Descending, 10)
        .await
        .expect("fetch_sorted ByName DESC failed");
    assert_eq!(desc.len(), 2);
    assert_eq!(
        desc[0].name.as_deref(),
        Some("ZZZZ Corporation"),
        "ByName DESC: first should be ZZZZ Corporation"
    );
    assert_eq!(
        desc[1].name.as_deref(),
        Some("AAAA Corporation"),
        "ByName DESC: second should be AAAA Corporation"
    );

    // Ascending: "AAAA Corporation" should come first.
    let asc = fetch_sorted(&pool, SortMode::ByName, SortOrder::Ascending, 10)
        .await
        .expect("fetch_sorted ByName ASC failed");
    assert_eq!(asc.len(), 2);
    assert_eq!(
        asc[0].name.as_deref(),
        Some("AAAA Corporation"),
        "ByName ASC: first should be AAAA Corporation"
    );
}

/// Stores 15 quotes, then checks that fetch_all_quotes returns all 15 and that
/// fetch_sorted correctly respects its limit parameter, including when the
/// limit exceeds the total row count.
#[sqlx::test]
async fn test_store_many_then_paginate(pool: sqlx::PgPool) {
    for i in 0u32..15 {
        let ts = Utc.with_ymd_and_hms(2024, 10, 1, 0, 0, i).unwrap();
        let ticker = format!("TICK{i:02}");
        let quote = full_quote(&ticker, 100.0 + f64::from(i), 99.0, 1_000.0, ts);
        store_quote_to_db(&quote, &pool)
            .await
            .unwrap_or_else(|e| panic!("store {ticker} failed: {e}"));
    }

    let all = fetch_all_quotes(&pool)
        .await
        .expect("fetch_all_quotes failed");
    assert_eq!(all.len(), 15, "expected 15 rows after storing 15 quotes");

    let page_5 = fetch_sorted(&pool, SortMode::ByAsOf, SortOrder::Descending, 5)
        .await
        .expect("fetch_sorted(5) failed");
    assert_eq!(
        page_5.len(),
        5,
        "fetch_sorted(5) should return exactly 5 rows"
    );

    let page_15 = fetch_sorted(&pool, SortMode::ByAsOf, SortOrder::Descending, 15)
        .await
        .expect("fetch_sorted(15) failed");
    assert_eq!(
        page_15.len(),
        15,
        "fetch_sorted(15) should return all 15 rows"
    );

    let page_20 = fetch_sorted(&pool, SortMode::ByAsOf, SortOrder::Descending, 20)
        .await
        .expect("fetch_sorted(20) failed");
    assert_eq!(
        page_20.len(),
        15,
        "fetch_sorted(20) should return all 15 rows when limit exceeds count"
    );
}

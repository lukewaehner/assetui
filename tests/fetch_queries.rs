use chrono::{DateTime, TimeZone, Utc};
use assetui::db::quotes::store_quote_to_db;
use assetui::fetch::fetch_sorted;
use assetui::models::QuoteRecord;
use assetui::sort::{SortMode, SortOrder};

fn make_quote(ticker: &str, price: f64, as_of: DateTime<Utc>) -> QuoteRecord {
    QuoteRecord {
        id: None,
        ticker: Some(ticker.to_string()),
        name: Some(format!("{ticker} Inc.")),
        price: Some(price),
        previous_close: Some(price - 1.0),
        day_volume: Some(500_000.0),
        as_of: Some(as_of),
    }
}

#[sqlx::test]
async fn test_fetch_sorted_recent_empty(pool: sqlx::PgPool) {
    let rows = fetch_sorted(&pool, SortMode::ByAsOf, SortOrder::Descending, 10)
        .await
        .unwrap();
    assert!(rows.is_empty(), "expected empty vec from fresh DB");
}

#[sqlx::test]
async fn test_fetch_sorted_recent_respects_limit(pool: sqlx::PgPool) {
    for i in 0..5_u32 {
        let ts = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, i).unwrap();
        let q = make_quote(&format!("TICK{i}"), 100.0 + f64::from(i), ts);
        store_quote_to_db(&q, &pool).await.unwrap();
    }

    let rows = fetch_sorted(&pool, SortMode::ByAsOf, SortOrder::Descending, 3)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3, "fetch_sorted should respect the limit of 3");
}

#[sqlx::test]
async fn test_fetch_sorted_recent_ordering(pool: sqlx::PgPool) {
    let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 1).unwrap();
    let t3 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 2).unwrap();

    store_quote_to_db(&make_quote("FIRST", 10.0, t1), &pool)
        .await
        .unwrap();
    store_quote_to_db(&make_quote("SECOND", 20.0, t2), &pool)
        .await
        .unwrap();
    store_quote_to_db(&make_quote("THIRD", 30.0, t3), &pool)
        .await
        .unwrap();

    let rows = fetch_sorted(&pool, SortMode::ByAsOf, SortOrder::Descending, 3)
        .await
        .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].ticker.as_deref(), Some("THIRD"));
    assert_eq!(rows[1].ticker.as_deref(), Some("SECOND"));
    assert_eq!(rows[2].ticker.as_deref(), Some("FIRST"));
}

#[sqlx::test]
async fn test_fetch_sorted_by_price_asc(pool: sqlx::PgPool) {
    let ts = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();

    store_quote_to_db(&make_quote("MID", 100.0, ts), &pool)
        .await
        .unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 1).unwrap();
    store_quote_to_db(&make_quote("HIGH", 150.0, ts2), &pool)
        .await
        .unwrap();
    let ts3 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 2).unwrap();
    store_quote_to_db(&make_quote("LOW", 50.0, ts3), &pool)
        .await
        .unwrap();

    let rows = fetch_sorted(&pool, SortMode::ByPrice, SortOrder::Ascending, 10)
        .await
        .unwrap();

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].price, Some(50.0));
    assert_eq!(rows[1].price, Some(100.0));
    assert_eq!(rows[2].price, Some(150.0));
}

#[sqlx::test]
async fn test_fetch_sorted_by_price_desc(pool: sqlx::PgPool) {
    let ts = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();

    store_quote_to_db(&make_quote("MID", 100.0, ts), &pool)
        .await
        .unwrap();
    let ts2 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 1).unwrap();
    store_quote_to_db(&make_quote("HIGH", 150.0, ts2), &pool)
        .await
        .unwrap();
    let ts3 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 2).unwrap();
    store_quote_to_db(&make_quote("LOW", 50.0, ts3), &pool)
        .await
        .unwrap();

    let rows = fetch_sorted(&pool, SortMode::ByPrice, SortOrder::Descending, 10)
        .await
        .unwrap();

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].price, Some(150.0));
    assert_eq!(rows[1].price, Some(100.0));
    assert_eq!(rows[2].price, Some(50.0));
}

#[sqlx::test]
async fn test_fetch_sorted_by_ticker_asc(pool: sqlx::PgPool) {
    let t1 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 1).unwrap();

    store_quote_to_db(&make_quote("ZZZ", 99.0, t1), &pool)
        .await
        .unwrap();
    store_quote_to_db(&make_quote("AAA", 88.0, t2), &pool)
        .await
        .unwrap();

    let rows = fetch_sorted(&pool, SortMode::ByTicker, SortOrder::Ascending, 10)
        .await
        .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].ticker.as_deref(), Some("AAA"));
    assert_eq!(rows[1].ticker.as_deref(), Some("ZZZ"));
}

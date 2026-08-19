use assetui::db::quotes::store_quote_to_db;
use assetui::fetch::fetch_latest_per_ticker;
use assetui::models::QuoteRecord;
use assetui::sort::{SortMode, SortOrder};
use chrono::{TimeZone, Utc};

/// Construct a `QuoteRecord` suitable for TUI workflow tests.
fn tui_quote(ticker: &str, price: f64, seq: u32) -> QuoteRecord {
    let minute = seq / 60;
    let second = seq % 60;
    QuoteRecord {
        id: None,
        ticker: Some(ticker.to_string()),
        name: Some(format!("{ticker} Inc.")),
        price: Some(price),
        previous_close: Some(price - 2.0),
        day_volume: Some(1_000_000.0),
        as_of: Some(Utc.with_ymd_and_hms(2024, 1, 1, 0, minute, second).unwrap()),
    }
}

#[sqlx::test]
async fn test_tui_initial_load_returns_recent_200(pool: sqlx::PgPool) {
    for seq in 0u32..10 {
        let q = tui_quote(&format!("TICK{seq:02}"), 100.0 + f64::from(seq), seq);
        store_quote_to_db(&q, &pool).await.unwrap();
    }

    let rows = fetch_latest_per_ticker(&pool, SortMode::ByAsOf, SortOrder::Descending, 200)
        .await
        .unwrap();

    assert_eq!(rows.len(), 10, "all 10 quotes should be returned");

    assert_eq!(
        rows[0].ticker.as_deref(),
        Some("TICK09"),
        "most recent quote should be first"
    );
    assert_eq!(
        rows[9].ticker.as_deref(),
        Some("TICK00"),
        "oldest quote should be last"
    );
}

#[sqlx::test]
async fn test_tui_new_fetch_appears_at_top_of_recent(pool: sqlx::PgPool) {
    for seq in 0u32..5 {
        let q = tui_quote(&format!("OLD{seq}"), 50.0 + f64::from(seq), seq);
        store_quote_to_db(&q, &pool).await.unwrap();
    }

    let new_quote = tui_quote("NEWT", 999.0, 100);
    store_quote_to_db(&new_quote, &pool).await.unwrap();

    let rows = fetch_latest_per_ticker(&pool, SortMode::ByAsOf, SortOrder::Descending, 200)
        .await
        .unwrap();

    assert_eq!(rows.len(), 6, "should have 6 rows total");
    assert_eq!(
        rows[0].ticker.as_deref(),
        Some("NEWT"),
        "the newly fetched ticker must appear at the top"
    );
}

#[sqlx::test]
async fn test_tui_sort_change_reloads_correctly(pool: sqlx::PgPool) {
    store_quote_to_db(&tui_quote("APPLE", 200.0, 0), &pool)
        .await
        .unwrap();
    store_quote_to_db(&tui_quote("MANGO", 50.0, 1), &pool)
        .await
        .unwrap();
    store_quote_to_db(&tui_quote("CHERRY", 125.0, 2), &pool)
        .await
        .unwrap();

    let by_ticker = fetch_latest_per_ticker(&pool, SortMode::ByTicker, SortOrder::Ascending, 200)
        .await
        .unwrap();

    assert_eq!(by_ticker.len(), 3);
    assert_eq!(
        by_ticker[0].ticker.as_deref(),
        Some("APPLE"),
        "APPLE first alphabetically"
    );
    assert_eq!(
        by_ticker[1].ticker.as_deref(),
        Some("CHERRY"),
        "CHERRY second alphabetically"
    );
    assert_eq!(
        by_ticker[2].ticker.as_deref(),
        Some("MANGO"),
        "MANGO last alphabetically"
    );

    let by_price_desc =
        fetch_latest_per_ticker(&pool, SortMode::ByPrice, SortOrder::Descending, 200)
            .await
            .unwrap();

    assert_eq!(by_price_desc.len(), 3);
    assert_eq!(
        by_price_desc[0].price,
        Some(200.0),
        "APPLE (200) should be first"
    );
    assert_eq!(
        by_price_desc[1].price,
        Some(125.0),
        "CHERRY (125) should be second"
    );
    assert_eq!(
        by_price_desc[2].price,
        Some(50.0),
        "MANGO (50) should be last"
    );
}

#[sqlx::test]
async fn test_tui_sort_toggle_reverses_order(pool: sqlx::PgPool) {
    let prices = [10.0_f64, 40.0, 30.0, 20.0];
    for (seq, price) in prices.iter().enumerate() {
        let q = tui_quote(&format!("S{seq}"), *price, seq as u32);
        store_quote_to_db(&q, &pool).await.unwrap();
    }

    let asc = fetch_latest_per_ticker(&pool, SortMode::ByPrice, SortOrder::Ascending, 10)
        .await
        .unwrap();

    assert_eq!(asc.len(), 4);
    let asc_prices: Vec<f64> = asc.iter().map(|r| r.price.unwrap()).collect();
    assert_eq!(
        asc_prices,
        vec![10.0, 20.0, 30.0, 40.0],
        "ascending price order"
    );

    let desc = fetch_latest_per_ticker(&pool, SortMode::ByPrice, SortOrder::Descending, 10)
        .await
        .unwrap();

    assert_eq!(desc.len(), 4);
    let desc_prices: Vec<f64> = desc.iter().map(|r| r.price.unwrap()).collect();
    assert_eq!(
        desc_prices,
        vec![40.0, 30.0, 20.0, 10.0],
        "descending price order"
    );
}

#[sqlx::test]
async fn test_tui_page_limit_matches_startup(pool: sqlx::PgPool) {
    for seq in 0u32..250 {
        let hour = seq / 3600;
        let minute = (seq % 3600) / 60;
        let second = seq % 60;
        let as_of = Utc
            .with_ymd_and_hms(2024, 1, 1, hour, minute, second)
            .unwrap();
        let q = QuoteRecord {
            id: None,
            ticker: Some(format!("T{seq:03}")),
            name: Some(format!("T{seq:03} Inc.")),
            price: Some(f64::from(seq)),
            previous_close: Some(f64::from(seq) - 2.0),
            day_volume: Some(1_000_000.0),
            as_of: Some(as_of),
        };
        store_quote_to_db(&q, &pool).await.unwrap();
    }

    let rows = fetch_latest_per_ticker(&pool, SortMode::ByAsOf, SortOrder::Descending, 200)
        .await
        .unwrap();

    assert_eq!(
        rows.len(),
        200,
        "fetch_latest_per_ticker must respect the 200-row limit"
    );

    assert_eq!(
        rows[0].ticker.as_deref(),
        Some("T249"),
        "seq=249 (most recent) must be at index 0"
    );
}

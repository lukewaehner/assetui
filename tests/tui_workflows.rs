use chrono::{TimeZone, Utc};
use yfinance::db::quotes::store_quote_to_db;
use yfinance::fetch::{fetch_recent, fetch_sorted};
use yfinance::models::QuoteRecord;
use yfinance::sort::{SortMode, SortOrder};

/// Construct a `QuoteRecord` suitable for TUI workflow tests.
///
/// `seq` is used to derive a unique, ordered timestamp: it is interpreted as
/// a number of seconds past `2024-01-01T00:00:00Z`, wrapping through minutes
/// and hours so that seq=0 is the oldest and increasing seq values produce
/// strictly newer timestamps.  Valid range: 0..3600 (one hour of seconds).
fn tui_quote(ticker: &str, price: f64, seq: u32) -> QuoteRecord {
    // Spread seq across minutes and seconds so values beyond 59 stay valid.
    let minute = seq / 60;
    let second = seq % 60;
    QuoteRecord {
        id: None,
        ticker: Some(ticker.to_string()),
        name: Some(format!("{ticker} Inc.")),
        price: Some(price),
        previous_close: Some(price - 2.0),
        day_volume: Some(1_000_000.0),
        as_of: Some(
            Utc.with_ymd_and_hms(2024, 1, 1, 0, minute, second)
                .unwrap(),
        ),
    }
}

// ---------------------------------------------------------------------------
// Test 1 – startup load returns all rows when under the 200-row page limit
// ---------------------------------------------------------------------------

/// The TUI calls `fetch_recent(&pool, 200)` on startup to populate the
/// initial table.  Verify that when fewer than 200 rows exist every row is
/// returned and they arrive newest-first.
#[sqlx::test]
async fn test_tui_initial_load_returns_recent_200(pool: sqlx::PgPool) {
    // Insert 10 quotes with monotonically increasing timestamps (seq 0..10).
    for seq in 0u32..10 {
        let q = tui_quote(&format!("TICK{seq:02}"), 100.0 + f64::from(seq), seq);
        store_quote_to_db(&q, &pool).await.unwrap();
    }

    // Simulate TUI startup.
    let rows = fetch_recent(&pool, 200).await.unwrap();

    // All 10 rows should be present (200-row limit not reached).
    assert_eq!(rows.len(), 10, "all 10 quotes should be returned");

    // Newest first: seq=9 has the largest timestamp so it must be at index 0.
    assert_eq!(
        rows[0].ticker.as_deref(),
        Some("TICK09"),
        "most recent quote should be first"
    );
    // Oldest last: seq=0.
    assert_eq!(
        rows[9].ticker.as_deref(),
        Some("TICK00"),
        "oldest quote should be last"
    );
}

// ---------------------------------------------------------------------------
// Test 2 – a newly fetched ticker appears at the top of the recent list
// ---------------------------------------------------------------------------

/// When a user types a ticker in the TUI's input box the app calls
/// `fetch_quote_and_store`, then refreshes with `fetch_recent`.  The new
/// quote must appear at position 0.
#[sqlx::test]
async fn test_tui_new_fetch_appears_at_top_of_recent(pool: sqlx::PgPool) {
    // Simulate existing history: 5 quotes at seq 0..5.
    for seq in 0u32..5 {
        let q = tui_quote(&format!("OLD{seq}"), 50.0 + f64::from(seq), seq);
        store_quote_to_db(&q, &pool).await.unwrap();
    }

    // Simulate a user fetching a new ticker: seq=100 is well after the others
    // (minute=1, second=40 → 2024-01-01T00:01:40Z).
    let new_quote = tui_quote("NEWT", 999.0, 100);
    store_quote_to_db(&new_quote, &pool).await.unwrap();

    // Refresh the TUI table.
    let rows = fetch_recent(&pool, 200).await.unwrap();

    assert_eq!(rows.len(), 6, "should have 6 rows total");
    assert_eq!(
        rows[0].ticker.as_deref(),
        Some("NEWT"),
        "the newly fetched ticker must appear at the top"
    );
}

// ---------------------------------------------------------------------------
// Test 3 – changing the sort mode reloads correctly
// ---------------------------------------------------------------------------

/// The TUI lets the user press 't' (sort by ticker) or 'p' (sort by price).
/// Each keypress calls `fetch_sorted` with the new `SortMode` and current
/// `SortOrder`.  Verify both orderings produce the expected row sequence.
#[sqlx::test]
async fn test_tui_sort_change_reloads_correctly(pool: sqlx::PgPool) {
    // Three quotes with distinct tickers and prices, inserted in seq order.
    store_quote_to_db(&tui_quote("APPLE", 200.0, 0), &pool)
        .await
        .unwrap();
    store_quote_to_db(&tui_quote("MANGO", 50.0, 1), &pool)
        .await
        .unwrap();
    store_quote_to_db(&tui_quote("CHERRY", 125.0, 2), &pool)
        .await
        .unwrap();

    // User presses 't' → sort by ticker ascending.
    let by_ticker = fetch_sorted(&pool, SortMode::ByTicker, SortOrder::Ascending, 200)
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

    // User presses 'p' then 'o' → sort by price descending.
    let by_price_desc = fetch_sorted(&pool, SortMode::ByPrice, SortOrder::Descending, 200)
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

// ---------------------------------------------------------------------------
// Test 4 – toggling sort order reverses the result
// ---------------------------------------------------------------------------

/// The TUI's 'o' key toggles between `Ascending` and `Descending`.  After
/// each toggle the app calls `fetch_sorted` with the flipped `SortOrder`.
/// Verify that ascending and descending produce mirror-image orderings.
#[sqlx::test]
async fn test_tui_sort_toggle_reverses_order(pool: sqlx::PgPool) {
    let prices = [10.0_f64, 40.0, 30.0, 20.0];
    for (seq, price) in prices.iter().enumerate() {
        let q = tui_quote(&format!("S{seq}"), *price, seq as u32);
        store_quote_to_db(&q, &pool).await.unwrap();
    }

    // First fetch: price ascending.
    let asc = fetch_sorted(&pool, SortMode::ByPrice, SortOrder::Ascending, 10)
        .await
        .unwrap();

    assert_eq!(asc.len(), 4);
    let asc_prices: Vec<f64> = asc.iter().map(|r| r.price.unwrap()).collect();
    assert_eq!(
        asc_prices,
        vec![10.0, 20.0, 30.0, 40.0],
        "ascending price order"
    );

    // User presses 'o': price descending.
    let desc = fetch_sorted(&pool, SortMode::ByPrice, SortOrder::Descending, 10)
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

// ---------------------------------------------------------------------------
// Test 5 – page limit matches the TUI startup page size
// ---------------------------------------------------------------------------

/// The TUI hard-codes a 200-row startup page.  When more than 200 rows exist
/// `fetch_recent` must return exactly 200, and those 200 must be the most
/// recently inserted ones.
#[sqlx::test]
async fn test_tui_page_limit_matches_startup(pool: sqlx::PgPool) {
    // Insert 250 quotes.  Timestamps are spread across hours, minutes, and
    // seconds so that every (ticker, as_of) pair is unique and seq order
    // maps cleanly to chronological order.
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

    // Simulate TUI startup with a 200-row page.
    let rows = fetch_recent(&pool, 200).await.unwrap();

    assert_eq!(rows.len(), 200, "fetch_recent must respect the 200-row limit");

    // The most recent quote has seq=249 → ticker "T249".
    assert_eq!(
        rows[0].ticker.as_deref(),
        Some("T249"),
        "seq=249 (most recent) must be at index 0"
    );
}

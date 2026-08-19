//! Database-facing behavior of the headless snapshot run.

use assetui::db::portfolio_history::load_history;
use assetui::models::QuoteRecord;
use assetui::portfolio::Holding;
use assetui::snapshot::record_history_day;
use chrono::NaiveDate;

fn holding(ticker: &str, shares: f64, avg_cost: f64) -> Holding {
    Holding {
        ticker: ticker.to_string(),
        shares,
        avg_cost,
    }
}

fn priced_row(ticker: &str, price: f64) -> QuoteRecord {
    QuoteRecord {
        ticker: Some(ticker.to_string()),
        price: Some(price),
        ..Default::default()
    }
}

fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 8, 19).unwrap()
}

#[sqlx::test]
async fn record_history_day_writes_the_days_totals(pool: sqlx::PgPool) {
    // arrange
    let holdings = vec![holding("AAPL", 2.0, 100.0)];
    let rows = vec![priced_row("AAPL", 150.0)];

    // act
    let written = record_history_day(&pool, &holdings, &rows, day())
        .await
        .expect("record_history_day should succeed");

    // assert
    assert_eq!(written, Some((300.0, 200.0)));
    let history = load_history(&pool).await.expect("load_history failed");
    assert_eq!(history, vec![(day(), 300.0, 200.0)]);
}

#[sqlx::test]
async fn record_history_day_writes_nothing_when_a_holding_is_unpriced(pool: sqlx::PgPool) {
    let holdings = vec![holding("AAPL", 2.0, 100.0), holding("MSFT", 1.0, 50.0)];
    let rows = vec![priced_row("AAPL", 150.0)];

    let written = record_history_day(&pool, &holdings, &rows, day())
        .await
        .expect("record_history_day should succeed");

    assert_eq!(written, None);
    let history = load_history(&pool).await.expect("load_history failed");
    assert!(history.is_empty(), "expected no row, got {history:?}");
}

#[sqlx::test]
async fn record_history_day_writes_nothing_for_an_empty_portfolio(pool: sqlx::PgPool) {
    let written = record_history_day(&pool, &[], &[], day())
        .await
        .expect("record_history_day should succeed");

    assert_eq!(written, None);
}

#[sqlx::test]
async fn record_history_day_replaces_an_earlier_row_for_the_same_day(pool: sqlx::PgPool) {
    let holdings = vec![holding("AAPL", 2.0, 100.0)];
    record_history_day(&pool, &holdings, &[priced_row("AAPL", 150.0)], day())
        .await
        .expect("first write should succeed");

    record_history_day(&pool, &holdings, &[priced_row("AAPL", 160.0)], day())
        .await
        .expect("second write should succeed");

    let history = load_history(&pool).await.expect("load_history failed");
    assert_eq!(history, vec![(day(), 320.0, 200.0)]);
}

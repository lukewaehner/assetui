//! Table rendering for the CLI binary.

use crate::{AppError, db::quotes::fetch_all_quotes, models::QuoteRecord};
use comfy_table::{Cell, Color, Table, presets::UTF8_FULL};
use sqlx::{Pool, Postgres};

fn opt_fmt<T, F: Fn(T) -> String>(v: Option<T>, f: F) -> String {
    v.map(f).unwrap_or_else(|| "N/A".to_string())
}

/// Formats a single [`QuoteRecord`] as a row of [`Cell`]s for comfy-table.
///
/// The price cell is coloured green when the price is above the previous
/// close, red when below, and unstyled when the comparison cannot be made.
fn ticker_row(qr: &QuoteRecord) -> Vec<Cell> {
    let ticker = qr.ticker.as_deref().unwrap_or("-");
    let name = qr.name.as_deref().unwrap_or("Unknown");
    let price_str = opt_fmt(qr.price, |p| format!("${p:.2}"));
    let prev_close_str = opt_fmt(qr.previous_close, |p| format!("${p:.2}"));
    let volume_str = opt_fmt(qr.day_volume, |v| format!("{v:.2}"));
    let as_of_str = opt_fmt(qr.as_of, |dt| {
        dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
    });

    let price_cell = match (qr.price, qr.previous_close) {
        (Some(p), Some(pc)) if p > pc => Cell::new(price_str).fg(Color::Green),
        (Some(p), Some(pc)) if p < pc => Cell::new(price_str).fg(Color::Red),
        _ => Cell::new(price_str),
    };

    vec![
        Cell::new(ticker).fg(Color::Cyan),
        Cell::new(name),
        price_cell,
        Cell::new(prev_close_str),
        Cell::new(volume_str),
        Cell::new(as_of_str),
    ]
}

/// Fetches all quotes from the database and prints them as a UTF-8 table to
/// stdout.
pub async fn print_tickers(p: &Pool<Postgres>) -> Result<(), AppError> {
    let rows = fetch_all_quotes(p).await?;

    let mut table = Table::new();
    table.load_preset(UTF8_FULL).set_header(vec![
        "Ticker",
        "Name",
        "Price",
        "Prev Close",
        "Day Volume",
        "As Of",
    ]);

    for row in &rows {
        table.add_row(ticker_row(row));
    }

    println!("{table}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ticker_row;
    use crate::models::QuoteRecord;

    #[test]
    fn test_ticker_row_returns_five_cells() {
        let qr = QuoteRecord {
            id: Some(1),
            ticker: Some("AAPL".to_string()),
            name: Some("Apple Inc.".to_string()),
            price: Some(150.0),
            previous_close: Some(148.0),
            day_volume: Some(1_000_000.0),
            as_of: Some(chrono::Utc::now()),
        };
        let cells = ticker_row(&qr);
        assert_eq!(cells.len(), 6);
    }

    #[test]
    fn test_ticker_row_handles_all_none() {
        let qr = QuoteRecord::default();
        let cells = ticker_row(&qr);
        assert_eq!(cells.len(), 6);
    }

    #[test]
    fn test_ticker_row_price_above_prev_close() {
        let qr = QuoteRecord {
            id: None,
            ticker: Some("TSLA".to_string()),
            name: Some("Tesla Inc.".to_string()),
            price: Some(100.0),
            previous_close: Some(90.0),
            day_volume: Some(500_000.0),
            as_of: None,
        };
        let cells = ticker_row(&qr);
        assert_eq!(cells.len(), 6);
    }

    #[test]
    fn test_ticker_row_price_below_prev_close() {
        let qr = QuoteRecord {
            id: None,
            ticker: Some("TSLA".to_string()),
            name: Some("Tesla Inc.".to_string()),
            price: Some(80.0),
            previous_close: Some(90.0),
            day_volume: Some(500_000.0),
            as_of: None,
        };
        let cells = ticker_row(&qr);
        assert_eq!(cells.len(), 6);
    }
}

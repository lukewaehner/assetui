//! Table rendering for the CLI binary.

use crate::{db::quotes::fetch_all_quotes, models::QuoteRecord};
use comfy_table::{Cell, Color, Table, presets::UTF8_FULL};
use sqlx::{Pool, Postgres};

/// Formats a single [`QuoteRecord`] as a row of [`Cell`]s for comfy-table.
///
/// The price cell is coloured green when the price is above the previous
/// close, red when below, and unstyled when the comparison cannot be made.
fn ticker_row(qr: &QuoteRecord) -> Vec<Cell> {
    let name = qr.name.as_deref().unwrap_or("Unknown");

    let price_str = qr
        .price
        .map(|p| format!("${:.2}", p))
        .unwrap_or_else(|| "N/A".to_string());

    let prev_close_str = qr
        .previous_close
        .map(|p| format!("${:.2}", p))
        .unwrap_or_else(|| "N/A".to_string());

    let volume_str = qr
        .day_volume
        .map(|v| format!("{:.2}", v))
        .unwrap_or_else(|| "N/A".to_string());

    let as_of_str = qr
        .as_of
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "N/A".to_string());

    let price_cell = match (qr.price, qr.previous_close) {
        (Some(p), Some(pc)) if p > pc => Cell::new(price_str).fg(Color::Green),
        (Some(p), Some(pc)) if p < pc => Cell::new(price_str).fg(Color::Red),
        _ => Cell::new(price_str),
    };

    vec![
        Cell::new(name).fg(Color::Cyan),
        price_cell,
        Cell::new(prev_close_str),
        Cell::new(volume_str),
        Cell::new(as_of_str),
    ]
}

/// Fetches all quotes from the database and prints them as a UTF-8 table to
/// stdout.
pub async fn print_tickers(p: &Pool<Postgres>) {
    let rows = fetch_all_quotes(p).await.unwrap();

    let mut table = Table::new();
    table.load_preset(UTF8_FULL).set_header(vec![
        "Ticker",
        "Price",
        "Prev Close",
        "Day Volume",
        "As Of",
    ]);

    for row in &rows {
        table.add_row(ticker_row(row));
    }

    println!("{table}");
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
        assert_eq!(cells.len(), 5);
    }

    #[test]
    fn test_ticker_row_handles_all_none() {
        let qr = QuoteRecord::default();
        let cells = ticker_row(&qr);
        assert_eq!(cells.len(), 5);
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
        assert_eq!(cells.len(), 5);
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
        assert_eq!(cells.len(), 5);
    }
}

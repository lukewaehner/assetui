use crate::{db::quotes::fetch_all_quotes, models::QuoteRecord};
use comfy_table::{Cell, Color, Table, presets::UTF8_FULL};
use sqlx::{Pool, Postgres};

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

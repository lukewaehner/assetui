use sqlx::{Pool, Postgres};
use tracing::{debug, error, info, warn};
use yfinance_rs::{Ticker, YfClient};

use crate::cli::pick_tickers;
use crate::db::quotes::store_quote_to_db;
use crate::fetch::{fetch_quote, prepare_tickers};
use crate::models::QuoteRecord;

pub async fn fetch_and_store(pool: &Pool<Postgres>) -> Result<(), Box<dyn std::error::Error>> {
    let tickers: Vec<String> = pick_tickers();

    let buffer: usize = 100;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<QuoteRecord>(buffer);

    let client: YfClient = YfClient::default();

    let tickers: Vec<(String, Ticker)> = prepare_tickers(&tickers, &client);
    info!(count = tickers.len(), "spawning fetch tasks");

    for (symbol, t) in tickers {
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            debug!(%symbol, "fetch task started");
            match fetch_quote(&symbol, &t).await {
                Ok(Some(quote)) => {
                    if let Err(e) = tx_clone.send(quote).await {
                        error!(%symbol, error = %e, "failed to send quote to channel");
                    } else {
                        debug!(%symbol, "quote forwarded to channel");
                    }
                }
                Ok(None) => warn!(%symbol, "no quote returned"),
                Err(e) => error!(%symbol, error = %e, "fetch failed"),
            }
        });
    }

    drop(tx);

    let mut stored = 0usize;
    let mut failed = 0usize;
    while let Some(quote) = rx.recv().await {
        match store_quote_to_db(&quote, pool).await {
            Ok(()) => {
                stored += 1;
                info!(name = ?quote.name.unwrap_or_default(), "stored quote");
            }
            Err(e) => {
                failed += 1;
                error!(name = ?quote.name.unwrap_or_default(), error = %e, "store failed");
            }
        }
    }

    info!(stored, failed, "yfinance run complete");
    Ok(())
}

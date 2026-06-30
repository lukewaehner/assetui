//! Async batch-fetch pipeline.
//!
//! Spawns one Tokio task per ticker, funnels results through an mpsc channel,
//! then drains the channel and writes each quote to the database serially.
//! This keeps DB writes on a single code path while letting all Yahoo Finance
//! HTTP calls overlap.

use sqlx::{Pool, Postgres};
use tracing::{debug, error, info, warn};
use yfinance_rs::{Ticker, YfClient};

use crate::db::quotes::store_quote_to_db;
use crate::fetch::{fetch_quote, prepare_tickers};
use crate::models::QuoteRecord;

/// Fetches real-time quotes for all `tickers` concurrently and stores each
/// one to the database.
///
/// A channel with a buffer of 100 decouples the fetch tasks from the write
/// loop, so the fetch tasks can proceed without waiting for each DB insert.
/// Errors on individual tickers are logged and counted but do not abort the
/// run; the function only returns `Err` on unrecoverable setup failures.
pub async fn fetch_and_store(
    pool: &Pool<Postgres>,
    tickers: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let buffer: usize = 100;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<QuoteRecord>(buffer);

    let client: YfClient = YfClient::default();

    let tickers: Vec<(String, Ticker)> = prepare_tickers(tickers, &client);
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

    // Drop the original sender so the channel closes once all spawned tasks
    // finish, which lets the recv loop below terminate naturally.
    drop(tx);

    let mut stored = 0usize;
    let mut failed = 0usize;
    while let Some(quote) = rx.recv().await {
        match store_quote_to_db(&quote, pool).await {
            Ok(id) => {
                stored += 1;
                info!(ticker = ?quote.ticker.unwrap_or_default(), ?id, "stored quote");
            }
            Err(e) => {
                failed += 1;
                error!(ticker = ?quote.ticker.unwrap_or_default(), error = %e, "store failed");
            }
        }
    }

    info!(stored, failed, "yfinance run complete");
    Ok(())
}

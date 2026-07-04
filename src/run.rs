//! Async batch-fetch pipeline.
//!
//! Spawns one Tokio task per ticker, funnels results through an mpsc channel,
//! then drains the channel and writes each quote to the database serially.
//! This keeps DB writes on a single code path while letting all Yahoo Finance
//! HTTP calls overlap.

use std::sync::Arc;

use sqlx::{Pool, Postgres};
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};
use yfinance_rs::{Ticker, YfClient};

use crate::AppError;
use crate::db::quotes::store_quote_to_db;
use crate::fetch::{fetch_quote, prepare_tickers};
use crate::models::QuoteRecord;

const CHANNEL_BUFFER: usize = 100;

/// Maximum number of Yahoo Finance requests in flight at once.  Keeps large
/// ticker lists from opening hundreds of simultaneous connections (and from
/// tripping rate limits).
const MAX_CONCURRENT_FETCHES: usize = 12;

/// Fetches real-time quotes for all `tickers` concurrently and stores each
/// one to the database.
///
/// A channel with a buffer of [`CHANNEL_BUFFER`] decouples the fetch tasks
/// from the write loop, so the fetch tasks can proceed without waiting for
/// each DB insert.  Concurrency is capped at [`MAX_CONCURRENT_FETCHES`] via a
/// semaphore.  Errors on individual tickers are logged and counted but do not
/// abort the run; the function only returns `Err` on unrecoverable setup
/// failures.  A warning is logged when any individual tickers fail.
pub async fn fetch_and_store(
    pool: &Pool<Postgres>,
    client: &YfClient,
    tickers: &[String],
) -> Result<(), AppError> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<QuoteRecord>(CHANNEL_BUFFER);

    let tickers: Vec<(String, Ticker)> = prepare_tickers(tickers, client);
    info!(count = tickers.len(), "spawning fetch tasks");

    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_FETCHES));
    let mut handles = Vec::with_capacity(tickers.len());
    for (symbol, t) in tickers {
        let tx_clone = tx.clone();
        let semaphore = Arc::clone(&semaphore);
        let handle = tokio::spawn({
            let symbol = symbol.clone();
            async move {
                // The semaphore is never closed, so acquire only fails if it
                // is dropped mid-run - treat that as a cancelled task.
                let Ok(_permit) = semaphore.acquire_owned().await else {
                    return;
                };
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
            }
        });
        handles.push((symbol, handle));
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
                info!(ticker = %quote.ticker.as_deref().unwrap_or("<none>"), id, "stored quote");
            }
            Err(e) => {
                failed += 1;
                error!(ticker = %quote.ticker.as_deref().unwrap_or("<none>"), error = %e, "store failed");
            }
        }
    }

    for (symbol, handle) in handles {
        if let Err(e) = handle.await {
            error!(%symbol, "fetch task panicked: {e}");
        }
    }

    if failed > 0 {
        warn!(stored, failed, "assetui run complete with errors");
    } else {
        info!(stored, "assetui run complete");
    }
    Ok(())
}

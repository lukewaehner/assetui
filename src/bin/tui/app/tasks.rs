//! Async task spawners for [`App`].
//!
//! Every method here follows the same shape: clone `event_tx` (and any state
//! the task needs), `tokio::spawn` a future, and report results back to the
//! event loop as [`AppEvent`]s. Grouping them isolates the async layer from the
//! synchronous state, input, and rendering logic in the parent module.

use std::time::Duration;

use assetui::fetch::{fetch_analysis, fetch_chart_data, fetch_quote_and_store, fetch_sorted};
use assetui::models::QuoteTick;
use assetui::stream::start_quote_stream;

use crate::theme::{Appearance, parse_appearance};

use super::{App, AppEvent, PAGE_FETCH_LIMIT};

impl App {
    /// Spawns a task to re-fetch the current page with the active sort applied,
    /// then sends a [`AppEvent::PageLoaded`] back when done.
    pub(crate) fn spawn_reload(&self) {
        let pool = self.pool.clone();
        let tx = self.event_tx.clone();
        let mode = self.db_display.sort_mode;
        let order = self.db_display.sort_order;
        tokio::spawn(async move {
            match fetch_sorted(&pool, mode, order, PAGE_FETCH_LIMIT).await {
                Ok(rows) => {
                    let _ = tx.send(AppEvent::PageLoaded(rows));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to fetch quotes: {e}")));
                }
            }
        });
    }

    /// Spawns a task to fetch and store a quote for `symbol`, reporting
    /// progress and the result back as [`AppEvent`]s.
    pub(crate) fn spawn_fetch(&self, symbol: String) {
        let tx = self.event_tx.clone();
        let pool = self.pool.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            let _ = tx.send(AppEvent::FetchSpawned(symbol.clone()));
            match fetch_quote_and_store(&pool, &client, &symbol).await {
                Ok(Some(record)) => {
                    let _ = tx.send(AppEvent::FetchCompleted(record));
                }
                Ok(None) => {
                    let _ = tx.send(AppEvent::LogLine(format!(
                        "[ERROR] no quote found for {symbol}"
                    )));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("{symbol}: {e}")));
                }
            }
        });
    }

    /// Spawns a background task to fetch analyst data for `symbol` and sends
    /// the result back as [`AppEvent::StockAnalysisReady`].
    pub(crate) fn spawn_analysis(&self, symbol: &str) {
        let tx = self.event_tx.clone();
        let client = self.client.clone();
        let symbol = symbol.to_owned();
        tokio::spawn(async move {
            match fetch_analysis(&client, &symbol).await {
                Ok(a) => {
                    let _ = tx.send(AppEvent::StockAnalysisReady(a));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("analysis {symbol}: {e}")));
                }
            }
        });
    }

    /// Spawns a background task to fetch price history for `symbol` and sends
    /// the candles back as [`AppEvent::ChartDataReady`].
    pub(crate) fn spawn_chart_data(&self, symbol: &str) {
        let tx = self.event_tx.clone();
        let client = self.client.clone();
        let symbol = symbol.to_owned();
        tokio::spawn(async move {
            match fetch_chart_data(&client, &symbol).await {
                Ok(a) => {
                    let _ = tx.send(AppEvent::ChartDataReady(a));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("chart {symbol}: {e}")));
                }
            }
        });
    }

    /// Spawns a task to start a live quote stream for symbols, sends incoming
    /// ticks back as [`AppEvent::QuoteTick`]s, and logs the stream lifecycle.
    pub(crate) fn spawn_stream(&self, symbols: Vec<String>) {
        let tx = self.event_tx.clone();
        let client = self.client.clone();
        let count = symbols.len();
        tokio::spawn(async move {
            match start_quote_stream(&client, symbols).await {
                Ok(Some((handle, mut receiver))) => {
                    let _ = tx.send(AppEvent::StreamStarted(handle));
                    let _ = tx.send(AppEvent::LogLine(format!(
                        "[STREAM] connected: {count} tickers"
                    )));
                    while let Some(update) = receiver.recv().await {
                        if tx
                            .send(AppEvent::QuoteTick(QuoteTick::from(update)))
                            .is_err()
                        {
                            break; // Event loop is gone
                        };
                    }
                    let _ = tx.send(AppEvent::LogLine("[STREAM] ended stream".to_string()));
                }
                Ok(None) => {
                    let _ = tx.send(AppEvent::LogLine(
                        "[STREAM] no symbols to stream".to_string(),
                    ));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("stream error: {e}")));
                }
            }
        });
    }

    /// Restarts the quote stream when the visible tickers differ from the
    /// currently-subscribed set, aborting the previous stream first. The
    /// symbol-set equality check makes this a no-op when nothing changed, so it
    /// is safe to call from every window-change site.
    ///
    // TODO(STREAM): debounce if pagination thrash causes reconnect spam.
    pub(crate) fn resubscribe_stream(&mut self) {
        let tickers = self.db_display.window_tickers();
        if self.subscribed_symbols == tickers {
            return;
        }
        if let Some(handle) = self.stream_handle.take() {
            handle.abort();
        }
        self.subscribed_symbols = tickers.clone();
        self.spawn_stream(tickers);
    }

    /// Spawns a background task that polls the macOS system appearance every two
    /// seconds and sends [`AppEvent::ThemeChanged`] whenever it flips. The
    /// blocking `defaults` call runs off the render loop.
    ///
    /// A no-op on other platforms, where there is no `defaults` binary to
    /// poll - the theme stays at whatever `detect_appearance` chose at startup.
    pub(crate) fn spawn_theme_watcher(&self) {
        if !cfg!(target_os = "macos") {
            return;
        }
        let tx = self.event_tx.clone();
        // Seed from the appearance already detected in `App::new` so startup
        // performs a single `defaults` read and there is no window where the
        // watcher's baseline disagrees with the rendered theme.
        let mut last = self.appearance;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            loop {
                ticker.tick().await;
                let current = poll_appearance().await;
                if current != last {
                    last = current;
                    if tx.send(AppEvent::ThemeChanged(current)).is_err() {
                        break; // event loop is gone
                    }
                }
            }
        });
    }
}

/// Async counterpart to `theme::detect_appearance`, used by the watcher task so
/// the `defaults` subprocess never blocks the render loop.
async fn poll_appearance() -> Appearance {
    match tokio::process::Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .await
    {
        Ok(output) => parse_appearance(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Appearance::Dark,
    }
}

//! Async task spawners for [`App`].

use std::time::Duration;

use crate::db::watchlist::{add_to_watchlist, fetch_watchlist, remove_from_watchlist};
use crate::fetch::{
    fetch_analysis, fetch_chart_data, fetch_key_statistics, fetch_news, fetch_quote_and_store,
    fetch_sorted,
};
use crate::models::QuoteTick;
use crate::tui::stream::start_quote_stream;
use crate::tui::theme::{Appearance, parse_appearance};

use super::{App, AppEvent, PAGE_FETCH_LIMIT};

impl App {
    pub(crate) fn spawn_reload(&self) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
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

    pub(crate) fn spawn_fetch(&self, symbol: String) {
        let tx = self.services.event_tx.clone();
        let pool = self.services.pool.clone();
        let client = self.services.client.clone();
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

    pub(crate) fn spawn_analysis(&self, symbol: &str) {
        let tx = self.services.event_tx.clone();
        let client = self.services.client.clone();
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

    pub(crate) fn spawn_statistics(&self, symbol: &str) {
        let tx = self.services.event_tx.clone();
        let client = self.services.client.clone();
        let symbol = symbol.to_owned();
        tokio::spawn(async move {
            match fetch_key_statistics(&client, &symbol).await {
                Ok(s) => {
                    let _ = tx.send(AppEvent::StockStatisticsReady(s));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("statistics {symbol}: {e}")));
                }
            }
        });
    }

    pub(crate) fn spawn_news(&self, symbol: &str) {
        let tx = self.services.event_tx.clone();
        let client = self.services.client.clone();
        let symbol = symbol.to_owned();
        tokio::spawn(async move {
            match fetch_news(&client, &symbol).await {
                Ok(n) => {
                    let _ = tx.send(AppEvent::StockNewsReady(n));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("news {symbol}: {e}")));
                }
            }
        });
    }

    pub(crate) fn spawn_chart_data(&self, symbol: &str) {
        let tx = self.services.event_tx.clone();
        let client = self.services.client.clone();
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

    pub(crate) fn spawn_stream(&self, symbols: Vec<String>) {
        let tx = self.services.event_tx.clone();
        let client = self.services.client.clone();
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
                            break;
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

    pub(crate) fn spawn_theme_watcher(&self) {
        if !cfg!(target_os = "macos") {
            return;
        }
        let tx = self.services.event_tx.clone();
        let mut last = self.appearance;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(2));
            loop {
                ticker.tick().await;
                let current = poll_appearance().await;
                if current != last {
                    last = current;
                    if tx.send(AppEvent::ThemeChanged(current)).is_err() {
                        break;
                    }
                }
            }
        });
    }

    pub(crate) fn spawn_load_watchlist(&self) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            match fetch_watchlist(&pool).await {
                Ok(tickers) => {
                    let _ = tx.send(AppEvent::WatchlistLoaded(tickers));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to fetch watchlist: {e}")));
                }
            }
        });
    }

    pub(crate) fn spawn_save_to_watchlist(&self, ticker: String) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            add_to_watchlist(&pool, &ticker).await.unwrap_or_else(|e| {
                let _ = tx.send(AppEvent::Error(format!(
                    "Failed to add {ticker} to watchlist: {e}"
                )));
            });
        });
    }

    pub(crate) fn spawn_remove_from_watchlist(&self, ticker: String) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            remove_from_watchlist(&pool, &ticker)
                .await
                .unwrap_or_else(|e| {
                    let _ = tx.send(AppEvent::Error(format!(
                        "Failed to add {ticker} to watchlist: {e}"
                    )));
                });
        });
    }
}

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

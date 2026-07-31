//! Async task spawners for [`App`].

use std::future::Future;
use std::time::Duration;

use chrono::{DateTime, Utc};
use yfinance_rs::YfClient;

use crate::AppError;

use crate::db::alerts::{add_alert, list_alerts, mark_alert_triggered, remove_alert};
use crate::db::portfolio::{list_holdings, remove_holding, upsert_holding};
use crate::models::AlertDirection;
use crate::portfolio::Holding;
use crate::db::watchlist::{add_to_watchlist, fetch_watchlist, remove_from_watchlist};
use crate::fetch::{
    fetch_analysis, fetch_chart_data, fetch_key_statistics, fetch_news, fetch_option_chain,
    fetch_option_chain_for_expiry, fetch_quote_and_store, fetch_sorted,
};
use crate::models::QuoteTick;
use crate::tui::stream::start_quote_stream;
use crate::tui::theme::{Appearance, parse_appearance};

use super::{App, AppEvent, FetchOrigin, PAGE_FETCH_LIMIT, fetch_failure_event};

impl App {
    pub(crate) fn spawn_reload(&self) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        let mode = self.market.db_display.sort_mode;
        let order = self.market.db_display.sort_order;
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

    pub(crate) fn spawn_fetch(&self, symbol: String, origin: FetchOrigin) {
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
                    let _ = tx.send(fetch_failure_event(origin, &symbol, &e.to_string()));
                }
            }
        });
    }

    fn spawn_client_fetch<T, Fut>(
        &self,
        symbol: String,
        err_prefix: &'static str,
        fetch: impl FnOnce(YfClient, String) -> Fut + Send + 'static,
        to_event: impl FnOnce(T) -> AppEvent + Send + 'static,
    ) where
        T: Send + 'static,
        Fut: Future<Output = Result<T, AppError>> + Send + 'static,
    {
        let tx = self.services.event_tx.clone();
        let client = self.services.client.clone();
        tokio::spawn(async move {
            match fetch(client, symbol.clone()).await {
                Ok(value) => {
                    let _ = tx.send(to_event(value));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("{err_prefix} {symbol}: {e}")));
                }
            }
        });
    }

    pub(crate) fn spawn_analysis(&self, symbol: &str) {
        self.spawn_client_fetch(
            symbol.to_owned(),
            "analysis",
            |client, symbol| async move { fetch_analysis(&client, &symbol).await },
            AppEvent::StockAnalysisReady,
        );
    }

    pub(crate) fn spawn_statistics(&self, symbol: &str) {
        self.spawn_client_fetch(
            symbol.to_owned(),
            "statistics",
            |client, symbol| async move { fetch_key_statistics(&client, &symbol).await },
            AppEvent::StockStatisticsReady,
        );
    }

    pub(crate) fn spawn_news(&self, symbol: &str) {
        self.spawn_client_fetch(
            symbol.to_owned(),
            "news",
            |client, symbol| async move { fetch_news(&client, &symbol).await },
            AppEvent::StockNewsReady,
        );
    }

    pub(crate) fn spawn_chart_data(&self, symbol: &str) {
        self.spawn_client_fetch(
            symbol.to_owned(),
            "chart",
            |client, symbol| async move { fetch_chart_data(&client, &symbol).await },
            AppEvent::ChartDataReady,
        );
    }

    pub(crate) fn spawn_option_chain(&self, symbol: &str) {
        self.spawn_client_fetch(
            symbol.to_owned(),
            "options",
            |client, symbol| async move { fetch_option_chain(&client, &symbol).await },
            AppEvent::OptionChainReady,
        );
    }

    pub(crate) fn spawn_option_chain_for_expiry(&self, symbol: &str, expiry: DateTime<Utc>) {
        self.spawn_client_fetch(
            symbol.to_owned(),
            "options",
            move |client, symbol| async move {
                fetch_option_chain_for_expiry(&client, &symbol, expiry).await
            },
            AppEvent::OptionChainReady,
        );
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

    /// Restarts the quote stream when the target symbol set.
    // TODO(STREAM): debounce if pagination thrash causes reconnect spam.
    pub(crate) fn resubscribe_stream(&mut self) {
        let tickers = self.stream_symbols();
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

    pub(crate) fn spawn_load_alerts(&self) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            match list_alerts(&pool).await {
                Ok(alerts) => {
                    let _ = tx.send(AppEvent::AlertsLoaded(alerts));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to load alerts: {e}")));
                }
            }
        });
    }

    pub(crate) fn spawn_add_alert(
        &self,
        ticker: String,
        direction: AlertDirection,
        threshold: f64,
    ) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = add_alert(&pool, &ticker, direction, threshold).await {
                let _ = tx.send(AppEvent::Error(format!("Failed to save alert for {ticker}: {e}")));
            }
            reload_alerts(&pool, &tx).await;
        });
    }

    pub(crate) fn spawn_remove_alert(&self, id: i32) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = remove_alert(&pool, id).await {
                let _ = tx.send(AppEvent::Error(format!("Failed to remove alert {id}: {e}")));
            }
            reload_alerts(&pool, &tx).await;
        });
    }

    /// Spawns a task to persist an alert's `triggered_at`, reporting a failure via [`AppEvent::Error`].
    pub(crate) fn spawn_mark_alert_triggered(&self, id: i32, triggered_at: DateTime<Utc>) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = mark_alert_triggered(&pool, id, triggered_at).await {
                let _ = tx.send(AppEvent::Error(format!("Failed to persist alert {id}: {e}")));
            }
        });
    }

    pub(crate) fn spawn_load_portfolio(&self) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            match list_holdings(&pool).await {
                Ok(holdings) => {
                    let _ = tx.send(AppEvent::PortfolioLoaded(holdings));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to load portfolio: {e}")));
                }
            }
        });
    }

    pub(crate) fn spawn_upsert_holding(&self, holding: Holding) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = upsert_holding(&pool, &holding).await {
                let _ = tx.send(AppEvent::Error(format!(
                    "Failed to save holding {}: {e}",
                    holding.ticker
                )));
            }
            reload_portfolio(&pool, &tx).await;
        });
    }

    pub(crate) fn spawn_remove_holding(&self, ticker: String) {
        let pool = self.services.pool.clone();
        let tx = self.services.event_tx.clone();
        tokio::spawn(async move {
            if let Err(e) = remove_holding(&pool, &ticker).await {
                let _ = tx.send(AppEvent::Error(format!("Failed to remove holding {ticker}: {e}")));
            }
            reload_portfolio(&pool, &tx).await;
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
                        "Failed to remove {ticker} from watchlist: {e}"
                    )));
                });
        });
    }
}

async fn reload_portfolio(
    pool: &sqlx::PgPool,
    tx: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    match list_holdings(pool).await {
        Ok(holdings) => {
            let _ = tx.send(AppEvent::PortfolioLoaded(holdings));
        }
        Err(e) => {
            let _ = tx.send(AppEvent::Error(format!("Failed to reload portfolio: {e}")));
        }
    }
}

async fn reload_alerts(
    pool: &sqlx::PgPool,
    tx: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
) {
    match list_alerts(pool).await {
        Ok(alerts) => {
            let _ = tx.send(AppEvent::AlertsLoaded(alerts));
        }
        Err(e) => {
            let _ = tx.send(AppEvent::Error(format!("Failed to reload alerts: {e}")));
        }
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

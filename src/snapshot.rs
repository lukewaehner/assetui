//! The headless snapshot run: one unattended pass that records the state of everything the user tracks.

use chrono::NaiveDate;
use sqlx::{Pool, Postgres};
use tracing::{info, instrument, warn};

use crate::AppError;
use crate::db::portfolio::list_holdings;
use crate::db::portfolio_history::upsert_day;
use crate::db::quotes::{QUOTE_HISTORY_RETENTION, prune_quotes_history};
use crate::db::watchlist::fetch_watchlist;
use crate::fetch::fetch_latest_for_tickers;
use crate::models::QuoteRecord;
use crate::portfolio::{Holding, persistable_totals, portfolio_positions};
use crate::run::fetch_and_store;

/// What a snapshot run did, for the caller to print or log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotReport {
    /// Tickers the run covered.
    pub tickers: usize,
    /// Quote rows deleted by the retention prune.
    pub pruned: u64,
    /// Whether a `portfolio_value_history` row was written for the day.
    pub history_recorded: bool,
}

/// The tickers a snapshot should cover: the watchlist and every held position.
pub fn snapshot_tickers(watchlist: &[String], holdings: &[Holding]) -> Vec<String> {
    let mut tickers: Vec<String> = watchlist
        .iter()
        .map(|t| t.to_ascii_uppercase())
        .chain(holdings.iter().map(|h| h.ticker.to_ascii_uppercase()))
        .collect();
    tickers.sort_unstable();
    tickers.dedup();
    tickers
}

/// Records `day`'s portfolio totals from `rows`, returning what was written.
#[instrument(skip(pool, holdings, rows))]
pub async fn record_history_day(
    pool: &Pool<Postgres>,
    holdings: &[Holding],
    rows: &[QuoteRecord],
    day: NaiveDate,
) -> Result<Option<(f64, f64)>, AppError> {
    let positions = portfolio_positions(holdings, rows);
    let Some((market_value, cost_basis)) = persistable_totals(&positions) else {
        return Ok(None);
    };
    upsert_day(pool, day, market_value, cost_basis).await?;
    Ok(Some((market_value, cost_basis)))
}

#[instrument(skip(pool, client))]
pub async fn run_snapshot(
    pool: &Pool<Postgres>,
    client: &yfinance_rs::YfClient,
    day: NaiveDate,
) -> Result<SnapshotReport, AppError> {
    let watchlist = fetch_watchlist(pool).await?;
    let holdings = list_holdings(pool).await?;
    let tickers = snapshot_tickers(&watchlist, &holdings);

    if tickers.is_empty() {
        info!("nothing tracked: no watchlist entries and no holdings");
        return Ok(SnapshotReport {
            tickers: 0,
            pruned: 0,
            history_recorded: false,
        });
    }

    info!(count = tickers.len(), "snapshotting tracked tickers");
    fetch_and_store(pool, client, &tickers).await?;

    let mut pruned = 0u64;
    for ticker in &tickers {
        match prune_quotes_history(pool, ticker, QUOTE_HISTORY_RETENTION).await {
            Ok(deleted) => pruned += deleted,
            Err(e) => warn!(%ticker, error = %e, "prune failed"),
        }
    }

    let rows = fetch_latest_for_tickers(pool, &tickers).await?;

    let recorded = record_history_day(pool, &holdings, &rows, day).await?;
    match recorded {
        Some((market_value, cost_basis)) => {
            info!(%day, market_value, cost_basis, "recorded portfolio history")
        }
        None => info!("no portfolio history row: no holdings, or a holding has no price"),
    }

    Ok(SnapshotReport {
        tickers: tickers.len(),
        pruned,
        history_recorded: recorded.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(ticker: &str, shares: f64, avg_cost: f64) -> Holding {
        Holding {
            ticker: ticker.to_string(),
            shares,
            avg_cost,
        }
    }

    #[test]
    fn snapshot_tickers_unions_the_two_sources_without_duplicates() {
        let watchlist = vec!["AAPL".to_string(), "MSFT".to_string()];
        let holdings = vec![holding("AAPL", 1.0, 1.0), holding("NVDA", 1.0, 1.0)];

        let tickers = snapshot_tickers(&watchlist, &holdings);

        assert_eq!(tickers, vec!["AAPL", "MSFT", "NVDA"]);
    }

    #[test]
    fn snapshot_tickers_uppercases_before_deduplicating() {
        let watchlist = vec!["AAPL".to_string()];
        let holdings = vec![holding("aapl", 1.0, 1.0)];

        let tickers = snapshot_tickers(&watchlist, &holdings);

        assert_eq!(tickers, vec!["AAPL"]);
    }

    #[test]
    fn snapshot_tickers_is_empty_when_nothing_is_tracked() {
        assert!(snapshot_tickers(&[], &[]).is_empty());
    }
}

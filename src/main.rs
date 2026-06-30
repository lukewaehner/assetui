//! CLI binary entry point.

use tracing_subscriber::EnvFilter;
use yfinance::AppError;
use yfinance::cli::{Mode, pick_tickers, print_tickers, select_mode};
use yfinance::db::connection::setup_pool;
use yfinance::db::quotes::dump_table_to_csv;
use yfinance::run::fetch_and_store;

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    init_tracing();

    let database_url = dotenvy::var("DATABASE_URL")?;
    let pool = setup_pool(&database_url, 5).await?;

    match select_mode() {
        Mode::FetchAndStore => fetch_and_store(&pool, &pick_tickers()).await?,
        Mode::DumpToCsv => dump_table_to_csv(&pool).await?,
        Mode::PullFromDb => print_tickers(&pool).await?,
    }

    Ok(())
}

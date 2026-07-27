//! `assetui` binary entry point.

use clap::{Parser, Subcommand};
use std::path::Path;

use assetui::AppError;
use assetui::cli::print_tickers;
use assetui::db::connection::{DEFAULT_MAX_CONNECTIONS, setup_pool};
use assetui::db::quotes::dump_table_to_csv;
use assetui::run::fetch_and_store;
use tracing_subscriber::EnvFilter;
use std::time::Duration;
use yfinance_rs::{CacheEndpoint, YfClient};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long, value_delimiter = ',')]
    ticker: Option<Vec<String>>,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetches quotes for the specified ticker and stores them in the database.
    FetchAndStore,

    /// Dumps the quotes table to a CSV file in the current directory.
    DumpToCsv,

    /// Pulls all tickers from the database and prints them to stdout.
    PullFromDb,
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
}

/// Builds the Yahoo Finance client with per-endpoint response caching enabled.
fn build_yf_client() -> Result<YfClient, AppError> {
    Ok(YfClient::builder()
        .cache_ttl_for(CacheEndpoint::QuoteSummary, Duration::from_secs(30 * 60))
        .cache_ttl_for(CacheEndpoint::Fundamentals, Duration::from_secs(30 * 60))
        .cache_ttl_for(CacheEndpoint::Chart, Duration::from_secs(10 * 60))
        .cache_ttl_for(CacheEndpoint::News, Duration::from_secs(5 * 60))
        .cache_ttl_for(CacheEndpoint::Options, Duration::from_secs(20))
        .build()?)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), AppError> {
    let args = Args::parse();

    if args.command.is_some() {
        init_tracing();
    }

    let database_url = dotenvy::var("DATABASE_URL")?;
    let pool = setup_pool(&database_url, DEFAULT_MAX_CONNECTIONS).await?;
    let client = build_yf_client()?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    match &args.command {
        Some(Commands::FetchAndStore) => {
            let ticker = match &args.ticker {
                Some(t) => t.clone(),
                None => {
                    println!("Please provide tickers with --ticker or -t.");
                    return Ok(());
                }
            };
            fetch_and_store(&pool, &client, &ticker).await?;
        }
        Some(Commands::DumpToCsv) => {
            let output_path = Path::new("quotes_dump.csv");
            dump_table_to_csv(&pool, output_path).await?;
            println!("Quotes table dumped to {:?}", output_path);
        }
        Some(Commands::PullFromDb) => {
            print_tickers(&pool).await?;
        }
        None => {
            assetui::tui::run(pool, client).await?;
        }
    }
    Ok(())
}

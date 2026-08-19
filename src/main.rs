//! `assetui` binary entry point.

use clap::{Parser, Subcommand};
use std::path::Path;

use assetui::AppError;
use assetui::cli::print_tickers;
use assetui::db::connection::{DEFAULT_MAX_CONNECTIONS, setup_pool};
use assetui::db::quotes::dump_table_to_csv;
use assetui::run::fetch_and_store;
use assetui::snapshot::run_snapshot;
use std::time::Duration;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::EnvFilter;
use yfinance_rs::{CacheEndpoint, YfClient};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Fetches quotes for the specified ticker and stores them in the database.
    FetchAndStore {
        #[arg(short, long, value_delimiter = ',', required = true)]
        ticker: Vec<String>,
    },

    Snapshot,

    /// Dumps the quotes table to a CSV file in the current directory.
    DumpToCsv,

    /// Pulls all tickers from the database and prints them to stdout.
    PullFromDb,
}

/// Log verbosity, from `RUST_LOG` and defaulting to `info`.
fn log_filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
}

fn init_cli_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(log_filter())
        .with_target(false)
        .init();
}

fn init_tui_tracing() -> Option<WorkerGuard> {
    let dir = assetui::logging::log_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "warning: could not create log directory {}: {e}; continuing without file logging",
            dir.display()
        );
        return None;
    }

    let appender = match rolling::Builder::new()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix(assetui::logging::LOG_FILE_PREFIX)
        .filename_suffix("log")
        .max_log_files(assetui::logging::LOG_FILES_KEPT)
        .build(&dir)
    {
        Ok(appender) => appender,
        Err(e) => {
            eprintln!(
                "warning: could not open a log file in {}: {e}; continuing without file logging",
                dir.display()
            );
            return None;
        }
    };

    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .with_env_filter(log_filter())
        .with_target(false)
        .with_ansi(false)
        .with_writer(writer)
        .init();
    Some(guard)
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

    let _log_guard = if args.command.is_some() {
        init_cli_tracing();
        None
    } else {
        init_tui_tracing()
    };

    let database_url = dotenvy::var("DATABASE_URL")?;
    let pool = setup_pool(&database_url, DEFAULT_MAX_CONNECTIONS).await?;
    let client = build_yf_client()?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    match &args.command {
        Some(Commands::FetchAndStore { ticker }) => {
            fetch_and_store(&pool, &client, ticker).await?;
        }
        Some(Commands::Snapshot) => {
            let day = chrono::Local::now().date_naive();
            let report = run_snapshot(&pool, &client, day).await?;
            println!(
                "Snapshot: {} ticker(s), {} old row(s) pruned, portfolio history {}",
                report.tickers,
                report.pruned,
                if report.history_recorded {
                    "recorded"
                } else {
                    "skipped"
                }
            );
        }
        Some(Commands::DumpToCsv) => {
            let written = dump_table_to_csv(&pool, Path::new("quotes_dump.csv")).await?;
            println!("Quotes table dumped to {}", written.display());
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

//! `assetui` binary entry point.
//!
//! With no subcommand it launches the interactive TUI (the default mode).
//! Subcommands expose the individual CLI operations against the configured
//! Postgres database.

use clap::{Parser, Subcommand};
use std::path::Path;

use assetui::AppError;
use assetui::cli::print_tickers;
use assetui::db::connection::{DEFAULT_MAX_CONNECTIONS, setup_pool};
use assetui::db::quotes::dump_table_to_csv;
use assetui::run::fetch_and_store;
use tracing_subscriber::EnvFilter;
use yfinance_rs::YfClient;

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

/// Initialises the `tracing` subscriber with an `EnvFilter` so log verbosity
/// can be controlled via `RUST_LOG`.  Defaults to `info` when the env var is
/// absent.
fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
}

// A single-threaded runtime: this is an I/O-bound, mostly-idle TUI, so extra
// worker threads add memory and scheduler overhead with no throughput benefit.
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), AppError> {
    let args = Args::parse();

    // The TUI owns the alternate screen, so `tracing` output to stderr would
    // corrupt its rendering; only initialise the subscriber for the CLI
    // subcommands, which log to the terminal normally.
    if args.command.is_some() {
        init_tracing();
    }

    // Pull database url, setup env and client, run migrations
    let database_url = dotenvy::var("DATABASE_URL")?;
    let pool = setup_pool(&database_url, DEFAULT_MAX_CONNECTIONS).await?;
    let client = YfClient::default();
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
        // No subcommand: launch the interactive TUI, the default experience.
        None => {
            assetui::tui::run(pool, client).await?;
        }
    }
    Ok(())
}

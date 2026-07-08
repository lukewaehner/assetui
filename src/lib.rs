//! A library for fetching, storing, and querying Yahoo Finance stock quotes.
//!
//! The library exposes a fetch layer ([`fetch`]), a database layer ([`db`]),
//! shared data models ([`models`]), sort configuration ([`sort`]), fuzzy
//! matching ([`search`]), an async pipeline ([`run`]), CLI presentation
//! helpers ([`cli`]), and the interactive ratatui interface ([`tui`]).  The
//! single `assetui` binary drives the TUI by default and exposes the CLI
//! operations behind subcommands.

pub mod cli;
pub mod db;
pub mod fetch;
pub mod models;
pub mod run;
pub mod search;
pub mod sort;
pub mod tui;

/// Shared error type used throughout the crate.
///
/// A closed enum (rather than `Box<dyn Error>`) so callers can match on the
/// failure domain - database vs. Yahoo Finance vs. local I/O.  Every variant
/// is `Send + Sync`, so errors can cross `tokio::spawn` boundaries.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("Yahoo Finance error: {0}")]
    Yahoo(#[from] yfinance_rs::YfError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("environment error: {0}")]
    Env(#[from] dotenvy::Error),
}

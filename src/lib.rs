//! A library for fetching, storing, and querying Yahoo Finance stock quotes.

pub mod black_scholes;
pub mod cli;
pub mod db;
pub mod fetch;
pub mod indicators;
pub mod models;
pub mod news_feed;
pub mod portfolio;
pub mod run;
pub mod search;
pub mod sectors;
pub mod sort;
pub mod tui;

/// Shared error type used throughout the crate.
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
    #[error("data error: {0}")]
    Data(String),
}

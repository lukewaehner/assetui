//! A library for fetching, storing, and querying Yahoo Finance stock quotes.

pub mod cli;
pub mod db;
pub mod fetch;
pub mod models;
pub mod run;
pub mod sort;
pub mod stream;

/// Shared error type used throughout the crate.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Yahoo Finance error: {0}")]
    Yahoo(#[from] yfinance_rs::YfError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("environment error: {0}")]
    Env(#[from] dotenvy::Error),
}

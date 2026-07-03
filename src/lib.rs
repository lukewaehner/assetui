//! A library for fetching, storing, and querying Yahoo Finance stock quotes.

pub mod cli;
pub mod db;
pub mod fetch;
pub mod models;
pub mod run;
pub mod sort;
pub mod stream;

/// Shared error type used throughout the crate.
pub type AppError = Box<dyn std::error::Error + Send + Sync>;

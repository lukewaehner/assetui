//! A library for fetching, storing, and querying Yahoo Finance stock quotes.
//!
//! The library exposes a fetch layer ([`fetch`]), a database layer ([`db`]),
//! shared data models ([`models`]), sort configuration ([`sort`]), an async
//! pipeline ([`run`]), and CLI presentation helpers ([`cli`]).  Both the
//! interactive CLI binary and the ratatui TUI binary are built on top of this
//! crate.

pub mod cli;
pub mod db;
pub mod fetch;
pub mod models;
pub mod run;
pub mod sort;
pub mod stream;

/// Shared error type used throughout the crate.
///
/// `Send + Sync` bounds allow errors to cross `tokio::spawn` boundaries.
pub type AppError = Box<dyn std::error::Error + Send + Sync>;

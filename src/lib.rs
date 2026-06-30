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

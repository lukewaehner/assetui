//! Database layer for the quotes table.

pub mod alerts;
pub mod connection;
pub mod portfolio;
pub mod quotes;
pub mod watchlist;

pub use connection::{DEFAULT_MAX_CONNECTIONS, setup_pool};
pub use quotes::{dump_table_to_csv, fetch_all_quotes, prune_quotes_history, store_quote_to_db};

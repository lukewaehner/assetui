//! Database layer for the quotes table.
//!
//! [`connection`] sets up the Postgres pool; [`quotes`] holds all the
//! queries that read and write quote data.

pub mod connection;
pub mod quotes;

pub use connection::{DEFAULT_MAX_CONNECTIONS, setup_pool};
pub use quotes::{dump_table_to_csv, fetch_all_quotes, store_quote_to_db};

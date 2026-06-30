//! Database layer for the quotes table.
//!
//! [`connection`] sets up the Postgres pool; [`quotes`] holds all the
//! queries that read and write quote data.

pub mod connection;
pub mod quotes;

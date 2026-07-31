//! Terminal-specific presentation for the CLI binary.

pub mod input;
pub mod output;

pub use input::parse_tickers;
pub use output::print_tickers;

//! Terminal-specific presentation for the CLI binary.

pub mod input;
pub mod output;

pub use input::{Mode, pick_tickers, select_mode};
pub use output::print_tickers;

//! Terminal-specific presentation for the CLI binary.
//!
//! The shared engine (fetch, db, models) lives at the crate root.  This module
//! holds the stdin prompts ([`input`]) and table rendering ([`output`]) that
//! only the CLI binary uses.

pub mod input;
pub mod output;

pub use input::{Mode, parse_tickers, pick_tickers, select_mode};
pub use output::print_tickers;

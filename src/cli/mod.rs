// Terminal-specific presentation for the CLI binary.
// Shared engine code (fetch, run, db, models) lives at the crate root; this
// module holds the stdin prompts and table rendering that only the CLI uses.
pub mod input;
pub mod output;

pub use input::{Mode, pick_tickers, select_mode};
pub use output::print_tickers;

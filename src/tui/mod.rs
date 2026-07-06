//! Interactive ratatui terminal interface.
//!
//! This is the default experience when the `assetui` binary is run without a
//! subcommand.  [`run`] owns the terminal and the event loop; [`app`] holds
//! the application state and input handling, [`draw`] renders it, and [`theme`]
//! supplies the colour palette.

mod app;
mod draw;
mod run;
mod stream;
mod theme;

pub use run::run;

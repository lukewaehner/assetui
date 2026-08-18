//! Interactive ratatui terminal interface.

mod app;
mod draw;
mod run;
mod stream;
mod theme;
pub(crate) mod treemap;

pub mod action;
pub mod components;

pub use run::run;

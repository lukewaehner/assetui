//! Centralised colour theme for the TUI.
//!
//! Every colour rendered by [`draw`](super::draw) flows from one [`Theme`]
//! value so the palette can be swapped in a single place.  The struct and the
//! "Muted Slate" palette are adapted from the tuxedo project's theme system,
//! with the todo-oriented fields replaced by market-sentiment colours
//! (up / down / neutral).

use ratatui::style::Color;

/// A complete colour palette for the TUI.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Main background behind every widget.
    pub bg: Color,
    /// Secondary background for panels and overlay modals.
    pub panel: Color,
    /// Border colour for unfocused widgets.
    pub border: Color,
    /// Primary foreground text.
    pub fg: Color,
    /// Dimmed/secondary text: labels, hints, log lines.
    pub dim: Color,
    /// Accent colour: titles, focused borders, key hints.
    pub accent: Color,
    /// Background of the selected table row.
    pub cursor: Color,
    /// Status bar background.
    pub statusbar: Color,
    /// Status bar hint text.
    pub status_fg: Color,
    /// Mode chip foreground (text on the chip).
    pub mode_fg: Color,
    /// Mode chip background.
    pub mode_bg: Color,
    /// Positive movement: gains, buy ratings, high targets.
    pub up: Color,
    /// Emphasised positive: strong-buy ratings, up days on charts.
    pub up_strong: Color,
    /// Negative movement: losses, sell ratings, low targets.
    pub down: Color,
    /// Emphasised negative: strong-sell ratings, down days on charts.
    pub down_strong: Color,
    /// Unchanged / hold / reference values (e.g. previous close).
    pub neutral: Color,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// The default "Muted Slate" palette: a dark slate background with a soft
/// steel-blue accent and desaturated market colours.
pub const MUTED: Theme = Theme {
    bg: rgb(0x1a, 0x1d, 0x23),
    panel: rgb(0x1f, 0x23, 0x2b),
    border: rgb(0x2a, 0x2f, 0x38),
    fg: rgb(0xc8, 0xcc, 0xd4),
    dim: rgb(0x6b, 0x72, 0x80),
    accent: rgb(0x8a, 0xa9, 0xc9),
    cursor: rgb(0x3a, 0x41, 0x50),
    statusbar: rgb(0x25, 0x2a, 0x33),
    status_fg: rgb(0xa8, 0xb0, 0xbc),
    mode_fg: rgb(0x1a, 0x1d, 0x23),
    mode_bg: rgb(0x8a, 0xa9, 0xc9),
    up: rgb(0x7a, 0xa6, 0x7a),
    up_strong: rgb(0x9c, 0xd0, 0x9c),
    down: rgb(0xe0, 0x7a, 0x7a),
    down_strong: rgb(0xf0, 0x9a, 0x9a),
    neutral: rgb(0xd4, 0xb0, 0x6a),
};

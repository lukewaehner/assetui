//! Centralised colour theme for the TUI.
//!
//! Every colour rendered by [`draw`](super::draw) flows from one [`Theme`]
//! value so the palette can be swapped in a single place.  The struct and the
//! "Muted Slate" palette are adapted from the tuxedo project's theme system,
//! with the todo-oriented fields replaced by market-sentiment colours
//! (up / down / neutral).

use std::process::Command;

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

/// The active colour scheme, following the macOS system appearance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Light,
    Dark,
}

/// Light companion to [`MUTED`]: a soft off-white background with a steel-blue
/// accent and market colours darkened for contrast on a light surface.
pub const LIGHT: Theme = Theme {
    bg: rgb(0xf4, 0xf5, 0xf7),
    panel: rgb(0xea, 0xec, 0xf0),
    border: rgb(0xcf, 0xd4, 0xdc),
    fg: rgb(0x22, 0x27, 0x2f),
    dim: rgb(0x6b, 0x72, 0x80),
    accent: rgb(0x3a, 0x63, 0x8f),
    cursor: rgb(0xd4, 0xdd, 0xe8),
    statusbar: rgb(0xe0, 0xe3, 0xe9),
    status_fg: rgb(0x3a, 0x41, 0x50),
    mode_fg: rgb(0xf4, 0xf5, 0xf7),
    mode_bg: rgb(0x3a, 0x63, 0x8f),
    up: rgb(0x2f, 0x8a, 0x4e),
    up_strong: rgb(0x1e, 0x6b, 0x38),
    down: rgb(0xc0, 0x3a, 0x3a),
    down_strong: rgb(0x9a, 0x24, 0x24),
    neutral: rgb(0xb0, 0x82, 0x1a),
};

impl Theme {
    /// Selects the palette matching a system [`Appearance`].
    pub const fn for_appearance(appearance: Appearance) -> Theme {
        match appearance {
            Appearance::Light => LIGHT,
            Appearance::Dark => MUTED,
        }
    }
}

/// Maps the stdout of `defaults read -g AppleInterfaceStyle` to an
/// [`Appearance`]. macOS prints `Dark` in dark mode; in light mode the key is
/// absent, so empty/failed output maps to Light.
pub fn parse_appearance(defaults_stdout: &str) -> Appearance {
    if defaults_stdout.trim() == "Dark" {
        Appearance::Dark
    } else {
        Appearance::Light
    }
}

/// Reads the current macOS system appearance via
/// `defaults read -g AppleInterfaceStyle`. Any failure (non-macOS host, missing
/// binary) is treated as [`Appearance::Dark`], preserving the original default.
/// A non-zero exit (light mode, where the key is absent) yields empty stdout,
/// which `parse_appearance` maps to Light.
pub fn detect_appearance() -> Appearance {
    match Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
    {
        Ok(output) => parse_appearance(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Appearance::Dark,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_appearance_dark() {
        assert_eq!(parse_appearance("Dark\n"), Appearance::Dark);
    }

    #[test]
    fn test_parse_appearance_light_variants() {
        assert_eq!(parse_appearance(""), Appearance::Light);
        assert_eq!(parse_appearance("Light"), Appearance::Light);
        assert_eq!(parse_appearance("garbage"), Appearance::Light);
    }

    #[test]
    fn test_for_appearance_selects_palette() {
        assert_eq!(Theme::for_appearance(Appearance::Dark).bg, MUTED.bg);
        assert_eq!(Theme::for_appearance(Appearance::Light).bg, LIGHT.bg);
    }
}

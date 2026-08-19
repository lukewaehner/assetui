use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Alignment,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::tui::{
    action::Action,
    components::Component,
    draw::{help_row, modal_frame, section_heading},
    theme::Theme,
};

#[derive(Default, Debug, Clone)]
pub struct HelpModal {
    pub is_open: bool,
}

impl HelpModal {
    pub fn new() -> Self {
        Self { is_open: false }
    }
}

impl Component for HelpModal {
    fn handle_key_events(&mut self, key: KeyEvent) -> Option<Action> {
        if self.is_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                    self.is_open = false;
                    Some(Action::Render)
                }
                _ => Some(Action::Render),
            }
        } else {
            None
        }
    }

    fn update(&mut self, action: Action) -> Option<Action> {
        if let Action::ToggleHelp = action {
            self.is_open = !self.is_open;
            return Some(Action::Render);
        }
        None
    }
}

impl HelpModal {
    pub fn draw(&self, f: &mut Frame, t: &Theme) {
        if !self.is_open {
            return;
        }
        let title = Line::from(vec![
            Span::raw(" "),
            Span::styled(
                "assetui",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · keybindings ", Style::default().fg(t.dim)),
        ]);
        let (content, footer) = modal_frame(f, 48, 80, title, t);

        let lines = vec![
            section_heading("NAVIGATION", None, t),
            help_row("j / k", "move selection up / down", t),
            help_row("h / l", "previous / next page", t),
            Line::raw(""),
            section_heading("VIEWS", None, t),
            help_row("s", "stock details", t),
            help_row("enter", "price chart", t),
            help_row("x", "options chain (h/l expiry)", t),
            help_row("e", "earnings history", t),
            help_row("L", "toggle dashboard / logs", t),
            help_row("?", "this help", t),
            Line::raw(""),
            section_heading("WATCHLIST", None, t),
            help_row("w / r", "add / remove favourite (★)", t),
            Line::raw(""),
            section_heading("MANAGE PAGE", None, t),
            help_row("tab", "open the page picker (j/k move, enter go)", t),
            help_row("a / e / d", "add / edit / delete a holding or alert", t),
            Line::from(Span::styled(
                "  alerts fire once for on-screen tickers only",
                Style::default().fg(t.dim),
            )),
            Line::raw(""),
            section_heading("QUERY & SEARCH", None, t),
            help_row("i", "fetch tickers (input box)", t),
            help_row("/", "fuzzy search the table", t),
            Line::raw(""),
            section_heading("SORT", None, t),
            help_row("o", "toggle ascending / descending", t),
            help_row("t n p c v a", "sort by column", t),
            Line::raw(""),
            section_heading("GENERAL", None, t),
            help_row("esc", "close modal / cancel", t),
            help_row("q", "quit", t),
        ];
        f.render_widget(Paragraph::new(lines), content);

        f.render_widget(
            Paragraph::new(Span::styled("esc close ", Style::default().fg(t.dim)))
                .alignment(Alignment::Right),
            footer,
        );
    }
}

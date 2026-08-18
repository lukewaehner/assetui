use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::{
    action::Action,
    app::Page,
    components::Component,
    draw::{block_title, modal_frame},
    theme::Theme,
};

/// The page order used by the picker and its highlight movement.
pub const PAGE_ORDER: [Page; 5] =
    [Page::Market, Page::Manage, Page::Sectors, Page::News, Page::Optimizer];

/// The `Tab` page-picker modal: the pages in [`PAGE_ORDER`] with one of them highlighted.
#[derive(Default, Debug, Clone)]
pub struct PagePicker {
    pub is_open: bool,
    pub highlighted: Page,
}

impl PagePicker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn toggle(&mut self, current: Page) {
        if self.is_open {
            self.is_open = false;
        } else {
            self.highlighted = current;
            self.is_open = true;
        }
    }

    /// Moves the highlight through [`PAGE_ORDER`], wrapping.
    fn move_highlight(&mut self, delta: isize) {
        let idx = PAGE_ORDER
            .iter()
            .position(|&p| p == self.highlighted)
            .unwrap_or(0) as isize;
        let n = PAGE_ORDER.len() as isize;
        let next = (idx + delta).rem_euclid(n) as usize;
        self.highlighted = PAGE_ORDER[next];
    }
}

impl Component for PagePicker {
    fn handle_key_events(&mut self, key: KeyEvent) -> Option<Action> {
        if !self.is_open {
            return None;
        }
        match key.code {
            KeyCode::Esc => {
                self.is_open = false;
                Some(Action::Render)
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_highlight(1);
                Some(Action::Render)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_highlight(-1);
                Some(Action::Render)
            }
            KeyCode::Enter => {
                self.is_open = false;
                Some(Action::GoToPage(self.highlighted))
            }
            KeyCode::Char('q') => Some(Action::Quit),
            _ => Some(Action::Render),
        }
    }
}

impl PagePicker {
    pub fn draw(&self, f: &mut Frame, t: &Theme) {
        if !self.is_open {
            return;
        }
        let title = block_title("go to page".into(), t);
        let (content, _footer) = modal_frame(f, 30, 34, title, t);

        let lines: Vec<Line> = PAGE_ORDER
            .iter()
            .map(|&p| {
                let selected = p == self.highlighted;
                let caret = if selected { "› " } else { "  " };
                let style = if selected {
                    Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(t.fg)
                };
                Line::from(Span::styled(format!("{caret}{}", page_label(p)), style))
            })
            .collect();
        f.render_widget(Paragraph::new(lines), content);
    }
}

/// Human label for a page, shown in the picker.
fn page_label(p: Page) -> &'static str {
    match p {
        Page::Market => "Market",
        Page::Manage => "Manage",
        Page::Sectors => "Sectors",
        Page::News => "News",
        Page::Optimizer => "Optimizer",
    }
}

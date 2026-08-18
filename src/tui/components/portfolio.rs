use ratatui::{
    symbols::Marker,
    text::{Line, Span},
    widgets::{Clear, GraphType, Dataset, Chart, Axis},
    style::Style,
    Frame,
};

use crate::tui::{
    components::Component,
    draw::{block_title, centered_rect, modal_block, placeholder},
    theme::Theme,
    action::Action,
};
use crossterm::event::{KeyCode, KeyEvent};

/// Overlay modal showing the persisted portfolio value history and return.
#[derive(Default, Debug, Clone)]
pub struct PortfolioModal {
    pub is_open: bool,
    pub history: Vec<(chrono::NaiveDate, f64, f64)>,
    pub loading: bool,
}

/// Each day's cost basis, indexed for plotting against the value series.
fn basis_series(history: &[(chrono::NaiveDate, f64, f64)]) -> Vec<(f64, f64)> {
    history
        .iter()
        .enumerate()
        .map(|(i, (_, _, basis))| (i as f64, *basis))
        .collect()
}

impl PortfolioModal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn draw(&self, f: &mut Frame, t: &Theme) {
        if !self.is_open {
            return;
        }
        let area = centered_rect(65, 55, f.area());
        f.render_widget(Clear, area);

        if self.history.len() < 2 {
            let block = modal_block(block_title("portfolio · performance".into(), t), t);
            let inner = block.inner(area);
            f.render_widget(block, area);
            let msg = if self.loading {
                "loading history…"
            } else if self.history.is_empty() {
                "no history yet — recorded daily while running"
            } else {
                "not enough history yet — one more day"
            };
            f.render_widget(placeholder(msg, t), inner);
            return;
        }

        let series: Vec<(chrono::NaiveDate, f64)> =
            self.history.iter().map(|(d, v, _)| (*d, *v)).collect();
        let latest_basis = self.history.last().map_or(0.0, |(_, _, basis)| *basis);
        let ret = crate::portfolio::series_return(&series, latest_basis);
        let pct = ret
            .total_pnl_pct
            .map(|v| format!("{v:+.1}%"))
            .unwrap_or_else(|| "-".into());
        let title = block_title(format!("portfolio · ${:.2} ({pct})", ret.latest_value), t);
        let block = modal_block(title, t);
        let inner = block.inner(area);
        f.render_widget(block, area);

        let data: Vec<(f64, f64)> = self
            .history
            .iter()
            .enumerate()
            .map(|(i, (_, v, _))| (i as f64, *v))
            .collect();
        let basis_line = basis_series(&self.history);
        let xmax = (self.history.len() - 1) as f64;

        let trend_up = ret.total_pnl >= 0.0;
        let value_ds = Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(if trend_up { t.up } else { t.down }))
            .data(&data);
        let basis_ds = Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(t.dim))
            .data(&basis_line);

        let ymin = data
            .iter()
            .map(|(_, v)| *v)
            .chain(basis_line.iter().map(|(_, v)| *v))
            .fold(f64::INFINITY, f64::min);
        let ymax = data
            .iter()
            .map(|(_, v)| *v)
            .chain(basis_line.iter().map(|(_, v)| *v))
            .fold(f64::NEG_INFINITY, f64::max);
        let pad = ((ymax - ymin) * 0.04).max(0.01);
        let date_label = |i: usize| {
            Line::from(Span::styled(
                self.history[i].0.format("%b %d").to_string(),
                Style::default().fg(t.dim),
            ))
        };

        let chart = Chart::new(vec![value_ds, basis_ds])
            .x_axis(
                Axis::default()
                    .style(Style::default().fg(t.dim))
                    .bounds([0.0, xmax])
                    .labels([
                        date_label(0),
                        date_label(self.history.len() / 2),
                        date_label(self.history.len() - 1),
                    ]),
            )
            .y_axis(
                Axis::default()
                    .style(Style::default().fg(t.dim))
                    .bounds([ymin - pad, ymax + pad])
                    .labels([
                        Line::from(format!("{:.0}", ymin - pad)),
                        Line::from(format!("{:.0}", ymax + pad)),
                    ]),
            );
        f.render_widget(chart, inner);
    }
}

impl Component for PortfolioModal {
    fn handle_key_events(&mut self, key: KeyEvent) -> Option<Action> {
        if self.is_open {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('p') | KeyCode::Char('P') => {
                    self.is_open = false;
                    Some(Action::Render)
                }
                _ => Some(Action::Render),
            }
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::Appearance;
    use ratatui::{backend::TestBackend, Terminal};

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    fn d(day: u32) -> chrono::NaiveDate {
        chrono::NaiveDate::from_ymd_opt(2026, 1, day).unwrap()
    }

    fn draw_modal(modal: &PortfolioModal) -> String {
        let theme = Theme::for_appearance(Appearance::Dark);
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
        terminal.draw(|f| modal.draw(f, &theme)).unwrap();
        buffer_text(&terminal)
    }

    #[test]
    fn basis_series_steps_when_the_basis_changes_mid_series() {
        let history = vec![(d(1), 100.0, 500.0), (d(2), 110.0, 500.0), (d(3), 140.0, 700.0)];

        let points = basis_series(&history);

        assert_eq!(points, vec![(0.0, 500.0), (1.0, 500.0), (2.0, 700.0)]);
    }

    #[test]
    fn basis_series_is_empty_for_empty_history() {
        assert!(basis_series(&[]).is_empty());
    }

    #[test]
    fn draw_shows_loading_message_while_loading_with_no_history_yet() {
        let modal = PortfolioModal { is_open: true, history: Vec::new(), loading: true };

        let text = draw_modal(&modal);

        assert!(text.contains("loading history"));
    }

    #[test]
    fn draw_shows_empty_state_when_not_loading_and_history_is_empty() {
        let modal = PortfolioModal { is_open: true, history: Vec::new(), loading: false };

        let text = draw_modal(&modal);

        assert!(text.contains("no history yet"));
    }

    #[test]
    fn draw_shows_not_enough_history_message_for_a_single_day() {
        let modal = PortfolioModal {
            is_open: true,
            history: vec![(d(1), 1000.0, 900.0)],
            loading: false,
        };

        let text = draw_modal(&modal);

        assert!(text.contains("not enough history yet"));
    }

    #[test]
    fn draw_renders_the_chart_title_for_multi_day_history() {
        let modal = PortfolioModal {
            is_open: true,
            history: vec![(d(1), 1000.0, 900.0), (d(2), 1100.0, 900.0)],
            loading: false,
        };

        let text = draw_modal(&modal);

        assert!(text.contains("$1100.00"), "latest value is drawn in the title");
        assert!(!text.contains("no history yet"));
        assert!(!text.contains("not enough history yet"));
    }
}

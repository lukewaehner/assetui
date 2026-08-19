use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::earnings::{TickerEarnings, surprise_pct};
use crate::tui::{
    action::Action,
    components::Component,
    draw::{
        bar_cells, block_title, fmt_market_cap, modal_frame, placeholder, pnl_color,
        section_heading,
    },
    theme::Theme,
};

const REVENUE_BAR_WIDTH: usize = 12;

#[derive(Default, Debug, Clone)]
pub struct EarningsModal {
    /// The ticker the modal was opened for.
    pub ticker: Option<String>,
    /// Fetched earnings, `None` until the first fetch lands.
    pub data: Option<TickerEarnings>,
    /// Whether a fetch is in flight.
    pub loading: bool,
}

impl EarningsModal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn draw(&self, f: &mut Frame, t: &Theme) {
        let ticker = self.ticker.as_deref().unwrap_or("-");
        let title = block_title(format!("{ticker} · earnings"), t);
        let (content, footer) = modal_frame(f, 60, 60, title, t);

        let Some(data) = self.data.as_ref().filter(|d| !d.is_empty()) else {
            let msg = if self.loading {
                "loading earnings…"
            } else {
                "no earnings data"
            };
            f.render_widget(placeholder(msg, t), content);
            f.render_widget(
                Paragraph::new(Span::styled("esc close ", Style::default().fg(t.dim)))
                    .alignment(Alignment::Right),
                footer,
            );
            return;
        };

        let mut lines = vec![
            section_heading("EPS", None, t),
            Line::from(Span::styled(
                format!(
                    "  {:<10}{:>9}{:>9}{:>11}",
                    "PERIOD", "EST", "ACTUAL", "SURPRISE"
                ),
                Style::default().fg(t.dim),
            )),
        ];
        for q in &data.eps {
            let surprise = surprise_pct(q.estimate, q.actual);
            lines.push(Line::from(vec![
                Span::raw(format!(
                    "  {:<10}{:>9}{:>9}",
                    q.period,
                    fmt_eps(q.estimate),
                    fmt_eps(q.actual)
                )),
                Span::styled(
                    format!("{:>11}", fmt_surprise(surprise)),
                    Style::default().fg(pnl_color(surprise, t)),
                ),
            ]));
        }

        if !data.revenue.is_empty() {
            lines.push(Line::raw(""));
            lines.push(section_heading("REVENUE", None, t));
            let max = data
                .revenue
                .iter()
                .filter_map(|r| r.revenue)
                .fold(0.0_f64, f64::max);
            for r in &data.revenue {
                let cells = r
                    .revenue
                    .map_or(0, |v| bar_cells(v, max, REVENUE_BAR_WIDTH));
                lines.push(Line::from(vec![
                    Span::raw(format!(
                        "  {:<10}{:>9}  ",
                        r.period,
                        fmt_market_cap(r.revenue)
                    )),
                    Span::styled("▆".repeat(cells), Style::default().fg(t.accent)),
                ]));
            }
        }

        f.render_widget(Paragraph::new(lines), content);

        let [left, right] =
            Layout::horizontal([Constraint::Fill(1), Constraint::Length(11)]).areas(footer);
        let next = data.next_report.map_or_else(
            || "next report · -".to_string(),
            |d| format!("next report · {}", d.format("%b %d")),
        );
        f.render_widget(
            Paragraph::new(Span::styled(next, Style::default().fg(t.dim))),
            left,
        );
        f.render_widget(
            Paragraph::new(Span::styled("esc close ", Style::default().fg(t.dim)))
                .alignment(Alignment::Right),
            right,
        );
    }
}

impl Component for EarningsModal {
    fn handle_key_events(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('e') => Some(Action::CloseOverlay),
            _ => Some(Action::Render),
        }
    }
}

/// An EPS figure to two decimals, or `-` when upstream has none.
fn fmt_eps(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| format!("{v:.2}"))
}

fn fmt_surprise(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| format!("{v:+.1}%"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::QuarterEps;
    use crate::tui::theme::Appearance;
    use ratatui::{Terminal, backend::TestBackend};

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn draw_shows_surprise_column_for_a_beat_and_a_miss() {
        // arrange
        let mut modal = EarningsModal::new();
        modal.ticker = Some("AAPL".to_string());
        modal.data = Some(TickerEarnings {
            ticker: "AAPL".to_string(),
            eps: vec![
                QuarterEps {
                    period: "2Q2026".to_string(),
                    estimate: Some(2.0),
                    actual: Some(2.5),
                },
                QuarterEps {
                    period: "1Q2026".to_string(),
                    estimate: Some(2.0),
                    actual: Some(1.5),
                },
            ],
            revenue: vec![],
            next_report: None,
        });
        let theme = Theme::for_appearance(Appearance::Dark);
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();

        // act
        terminal.draw(|f| modal.draw(f, &theme)).unwrap();

        // assert
        let text = buffer_text(&terminal);
        assert!(text.contains("SURPRISE"), "surprise column header is drawn");
        assert!(
            text.contains("+25.0%"),
            "the beat's surprise percentage is drawn"
        );
        assert!(
            text.contains("-25.0%"),
            "the miss's surprise percentage is drawn"
        );
    }

    #[test]
    fn draw_shows_no_earnings_data_for_vacuous_data() {
        // arrange
        let mut modal = EarningsModal::new();
        modal.ticker = Some("SPY".to_string());
        modal.data = Some(TickerEarnings::default());
        let theme = Theme::for_appearance(Appearance::Dark);
        let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();

        // act
        terminal.draw(|f| modal.draw(f, &theme)).unwrap();

        // assert
        assert!(buffer_text(&terminal).contains("no earnings data"));
    }

    #[test]
    fn fmt_eps_shows_two_decimals() {
        assert_eq!(fmt_eps(Some(1.5)), "1.50");
    }

    #[test]
    fn fmt_eps_absent_is_dash() {
        assert_eq!(fmt_eps(None), "-");
    }

    #[test]
    fn fmt_surprise_signs_the_percentage() {
        assert_eq!(fmt_surprise(Some(11.3)), "+11.3%");
        assert_eq!(fmt_surprise(Some(-5.8)), "-5.8%");
    }

    #[test]
    fn fmt_surprise_absent_is_dash() {
        assert_eq!(fmt_surprise(None), "-");
    }
}

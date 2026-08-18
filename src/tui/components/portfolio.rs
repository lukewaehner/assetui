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

/// Overlay modal showing the portfolio value history and return.
#[derive(Default)]
pub struct PortfolioModal {
    pub is_open: bool,
    pub series: Option<Vec<(chrono::NaiveDate, f64)>>,
    pub cost_basis: f64,
    pub loading: bool,
}

impl PortfolioModal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn draw(&self, f: &mut Frame, t: &Theme, has_holdings: bool) {
        if !self.is_open {
            return;
        }
        let area = centered_rect(65, 55, f.area());
        f.render_widget(Clear, area);

        let series = match &self.series {
            None => {
                let block = modal_block(block_title("portfolio · performance".into(), t), t);
                let inner = block.inner(area);
                f.render_widget(block, area);
                let msg = if self.loading { "reconstructing history…" } else { "no data" };
                f.render_widget(placeholder(msg, t), inner);
                return;
            }
            Some(s) => s,
        };

        let ret = crate::portfolio::series_return(series, self.cost_basis);
        let pct = ret
            .total_pnl_pct
            .map(|v| format!("{v:+.1}%"))
            .unwrap_or_else(|| "-".into());
        let title = block_title(
            format!("portfolio · ${:.2} ({pct})", ret.latest_value),
            t,
        );
        let block = modal_block(title, t);
        let inner = block.inner(area);
        f.render_widget(block, area);

        if series.len() < 2 {
            let msg = if !has_holdings { "no holdings" } else { "not enough history" };
            f.render_widget(placeholder(msg, t), inner);
            return;
        }

        let data: Vec<(f64, f64)> = series
            .iter()
            .enumerate()
            .map(|(i, (_, v))| (i as f64, *v))
            .collect();
        let xmax = (series.len() - 1) as f64;
        let basis_line = vec![(0.0, self.cost_basis), (xmax, self.cost_basis)];

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

        let ymin = data.iter().map(|(_, v)| *v).chain([self.cost_basis]).fold(f64::INFINITY, f64::min);
        let ymax = data.iter().map(|(_, v)| *v).chain([self.cost_basis]).fold(f64::NEG_INFINITY, f64::max);
        let pad = ((ymax - ymin) * 0.04).max(0.01);
        let date_label = |i: usize| Line::from(Span::styled(series[i].0.format("%b %d").to_string(), Style::default().fg(t.dim)));

        let chart = Chart::new(vec![value_ds, basis_ds])
            .x_axis(
                Axis::default()
                    .style(Style::default().fg(t.dim))
                    .bounds([0.0, xmax])
                    .labels([date_label(0), date_label(series.len() / 2), date_label(series.len() - 1)]),
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

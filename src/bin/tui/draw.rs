//! Ratatui rendering for the TUI binary.
//!
//! The layout is a 20/80 horizontal split: a narrow left column holds the
//! ticker input box and a scrolling log panel; the right column holds the
//! sortable quotes table.  When a stock-detail modal is open it overlays the
//! full frame.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
};
use yfinance::models::QuoteRecord;

use super::app::App;

/// Renders the full TUI frame: input box, log panel, quotes table, and
/// optionally the stock-detail modal.
pub fn draw(f: &mut Frame, app: &mut App) {
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(20), Constraint::Percentage(80)])
        .split(f.area());

    let left = outer[0];
    let right = outer[1];

    let left_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(left);

    let input_area = left_split[0];
    let log_area = left_split[1];

    // Border colours swap between input and table to show which is "active".
    let in_input = app.input_mode.toggled;
    let input_border = if in_input {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let db_border = if in_input {
        Color::DarkGray
    } else {
        Color::Cyan
    };

    let cursor = if in_input && app.blink_state {
        "▌"
    } else {
        " "
    };
    let input_display = format!("{}{}", app.input_mode.input, cursor);
    let input_block = Block::default()
        .title("Query")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(input_border));
    f.render_widget(
        Paragraph::new(input_display.as_str()).block(input_block),
        input_area,
    );

    let log_block = Block::default()
        .title("Logs")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let visible = log_area.height.saturating_sub(2) as usize;
    let start = app.logs.len().saturating_sub(visible);
    let log_lines: Vec<Line> = app.logs[start..]
        .iter()
        .map(|s| Line::raw(s.as_str()))
        .collect();
    f.render_widget(Paragraph::new(log_lines).block(log_block), log_area);

    draw_quotes_table(f, right, app, db_border);

    if app.stock_modal.visible {
        draw_stock_modal(f, app);
    }

    let area = f.area();
    app.notifications.render(f, area);
}

/// Renders the sortable quotes table into `area`.
///
/// Each header cell shows its sort key in yellow so users know which key to
/// press.  The selected row is highlighted in dark gray with bold text, and
/// `>> ` is drawn in the gutter.
fn draw_quotes_table(f: &mut Frame, area: Rect, app: &mut App, border_color: Color) {
    // 2 borders + 1 header row + 1 header bottom-margin = 4 overhead rows.
    let page_size = area.height.saturating_sub(4).max(1) as usize;
    app.set_page_size(page_size);

    let box_bg = Color::DarkGray;
    let label_sty = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let key_sty = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let cell_sty = Style::default().bg(box_bg);

    let hcell = |label: &'static str, key: &'static str| -> Cell<'static> {
        Cell::from(Line::from(vec![
            Span::styled(label, label_sty),
            Span::styled(key, key_sty),
        ]))
        .style(cell_sty)
    };

    let header = Row::new(vec![
        hcell("ID ", "(d)"),
        hcell("Ticker ", "(t)"),
        hcell("Name", "(n)"),
        hcell("Price ", "(p)"),
        hcell("Prev Close ", "(c)"),
        hcell("Volume ", "(v)"),
        hcell("As Of ", "(a)"),
    ])
    .bottom_margin(1);

    let rows = app.db_display.stocks.window.iter().map(|q| {
        Row::new(vec![
            match q.id {
                Some(id) => Cell::from(id.to_string()),
                None => Cell::from("-"),
            },
            Cell::from(q.ticker.as_deref().unwrap_or("-")),
            Cell::from(q.name.as_deref().unwrap_or("-")),
            match q.price {
                Some(p) => Cell::from(format!("{p:.2}")),
                None => Cell::from("-"),
            },
            match q.previous_close {
                Some(p) => Cell::from(format!("{p:.2}")),
                None => Cell::from("-"),
            },
            match q.day_volume {
                Some(v) => Cell::from(format!("{v:.0}")),
                None => Cell::from("-"),
            },
            match q.as_of {
                Some(dt) => Cell::from(dt.format("%Y-%m-%d").to_string()),
                None => Cell::from("-"),
            },
        ])
    });

    let widths = [
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(24),
        Constraint::Length(11),
        Constraint::Length(15),
        Constraint::Length(13),
        Constraint::Length(12),
    ];

    let total_pages = app.total_pages();
    let page = app.db_display.page + 1;
    let title = if app.db_display.status.is_empty() {
        format!("Quotes [{page}/{total_pages}]  h/l to paginate")
    } else {
        format!("Quotes [{page}/{total_pages}] — {}", app.db_display.status)
    };
    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        )
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut app.db_display.table_state);
}

/// Computes a centred [`Rect`] that occupies `percent_x`% of the width and
/// `percent_y`% of the height of `r`.
///
/// Uses `Fill(1)` for the padding halves so the modal stays perfectly centred
/// regardless of whether `100 - percent` is even or odd.
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Percentage(percent_y),
            Constraint::Fill(1),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Fill(1),
            Constraint::Percentage(percent_x),
            Constraint::Fill(1),
        ])
        .split(vert[1])[1]
}

/// Renders the stock-detail overlay modal at 65 × 55% of the terminal area.
///
/// Shows the ticker, company name, current price, previous close, and - once
/// the background fetch completes - the analyst consensus breakdown and price
/// targets.  While the analysis is loading a "Loading analysis…" placeholder
/// is shown instead.
fn draw_stock_modal(f: &mut Frame, app: &mut App) {
    let area = centered_rect(65, 55, f.area());
    f.render_widget(Clear, area);
    let modal = Block::bordered()
        .title("Stock Info")
        .style(Style::default());

    let inner = modal.inner(area);
    f.render_widget(modal, area);

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // ticker + name
            Constraint::Length(1), // price + prev close
            Constraint::Length(1), // blank
            Constraint::Min(0),    // analysis
        ])
        .split(inner);

    let stock = &app.stock_modal.stock;

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                stock.ticker.as_deref().unwrap_or("-"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  -  "),
            Span::raw(stock.name.as_deref().unwrap_or("-")),
        ])),
        sections[0],
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Price: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                stock
                    .price
                    .map(|p| format!("${p:.2}"))
                    .unwrap_or_else(|| "-".into()),
                Style::default().fg(configure_stock_price_color(stock)),
            ),
            Span::raw("    "),
            Span::styled("Prev Close: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                stock
                    .previous_close
                    .map(|p| format!("${p:.2}"))
                    .unwrap_or_else(|| "-".into()),
                Style::default().fg(Color::Yellow),
            ),
        ])),
        sections[1],
    );

    match &app.stock_modal.analysis {
        None => {
            f.render_widget(Paragraph::new("  Loading analysis..."), sections[3]);
        }
        Some(analysis) => {
            let analysis_sections = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // "Analyst Consensus" heading
                    Constraint::Length(1), // period + consensus rating
                    Constraint::Length(1), // blank
                    Constraint::Length(2), // buy/sell breakdown table
                    Constraint::Length(1), // blank
                    Constraint::Length(1), // "Price Target" heading
                    Constraint::Length(1), // mean / low / high / analysts
                    Constraint::Min(0),
                ])
                .split(sections[3]);

            f.render_widget(
                Paragraph::new(Span::styled(
                    "Analyst Consensus",
                    Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                )),
                analysis_sections[0],
            );

            if let Some(rec) = &analysis.recommendation_summary {
                let period_str = rec
                    .latest_period
                    .as_ref()
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".into());
                let mean_str = rec
                    .mean
                    .map(|m| format!("{m:.2}"))
                    .unwrap_or_else(|| "-".into());
                let rating = rec.mean_rating_text.as_deref().unwrap_or("-");
                let rating_color = match rating.to_lowercase().as_str() {
                    "strong buy" => Color::LightGreen,
                    "buy" => Color::Green,
                    "hold" => Color::Yellow,
                    "sell" => Color::Red,
                    "strong sell" => Color::LightRed,
                    _ => Color::White,
                };

                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("Period: ", Style::default().fg(Color::DarkGray)),
                        Span::raw(period_str),
                        Span::raw("    "),
                        Span::styled("Consensus: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            rating,
                            Style::default()
                                .fg(rating_color)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            format!("  ({mean_str})"),
                            Style::default().fg(Color::DarkGray),
                        ),
                    ])),
                    analysis_sections[1],
                );

                let fmt_u = |v: Option<u32>| v.map(|n| n.to_string()).unwrap_or_else(|| "-".into());
                f.render_widget(
                    Table::new(
                        [
                            Row::new([
                                Cell::from("Str Buy").style(
                                    Style::default()
                                        .fg(Color::LightGreen)
                                        .add_modifier(Modifier::BOLD),
                                ),
                                Cell::from("Buy").style(Style::default().fg(Color::Green)),
                                Cell::from("Hold").style(Style::default().fg(Color::Yellow)),
                                Cell::from("Sell").style(Style::default().fg(Color::Red)),
                                Cell::from("Str Sell").style(
                                    Style::default()
                                        .fg(Color::LightRed)
                                        .add_modifier(Modifier::BOLD),
                                ),
                            ]),
                            Row::new([
                                Cell::from(fmt_u(rec.strong_buy)),
                                Cell::from(fmt_u(rec.buy)),
                                Cell::from(fmt_u(rec.hold)),
                                Cell::from(fmt_u(rec.sell)),
                                Cell::from(fmt_u(rec.strong_sell)),
                            ]),
                        ],
                        [Constraint::Ratio(1, 5); 5],
                    ),
                    analysis_sections[3],
                );
            }

            f.render_widget(
                Paragraph::new(Span::styled(
                    "Price Target",
                    Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                )),
                analysis_sections[5],
            );

            if let Some(pt) = &analysis.price_target {
                let mean_str = pt
                    .mean
                    .as_ref()
                    .map(|p| format!("${:.2}", p.amount()))
                    .unwrap_or_else(|| "-".into());
                let low_str = pt
                    .low
                    .as_ref()
                    .map(|p| format!("${:.2}", p.amount()))
                    .unwrap_or_else(|| "-".into());
                let high_str = pt
                    .high
                    .as_ref()
                    .map(|p| format!("${:.2}", p.amount()))
                    .unwrap_or_else(|| "-".into());

                f.render_widget(
                    Paragraph::new(Line::from(vec![
                        Span::styled("Mean: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(mean_str, Style::default().fg(Color::Yellow)),
                        Span::raw("    "),
                        Span::styled("Low: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(low_str, Style::default().fg(Color::Red)),
                        Span::raw("    "),
                        Span::styled("High: ", Style::default().fg(Color::DarkGray)),
                        Span::styled(high_str, Style::default().fg(Color::Green)),
                        Span::raw("    "),
                        Span::styled("Analysts: ", Style::default().fg(Color::DarkGray)),
                        Span::raw(
                            pt.number_of_analysts
                                .map(|n| n.to_string())
                                .unwrap_or_else(|| "-".into()),
                        ),
                    ])),
                    analysis_sections[6],
                );
            }
        }
    }
}

fn configure_stock_price_color(stock: &QuoteRecord) -> Color {
    let price = stock.price.unwrap_or_default();
    let prev = stock.previous_close.unwrap_or_default();
    let diff = price - prev;
    // Use an epsilon band so rounding to display precision doesn't trigger a
    // false green/red when price and prev are effectively equal.
    if diff > 0.001 {
        Color::Green
    } else if diff < -0.001 {
        Color::Red
    } else {
        Color::Yellow
    }
}

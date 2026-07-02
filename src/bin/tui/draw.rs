//! Ratatui rendering for the TUI binary.
//!
//! The layout is a 20/80 horizontal split: a narrow left column holds the
//! ticker input box and a scrolling log panel; the right column holds the
//! sortable quotes table.  When a stock-detail modal is open it overlays the
//! full frame.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
};
use yfinance_rs::{Price, PriceTarget, RecommendationSummary};

use yfinance::models::{QuoteRecord, QuoteRecordAnalysis};

use super::app::App;

/// Renders the full TUI frame: input box, log panel, quotes table, and
/// optionally the stock-detail modal.
pub fn draw(f: &mut Frame, app: &mut App) {
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(20), Constraint::Percentage(80)])
            .areas(f.area());
    let [input_area, log_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(left);

    // Border colours swap between input and table to show which is "active".
    let (input_border, db_border) = if app.input_mode.toggled {
        (Color::Cyan, Color::DarkGray)
    } else {
        (Color::DarkGray, Color::Cyan)
    };

    draw_input(f, input_area, app, input_border);
    draw_logs(f, log_area, app);
    draw_quotes_table(f, right, app, db_border);

    if app.stock_modal.visible {
        draw_stock_modal(f, app);
    }

    let area = f.area();
    app.notifications.render(f, area);
}

/// Renders the ticker input box with a blinking cursor while focused.
fn draw_input(f: &mut Frame, area: Rect, app: &App, border_color: Color) {
    let cursor = if app.input_mode.toggled && app.blink_state {
        "▌"
    } else {
        " "
    };
    let input_display = format!("{}{}", app.input_mode.input, cursor);
    let input_block = Block::default()
        .title("Query")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));
    f.render_widget(
        Paragraph::new(input_display.as_str()).block(input_block),
        area,
    );
}

/// Renders the log panel, keeping the most recent lines visible.
fn draw_logs(f: &mut Frame, area: Rect, app: &App) {
    let log_block = Block::default()
        .title("Logs")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let visible = area.height.saturating_sub(2) as usize;
    let start = app.logs.len().saturating_sub(visible);
    let log_lines: Vec<Line> = app.logs[start..]
        .iter()
        .map(|s| Line::raw(s.as_str()))
        .collect();
    f.render_widget(Paragraph::new(log_lines).block(log_block), area);
}

/// Builds a table cell from an optional value, showing `-` when absent.
fn opt_cell<T>(value: Option<T>, fmt: impl Fn(T) -> String) -> Cell<'static> {
    Cell::from(value.map(fmt).unwrap_or_else(|| "-".to_string()))
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

    let window = app.db_display.window_range();
    let rows = app.db_display.rows[window].iter().map(|q| {
        Row::new(vec![
            opt_cell(q.id, |id| id.to_string()),
            Cell::from(q.ticker.as_deref().unwrap_or("-")),
            Cell::from(q.name.as_deref().unwrap_or("-")),
            opt_cell(q.price, |p| format!("{p:.2}")),
            opt_cell(q.previous_close, |p| format!("{p:.2}")),
            opt_cell(q.day_volume, |v| format!("{v:.0}")),
            opt_cell(q.as_of, |dt| dt.format("%Y-%m-%d").to_string()),
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

    let total_pages = app.db_display.total_pages();
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
    let [_, mid, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Percentage(percent_y),
        Constraint::Fill(1),
    ])
    .areas(r);
    let [_, mid, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Percentage(percent_x),
        Constraint::Fill(1),
    ])
    .areas(mid);
    mid
}

/// A dark-gray label span for `Label: value` pairs.
fn label(text: &'static str) -> Span<'static> {
    Span::styled(text, Style::default().fg(Color::DarkGray))
}

/// A bold underlined section heading.
fn heading(text: &'static str) -> Paragraph<'static> {
    Paragraph::new(Span::styled(
        text,
        Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
    ))
}

/// Formats an optional dollar amount, showing `-` when absent.
fn money_opt(value: Option<f64>) -> String {
    value
        .map(|p| format!("${p:.2}"))
        .unwrap_or_else(|| "-".into())
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

    let [title_area, price_area, _, analysis_area] = Layout::vertical([
        Constraint::Length(1), // ticker + name
        Constraint::Length(1), // price + prev close
        Constraint::Length(1), // blank
        Constraint::Min(0),    // analysis
    ])
    .areas(inner);

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
        title_area,
    );

    f.render_widget(
        Paragraph::new(Line::from(vec![
            label("Price: "),
            Span::styled(
                money_opt(stock.price),
                Style::default().fg(price_change_color(stock)),
            ),
            Span::raw("    "),
            label("Prev Close: "),
            Span::styled(
                money_opt(stock.previous_close),
                Style::default().fg(Color::Yellow),
            ),
        ])),
        price_area,
    );

    match &app.stock_modal.analysis {
        None => {
            f.render_widget(Paragraph::new("  Loading analysis..."), analysis_area);
        }
        Some(analysis) => draw_analysis(f, analysis_area, analysis),
    }
}

/// Renders the analyst consensus and price-target sections of the modal.
fn draw_analysis(f: &mut Frame, area: Rect, analysis: &QuoteRecordAnalysis) {
    let [
        consensus_heading,
        consensus_line,
        _,
        breakdown,
        _,
        target_heading,
        target_line,
        _,
    ] = Layout::vertical([
        Constraint::Length(1), // "Analyst Consensus" heading
        Constraint::Length(1), // period + consensus rating
        Constraint::Length(1), // blank
        Constraint::Length(2), // buy/sell breakdown table
        Constraint::Length(1), // blank
        Constraint::Length(1), // "Price Target" heading
        Constraint::Length(1), // mean / low / high / analysts
        Constraint::Min(0),
    ])
    .areas(area);

    f.render_widget(heading("Analyst Consensus"), consensus_heading);
    if let Some(rec) = &analysis.recommendation_summary {
        draw_consensus(f, consensus_line, breakdown, rec);
    }

    f.render_widget(heading("Price Target"), target_heading);
    if let Some(pt) = &analysis.price_target {
        draw_price_target(f, target_line, pt);
    }
}

/// Renders the consensus rating line and the buy/sell breakdown table.
fn draw_consensus(f: &mut Frame, line_area: Rect, table_area: Rect, rec: &RecommendationSummary) {
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

    f.render_widget(
        Paragraph::new(Line::from(vec![
            label("Period: "),
            Span::raw(period_str),
            Span::raw("    "),
            label("Consensus: "),
            Span::styled(
                rating,
                Style::default()
                    .fg(rating_color(rating))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ({mean_str})"),
                Style::default().fg(Color::DarkGray),
            ),
        ])),
        line_area,
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
        table_area,
    );
}

/// Renders the mean/low/high price targets and the analyst count.
fn draw_price_target(f: &mut Frame, area: Rect, pt: &PriceTarget) {
    let money = |m: &Option<Price>| {
        m.as_ref()
            .map(|p| format!("${:.2}", p.amount()))
            .unwrap_or_else(|| "-".into())
    };

    f.render_widget(
        Paragraph::new(Line::from(vec![
            label("Mean: "),
            Span::styled(money(&pt.mean), Style::default().fg(Color::Yellow)),
            Span::raw("    "),
            label("Low: "),
            Span::styled(money(&pt.low), Style::default().fg(Color::Red)),
            Span::raw("    "),
            label("High: "),
            Span::styled(money(&pt.high), Style::default().fg(Color::Green)),
            Span::raw("    "),
            label("Analysts: "),
            Span::raw(
                pt.number_of_analysts
                    .map(|n| n.to_string())
                    .unwrap_or_else(|| "-".into()),
            ),
        ])),
        area,
    );
}

/// Maps an analyst rating string to its display colour.
fn rating_color(rating: &str) -> Color {
    match rating.to_lowercase().as_str() {
        "strong buy" => Color::LightGreen,
        "buy" => Color::Green,
        "hold" => Color::Yellow,
        "sell" => Color::Red,
        "strong sell" => Color::LightRed,
        _ => Color::White,
    }
}

/// Green when the price is above the previous close, red when below, yellow
/// when effectively unchanged.
fn price_change_color(stock: &QuoteRecord) -> Color {
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

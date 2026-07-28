//! Ratatui rendering for the TUI binary.

use std::{
    collections::{HashMap, HashSet},
    time::Instant,
};

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        Axis, Block, Borders, Cell, Chart, Clear, Dataset, GraphType, LegendPosition, Padding,
        Paragraph, Row, Table, Wrap,
    },
};
use yfinance_rs::{Candle, Price, PriceTarget, RecommendationSummary};

use crate::black_scholes::{DEFAULT_RISK_FREE_RATE, OptionKind};
use crate::models::{
    Alert, FLASH_TTL, OptionChain, OptionContract, QuoteRecord, QuoteRecordAnalysis,
    QuoteRecordStatistics, StockNews, day_change_pct,
};
use crate::portfolio::{Holding, Position, summarize, value_position};
use crate::search::subseq_match_ci;

use super::app::App;
use super::theme::Theme;

pub fn draw(f: &mut Frame, app: &mut App) {
    let t = app.theme;

    f.render_widget(
        Block::default().style(Style::default().bg(t.bg).fg(t.fg)),
        f.area(),
    );

    let [body, status_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(20), Constraint::Percentage(80)]).areas(body);
    let [input_area, log_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(left);

    let (input_border, db_border) = if app.input_mode.is_focused() {
        (t.accent, t.border)
    } else {
        (t.border, t.accent)
    };

    draw_input(f, input_area, app, input_border);
    draw_logs(f, log_area, app);
    draw_quotes_table(f, right, app, db_border);
    draw_status_bar(f, status_area, app);

    if app.stock_modal.info_visible {
        draw_stock_modal(f, app);
    }

    if app.stock_modal.chart_visible {
        draw_chart_modal(f, app);
    }

    if app.stock_modal.options_visible {
        draw_options_modal(f, app);
    }

    if app.portfolio_visible {
        draw_portfolio_modal(f, app);
    }

    if app.help_visible {
        draw_help_modal(f, app);
    }

    let area = f.area();
    app.notifications.render(f, area);
}

/// Builds a ` app · section ` block title in the bold accent colour.
fn block_title(text: String, t: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        format!(" {text} "),
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    ))
}

/// Renders the ticker input box with a blinking cursor while focused.
fn draw_input(f: &mut Frame, area: Rect, app: &App, border_color: Color) {
    let t = app.theme;
    let cursor = if app.input_mode.is_focused() && app.animations.blink_state {
        "▌"
    } else {
        " "
    };
    let title = if let Some(ticker) = app.input_mode.alert_ticker() {
        format!("assetui · alert {ticker}")
    } else if let Some(ticker) = app.input_mode.holding_ticker() {
        format!("assetui · holding {ticker}")
    } else if app.input_mode.is_fuzzy() {
        "assetui · search".to_string()
    } else {
        "assetui · query".to_string()
    };
    let input_block = Block::default()
        .title(block_title(title, &t))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color).bg(t.panel))
        .style(Style::default().bg(t.panel));
    let mut spans = Vec::with_capacity(3);
    if app.input_mode.is_fuzzy() {
        spans.push(Span::styled("/", Style::default().fg(t.dim)));
    } else if app.input_mode.alert_ticker().is_some() && app.input_mode.input.is_empty() {
        spans.push(Span::styled(">150 / <100", Style::default().fg(t.dim)));
    } else if app.input_mode.holding_ticker().is_some() && app.input_mode.input.is_empty() {
        spans.push(Span::styled("shares avg_cost", Style::default().fg(t.dim)));
    }
    spans.push(Span::styled(
        app.input_mode.input.clone(),
        Style::default().fg(t.fg),
    ));
    spans.push(Span::styled(cursor, Style::default().fg(t.accent)));
    f.render_widget(Paragraph::new(Line::from(spans)).block(input_block), area);
}

/// Renders the log panel, keeping the most recent lines visible.
fn draw_logs(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let log_block = Block::default()
        .title(block_title("assetui · logs".into(), &t))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border).bg(t.panel))
        .style(Style::default().bg(t.panel));
    let visible = area.height.saturating_sub(2) as usize;
    let start = app.logs.len().saturating_sub(visible);
    let log_lines: Vec<Line> = app
        .logs
        .iter()
        .skip(start)
        .map(|s| Line::styled(s.as_str(), Style::default().fg(t.dim)))
        .collect();
    f.render_widget(
        Paragraph::new(log_lines)
            .block(log_block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// Builds a table cell from an optional value, showing `-` when absent.
fn opt_cell<T>(value: Option<T>, fmt: impl Fn(T) -> String) -> Cell<'static> {
    Cell::from(value.map(fmt).unwrap_or_else(|| "-".to_string()))
}

fn fuzzy_cell(text: Option<&str>, query: &str, active: bool, t: &Theme) -> Cell<'static> {
    let Some(text) = text else {
        return Cell::from("-");
    };
    let query = query.trim();
    let positions = if active && !query.is_empty() {
        subseq_match_ci(text, query)
    } else {
        None
    };
    let Some(positions) = positions else {
        return Cell::from(text.to_string());
    };

    let hl = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut plain = String::new();
    let mut matched = positions.iter().peekable();
    for (byte, ch) in text.char_indices() {
        if matched.peek() == Some(&&byte) {
            matched.next();
            if !plain.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut plain)));
            }
            spans.push(Span::styled(ch.to_string(), hl));
        } else {
            plain.push(ch);
        }
    }
    if !plain.is_empty() {
        spans.push(Span::raw(plain));
    }
    Cell::from(Line::from(spans))
}

fn price_change_color(
    stock: &QuoteRecord,
    flash_map: &HashMap<String, (f64, Instant)>,
    t: &Theme,
) -> Color {
    let diff = stock
        .ticker
        .as_deref()
        .and_then(|t| flash_map.get(&t.to_ascii_uppercase()))
        .filter(|(_, ts)| ts.elapsed() < FLASH_TTL)
        .map(|(diff, _)| *diff)
        .or(match (stock.price, stock.previous_close) {
            (Some(price), Some(prev)) => Some(price - prev),
            _ => None,
        });
    match diff {
        Some(d) if d > 0.001 => t.up,
        Some(d) if d < -0.001 => t.down,
        _ => t.neutral,
    }
}

fn draw_quotes_table(f: &mut Frame, area: Rect, app: &mut App, border_color: Color) {
    let t = app.theme;

    let label_sty = Style::default().fg(t.fg).add_modifier(Modifier::BOLD);
    let key_sty = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);

    let hcell = |label: &'static str, key: &'static str| -> Cell<'static> {
        Cell::from(Line::from(vec![
            Span::styled(label, label_sty),
            Span::styled(key, key_sty),
        ]))
    };

    let header = Row::new(vec![
        Cell::from(""),
        Cell::from(""),
        hcell("ID ", "(d)"),
        hcell("Ticker ", "(t)"),
        hcell("Name", "(n)"),
        hcell("Price ", "(p)"),
        hcell("Chg%", ""),
        hcell("Prev Close ", "(c)"),
        hcell("Volume ", "(v)"),
        hcell("As Of ", "(a)"),
    ])
    .bottom_margin(1);

    let window = app.db_display.window_range();
    let query = &app.input_mode.input;
    let searching = app.input_mode.is_fuzzy();

    if app.db_display.rows.is_empty() {
        let centered = centered_rect(40, 10, area);
        f.render_widget(
            Paragraph::new(Span::styled(
                "no subscribed stocks",
                Style::default().fg(t.dim),
            ))
            .alignment(Alignment::Center),
            centered,
        );
        return;
    }
    let rows = app.db_display.rows[window].iter().map(|q| {
        let star = if is_watchlisted(q.ticker.as_deref(), &app.db_display.watchlist) {
            Cell::from(Span::styled("★", Style::default().fg(t.gold)))
        } else {
            Cell::from("")
        };
        let bell = if has_active_alert(q.ticker.as_deref(), &app.alerts) {
            Cell::from(Span::styled("!", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)))
        } else {
            Cell::from("")
        };
        Row::new(vec![
            star,
            bell,
            opt_cell(q.id, |id| id.to_string()),
            fuzzy_cell(q.ticker.as_deref(), query, searching, &t),
            fuzzy_cell(q.name.as_deref(), query, searching, &t),
            opt_cell(q.price, |p| format!("{p:.2}")).style(
                Style::default().fg(price_change_color(q, &app.animations.row_flash_map, &t)),
            ),
            opt_cell(day_change_pct(q.price, q.previous_close), |pct| {
                format!("{pct:+.2}%")
            })
            .style(Style::default().fg(price_change_color(q, &app.animations.row_flash_map, &t))),
            opt_cell(q.previous_close, |p| format!("{p:.2}")),
            opt_cell(q.day_volume, fmt_volume),
            opt_cell(q.as_of, |dt| dt.format("%Y-%m-%d").to_string()),
        ])
    });

    let widths = [
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(24),
        Constraint::Length(11),
        Constraint::Length(9),
        Constraint::Length(15),
        Constraint::Length(13),
        Constraint::Length(12),
    ];

    let mut quotes_table_title_text = "assetui · ".to_string();
    quotes_table_title_text.push_str(if app.db_display.watchlist_only {
        "watchlist"
    } else {
        "quotes"
    });

    let mut title = vec![Span::styled(
        quotes_table_title_text,
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
    )];
    if searching && !query.trim().is_empty() {
        title.push(Span::styled(
            format!("filter: {} ", query.trim()),
            Style::default().fg(t.dim),
        ));
    }
    if !app.db_display.status.is_empty() {
        title.push(Span::styled(
            format!("{} ", app.db_display.status),
            Style::default().fg(t.dim),
        ));
    }

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(Line::from(title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        )
        .row_highlight_style(Style::default().bg(t.cursor).add_modifier(Modifier::BOLD))
        .highlight_symbol(Span::styled(
            "▸ ",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_stateful_widget(table, area, &mut app.db_display.table_state);
}

fn is_watchlisted(ticker: Option<&str>, watchlist: &HashSet<String>) -> bool {
    ticker.is_some_and(|t| watchlist.contains(&t.to_ascii_uppercase()))
}

fn has_active_alert(ticker: Option<&str>, alerts: &[Alert]) -> bool {
    ticker.is_some_and(|t| alerts.iter().any(|a| a.ticker.eq_ignore_ascii_case(t)))
}

fn draw_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    f.render_widget(
        Block::default().style(Style::default().bg(t.statusbar)),
        area,
    );

    let (mode, hint) = if app.help_visible {
        ("HELP", "esc close")
    } else if app.stock_modal.options_visible {
        ("VIEW", "h/l expiry · esc close")
    } else if app.portfolio_visible {
        ("VIEW", "P/esc close")
    } else if app.stock_modal.info_visible || app.stock_modal.chart_visible {
        ("VIEW", "s info · enter chart · esc close")
    } else if app.input_mode.alert_ticker().is_some() {
        ("ALERT", "type >150 or <100 · enter arm · esc cancel")
    } else if app.input_mode.holding_ticker().is_some() {
        ("HOLDING", "type shares avg_cost · enter save · esc cancel")
    } else if app.input_mode.is_focused() && app.input_mode.is_fuzzy() {
        ("SEARCH", "type to filter · enter apply · esc cancel")
    } else if app.input_mode.is_focused() {
        ("INPUT", "type ticker · enter fetch · esc done")
    } else if app.input_mode.is_fuzzy() {
        (
            "FILTER",
            "j/k move · s info · enter chart · w ★ · / new search · esc clear · ? help",
        )
    } else {
        (
            "NORMAL",
            "j/k move · h/l page · s info · enter chart · x options · A alert · H/R/P holdings · w ★ · i query · / search · o order · ? help · q quit",
        )
    };

    let chip = format!(" {mode} ");
    let page = app.db_display.page + 1;
    let total_pages = app.db_display.total_pages();
    let count = if app.input_mode.is_fuzzy() {
        format!(
            "{}/{}",
            app.db_display.rows.len(),
            app.db_display.all_rows.len()
        )
    } else {
        app.db_display.rows.len().to_string()
    };
    let right = format!("{count} quotes · page {page}/{total_pages} ");

    let [chip_area, hint_area, right_area] = Layout::horizontal([
        Constraint::Length(chip.len() as u16),
        Constraint::Min(0),
        Constraint::Length(right.len() as u16),
    ])
    .areas(area);

    f.render_widget(
        Paragraph::new(Span::styled(
            chip,
            Style::default()
                .fg(t.mode_fg)
                .bg(t.mode_bg)
                .add_modifier(Modifier::BOLD),
        )),
        chip_area,
    );
    f.render_widget(
        Paragraph::new(Span::styled(
            format!(" {hint}"),
            Style::default().fg(t.status_fg),
        )),
        hint_area,
    );
    f.render_widget(
        Paragraph::new(Span::styled(right, Style::default().fg(t.dim))),
        right_area,
    );
}

fn help_row(keys: &'static str, desc: &'static str, t: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {keys:<14}"),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(desc, Style::default().fg(t.fg)),
    ])
}

/// Renders the keybinding help overlay: a centred reference of every command key, grouped by purpose.
fn draw_help_modal(f: &mut Frame, app: &App) {
    let t = app.theme;
    let area = centered_rect(48, 80, f.area());
    f.render_widget(Clear, area);

    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            "assetui",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · keybindings ", Style::default().fg(t.dim)),
    ]);
    let modal = modal_block(title, &t);
    let inner = modal.inner(area);
    f.render_widget(modal, area);

    let [content, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    let lines = vec![
        section_heading("NAVIGATION", None, &t),
        help_row("j / k", "move selection up / down", &t),
        help_row("h / l", "previous / next page", &t),
        Line::raw(""),
        section_heading("VIEWS", None, &t),
        help_row("s", "stock details", &t),
        help_row("enter", "price chart", &t),
        help_row("x", "options chain (h/l expiry)", &t),
        help_row("?", "this help", &t),
        Line::raw(""),
        section_heading("WATCHLIST", None, &t),
        help_row("w / r", "add / remove favourite (★)", &t),
        Line::raw(""),
        section_heading("ALERTS", None, &t),
        help_row("A", "arm price alert (>150 / <100)", &t),
        Line::from(Span::styled(
            "  fires once for on-screen tickers only",
            Style::default().fg(t.dim),
        )),
        Line::raw(""),
        section_heading("PORTFOLIO", None, &t),
        help_row("P", "toggle portfolio view", &t),
        help_row("H / R", "add-edit / remove holding (shares avg_cost)", &t),
        Line::raw(""),
        section_heading("QUERY & SEARCH", None, &t),
        help_row("i", "fetch tickers (input box)", &t),
        help_row("/", "fuzzy search the table", &t),
        Line::raw(""),
        section_heading("SORT", None, &t),
        help_row("o", "toggle ascending / descending", &t),
        help_row("d t n p c v a", "sort by column", &t),
        Line::raw(""),
        section_heading("GENERAL", None, &t),
        help_row("esc", "close modal / cancel", &t),
        help_row("q", "quit", &t),
    ];
    f.render_widget(Paragraph::new(lines), content);

    f.render_widget(
        Paragraph::new(Span::styled("esc close ", Style::default().fg(t.dim)))
            .alignment(Alignment::Right),
        footer,
    );
}

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

fn modal_block(title: Line<'static>, t: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border).bg(t.panel))
        .title(title)
        .padding(Padding::new(2, 2, 1, 0))
        .style(Style::default().bg(t.panel))
}

fn modal_title(ticker: &str, suffix: &str, t: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            ticker.to_string(),
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" · {suffix} "), Style::default().fg(t.dim)),
    ])
}

/// A dimmed label span for `label value` pairs.
fn label(text: &'static str, t: &Theme) -> Span<'static> {
    Span::styled(text, Style::default().fg(t.dim))
}

fn section_heading(text: &'static str, meta: Option<String>, t: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(
        text,
        Style::default().fg(t.dim).add_modifier(Modifier::BOLD),
    )];
    if let Some(meta) = meta {
        spans.push(Span::styled(
            format!(" · {meta}"),
            Style::default().fg(t.dim),
        ));
    }
    Line::from(spans)
}

/// A dimmed "Loading…"-style placeholder paragraph.
fn placeholder(text: &'static str, t: &Theme) -> Paragraph<'static> {
    Paragraph::new(Span::styled(text, Style::default().fg(t.dim)))
}

/// Formats an optional dollar amount, showing `-` when absent.
fn money_opt(value: Option<f64>) -> String {
    value
        .map(|p| format!("${p:.2}"))
        .unwrap_or_else(|| "-".into())
}

/// Human-readable magnitude with a K/M/B/T suffix: `55_000_000.0` → `"55.0M"`, `3.5e12` → `"3.5T"`.
fn fmt_volume(v: f64) -> String {
    let abs = v.abs();
    if abs >= 1e12 {
        format!("{:.1}T", v / 1e12)
    } else if abs >= 1e9 {
        format!("{:.1}B", v / 1e9)
    } else if abs >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if abs >= 1e3 {
        format!("{:.1}K", v / 1e3)
    } else {
        format!("{v:.0}")
    }
}

fn fmt_market_cap(value: Option<f64>) -> String {
    value
        .map(|v| format!("${}", fmt_volume(v)))
        .unwrap_or_else(|| "-".into())
}

/// Formats a fractional rate as a percentage (`Some(0.025)` → `"2.50%"`).
fn fmt_pct_opt(value: Option<f64>) -> String {
    value
        .map(|v| format!("{:.2}%", v * 100.0))
        .unwrap_or_else(|| "-".into())
}

/// Formats an optional unitless ratio to two decimals (P/E, beta).
fn fmt_ratio_opt(value: Option<f64>) -> String {
    value
        .map(|v| format!("{v:.2}"))
        .unwrap_or_else(|| "-".into())
}

fn fmt_news_meta(
    publisher: Option<&str>,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
) -> String {
    let date = published_at.map(|d| d.format("%Y-%m-%d").to_string());
    match (publisher, date) {
        (Some(p), Some(d)) => format!("{p} · {d}"),
        (Some(p), None) => p.to_string(),
        (None, Some(d)) => d,
        (None, None) => String::new(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

/// Renders the stock-detail overlay modal at 65 × 55% of the terminal area.
fn draw_stock_modal(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = centered_rect(65, 55, f.area());
    f.render_widget(Clear, area);
    let stock = &app.stock_modal.stock;
    let modal = modal_block(
        modal_title(
            stock.ticker.as_deref().unwrap_or("-"),
            stock.name.as_deref().unwrap_or("-"),
            &t,
        ),
        &t,
    );

    let inner = modal.inner(area);
    f.render_widget(modal, area);

    let [content, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    let mut lines: Vec<Line> = vec![
        price_line(stock, &app.animations.row_flash_map, &t),
        Line::raw(""),
        Line::from(vec![
            label("prev close  ", &t),
            Span::styled(money_opt(stock.previous_close), Style::default().fg(t.fg)),
            Span::raw("      "),
            label("volume  ", &t),
            Span::styled(
                stock
                    .day_volume
                    .map(fmt_volume)
                    .unwrap_or_else(|| "-".into()),
                Style::default().fg(t.fg),
            ),
        ]),
        Line::from(vec![
            label("as of       ", &t),
            Span::styled(
                stock
                    .as_of
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| "-".into()),
                Style::default().fg(t.fg),
            ),
        ]),
        Line::raw(""),
    ];

    match &app.stock_modal.statistics {
        None => {
            lines.push(section_heading("KEY STATISTICS", None, &t));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "loading statistics…",
                Style::default().fg(t.dim),
            ));
        }
        Some(stats) => push_statistics_lines(&mut lines, stats, &t),
    }
    lines.push(Line::raw(""));

    match &app.stock_modal.analysis {
        None => {
            lines.push(section_heading("ANALYST CONSENSUS", None, &t));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "loading analysis…",
                Style::default().fg(t.dim),
            ));
        }
        Some(analysis) => {
            push_analysis_lines(
                &mut lines,
                analysis,
                stock.price,
                content.width as usize,
                &t,
            );
        }
    }
    lines.push(Line::raw(""));

    match &app.stock_modal.news {
        None => {
            lines.push(section_heading("NEWS", None, &t));
            lines.push(Line::raw(""));
            lines.push(Line::styled("loading news…", Style::default().fg(t.dim)));
        }
        Some(news) => push_news_lines(&mut lines, news, content.width as usize, &t),
    }

    f.render_widget(Paragraph::new(lines), content);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("enter", Style::default().fg(t.accent)),
            Span::styled(" chart · ", Style::default().fg(t.dim)),
            Span::styled("esc", Style::default().fg(t.accent)),
            Span::styled(" close", Style::default().fg(t.dim)),
        ]))
        .alignment(Alignment::Right),
        footer,
    );
}

fn price_line(
    stock: &QuoteRecord,
    flash_map: &HashMap<String, (f64, Instant)>,
    t: &Theme,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        money_opt(stock.price),
        Style::default()
            .fg(price_change_color(stock, flash_map, t))
            .add_modifier(Modifier::BOLD),
    )];
    if let (Some(price), Some(prev)) = (stock.price, stock.previous_close)
        && let Some(pct) = day_change_pct(stock.price, stock.previous_close)
    {
        let diff = price - prev;
        let (arrow, color) = if diff > 0.001 {
            ("▲", t.up)
        } else if diff < -0.001 {
            ("▼", t.down)
        } else {
            ("·", t.neutral)
        };
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("{arrow} ${:.2} ({pct:+.2}%)", diff.abs()),
            Style::default().fg(color),
        ));
    }
    Line::from(spans)
}

/// Appends the ANALYST CONSENSUS and PRICE TARGET sections to `lines`.
fn push_analysis_lines(
    lines: &mut Vec<Line<'static>>,
    analysis: &QuoteRecordAnalysis,
    current_price: Option<f64>,
    width: usize,
    t: &Theme,
) {
    let period = analysis
        .recommendation_summary
        .as_ref()
        .and_then(|r| r.latest_period.as_ref().map(|p| p.to_string()));
    lines.push(section_heading("ANALYST CONSENSUS", period, t));
    lines.push(Line::raw(""));
    match &analysis.recommendation_summary {
        Some(rec) => push_consensus_lines(lines, rec, width, t),
        None => lines.push(Line::styled(
            "no analyst coverage",
            Style::default().fg(t.dim),
        )),
    }
    lines.push(Line::raw(""));

    let analysts = analysis
        .price_target
        .as_ref()
        .and_then(|p| p.number_of_analysts)
        .map(|n| format!("{n} analysts"));
    lines.push(section_heading("PRICE TARGET", analysts, t));
    lines.push(Line::raw(""));
    match &analysis.price_target {
        Some(pt) => push_target_lines(lines, pt, current_price, width, t),
        None => lines.push(Line::styled(
            "no price target data",
            Style::default().fg(t.dim),
        )),
    }
}

fn push_statistics_lines(
    lines: &mut Vec<Line<'static>>,
    stats: &QuoteRecordStatistics,
    t: &Theme,
) {
    lines.push(section_heading("KEY STATISTICS", None, t));
    lines.push(Line::raw(""));

    let value = |s: String| Span::styled(s, Style::default().fg(t.fg));

    lines.push(Line::from(vec![
        label("market cap  ", t),
        value(fmt_market_cap(stats.market_cap)),
        Span::raw("    "),
        label("p/e  ", t),
        value(fmt_ratio_opt(stats.pe_ratio)),
    ]));
    lines.push(Line::from(vec![
        label("eps         ", t),
        value(money_opt(stats.eps)),
        Span::raw("    "),
        label("beta ", t),
        value(fmt_ratio_opt(stats.beta)),
    ]));
    lines.push(Line::from(vec![
        label("div yield   ", t),
        value(fmt_pct_opt(stats.dividend_yield)),
    ]));
    lines.push(Line::from(vec![
        label("52w low     ", t),
        value(money_opt(stats.fifty_two_week_low)),
        Span::raw("    "),
        label("52w high ", t),
        value(money_opt(stats.fifty_two_week_high)),
    ]));
}

fn push_news_lines(lines: &mut Vec<Line<'static>>, news: &StockNews, width: usize, t: &Theme) {
    /// How many headlines to show before the section would crowd the modal.
    const MAX_HEADLINES: usize = 4;

    lines.push(section_heading("NEWS", None, t));
    lines.push(Line::raw(""));

    if news.items.is_empty() {
        lines.push(Line::styled("no recent news", Style::default().fg(t.dim)));
        return;
    }

    for item in news.items.iter().take(MAX_HEADLINES) {
        lines.push(Line::styled(
            truncate(&item.title, width.saturating_sub(2)),
            Style::default().fg(t.fg),
        ));
        let meta = fmt_news_meta(item.publisher.as_deref(), item.published_at);
        if !meta.is_empty() {
            lines.push(Line::styled(format!("  {meta}"), Style::default().fg(t.dim)));
        }
    }
}

fn push_consensus_lines(
    lines: &mut Vec<Line<'static>>,
    rec: &RecommendationSummary,
    width: usize,
    t: &Theme,
) {
    let rating = rec.mean_rating_text.as_deref().unwrap_or("-");
    let mut head = vec![Span::styled(
        format!(" {} ", rating.to_uppercase()),
        Style::default()
            .fg(t.mode_fg)
            .bg(rating_color(rating, t))
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(mean) = rec.mean {
        head.push(Span::styled(
            format!("  {mean:.1} mean score"),
            Style::default().fg(t.dim),
        ));
    }
    lines.push(Line::from(head));

    let buckets = [
        (rec.strong_buy.unwrap_or(0), t.up_strong, "str buy"),
        (rec.buy.unwrap_or(0), t.up, "buy"),
        (rec.hold.unwrap_or(0), t.neutral, "hold"),
        (rec.sell.unwrap_or(0), t.down, "sell"),
        (rec.strong_sell.unwrap_or(0), t.down_strong, "str sell"),
    ];
    let total: u32 = buckets.iter().map(|(n, _, _)| n).sum();
    if total == 0 {
        return;
    }

    let bar_width = width.clamp(10, 48);
    let mut bar: Vec<Span> = Vec::new();
    let mut used = 0usize;
    let mut acc = 0u32;
    for (n, color, _) in buckets {
        acc += n;
        let end = ((f64::from(acc) / f64::from(total)) * bar_width as f64).round() as usize;
        if end > used {
            bar.push(Span::styled(
                "█".repeat(end - used),
                Style::default().fg(color),
            ));
            used = end;
        }
    }
    lines.push(Line::from(bar));

    let mut legend: Vec<Span> = Vec::new();
    for (n, color, name) in buckets {
        if n == 0 {
            continue;
        }
        if !legend.is_empty() {
            legend.push(Span::styled(" · ", Style::default().fg(t.dim)));
        }
        legend.push(Span::styled(
            format!("{n} "),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ));
        legend.push(Span::styled(name, Style::default().fg(t.dim)));
    }
    lines.push(Line::from(legend));
}

fn push_target_lines(
    lines: &mut Vec<Line<'static>>,
    pt: &PriceTarget,
    current_price: Option<f64>,
    width: usize,
    t: &Theme,
) {
    let to_f64 = |p: &Option<Price>| p.as_ref().map(|p| p.amount().as_f64());
    let low = to_f64(&pt.low);
    let high = to_f64(&pt.high);
    let mean = to_f64(&pt.mean);

    let gauge = match (low, high) {
        (Some(lo), Some(hi)) if hi > lo => target_gauge(lo, hi, mean, current_price, width, t),
        _ => None,
    };
    match gauge {
        Some(gauge) => lines.push(gauge),
        None => lines.push(Line::from(vec![
            label("low  ", t),
            Span::styled(money_opt(low), Style::default().fg(t.down)),
            Span::raw("    "),
            label("mean  ", t),
            Span::styled(money_opt(mean), Style::default().fg(t.neutral)),
            Span::raw("    "),
            label("high  ", t),
            Span::styled(money_opt(high), Style::default().fg(t.up)),
        ])),
    }

    let mut summary: Vec<Span> = Vec::new();
    if let Some(cur) = current_price {
        summary.push(Span::styled("● ", Style::default().fg(t.accent)));
        summary.push(label("price ", t));
        summary.push(Span::styled(
            format!("${cur:.2}"),
            Style::default().fg(t.fg),
        ));
        summary.push(Span::raw("   "));
    }
    summary.push(Span::styled("◆ ", Style::default().fg(t.neutral)));
    summary.push(label("mean ", t));
    summary.push(Span::styled(money_opt(mean), Style::default().fg(t.fg)));
    if let (Some(m), Some(cur)) = (mean, current_price)
        && cur > 0.0
    {
        let pct = (m - cur) / cur * 100.0;
        let (arrow, color) = if pct >= 0.0 {
            ("▲", t.up)
        } else {
            ("▼", t.down)
        };
        summary.push(Span::styled(" · ", Style::default().fg(t.dim)));
        summary.push(Span::styled(
            format!("{arrow} {pct:+.1}%"),
            Style::default().fg(color),
        ));
        summary.push(label(" to mean", t));
    }
    lines.push(Line::from(summary));
}

fn target_gauge(
    low: f64,
    high: f64,
    mean: Option<f64>,
    current: Option<f64>,
    width: usize,
    t: &Theme,
) -> Option<Line<'static>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Track,
        Mean,
        Current,
    }

    let low_label = format!("${low:.2} ");
    let high_label = format!(" ${high:.2}");
    let track_len = width.checked_sub(low_label.len() + high_label.len() + 2)?;
    if track_len < 8 {
        return None;
    }

    let pos = |v: f64| -> usize {
        let frac = (v - low) / (high - low);
        ((frac * (track_len - 1) as f64).round()).clamp(0.0, (track_len - 1) as f64) as usize
    };
    let mut cells = vec![Mark::Track; track_len];
    if let Some(m) = mean {
        cells[pos(m)] = Mark::Mean;
    }
    if let Some(c) = current {
        cells[pos(c)] = Mark::Current;
    }

    let dim = Style::default().fg(t.dim);
    let mut spans = vec![
        Span::styled(low_label, Style::default().fg(t.down)),
        Span::styled("├", dim),
    ];
    let mut run = 0usize;
    for cell in cells {
        match cell {
            Mark::Track => run += 1,
            mark => {
                if run > 0 {
                    spans.push(Span::styled("─".repeat(run), dim));
                    run = 0;
                }
                spans.push(match mark {
                    Mark::Current => Span::styled(
                        "●",
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                    ),
                    _ => Span::styled(
                        "◆",
                        Style::default().fg(t.neutral).add_modifier(Modifier::BOLD),
                    ),
                });
            }
        }
    }
    if run > 0 {
        spans.push(Span::styled("─".repeat(run), dim));
    }
    spans.push(Span::styled("┤", dim));
    spans.push(Span::styled(high_label, Style::default().fg(t.up)));
    Some(Line::from(spans))
}

/// Maps an analyst rating string to its display colour.
fn rating_color(rating: &str, t: &Theme) -> Color {
    match rating.to_lowercase().as_str() {
        "strong buy" => t.up_strong,
        "buy" => t.up,
        "hold" => t.neutral,
        "sell" => t.down,
        "strong sell" => t.down_strong,
        _ => t.fg,
    }
}

/// Returns the chain's contracts of `kind` on the expiry at `expiry_index`, sorted ascending by strike.
fn contracts_for_expiry(
    chain: &OptionChain,
    expiry_index: usize,
    kind: OptionKind,
) -> Vec<&OptionContract> {
    let Some(&expiry) = chain.expirations.get(expiry_index) else {
        return Vec::new();
    };
    let target = expiry.date_naive();
    let mut out: Vec<&OptionContract> = chain
        .contracts
        .iter()
        .filter(|c| c.kind == kind && c.expiry.date_naive() == target)
        .collect();
    out.sort_by(|a, b| {
        a.strike
            .partial_cmp(&b.strike)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

fn draw_options_modal(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = centered_rect(85, 72, f.area());
    f.render_widget(Clear, area);
    let ticker = app.stock_modal.stock.ticker.as_deref().unwrap_or("-");
    let modal = modal_block(modal_title(ticker, "options", &t), &t);
    let inner = modal.inner(area);
    f.render_widget(modal, area);

    let Some(chain) = &app.stock_modal.option_chain else {
        f.render_widget(placeholder("Loading options…", &t), inner);
        return;
    };
    if chain.expirations.is_empty() {
        f.render_widget(placeholder("No options for this ticker.", &t), inner);
        return;
    }
    if inner.width < 44 {
        f.render_widget(placeholder("Terminal too narrow for options.", &t), inner);
        return;
    }

    let [selector_area, tables_area, footer_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);

    let idx = app
        .stock_modal
        .selected_expiry
        .min(chain.expirations.len() - 1);
    let expiry = chain.expirations[idx];
    let mut selector_spans = vec![
        label("Expiry  ", &t),
        Span::styled("‹ ", Style::default().fg(t.accent)),
        Span::styled(
            expiry.format("%Y-%m-%d").to_string(),
            Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ›", Style::default().fg(t.accent)),
        Span::styled(
            format!("   {}/{}", idx + 1, chain.expirations.len()),
            Style::default().fg(t.dim),
        ),
    ];
    if let Some(spot) = chain.underlying_price {
        selector_spans.push(Span::styled(
            format!("      spot ${spot:.2}"),
            Style::default().fg(t.dim),
        ));
    }
    f.render_widget(Paragraph::new(Line::from(selector_spans)), selector_area);

    let [calls_area, puts_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(tables_area);
    let calls = contracts_for_expiry(chain, idx, OptionKind::Call);
    let puts = contracts_for_expiry(chain, idx, OptionKind::Put);
    let spot = chain.underlying_price;
    let now = chrono::Utc::now();
    draw_option_table(f, calls_area, "CALLS", &calls, spot, now, &t);
    draw_option_table(f, puts_area, "PUTS", &puts, spot, now, &t);

    let key_sty = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(t.dim);
    let footer = Line::from(vec![
        Span::styled("h/l", key_sty),
        Span::styled(" expiry · ", dim),
        Span::styled("esc", key_sty),
        Span::styled(" close", dim),
    ])
    .alignment(Alignment::Right);
    f.render_widget(Paragraph::new(footer), footer_area);
}

fn draw_option_table(
    f: &mut Frame,
    area: Rect,
    title: &'static str,
    contracts: &[&OptionContract],
    spot: Option<f64>,
    now: chrono::DateTime<chrono::Utc>,
    t: &Theme,
) {
    let wide = area.width >= 46;
    let greeks = area.width >= 96;

    let head_sty = Style::default().fg(t.dim).add_modifier(Modifier::BOLD);
    let mut headers = vec!["Strike", "Bid", "Ask", "Last", "IV"];
    if wide {
        headers.push("Vol");
        headers.push("OI");
    }
    if greeks {
        headers.extend(["Theo", "Δ", "Γ", "Θ/d", "Vega", "ρ"]);
    }
    let header = Row::new(
        headers
            .into_iter()
            .map(|h| Cell::from(Span::styled(h, head_sty)))
            .collect::<Vec<_>>(),
    )
    .bottom_margin(1);

    let rows = contracts.iter().map(|c| {
        let mut cells = vec![
            Cell::from(format!("{:.2}", c.strike))
                .style(Style::default().fg(t.fg).add_modifier(Modifier::BOLD)),
            opt_cell(c.bid, |v| format!("{v:.2}")),
            opt_cell(c.ask, |v| format!("{v:.2}")),
            opt_cell(c.last, |v| format!("{v:.2}")),
            opt_cell(c.implied_volatility, |iv| format!("{:.1}%", iv * 100.0)),
        ];
        if wide {
            cells.push(opt_cell(c.volume, |v| fmt_volume(v as f64)));
            cells.push(opt_cell(c.open_interest, |v| fmt_volume(v as f64)));
        }
        if greeks {
            let a = c.analytics(spot, DEFAULT_RISK_FREE_RATE, 0.0, now);
            cells.push(opt_cell(a.map(|a| a.theo_price), |v| format!("{v:.2}")));
            cells.push(opt_cell(a.map(|a| a.greeks.delta), |v| format!("{v:.3}")));
            cells.push(opt_cell(a.map(|a| a.greeks.gamma), |v| format!("{v:.4}")));
            cells.push(opt_cell(a.map(|a| a.greeks.theta / 365.0), |v| format!("{v:.3}")));
            cells.push(opt_cell(a.map(|a| a.greeks.vega / 100.0), |v| format!("{v:.3}")));
            cells.push(opt_cell(a.map(|a| a.greeks.rho / 100.0), |v| format!("{v:.3}")));
        }
        Row::new(cells)
    });

    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(7),
    ];
    if wide {
        widths.extend([Constraint::Length(6), Constraint::Length(6)]);
    }
    if greeks {
        widths.extend([
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Length(7),
        ]);
    }

    let block = Block::default()
        .title(section_heading(title, Some(format!("{}", contracts.len())), t))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border));

    if contracts.is_empty() {
        let empty = Paragraph::new(Span::styled("no contracts", Style::default().fg(t.dim)))
            .block(block);
        f.render_widget(empty, area);
        return;
    }

    let table = Table::new(rows, widths).header(header).block(block);
    f.render_widget(table, area);
}

fn resolve_price(ticker: &str, rows: &[QuoteRecord]) -> (Option<f64>, Option<f64>) {
    rows.iter()
        .find(|r| r.ticker.as_deref().is_some_and(|t| t.eq_ignore_ascii_case(ticker)))
        .map_or((None, None), |r| (r.price, r.previous_close))
}

pub(crate) fn portfolio_positions(holdings: &[Holding], rows: &[QuoteRecord]) -> Vec<Position> {
    holdings
        .iter()
        .map(|h| {
            let (price, previous_close) = resolve_price(&h.ticker, rows);
            value_position(h.clone(), price, previous_close)
        })
        .collect()
}

fn pnl_color(value: Option<f64>, t: &Theme) -> Color {
    match value {
        Some(v) if v > f64::EPSILON => t.up,
        Some(v) if v < -f64::EPSILON => t.down,
        Some(_) => t.neutral,
        None => t.dim,
    }
}

/// A right-aligned signed-money cell (`+1.20` / `-3.40` / `-` when absent), coloured by sign.
fn pnl_cell(value: Option<f64>, t: &Theme) -> Cell<'static> {
    opt_cell(value, |v| format!("{v:+.2}")).style(Style::default().fg(pnl_color(value, t)))
}

fn draw_portfolio_modal(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = centered_rect(80, 70, f.area());
    f.render_widget(Clear, area);
    let modal = modal_block(modal_title("PORTFOLIO", "holdings", &t), &t);
    let inner = modal.inner(area);
    f.render_widget(modal, area);

    if app.holdings.is_empty() {
        f.render_widget(
            placeholder("No holdings — add one with H from a quote row.", &t),
            inner,
        );
        return;
    }

    let positions = portfolio_positions(&app.holdings, &app.db_display.all_rows);
    let summary = summarize(&positions);

    let [table_area, totals_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(2)]).areas(inner);

    let head_sty = Style::default().fg(t.dim).add_modifier(Modifier::BOLD);
    let header = Row::new(
        ["Ticker", "Shares", "Avg Cost", "Price", "Mkt Value", "P&L $", "P&L %", "Day $"]
            .into_iter()
            .map(|h| Cell::from(Span::styled(h, head_sty)))
            .collect::<Vec<_>>(),
    )
    .bottom_margin(1);

    let rows = positions.iter().map(|p| {
        Row::new(vec![
            Cell::from(Span::styled(
                p.holding.ticker.clone(),
                Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
            )),
            Cell::from(format!("{}", p.holding.shares)),
            Cell::from(format!("${:.2}", p.holding.avg_cost)),
            opt_cell(p.price, |v| format!("${v:.2}")),
            opt_cell(p.market_value, |v| format!("${v:.2}")),
            pnl_cell(p.unrealized_pnl, &t),
            opt_cell(p.unrealized_pnl_pct, |v| format!("{v:+.2}%"))
                .style(Style::default().fg(pnl_color(p.unrealized_pnl_pct, &t))),
            pnl_cell(p.day_change, &t),
        ])
    });

    let widths = [
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(11),
        Constraint::Length(11),
        Constraint::Length(13),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(12),
    ];
    f.render_widget(Table::new(rows, widths).header(header), table_area);

    let pct = summary
        .total_pnl_pct
        .map_or_else(|| "-".to_string(), |v| format!("{v:+.2}%"));
    let totals = Line::from(vec![
        label("Cost ", &t),
        Span::styled(format!("${:.2}   ", summary.total_cost), Style::default().fg(t.fg)),
        label("Value ", &t),
        Span::styled(format!("${:.2}   ", summary.total_market_value), Style::default().fg(t.fg)),
        label("P&L ", &t),
        Span::styled(
            format!("{:+.2} ({pct})   ", summary.total_pnl),
            Style::default()
                .fg(pnl_color(Some(summary.total_pnl), &t))
                .add_modifier(Modifier::BOLD),
        ),
        label("Day ", &t),
        Span::styled(
            format!("{:+.2}", summary.total_day_change),
            Style::default().fg(pnl_color(Some(summary.total_day_change), &t)),
        ),
    ]);
    f.render_widget(
        Paragraph::new(totals).block(Block::default().borders(Borders::TOP).border_style(Style::default().fg(t.border))),
        totals_area,
    );
}

fn draw_chart_modal(f: &mut Frame, app: &mut App) {
    let t = app.theme;
    let area = centered_rect(65, 55, f.area());
    f.render_widget(Clear, area);
    let stock = &app.stock_modal.stock;
    let modal = modal_block(
        modal_title(stock.ticker.as_deref().unwrap_or("-"), "ytd", &t),
        &t,
    );

    let inner = modal.inner(area);
    f.render_widget(modal, area);

    match &app.stock_modal.chart_data {
        None => {
            f.render_widget(placeholder("Loading data...", &t), inner);
        }
        Some(candles) => draw_stock_chart(
            f,
            inner,
            candles,
            stock.ticker.as_deref().unwrap_or("-"),
            &t,
        ),
    }
}

fn draw_stock_chart(f: &mut Frame, area: Rect, candles: &[Candle], ticker: &str, t: &Theme) {
    if candles.is_empty() {
        f.render_widget(placeholder("No chart data.", t), area);
        return;
    }

    let data = candles
        .iter()
        .enumerate()
        .map(|(i, c)| (i as f64, c.ohlc.close.as_decimal().round_dp(2).as_f64()))
        .collect::<Vec<(f64, f64)>>();

    let first = data[0].1;
    let last = data[data.len() - 1].1;
    let change = last - first;
    let pct = if first != 0.0 {
        change / first * 100.0
    } else {
        0.0
    };
    let trend_up = change >= 0.0;
    let trend_color = if trend_up { t.up } else { t.down };
    let sign = if trend_up { "+" } else { "" };

    let mut up_pts: Vec<(f64, f64)> = Vec::new();
    let mut down_pts: Vec<(f64, f64)> = Vec::new();
    for (i, &pt) in data.iter().enumerate() {
        let prev = if i == 0 { pt.1 } else { data[i - 1].1 };
        if pt.1 >= prev {
            up_pts.push(pt);
        } else {
            down_pts.push(pt);
        }
    }

    let legend = format!("{ticker}  ${last:.2}  ({sign}{pct:.1}%)");
    let trend_line = Dataset::default()
        .name(legend)
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(trend_color))
        .data(&data);
    let up_dots = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Scatter)
        .style(Style::default().fg(t.up_strong))
        .data(&up_pts);
    let down_dots = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Scatter)
        .style(Style::default().fg(t.down_strong))
        .data(&down_pts);

    let axis_style = Style::default().fg(t.dim);
    let title_style = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(t.dim);

    let xmax = (candles.len().saturating_sub(1)) as f64;
    let mid_idx = candles.len() / 2;
    let date_label = |i: usize| {
        Line::from(Span::styled(
            candles[i].ts.format("%b %d").to_string(),
            label_style,
        ))
    };
    let x_axis = Axis::default()
        .title(Span::styled("Time", title_style))
        .style(axis_style)
        .bounds([0.0, xmax])
        .labels([
            date_label(0),
            date_label(mid_idx),
            date_label(candles.len() - 1),
        ]);

    let (ymin, ymax) = data
        .iter()
        .fold((f64::MAX, f64::MIN), |(min, max), (_, y)| {
            (min.min(*y), max.max(*y))
        });
    let pad = ((ymax - ymin) * 0.04).max(0.01);
    let (ylo, yhi) = (ymin - pad, ymax + pad);
    let price_label = |v: f64| Line::from(Span::styled(format!("{v:.2}"), label_style));
    let y_axis = Axis::default()
        .title(Span::styled("Price", title_style))
        .style(axis_style)
        .bounds([ylo, yhi])
        .labels([
            price_label(ylo),
            price_label((ylo + yhi) / 2.0),
            price_label(yhi),
        ]);

    let chart = Chart::new(vec![trend_line, up_dots, down_dots])
        .x_axis(x_axis)
        .y_axis(y_axis)
        .legend_position(Some(LegendPosition::TopLeft));
    f.render_widget(chart, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watchlist(tickers: &[&str]) -> HashSet<String> {
        tickers.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn watchlisted_ticker_is_starred() {
        let wl = watchlist(&["AAPL"]);
        assert!(is_watchlisted(Some("AAPL"), &wl));
    }

    #[test]
    fn untracked_ticker_is_not_starred() {
        let wl = watchlist(&["AAPL"]);
        assert!(!is_watchlisted(Some("TSLA"), &wl));
    }

    #[test]
    fn match_is_case_insensitive() {
        let wl = watchlist(&["AAPL"]);
        assert!(is_watchlisted(Some("aapl"), &wl));
    }

    #[test]
    fn missing_ticker_is_not_starred() {
        let wl = watchlist(&["AAPL"]);
        assert!(!is_watchlisted(None, &wl));
    }

    fn alert_on(ticker: &str) -> Alert {
        Alert {
            id: 1,
            ticker: ticker.to_string(),
            direction: crate::models::AlertDirection::Above,
            threshold: 100.0,
            created_at: chrono::Utc::now(),
            triggered_at: None,
        }
    }

    #[test]
    fn ticker_with_active_alert_is_flagged() {
        let alerts = vec![alert_on("AAPL")];
        assert!(has_active_alert(Some("AAPL"), &alerts));
    }

    #[test]
    fn alert_match_is_case_insensitive() {
        let alerts = vec![alert_on("AAPL")];
        assert!(has_active_alert(Some("aapl"), &alerts));
    }

    #[test]
    fn ticker_without_alert_is_not_flagged() {
        let alerts = vec![alert_on("AAPL")];
        assert!(!has_active_alert(Some("TSLA"), &alerts));
    }

    #[test]
    fn missing_ticker_has_no_alert() {
        let alerts = vec![alert_on("AAPL")];
        assert!(!has_active_alert(None, &alerts));
    }

    #[test]
    fn fmt_volume_shortens_trillions() {
        assert_eq!(fmt_volume(3.5e12), "3.5T");
    }

    #[test]
    fn fmt_market_cap_prefixes_dollar_and_shortens() {
        assert_eq!(fmt_market_cap(Some(3.5e12)), "$3.5T");
    }

    #[test]
    fn fmt_market_cap_absent_is_dash() {
        assert_eq!(fmt_market_cap(None), "-");
    }

    #[test]
    fn fmt_pct_opt_scales_fraction_to_percent() {
        assert_eq!(fmt_pct_opt(Some(0.025)), "2.50%");
    }

    #[test]
    fn fmt_pct_opt_absent_is_dash() {
        assert_eq!(fmt_pct_opt(None), "-");
    }

    fn opt_contract(kind: OptionKind, strike: f64, expiry: chrono::DateTime<chrono::Utc>) -> OptionContract {
        OptionContract {
            kind,
            expiry,
            strike,
            bid: None,
            ask: None,
            last: None,
            implied_volatility: None,
            volume: None,
            open_interest: None,
        }
    }

    #[test]
    fn contracts_for_expiry_filters_by_kind_and_sorts_by_strike() {
        // arrange
        let exp = chrono::DateTime::from_timestamp(1_768_521_600, 0).unwrap();
        let chain = OptionChain {
            underlying: "AAPL".into(),
            expirations: vec![exp],
            contracts: vec![
                opt_contract(OptionKind::Call, 160.0, exp),
                opt_contract(OptionKind::Put, 150.0, exp),
                opt_contract(OptionKind::Call, 140.0, exp),
                opt_contract(OptionKind::Call, 150.0, exp),
            ],
            ..Default::default()
        };

        // act
        let calls = contracts_for_expiry(&chain, 0, OptionKind::Call);

        // assert
        let strikes: Vec<f64> = calls.iter().map(|c| c.strike).collect();
        assert_eq!(strikes, vec![140.0, 150.0, 160.0]);
    }

    #[test]
    fn contracts_for_expiry_excludes_other_expiries() {
        let jan = chrono::DateTime::from_timestamp(1_768_521_600, 0).unwrap();
        let feb = chrono::DateTime::from_timestamp(1_771_200_000, 0).unwrap();
        let chain = OptionChain {
            underlying: "AAPL".into(),
            expirations: vec![jan, feb],
            contracts: vec![
                opt_contract(OptionKind::Call, 150.0, jan),
                opt_contract(OptionKind::Call, 155.0, feb),
            ],
            ..Default::default()
        };

        let calls = contracts_for_expiry(&chain, 1, OptionKind::Call);

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].strike, 155.0);
    }

    #[test]
    fn contracts_for_expiry_out_of_range_index_is_empty() {
        let chain = OptionChain::default();
        assert!(contracts_for_expiry(&chain, 5, OptionKind::Call).is_empty());
    }

    fn quote(ticker: &str, price: Option<f64>, prev: Option<f64>) -> QuoteRecord {
        QuoteRecord {
            ticker: Some(ticker.to_string()),
            price,
            previous_close: prev,
            ..Default::default()
        }
    }

    #[test]
    fn resolve_price_finds_row_case_insensitively() {
        let rows = vec![quote("AAPL", Some(150.0), Some(148.0))];
        assert_eq!(resolve_price("aapl", &rows), (Some(150.0), Some(148.0)));
    }

    #[test]
    fn resolve_price_absent_ticker_is_none() {
        let rows = vec![quote("AAPL", Some(150.0), Some(148.0))];
        assert_eq!(resolve_price("TSLA", &rows), (None, None));
    }

    #[test]
    fn pnl_color_is_up_for_gain_down_for_loss() {
        let t = Theme::for_appearance(crate::tui::theme::Appearance::Dark);
        assert_eq!(pnl_color(Some(5.0), &t), t.up);
        assert_eq!(pnl_color(Some(-5.0), &t), t.down);
        assert_eq!(pnl_color(Some(0.0), &t), t.neutral);
        assert_eq!(pnl_color(None, &t), t.dim);
    }

    #[test]
    fn fmt_ratio_opt_two_decimals() {
        assert_eq!(fmt_ratio_opt(Some(28.5)), "28.50");
    }

    #[test]
    fn fmt_ratio_opt_absent_is_dash() {
        assert_eq!(fmt_ratio_opt(None), "-");
    }

    fn dt(y: i32, m: u32, d: u32) -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    #[test]
    fn fmt_news_meta_joins_publisher_and_date() {
        assert_eq!(
            fmt_news_meta(Some("Reuters"), Some(dt(2026, 7, 24))),
            "Reuters · 2026-07-24"
        );
    }

    #[test]
    fn fmt_news_meta_date_only_when_publisher_absent() {
        assert_eq!(fmt_news_meta(None, Some(dt(2026, 7, 24))), "2026-07-24");
    }

    #[test]
    fn fmt_news_meta_publisher_only_when_date_absent() {
        assert_eq!(fmt_news_meta(Some("Reuters"), None), "Reuters");
    }

    #[test]
    fn fmt_news_meta_empty_when_both_absent() {
        assert_eq!(fmt_news_meta(None, None), "");
    }
}

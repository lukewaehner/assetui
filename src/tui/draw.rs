//! Ratatui rendering for the TUI binary.

use std::{
    collections::{HashMap, HashSet, VecDeque},
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
        Paragraph, Row, Sparkline, Table, Wrap,
    },
};
use yfinance_rs::{Candle, Price, PriceTarget, RecommendationSummary};

use crate::black_scholes::{DEFAULT_RISK_FREE_RATE, OptionKind};
use crate::models::{
    Alert, AlertDirection, FLASH_TTL, OptionChain, OptionContract, QuoteRecord,
    QuoteRecordAnalysis, QuoteRecordStatistics, StockNews, Ticker, day_change_pct,
};
use crate::models::quote::NewsItem;
use crate::portfolio::{portfolio_positions, resolve_price, summarize};
use crate::search::subseq_match_ci;

use super::app::{App, ManageInput, ManageInputKind, ManageSection, Overlay, Page};
use super::theme::Theme;
use super::treemap::squarify;

pub fn draw(f: &mut Frame, app: &mut App) {
    let t = app.theme;

    f.render_widget(
        Block::default().style(Style::default().bg(t.bg).fg(t.fg)),
        f.area(),
    );

    let [body, status_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());

    match app.page {
        Page::Market => draw_market_body(f, body, app),
        Page::Manage => draw_manage_body(f, body, app),
        Page::Sectors => draw_sectors_page(f, body, app),
        Page::News => draw_news_page(f, body, app),
    }
    draw_status_bar(f, status_area, app);

    if app.page == Page::Market {
        match app.overlay {
            Overlay::Info => draw_stock_modal(f, app),
            Overlay::Chart => draw_chart_modal(f, app),
            Overlay::Options => draw_options_modal(f, app),
            _ => {}
        }
    }
    if app.overlay == Overlay::Help {
        draw_help_modal(f, app);
    }
    if app.overlay == Overlay::Portfolio {
        draw_portfolio_modal(f, app);
    }
    if app.overlay == Overlay::PagePicker {
        draw_page_picker(f, app);
    }

    let area = f.area();
    app.notifications.render(f, area);
}

/// Renders the Market page body: the ticker input box, the log panel, and the sortable quotes table.
fn draw_market_body(f: &mut Frame, body: Rect, app: &mut App) {
    let t = app.theme;
    let [left, right] =
        Layout::horizontal([Constraint::Percentage(20), Constraint::Percentage(80)]).areas(body);
    let [input_area, log_area] =
        Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).areas(left);

    let (input_border, db_border) = if app.market.input_mode.is_focused() {
        (t.accent, t.border)
    } else {
        (t.border, t.accent)
    };

    draw_input(f, input_area, app, input_border);
    if app.show_logs {
        draw_logs(f, log_area, app);
    } else {
        draw_dashboard(f, log_area, app);
    }
    draw_quotes_table(f, right, app, db_border);
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
    let cursor = if app.market.input_mode.is_focused() && app.animations.blink_state {
        "▌"
    } else {
        " "
    };
    let title = if app.market.input_mode.is_fuzzy() {
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
    if app.market.input_mode.is_fuzzy() {
        spans.push(Span::styled("/", Style::default().fg(t.dim)));
    }
    spans.push(Span::styled(
        app.market.input_mode.input.clone(),
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
    let start = app.market.logs.len().saturating_sub(visible);
    let log_lines: Vec<Line> = app
        .market
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

fn cube_block(title: &str, t: &Theme) -> Block<'static> {
    Block::default()
        .title(block_title(title.to_string(), t))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border).bg(t.panel))
        .style(Style::default().bg(t.panel))
}

fn dim_line(text: &str, t: &Theme) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), Style::default().fg(t.dim)))
}

/// `▲`/`▼`/`•` for a positive/negative/flat value: the direction marker shared by the dashboard cubes.
fn move_arrow(value: f64) -> &'static str {
    if value > f64::EPSILON {
        "▲"
    } else if value < -f64::EPSILON {
        "▼"
    } else {
        "•"
    }
}

/// Maps a portfolio value series to `Sparkline` bar heights (0..=100).
pub(crate) fn spark_series(values: &VecDeque<f64>) -> Vec<u64> {
    if values.is_empty() {
        return Vec::new();
    }
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    if span <= 0.0 {
        return vec![0; values.len()];
    }
    values
        .iter()
        .map(|v| (((v - min) / span) * 100.0).round() as u64)
        .collect()
}

/// The eight block characters used to draw a text sparkline, low→high.
const SPARK_BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A unicode block-character sparkline of the last `width` values, scaled to their own min/max band.
fn spark_text(values: &[f64], width: usize) -> String {
    let start = values.len().saturating_sub(width);
    let vals = &values[start..];
    if vals.len() < 2 {
        return String::new();
    }
    let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
    let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    vals.iter()
        .map(|v| {
            let idx = if span <= 0.0 {
                0
            } else {
                (((v - min) / span) * 7.0).round() as usize
            };
            SPARK_BARS[idx.min(7)]
        })
        .collect()
}

fn draw_dashboard(f: &mut Frame, area: Rect, app: &App) {
    let [portfolio_area, movers_area, news_area, alerts_area, breadth_area, active_area, status_area] =
        Layout::vertical([
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
        ])
        .areas(area);
    draw_portfolio_cube(f, portfolio_area, app);
    draw_movers_cube(f, movers_area, app);
    draw_news_cube(f, news_area, app);
    draw_alerts_cube(f, alerts_area, app);
    draw_breadth_cube(f, breadth_area, app);
    draw_active_cube(f, active_area, app);
    draw_status_cube(f, status_area, app);
}

fn draw_portfolio_cube(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let snap = &app.market.dashboard;
    let positions = portfolio_positions(&snap.holdings, &snap.rows);
    let block = cube_block("portfolio", &t);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [text_area, spark_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    let lines = if snap.holdings.is_empty() {
        vec![dim_line("no holdings", &t)]
    } else if !positions.iter().any(|p| p.market_value.is_some()) {
        vec![dim_line("awaiting prices", &t)]
    } else {
        let s = summarize(&positions);
        let pnl_pct = s
            .total_pnl_pct
            .map(|v| format!("{v:+.1}%"))
            .unwrap_or_else(|| "-".into());
        vec![
            Line::from(Span::styled(
                format!("${:.2}", s.total_market_value),
                Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                format!("{} {}", move_arrow(s.total_pnl), pnl_pct),
                Style::default().fg(pnl_color(Some(s.total_pnl), &t)),
            )),
            Line::from(vec![
                Span::styled("today ", Style::default().fg(t.dim)),
                Span::styled(
                    format!("{:+.2}", s.total_day_change),
                    Style::default().fg(pnl_color(Some(s.total_day_change), &t)),
                ),
            ]),
        ]
    };
    f.render_widget(Paragraph::new(lines), text_area);

    if app.market.portfolio_value_series.len() < 2 {
        f.render_widget(Paragraph::new(dim_line("collecting…", &t)), spark_area);
    } else {
        let data = spark_series(&app.market.portfolio_value_series);
        f.render_widget(
            Sparkline::default()
                .data(&data)
                .style(Style::default().fg(t.accent)),
            spark_area,
        );
    }
}

fn draw_movers_cube(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let block = cube_block("movers", &t);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let movers = ranked_movers(&app.market.dashboard.rows, 3);
    if movers.is_empty() {
        f.render_widget(Paragraph::new(dim_line("no quotes yet", &t)), inner);
        return;
    }

    let max_pct = movers.iter().map(|m| m.day_pct.abs()).fold(0.0_f64, f64::max);
    let bar_width = (inner.width as usize).saturating_sub(6 + 1 + 7);
    let lines: Vec<Line> = movers
        .iter()
        .map(|m| mover_bar_line(m, max_pct, bar_width, &t))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn mover_bar_line(m: &Mover, max_pct: f64, bar_width: usize, t: &Theme) -> Line<'static> {
    let color = pnl_color(Some(m.day_pct), t);
    let bar = "█".repeat(bar_cells(m.day_pct, max_pct, bar_width));
    Line::from(vec![
        Span::styled(format!("{:<6} ", m.ticker), Style::default().fg(t.fg)),
        Span::styled(format!("{bar:<bar_width$}"), Style::default().fg(color)),
        Span::styled(format!(" {:+.1}%", m.day_pct), Style::default().fg(color)),
    ])
}

fn news_empty_message(watchlist_empty: bool, loading: bool, page: bool) -> &'static str {
    if loading {
        "loading news…"
    } else if watchlist_empty {
        if page {
            "watchlist empty — press w on a quote to follow it"
        } else {
            "watchlist empty — press w"
        }
    } else {
        "no headlines yet"
    }
}

fn draw_news_cube(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let block = cube_block("news", &t);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let feed = &app.market.news_feed;
    if feed.items.is_empty() {
        let msg = news_empty_message(app.market.db_display.watchlist.is_empty(), feed.loading, false);
        f.render_widget(Paragraph::new(dim_line(msg, &t)), inner);
        return;
    }

    let width = inner.width as usize;
    let lines: Vec<Line> = feed
        .items
        .iter()
        .take(inner.height as usize)
        .map(|fi| news_snippet_line(&fi.ticker, &fi.item.title, width, &t))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn news_snippet_line(ticker: &str, title: &str, width: usize, t: &Theme) -> Line<'static> {
    let prefix = format!("{ticker} ");
    let remaining = width.saturating_sub(prefix.chars().count());
    Line::from(vec![
        Span::styled(prefix, Style::default().fg(t.accent)),
        Span::styled(truncate_ellipsis(title, remaining), Style::default().fg(t.fg)),
    ])
}

/// Alerts cube: the active-alert count in the title and the alert nearest to firing in the body.
fn draw_alerts_cube(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let snap = &app.market.dashboard;
    let title = if snap.alerts.is_empty() {
        "alerts".to_string()
    } else {
        format!("alerts · {}", snap.alerts.len())
    };
    let lines = if snap.alerts.is_empty() {
        vec![dim_line("no alerts", &t)]
    } else {
        match nearest_alert(&snap.alerts, &snap.rows) {
            None => vec![dim_line("awaiting prices", &t)],
            Some(a) => {
                let op = match a.direction {
                    AlertDirection::Above => ">",
                    AlertDirection::Below => "<",
                };
                vec![Line::from(vec![
                    Span::styled(
                        format!("{} {}{:.2}", a.ticker, op, a.threshold),
                        Style::default().fg(t.fg),
                    ),
                    Span::styled(
                        format!("  {:.1}%", a.distance_pct),
                        Style::default().fg(t.dim),
                    ),
                ])]
            }
        }
    };
    f.render_widget(Paragraph::new(lines).block(cube_block(&title, &t)), area);
}

fn draw_breadth_cube(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let (advancers, decliners) = day_breadth(&app.market.dashboard.rows);
    let block = cube_block("breadth", &t);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if advancers == 0 && decliners == 0 {
        f.render_widget(Paragraph::new(dim_line("no quotes yet", &t)), inner);
        return;
    }

    let up_color = direction_color(Direction::Up, &t);
    let down_color = direction_color(Direction::Down, &t);
    let left = format!("{} {advancers} ", direction_arrow(Direction::Up));
    let right = format!(" {} {decliners}", direction_arrow(Direction::Down));
    let bar_width = (inner.width as usize)
        .saturating_sub(left.chars().count() + right.chars().count());
    let (up, down) = breadth_bar(advancers, decliners, bar_width);

    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(left, Style::default().fg(up_color)),
            Span::styled("█".repeat(up), Style::default().fg(up_color)),
            Span::styled("█".repeat(down), Style::default().fg(down_color)),
            Span::styled(right, Style::default().fg(down_color)),
        ])),
        inner,
    );
}

/// Most-active cube: the most-traded tracked quote by day volume.
fn draw_active_cube(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let lines = match most_active(&app.market.dashboard.rows) {
        None => vec![dim_line("no volume", &t)],
        Some((ticker, volume)) => vec![Line::from(vec![
            Span::styled(format!("{ticker:<6}"), Style::default().fg(t.fg)),
            Span::styled(fmt_volume(volume), Style::default().fg(t.dim)),
        ])],
    };
    f.render_widget(
        Paragraph::new(lines).block(cube_block("most active", &t)),
        area,
    );
}

/// Status cube: whether the live quote stream is running and how many symbols it covers.
fn draw_status_cube(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    let snap = &app.market.dashboard;
    let tracked = snap.tracked;
    let status = if snap.stream_live {
        Span::styled("live●", Style::default().fg(t.up))
    } else {
        Span::styled("offline", Style::default().fg(t.dim))
    };
    let line = Line::from(vec![
        status,
        Span::styled(format!("  {tracked} tracked"), Style::default().fg(t.fg)),
    ]);
    f.render_widget(Paragraph::new(line).block(cube_block("status", &t)), area);
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

const PRICE_EPSILON: f64 = 0.001;

/// Which way a value moved, once an epsilon band has collapsed near-zero deltas to flat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Neutral,
}

fn direction(delta: f64, eps: f64) -> Direction {
    if delta > eps {
        Direction::Up
    } else if delta < -eps {
        Direction::Down
    } else {
        Direction::Neutral
    }
}

/// The theme colour for a movement direction.
fn direction_color(dir: Direction, t: &Theme) -> Color {
    match dir {
        Direction::Up => t.up,
        Direction::Down => t.down,
        Direction::Neutral => t.neutral,
    }
}

fn direction_arrow(dir: Direction) -> &'static str {
    match dir {
        Direction::Up => "▲",
        Direction::Down => "▼",
        Direction::Neutral => "·",
    }
}

fn price_change_color(
    stock: &QuoteRecord,
    flash_map: &HashMap<Ticker, (f64, Instant)>,
    t: &Theme,
) -> Color {
    let diff = stock
        .ticker
        .as_deref()
        .and_then(|t| flash_map.get(&Ticker::new(t)))
        .filter(|(_, ts)| ts.elapsed() < FLASH_TTL)
        .map(|(diff, _)| *diff)
        .or(match (stock.price, stock.previous_close) {
            (Some(price), Some(prev)) => Some(price - prev),
            _ => None,
        });
    match diff {
        Some(d) => direction_color(direction(d, PRICE_EPSILON), t),
        None => t.neutral,
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
        hcell("Ticker ", "(t)"),
        hcell("Name", "(n)"),
        hcell("Price ", "(p)"),
        hcell("Chg%", ""),
        hcell("Trend", ""),
        hcell("Prev Close ", "(c)"),
        hcell("Volume ", "(v)"),
        hcell("As Of ", "(a)"),
    ])
    .bottom_margin(1);

    let window = app.market.db_display.window_range();
    let query = &app.market.input_mode.input;
    let searching = app.market.input_mode.is_fuzzy();

    if app.market.db_display.rows.is_empty() {
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
    let rows = app.market.db_display.rows[window].iter().map(|q| {
        let star = if is_watchlisted(q.ticker.as_deref(), &app.market.db_display.watchlist) {
            Cell::from(Span::styled("★", Style::default().fg(t.gold)))
        } else {
            Cell::from("")
        };
        let bell = if has_active_alert(q.ticker.as_deref(), &app.alerts) {
            Cell::from(Span::styled(
                "!",
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
            ))
        } else {
            Cell::from("")
        };
        let series = q
            .ticker
            .as_deref()
            .and_then(|tk| app.market.db_display.spark_series.get(tk))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let trend = spark_text(series, 12);
        let shown = &series[series.len().saturating_sub(12)..];
        let trend_color = match (shown.first(), shown.last()) {
            (Some(a), Some(b)) if shown.len() >= 2 => pnl_color(Some(b - a), &t),
            _ => t.dim,
        };
        let trend_cell = Cell::from(Span::styled(trend, Style::default().fg(trend_color)));

        Row::new(vec![
            star,
            bell,
            fuzzy_cell(q.ticker.as_deref(), query, searching, &t),
            fuzzy_cell(q.name.as_deref(), query, searching, &t),
            opt_cell(q.price, |p| format!("{p:.2}")).style(
                Style::default().fg(price_change_color(q, &app.animations.row_flash_map, &t)),
            ),
            opt_cell(day_change_pct(q.price, q.previous_close), |pct| {
                format!("{pct:+.2}%")
            })
            .style(Style::default().fg(price_change_color(
                q,
                &app.animations.row_flash_map,
                &t,
            ))),
            trend_cell,
            opt_cell(q.previous_close, |p| format!("{p:.2}")),
            opt_cell(q.day_volume, fmt_volume),
            opt_cell(q.as_of, |dt| dt.format("%Y-%m-%d").to_string()),
        ])
    });

    let widths = [
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(10),
        Constraint::Length(24),
        Constraint::Length(11),
        Constraint::Length(9),
        Constraint::Length(12),
        Constraint::Length(15),
        Constraint::Length(13),
        Constraint::Length(12),
    ];

    let mut quotes_table_title_text = "assetui · ".to_string();
    quotes_table_title_text.push_str(if app.market.db_display.watchlist_only {
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

    f.render_stateful_widget(table, area, &mut app.market.db_display.table_state);
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

    let (mode, hint) = app.status_mode();

    let chip = format!(" {} ", mode.label());
    let page = app.market.db_display.page + 1;
    let total_pages = app.market.db_display.total_pages();
    let count = if app.market.input_mode.is_fuzzy() {
        format!(
            "{}/{}",
            app.market.db_display.rows.len(),
            app.market.db_display.all_rows.len()
        )
    } else {
        app.market.db_display.rows.len().to_string()
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
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            "assetui",
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" · keybindings ", Style::default().fg(t.dim)),
    ]);
    let (content, footer) = modal_frame(f, 48, 80, title, &t);

    let lines = vec![
        section_heading("NAVIGATION", None, &t),
        help_row("j / k", "move selection up / down", &t),
        help_row("h / l", "previous / next page", &t),
        Line::raw(""),
        section_heading("VIEWS", None, &t),
        help_row("s", "stock details", &t),
        help_row("enter", "price chart", &t),
        help_row("x", "options chain (h/l expiry)", &t),
        help_row("L", "toggle dashboard / logs", &t),
        help_row("?", "this help", &t),
        Line::raw(""),
        section_heading("WATCHLIST", None, &t),
        help_row("w / r", "add / remove favourite (★)", &t),
        Line::raw(""),
        section_heading("MANAGE PAGE", None, &t),
        help_row("tab", "open the page picker (j/k move, enter go)", &t),
        help_row("a / e / d", "add / edit / delete a holding or alert", &t),
        Line::from(Span::styled(
            "  alerts fire once for on-screen tickers only",
            Style::default().fg(t.dim),
        )),
        Line::raw(""),
        section_heading("QUERY & SEARCH", None, &t),
        help_row("i", "fetch tickers (input box)", &t),
        help_row("/", "fuzzy search the table", &t),
        Line::raw(""),
        section_heading("SORT", None, &t),
        help_row("o", "toggle ascending / descending", &t),
        help_row("t n p c v a", "sort by column", &t),
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

fn draw_portfolio_modal(f: &mut Frame, app: &App) {
    let t = app.theme;
    let area = centered_rect(65, 55, f.area());
    f.render_widget(Clear, area);

    let pm = &app.portfolio_modal;
    let series = match &pm.series {
        None => {
            let block = modal_block(block_title("portfolio · performance".into(), &t), &t);
            let inner = block.inner(area);
            f.render_widget(block, area);
            let msg = if pm.loading { "reconstructing history…" } else { "no data" };
            f.render_widget(placeholder(msg, &t), inner);
            return;
        }
        Some(s) => s,
    };

    let ret = crate::portfolio::series_return(series, pm.cost_basis);
    let pct = ret
        .total_pnl_pct
        .map(|v| format!("{v:+.1}%"))
        .unwrap_or_else(|| "-".into());
    let title = block_title(
        format!("portfolio · ${:.2} ({pct})", ret.latest_value),
        &t,
    );
    let block = modal_block(title, &t);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if series.len() < 2 {
        let msg = if app.holdings.is_empty() { "no holdings" } else { "not enough history" };
        f.render_widget(placeholder(msg, &t), inner);
        return;
    }

    let data: Vec<(f64, f64)> = series
        .iter()
        .enumerate()
        .map(|(i, (_, v))| (i as f64, *v))
        .collect();
    let xmax = (series.len() - 1) as f64;
    let basis_line = vec![(0.0, pm.cost_basis), (xmax, pm.cost_basis)];

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

    let ymin = data.iter().map(|(_, v)| *v).chain([pm.cost_basis]).fold(f64::INFINITY, f64::min);
    let ymax = data.iter().map(|(_, v)| *v).chain([pm.cost_basis]).fold(f64::NEG_INFINITY, f64::max);
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

fn draw_page_picker(f: &mut Frame, app: &App) {
    use crate::tui::app::PAGE_ORDER;
    let t = app.theme;
    let title = block_title("go to page".into(), &t);
    let (content, _footer) = modal_frame(f, 30, 34, title, &t);

    let lines: Vec<Line> = PAGE_ORDER
        .iter()
        .map(|&p| {
            let selected = p == app.page_picker.highlighted;
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

/// Human label for a page, shown in the picker.
fn page_label(p: Page) -> &'static str {
    match p {
        Page::Market => "Market",
        Page::Manage => "Manage",
        Page::Sectors => "Sectors",
        Page::News => "News",
    }
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

/// Draws the shared overlay chrome and returns the `(content, footer)` regions to fill.
fn modal_frame(
    f: &mut Frame,
    pct_x: u16,
    pct_y: u16,
    title: Line<'static>,
    t: &Theme,
) -> (Rect, Rect) {
    let area = centered_rect(pct_x, pct_y, f.area());
    f.render_widget(Clear, area);
    let modal = modal_block(title, t);
    let inner = modal.inner(area);
    f.render_widget(modal, area);
    let [content, footer] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    (content, footer)
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

/// Truncates `s` to at most `max` display columns, appending `…` when it would overflow.
fn truncate_ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
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
    let stock = &app.stock_modal.stock;
    let title = modal_title(
        stock.ticker.as_deref().unwrap_or("-"),
        stock.name.as_deref().unwrap_or("-"),
        &t,
    );
    let (content, footer) = modal_frame(f, 65, 55, title, &t);

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
        Some(analysis) if analysis.coverage_not_applicable => {}
        None => {
            lines.push(section_heading("ANALYST CONSENSUS", None, &t));
            lines.push(Line::raw(""));
            lines.push(Line::styled(
                "loading analysis…",
                Style::default().fg(t.dim),
            ));
            lines.push(Line::raw(""));
        }
        Some(analysis) => {
            push_analysis_lines(
                &mut lines,
                analysis,
                stock.price,
                content.width as usize,
                &t,
            );
            lines.push(Line::raw(""));
        }
    }

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
    flash_map: &HashMap<Ticker, (f64, Instant)>,
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
        let dir = direction(diff, PRICE_EPSILON);
        let (arrow, color) = (direction_arrow(dir), direction_color(dir, t));
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

fn push_statistics_lines(lines: &mut Vec<Line<'static>>, stats: &QuoteRecordStatistics, t: &Theme) {
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
            lines.push(Line::styled(
                format!("  {meta}"),
                Style::default().fg(t.dim),
            ));
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
    let ticker = app.stock_modal.stock.ticker.as_deref().unwrap_or("-");
    let title = modal_title(ticker, "options", &t);
    let (content, footer_area) = modal_frame(f, 85, 72, title, &t);

    let Some(chain) = &app.stock_modal.option_chain else {
        f.render_widget(placeholder("Loading options…", &t), content);
        return;
    };
    if chain.expirations.is_empty() {
        f.render_widget(placeholder("No options for this ticker.", &t), content);
        return;
    }
    if content.width < 44 {
        f.render_widget(placeholder("Terminal too narrow for options.", &t), content);
        return;
    }

    let [selector_area, tables_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(0)]).areas(content);

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
            cells.push(opt_cell(a.map(|a| a.greeks.theta / 365.0), |v| {
                format!("{v:.3}")
            }));
            cells.push(opt_cell(a.map(|a| a.greeks.vega / 100.0), |v| {
                format!("{v:.3}")
            }));
            cells.push(opt_cell(a.map(|a| a.greeks.rho / 100.0), |v| {
                format!("{v:.3}")
            }));
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
        .title(section_heading(
            title,
            Some(format!("{}", contracts.len())),
            t,
        ))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border));

    if contracts.is_empty() {
        let empty =
            Paragraph::new(Span::styled("no contracts", Style::default().fg(t.dim))).block(block);
        f.render_widget(empty, area);
        return;
    }

    let table = Table::new(rows, widths).header(header).block(block);
    f.render_widget(table, area);
}

/// A quote's single-session move: its ticker and day-change percent.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Mover {
    /// The quote's ticker symbol.
    pub ticker: String,
    /// `(price - previous_close) / previous_close * 100`.
    pub day_pct: f64,
}

pub(crate) fn ranked_movers(rows: &[QuoteRecord], limit: usize) -> Vec<Mover> {
    let mut movers: Vec<Mover> = rows
        .iter()
        .filter_map(|r| {
            let ticker = r.ticker.as_deref()?;
            let day_pct = day_change_pct(r.price, r.previous_close)?;
            day_pct.is_finite().then(|| Mover {
                ticker: ticker.to_string(),
                day_pct,
            })
        })
        .collect();
    movers.sort_by(|a, b| b.day_pct.partial_cmp(&a.day_pct).expect("finite day_pct"));
    movers.truncate(limit);
    movers
}

fn bar_cells(pct: f64, max_pct: f64, width: usize) -> usize {
    if width == 0 || !(max_pct > 0.0) || !max_pct.is_finite() {
        return 0;
    }
    let frac = (pct.abs() / max_pct).clamp(0.0, 1.0);
    (frac * width as f64).round() as usize
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AlertProximity {
    /// The alert's ticker symbol.
    pub ticker: String,
    /// Whether the alert fires above or below its threshold.
    pub direction: AlertDirection,
    /// The price level the alert watches.
    pub threshold: f64,
    /// `|price - threshold| / threshold * 100`.
    pub distance_pct: f64,
}

/// Picks the alert nearest its threshold, valued against the latest prices in `rows`.
pub(crate) fn nearest_alert(alerts: &[Alert], rows: &[QuoteRecord]) -> Option<AlertProximity> {
    alerts
        .iter()
        .filter_map(|a| {
            let (price, _) = resolve_price(&a.ticker, rows);
            let price = price?;
            if a.threshold == 0.0 {
                return None;
            }
            let distance_pct = (price - a.threshold).abs() / a.threshold * 100.0;
            distance_pct.is_finite().then(|| AlertProximity {
                ticker: a.ticker.clone(),
                direction: a.direction,
                threshold: a.threshold,
                distance_pct,
            })
        })
        .min_by(|a, b| {
            a.distance_pct
                .partial_cmp(&b.distance_pct)
                .expect("finite distance_pct")
        })
}

/// Advancers and decliners across `rows`: how many tracked quotes are up versus down on the day.
pub(crate) fn day_breadth(rows: &[QuoteRecord]) -> (usize, usize) {
    let mut advancers = 0;
    let mut decliners = 0;
    for pct in rows
        .iter()
        .filter_map(|r| day_change_pct(r.price, r.previous_close))
    {
        if pct > 0.0 {
            advancers += 1;
        } else if pct < 0.0 {
            decliners += 1;
        }
    }
    (advancers, decliners)
}

pub(crate) fn breadth_bar(adv: usize, dec: usize, width: usize) -> (usize, usize) {
    let total = adv + dec;
    if width == 0 || total == 0 {
        return (0, 0);
    }
    let up = adv * width / total;
    let down = width - up;
    (up, down)
}

/// The most-traded quote in `rows` by day volume: its ticker and share volume.
pub(crate) fn most_active(rows: &[QuoteRecord]) -> Option<(String, f64)> {
    rows.iter()
        .filter_map(|r| Some((r.ticker.as_deref()?.to_string(), r.day_volume?)))
        .filter(|(_, v)| v.is_finite() && *v > 0.0)
        .max_by(|a, b| a.1.partial_cmp(&b.1).expect("finite volume"))
}

fn pnl_color(value: Option<f64>, t: &Theme) -> Color {
    match value {
        Some(v) => direction_color(direction(v, f64::EPSILON), t),
        None => t.dim,
    }
}

/// A right-aligned signed-money cell (`+1.20` / `-3.40` / `-` when absent), coloured by sign.
fn pnl_cell(value: Option<f64>, t: &Theme) -> Cell<'static> {
    opt_cell(value, |v| format!("{v:+.2}")).style(Style::default().fg(pnl_color(value, t)))
}

const HEAT_CAP_PCT: f64 = 3.0;

pub(crate) fn heat_color(day_pct: f64, t: &Theme) -> Color {
    let frac = (day_pct / HEAT_CAP_PCT).clamp(-1.0, 1.0);
    let (from, to, amount) = if frac >= 0.0 {
        (t.neutral, t.up_strong, frac)
    } else {
        (t.neutral, t.down_strong, -frac)
    };
    lerp_rgb(from, to, amount)
}

/// Linearly interpolates two `Color::Rgb` values by `amount` in `[0, 1]`.
fn lerp_rgb(from: Color, to: Color, amount: f64) -> Color {
    let (Color::Rgb(fr, fg, fb), Color::Rgb(tr, tg, tb)) = (from, to) else {
        return to;
    };
    let mix = |a: u8, b: u8| -> u8 {
        (f64::from(a) + (f64::from(b) - f64::from(a)) * amount).round() as u8
    };
    Color::Rgb(mix(fr, tr), mix(fg, tg), mix(fb, tb))
}

fn draw_sectors_page(f: &mut Frame, area: Rect, app: &mut App) {
    let t = app.theme;
    let map = &app.sectors.map;

    if map.groups.is_empty() {
        let msg = if app.sectors.loading { "loading sectors…" } else { "press r to load sectors" };
        f.render_widget(placeholder(msg, &t), area);
        return;
    }

    let sector_weights: Vec<f64> = map.groups.iter().map(|g| g.total_market_cap).collect();
    for (group, srect) in map.groups.iter().zip(squarify(&sector_weights, area)) {
        if srect.width < 3 || srect.height < 2 {
            continue;
        }
        let block = Block::default()
            .title(block_title(sector_label_short(group.sector), &t))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(t.border));
        let inner = block.inner(srect);
        f.render_widget(block, srect);

        let tile_weights: Vec<f64> = group.tiles.iter().map(|ti| ti.market_cap).collect();
        for (tile, trect) in group.tiles.iter().zip(squarify(&tile_weights, inner)) {
            if trect.width == 0 || trect.height == 0 {
                continue;
            }
            let bg = heat_color(tile.day_pct, &t);
            f.render_widget(Block::default().style(Style::default().bg(bg)), trect);
            if trect.width >= 4 && trect.height >= 1 {
                let mut lines = vec![Line::from(Span::styled(
                    tile.ticker.clone(),
                    Style::default().fg(t.bg).add_modifier(Modifier::BOLD),
                ))];
                if trect.height >= 2 {
                    lines.push(Line::from(Span::styled(
                        format!("{:+.1}%", tile.day_pct),
                        Style::default().fg(t.bg),
                    )));
                }
                f.render_widget(Paragraph::new(lines), trect);
            }
        }
    }
}

fn sector_label_short(s: yfinance_rs::screener::EquitySector) -> String {
    use yfinance_rs::screener::EquitySector::*;
    match s {
        Technology => "Technology",
        FinancialServices => "Financials",
        Healthcare => "Healthcare",
        ConsumerCyclical => "Cons Cyclical",
        ConsumerDefensive => "Cons Defensive",
        CommunicationServices => "Comm Services",
        Industrials => "Industrials",
        Energy => "Energy",
        Utilities => "Utilities",
        RealEstate => "Real Estate",
        BasicMaterials => "Materials",
        _ => "Other",
    }
    .to_string()
}

/// Renders the News page: the merged watchlist feed as a selectable list.
fn draw_news_page(f: &mut Frame, area: Rect, app: &mut App) {
    let t = app.theme;
    let block = Block::default()
        .title(block_title("news".into(), &t))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(t.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let feed = &app.market.news_feed;
    if feed.items.is_empty() {
        let msg = news_empty_message(app.market.db_display.watchlist.is_empty(), feed.loading, true);
        f.render_widget(placeholder(msg, &t), inner);
        return;
    }

    let selected = app.news_page.selected.min(feed.items.len() - 1);
    let height = inner.height.max(1) as usize;
    let start = if selected >= height { selected + 1 - height } else { 0 };
    let width = inner.width as usize;
    let lines: Vec<Line> = feed
        .items
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, fi)| news_row_line(i == selected, &fi.ticker, &fi.item, width, &t))
        .collect();
    f.render_widget(Paragraph::new(lines), inner);
}

fn news_row_line(selected: bool, ticker: &str, item: &NewsItem, width: usize, t: &Theme) -> Line<'static> {
    let caret = if selected { "› " } else { "  " };
    let ticker_field = format!("{ticker:<6} ");
    let meta = fmt_news_meta(item.publisher.as_deref(), item.published_at);
    let meta_field = if meta.is_empty() { String::new() } else { format!("  {meta}") };
    let used = caret.chars().count() + ticker_field.chars().count() + meta_field.chars().count();
    let title = truncate_ellipsis(&item.title, width.saturating_sub(used));
    let title_style = if selected {
        Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(t.fg)
    };
    Line::from(vec![
        Span::styled(caret.to_string(), Style::default().fg(t.accent)),
        Span::styled(ticker_field, Style::default().fg(t.dim)),
        Span::styled(title, title_style),
        Span::styled(meta_field, Style::default().fg(t.dim)),
    ])
}

/// Renders the Manage page body: a Holdings section above an Alerts section.
fn draw_manage_body(f: &mut Frame, body: Rect, app: &mut App) {
    let [holdings_area, alerts_area] =
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).areas(body);
    draw_holdings_section(
        f,
        holdings_area,
        app,
        app.manage.section == ManageSection::Holdings,
    );
    draw_alerts_section(
        f,
        alerts_area,
        app,
        app.manage.section == ManageSection::Alerts,
    );
}

fn manage_input_line(input: &ManageInput, blink: bool, t: &Theme) -> Line<'static> {
    let cursor = if blink { "▌" } else { " " };
    let mut spans = vec![Span::styled("› ", Style::default().fg(t.accent))];
    if input.buffer.is_empty() {
        spans.push(Span::styled(cursor, Style::default().fg(t.accent)));
        let hint = match input.kind {
            ManageInputKind::Holding => " TICKER SHARES COST",
            ManageInputKind::Alert => " TICKER >150 (or <100)",
        };
        spans.push(Span::styled(hint, Style::default().fg(t.dim)));
    } else {
        spans.push(Span::styled(
            input.buffer.clone(),
            Style::default().fg(t.fg),
        ));
        spans.push(Span::styled(cursor, Style::default().fg(t.accent)));
    }
    Line::from(spans)
}

fn draw_holdings_section(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let t = app.theme;
    let border = if focused { t.accent } else { t.border };
    let block = Block::default()
        .title(section_heading("HOLDINGS", None, &t))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let holding_input = app
        .manage
        .input
        .as_ref()
        .filter(|i| i.kind == ManageInputKind::Holding);

    let [content, bottom] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);

    if let Some(input) = holding_input {
        f.render_widget(
            Paragraph::new(manage_input_line(input, app.animations.blink_state, &t)),
            bottom,
        );
    }

    if app.holdings.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "no holdings — press a to add",
                Style::default().fg(t.dim),
            )),
            content,
        );
        return;
    }

    let positions = portfolio_positions(&app.holdings, &app.market.db_display.all_rows);
    let summary = summarize(&positions);
    let table_area = content;
    let totals_area = bottom;

    let head_sty = Style::default().fg(t.dim).add_modifier(Modifier::BOLD);
    let header = Row::new(
        [
            "Ticker",
            "Shares",
            "Avg Cost",
            "Price",
            "Mkt Value",
            "P&L $",
            "P&L %",
            "Day $",
        ]
        .into_iter()
        .map(|h| Cell::from(Span::styled(h, head_sty)))
        .collect::<Vec<_>>(),
    );
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
    let mut table = Table::new(rows, widths).header(header);
    let mut state = ratatui::widgets::TableState::default();
    if focused {
        table = table
            .row_highlight_style(Style::default().bg(t.cursor).add_modifier(Modifier::BOLD))
            .highlight_symbol(Span::styled("▸ ", Style::default().fg(t.accent)));
        state.select(Some(
            app.manage.selected.min(positions.len().saturating_sub(1)),
        ));
    }
    f.render_stateful_widget(table, table_area, &mut state);

    if holding_input.is_none() {
        let pct = summary
            .total_pnl_pct
            .map_or_else(|| "-".to_string(), |v| format!("{v:+.2}%"));
        let totals = Line::from(vec![
            label("Value ", &t),
            Span::styled(
                format!("${:.2}   ", summary.total_market_value),
                Style::default().fg(t.fg),
            ),
            label("P&L ", &t),
            Span::styled(
                format!("{:+.2} ({pct})", summary.total_pnl),
                Style::default()
                    .fg(pnl_color(Some(summary.total_pnl), &t))
                    .add_modifier(Modifier::BOLD),
            ),
        ]);
        f.render_widget(Paragraph::new(totals), totals_area);
    }
}

fn draw_alerts_section(f: &mut Frame, area: Rect, app: &App, focused: bool) {
    let t = app.theme;
    let border = if focused { t.accent } else { t.border };
    let block = Block::default()
        .title(section_heading("ALERTS", None, &t))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let alert_input = app
        .manage
        .input
        .as_ref()
        .filter(|i| i.kind == ManageInputKind::Alert);

    let [content, bottom] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(inner);
    if let Some(input) = alert_input {
        f.render_widget(
            Paragraph::new(manage_input_line(input, app.animations.blink_state, &t)),
            bottom,
        );
    }

    if app.alerts.is_empty() {
        f.render_widget(
            Paragraph::new(Span::styled(
                "no alerts — press a to add (e.g. AAPL >150)",
                Style::default().fg(t.dim),
            )),
            content,
        );
        return;
    }

    let head_sty = Style::default().fg(t.dim).add_modifier(Modifier::BOLD);
    let header = Row::new(
        ["Ticker", "Condition", "Threshold"]
            .into_iter()
            .map(|h| Cell::from(Span::styled(h, head_sty)))
            .collect::<Vec<_>>(),
    );
    let rows = app.alerts.iter().map(|a| {
        let dir = a.direction.label();
        Row::new(vec![
            Cell::from(Span::styled(
                a.ticker.clone(),
                Style::default().fg(t.fg).add_modifier(Modifier::BOLD),
            )),
            Cell::from(dir),
            Cell::from(format!("{:.2}", a.threshold)),
        ])
    });
    let widths = [
        Constraint::Length(8),
        Constraint::Length(11),
        Constraint::Length(12),
    ];
    let mut table = Table::new(rows, widths).header(header);
    let mut state = ratatui::widgets::TableState::default();
    if focused {
        table = table
            .row_highlight_style(Style::default().bg(t.cursor).add_modifier(Modifier::BOLD))
            .highlight_symbol(Span::styled("▸ ", Style::default().fg(t.accent)));
        state.select(Some(
            app.manage.selected.min(app.alerts.len().saturating_sub(1)),
        ));
    }
    f.render_stateful_widget(table, content, &mut state);
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
        Some(candles) => {
            let show_volume = app.stock_modal.show_volume;
            let show_rsi = app.stock_modal.show_rsi;
            let volume_h = if show_volume && inner.height >= 14 { 7 } else { 0 };
            let rsi_h = if show_rsi && inner.height >= 14 + volume_h { 8 } else { 0 };
            let [price_area, volume_area, rsi_area] = Layout::vertical([
                Constraint::Min(6),
                Constraint::Length(volume_h),
                Constraint::Length(rsi_h),
            ])
            .areas(inner);
            let ticker = stock.ticker.as_deref().unwrap_or("-").to_string();
            draw_stock_chart(f, price_area, candles, &ticker, &t);
            if volume_h > 0 {
                draw_volume_panel(f, volume_area, candles, &t);
            }
            if rsi_h > 0 {
                draw_rsi_panel(f, rsi_area, candles, &t);
            }
        }
    }
}

/// Per-candle share volume as `Sparkline` bar heights (0 when a candle carries no volume).
fn volume_series(candles: &[Candle]) -> Vec<u64> {
    candles
        .iter()
        .map(|c| {
            c.volume
                .as_ref()
                .map(|q| q.as_decimal().as_f64())
                .filter(|v| v.is_finite() && *v > 0.0)
                .map_or(0, |v| v as u64)
        })
        .collect()
}

/// Renders the volume sub-panel: a titled block with a volume Sparkline.
fn draw_volume_panel(f: &mut Frame, area: Rect, candles: &[Candle], t: &Theme) {
    let block = Block::default()
        .title(block_title("volume".into(), t))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(t.border));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let data = volume_series(candles);
    f.render_widget(
        Sparkline::default()
            .data(&data)
            .style(Style::default().fg(t.dim)),
        inner,
    );
}

fn draw_rsi_panel(f: &mut Frame, area: Rect, candles: &[Candle], t: &Theme) {
    let block = Block::default()
        .title(block_title(format!("RSI {}", crate::indicators::RSI_PERIOD), t))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(t.border));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let closes: Vec<f64> = candles
        .iter()
        .map(|c| c.ohlc.close.as_decimal().as_f64())
        .collect();
    let rsi = crate::indicators::rsi(&closes, crate::indicators::RSI_PERIOD);
    let pts: Vec<(f64, f64)> = rsi
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|y| (i as f64, y)))
        .collect();
    if pts.is_empty() {
        f.render_widget(placeholder("not enough data", t), inner);
        return;
    }
    let xmax = (candles.len().saturating_sub(1)) as f64;
    let band = |y: f64| vec![(0.0, y), (xmax, y)];
    let over = band(70.0);
    let under = band(30.0);
    let rsi_ds = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(t.accent))
        .data(&pts);
    let over_ds = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(t.dim))
        .data(&over);
    let under_ds = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(t.dim))
        .data(&under);
    let chart = Chart::new(vec![rsi_ds, over_ds, under_ds])
        .x_axis(Axis::default().style(Style::default().fg(t.dim)).bounds([0.0, xmax]))
        .y_axis(
            Axis::default()
                .style(Style::default().fg(t.dim))
                .bounds([0.0, 100.0])
                .labels(["0", "30", "70", "100"]),
        );
    f.render_widget(chart, inner);
}

/// (min, max) of paired low/high series, or `None` when empty.
fn range_of(lows: &[f64], highs: &[f64]) -> Option<(f64, f64)> {
    if lows.is_empty() || highs.is_empty() {
        return None;
    }
    let lo = lows.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = highs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some((lo, hi))
}

/// Period low/high across `candles` (min low, max high).
fn price_range(candles: &[Candle]) -> Option<(f64, f64)> {
    let lows: Vec<f64> = candles.iter().map(|c| c.ohlc.low.as_decimal().as_f64()).collect();
    let highs: Vec<f64> = candles.iter().map(|c| c.ohlc.high.as_decimal().as_f64()).collect();
    range_of(&lows, &highs)
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

    let closes: Vec<f64> = data.iter().map(|(_, c)| *c).collect();
    let sma_fast = crate::indicators::sma(&closes, crate::indicators::SMA_FAST);
    let sma_slow = crate::indicators::sma(&closes, crate::indicators::SMA_SLOW);
    let sma_points = |series: &[Option<f64>]| -> Vec<(f64, f64)> {
        series
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.map(|y| (i as f64, y)))
            .collect()
    };
    let sma_fast_pts = sma_points(&sma_fast);
    let sma_slow_pts = sma_points(&sma_slow);

    let range = price_range(candles);
    let hi_line: Vec<(f64, f64)> = match range {
        Some((_, hi)) => vec![(0.0, hi), (xmax, hi)],
        None => Vec::new(),
    };
    let lo_line: Vec<(f64, f64)> = match range {
        Some((lo, _)) => vec![(0.0, lo), (xmax, lo)],
        None => Vec::new(),
    };

    let sma_fast_ds = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(t.accent))
        .data(&sma_fast_pts);
    let sma_slow_ds = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(t.dim))
        .data(&sma_slow_pts);
    let hi_ds = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(t.up))
        .data(&hi_line);
    let lo_ds = Dataset::default()
        .marker(Marker::Braille)
        .graph_type(GraphType::Line)
        .style(Style::default().fg(t.down))
        .data(&lo_line);

    let (ymin, ymax) = data
        .iter()
        .fold((f64::MAX, f64::MIN), |(min, max), (_, y)| {
            (min.min(*y), max.max(*y))
        });
    let (ymin, ymax) = match range {
        Some((lo, hi)) => (ymin.min(lo), ymax.max(hi)),
        None => (ymin, ymax),
    };
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

    let chart = Chart::new(vec![
        trend_line,
        up_dots,
        down_dots,
        sma_fast_ds,
        sma_slow_ds,
        hi_ds,
        lo_ds,
    ])
    .x_axis(x_axis)
    .y_axis(y_axis)
    .legend_position(Some(LegendPosition::TopLeft));
    f.render_widget(chart, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_of_returns_min_low_and_max_high() {
        let lows = vec![10.0, 8.0, 9.0];
        let highs = vec![12.0, 15.0, 11.0];
        assert_eq!(range_of(&lows, &highs), Some((8.0, 15.0)));
    }

    #[test]
    fn range_of_empty_is_none() {
        assert_eq!(range_of(&[], &[]), None);
    }

    #[test]
    fn spark_series_subtracts_the_minimum_before_scaling() {
        let values: VecDeque<f64> = [100.0, 150.0, 200.0].into_iter().collect();
        assert_eq!(spark_series(&values), vec![0, 50, 100]);
    }

    #[test]
    fn spark_series_flat_series_is_all_baseline() {
        let values: VecDeque<f64> = [500.0, 500.0, 500.0].into_iter().collect();
        assert_eq!(spark_series(&values), vec![0, 0, 0]);
    }

    #[test]
    fn spark_series_empty_is_empty() {
        let values: VecDeque<f64> = VecDeque::new();
        assert!(spark_series(&values).is_empty());
    }

    #[test]
    fn spark_text_is_empty_below_two_points() {
        assert_eq!(spark_text(&[], 8), "");
        assert_eq!(spark_text(&[5.0], 8), "");
    }

    #[test]
    fn spark_text_maps_low_to_low_bar_and_high_to_full_bar() {
        let s = spark_text(&[1.0, 2.0, 3.0], 8);
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars.len(), 3);
        assert_eq!(chars[0], '▁', "min maps to the lowest bar");
        assert_eq!(chars[2], '█', "max maps to the full bar");
    }

    #[test]
    fn spark_text_takes_the_last_width_points() {
        let vals: Vec<f64> = (0..10).map(|i| i as f64).collect();
        assert_eq!(spark_text(&vals, 4).chars().count(), 4);
    }

    fn mover_row(ticker: &str, price: f64, previous_close: f64) -> QuoteRecord {
        QuoteRecord {
            ticker: Some(ticker.to_string()),
            price: Some(price),
            previous_close: Some(previous_close),
            ..Default::default()
        }
    }

    #[test]
    fn ranked_movers_sorts_biggest_gain_first() {
        let rows = vec![
            mover_row("LOSS", 95.0, 100.0),
            mover_row("GAIN", 110.0, 100.0),
            mover_row("MILD", 102.0, 100.0),
        ];

        let ranked = ranked_movers(&rows, 10);

        let tickers: Vec<&str> = ranked.iter().map(|m| m.ticker.as_str()).collect();
        assert_eq!(tickers, vec!["GAIN", "MILD", "LOSS"]);
    }

    #[test]
    fn ranked_movers_truncates_to_limit() {
        let rows = vec![
            mover_row("A", 110.0, 100.0),
            mover_row("B", 108.0, 100.0),
            mover_row("C", 106.0, 100.0),
        ];

        let ranked = ranked_movers(&rows, 2);

        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].ticker, "A");
        assert_eq!(ranked[1].ticker, "B");
    }

    #[test]
    fn ranked_movers_skips_quotes_without_a_day_move() {
        let mut no_close = mover_row("SKIP", 110.0, 0.0);
        no_close.previous_close = None;
        let rows = vec![no_close, mover_row("KEEP", 110.0, 100.0)];

        let ranked = ranked_movers(&rows, 10);

        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].ticker, "KEEP");
    }

    #[test]
    fn ranked_movers_empty_input_is_empty() {
        assert!(ranked_movers(&[], 10).is_empty());
    }

    #[test]
    fn truncate_ellipsis_adds_ellipsis_only_when_overflowing() {
        assert_eq!(truncate_ellipsis("short", 10), "short");
        assert_eq!(truncate_ellipsis("abcdef", 4), "abc…");
        assert_eq!(truncate_ellipsis("abc", 0), "");
    }

    #[test]
    fn ranked_movers_caps_at_three() {
        let rows: Vec<QuoteRecord> = ["A", "B", "C", "D", "E"]
            .iter()
            .enumerate()
            .map(|(i, tk)| QuoteRecord {
                ticker: Some((*tk).into()),
                price: Some(100.0),
                previous_close: Some(100.0 - i as f64),
                ..Default::default()
            })
            .collect();
        assert_eq!(ranked_movers(&rows, 3).len(), 3);
    }

    #[test]
    fn bar_cells_fills_width_for_the_max_mover() {
        assert_eq!(bar_cells(6.0, 6.0, 10), 10);
    }

    #[test]
    fn bar_cells_scales_proportionally() {
        assert_eq!(bar_cells(3.0, 6.0, 10), 5);
    }

    #[test]
    fn bar_cells_uses_absolute_value_for_losers() {
        assert_eq!(bar_cells(-6.0, 6.0, 10), 10);
    }

    #[test]
    fn bar_cells_is_zero_when_no_width_or_no_spread() {
        assert_eq!(bar_cells(3.0, 6.0, 0), 0);
        assert_eq!(bar_cells(3.0, 0.0, 10), 0);
    }

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

    #[test]
    fn news_empty_message_prioritizes_loading() {
        assert_eq!(news_empty_message(true, true, false), "loading news…");
    }

    #[test]
    fn news_empty_message_prompts_to_follow_when_watchlist_is_empty() {
        assert_eq!(news_empty_message(true, false, false), "watchlist empty — press w");
        assert_eq!(
            news_empty_message(true, false, true),
            "watchlist empty — press w on a quote to follow it"
        );
    }

    #[test]
    fn news_empty_message_distinguishes_no_headlines_from_empty_watchlist() {
        assert_eq!(news_empty_message(false, false, false), "no headlines yet");
        assert_eq!(news_empty_message(false, false, true), "no headlines yet");
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

    fn opt_contract(
        kind: OptionKind,
        strike: f64,
        expiry: chrono::DateTime<chrono::Utc>,
    ) -> OptionContract {
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

    #[test]
    fn pnl_color_is_up_for_gain_down_for_loss() {
        let t = Theme::for_appearance(crate::tui::theme::Appearance::Dark);
        assert_eq!(pnl_color(Some(5.0), &t), t.up);
        assert_eq!(pnl_color(Some(-5.0), &t), t.down);
        assert_eq!(pnl_color(Some(0.0), &t), t.neutral);
        assert_eq!(pnl_color(None, &t), t.dim);
    }

    #[test]
    fn direction_classifies_relative_to_epsilon_band() {
        assert_eq!(direction(0.5, PRICE_EPSILON), Direction::Up);
        assert_eq!(direction(-0.5, PRICE_EPSILON), Direction::Down);
        assert_eq!(direction(0.0, PRICE_EPSILON), Direction::Neutral);
        assert_eq!(
            direction(PRICE_EPSILON / 2.0, PRICE_EPSILON),
            Direction::Neutral
        );
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

    #[test]
    fn breadth_bar_splits_evenly_for_equal_counts() {
        assert_eq!(breadth_bar(5, 5, 10), (5, 5));
    }

    #[test]
    fn breadth_bar_all_advancers_fills_up_side() {
        assert_eq!(breadth_bar(3, 0, 10), (10, 0));
    }

    #[test]
    fn breadth_bar_all_decliners_fills_down_side() {
        assert_eq!(breadth_bar(0, 3, 10), (0, 10));
    }

    #[test]
    fn breadth_bar_gives_rounding_remainder_to_decliners() {
        assert_eq!(breadth_bar(1, 2, 9), (3, 6));
    }

    #[test]
    fn breadth_bar_is_zero_for_no_quotes_or_no_width() {
        assert_eq!(breadth_bar(0, 0, 10), (0, 0));
        assert_eq!(breadth_bar(5, 5, 0), (0, 0));
    }

    #[test]
    fn heat_color_zero_is_neutral() {
        let t = Theme::for_appearance(crate::tui::theme::Appearance::Dark);
        assert_eq!(heat_color(0.0, &t), t.neutral);
    }

    #[test]
    fn heat_color_saturates_up_beyond_cap() {
        let t = Theme::for_appearance(crate::tui::theme::Appearance::Dark);
        assert_eq!(heat_color(3.0, &t), t.up_strong);
        assert_eq!(heat_color(9.9, &t), t.up_strong);
    }

    #[test]
    fn heat_color_saturates_down_beyond_cap() {
        let t = Theme::for_appearance(crate::tui::theme::Appearance::Dark);
        assert_eq!(heat_color(-3.0, &t), t.down_strong);
        assert_eq!(heat_color(-9.9, &t), t.down_strong);
    }

    #[test]
    fn heat_color_midway_up_is_between_neutral_and_up_strong() {
        let t = Theme::for_appearance(crate::tui::theme::Appearance::Dark);
        let (Color::Rgb(nr, ng, nb), Color::Rgb(ur, ug, ub), Color::Rgb(mr, mg, mb)) =
            (t.neutral, t.up_strong, heat_color(1.5, &t))
        else {
            panic!("theme colors are RGB");
        };
        assert!(mr >= nr.min(ur) && mr <= nr.max(ur));
        assert!(mg >= ng.min(ug) && mg <= ng.max(ug));
        assert!(mb >= nb.min(ub) && mb <= nb.max(ub));
    }
}

//! Ratatui rendering for the TUI binary.

use std::{collections::HashMap, time::Instant};

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

use assetui::models::{FLASH_TTL, QuoteRecord, QuoteRecordAnalysis};
use assetui::search::subseq_match_ci;

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

    let (input_border, db_border) = if app.input_mode.toggled {
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
    let cursor = if app.input_mode.toggled && app.blink_state {
        "▌"
    } else {
        " "
    };
    let title = if app.input_mode.fuzzy_search {
        "assetui · search"
    } else {
        "assetui · query"
    };
    let input_block = Block::default()
        .title(block_title(title.into(), &t))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color).bg(t.panel))
        .style(Style::default().bg(t.panel));
    let mut spans = Vec::with_capacity(3);
    if app.input_mode.fuzzy_search {
        spans.push(Span::styled("/", Style::default().fg(t.dim)));
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
    let query = &app.input_mode.input;
    let searching = app.input_mode.fuzzy_search;
    let rows = app.db_display.rows[window].iter().map(|q| {
        Row::new(vec![
            opt_cell(q.id, |id| id.to_string()),
            fuzzy_cell(q.ticker.as_deref(), query, searching, &t),
            fuzzy_cell(q.name.as_deref(), query, searching, &t),
            opt_cell(q.price, |p| format!("{p:.2}"))
                .style(Style::default().fg(price_change_color(q, &app.row_flash_map, &t))),
            opt_cell(q.previous_close, |p| format!("{p:.2}")),
            opt_cell(q.day_volume, fmt_volume),
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

    let mut title = vec![Span::styled(
        " assetui · quotes ",
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

fn draw_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let t = app.theme;
    f.render_widget(
        Block::default().style(Style::default().bg(t.statusbar)),
        area,
    );

    let (mode, hint) = if app.stock_modal.info_visible || app.stock_modal.chart_visible {
        ("VIEW", "esc close")
    } else if app.input_mode.toggled && app.input_mode.fuzzy_search {
        ("SEARCH", "type to filter · enter apply · esc cancel")
    } else if app.input_mode.toggled {
        ("INPUT", "type ticker · enter fetch · esc done")
    } else if app.input_mode.fuzzy_search {
        (
            "FILTER",
            "j/k move · h/l page · / new search · esc clear · ? info · enter chart · q quit",
        )
    } else {
        (
            "NORMAL",
            "j/k move · h/l page · i query · / search · ? info · enter chart · o order · q quit",
        )
    };

    let chip = format!(" {mode} ");
    let page = app.db_display.page + 1;
    let total_pages = app.db_display.total_pages();
    let count = if app.input_mode.fuzzy_search {
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

/// Human-readable share volume: `55_000_000.0` → `"55.0M"`.
fn fmt_volume(v: f64) -> String {
    let abs = v.abs();
    if abs >= 1e9 {
        format!("{:.1}B", v / 1e9)
    } else if abs >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if abs >= 1e3 {
        format!("{:.1}K", v / 1e3)
    } else {
        format!("{v:.0}")
    }
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
        price_line(stock, &app.row_flash_map, &t),
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
        && prev != 0.0
    {
        let diff = price - prev;
        let pct = diff / prev * 100.0;
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

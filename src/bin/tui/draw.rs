use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table},
};

use super::app::App;

pub fn draw(f: &mut Frame, app: &mut App) {
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(f.area());

    let left = outer[0];
    let right = outer[1];

    let left_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(left);

    let input_area = left_split[0];
    let log_area = left_split[1];

    let in_input = app.input_mode.toggled;
    let input_border = if in_input { Color::Cyan } else { Color::DarkGray };
    let db_border = if in_input { Color::DarkGray } else { Color::Cyan };

    let cursor = if in_input && app.blink_state { "▌" } else { " " };
    let input_display = format!("{}{}", app.input_mode.input, cursor);
    let input_block = Block::default()
        .title("Query")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(input_border));
    f.render_widget(Paragraph::new(input_display.as_str()).block(input_block), input_area);

    let log_block = Block::default()
        .title("Logs")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let visible = log_area.height.saturating_sub(2) as usize;
    let start = app.logs.len().saturating_sub(visible);
    let log_text = app.logs[start..].join("\n");
    f.render_widget(Paragraph::new(log_text).block(log_block), log_area);

    draw_quotes_table(f, right, app, db_border);

    if app.stock_modal.visible {
        draw_stock_modal(f, app);
    }
}

fn draw_quotes_table(f: &mut Frame, area: Rect, app: &mut App, border_color: Color) {
    let box_bg = Color::DarkGray;
    let label_sty = Style::default().fg(Color::White).add_modifier(Modifier::BOLD);
    let key_sty = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
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
        hcell("Ticker ", "(n)"),
        hcell("Price ", "(p)"),
        hcell("Prev Close ", "(c)"),
        hcell("Volume ", "(v)"),
        hcell("As Of ", "(a)"),
    ])
    .bottom_margin(1);

    let rows = app.db_display.rows.iter().map(|q| {
        let price = q.price.map(|p| format!("{p:.2}")).unwrap_or_else(|| "-".to_string());
        let prev = q.previous_close.map(|p| format!("{p:.2}")).unwrap_or_else(|| "-".to_string());
        let vol = q.day_volume.map(|v| format!("{v:.0}")).unwrap_or_else(|| "-".to_string());
        let as_of = q.as_of.map(|dt| dt.format("%Y-%m-%d").to_string()).unwrap_or_else(|| "-".to_string());
        Row::new(vec![
            Cell::from(q.id.map(|id| id.to_string()).unwrap_or_else(|| "-".to_string())),
            Cell::from(q.ticker.clone().unwrap()),
            Cell::from(price),
            Cell::from(prev),
            Cell::from(vol),
            Cell::from(as_of),
        ])
    });

    let widths = [
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(11),
        Constraint::Length(15),
        Constraint::Length(13),
        Constraint::Length(22),
    ];

    let title = format!("Quote{}", app.db_display.status);
    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border_color)),
        )
        .row_highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
        .highlight_symbol(">> ");

    f.render_stateful_widget(table, area, &mut app.db_display.table_state);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vert[1])[1]
}

fn draw_stock_modal(f: &mut Frame, app: &mut App) {
    let area = centered_rect(60, 40, f.area());
    f.render_widget(Clear, area);
    let modal = Block::bordered()
        .title("Stock Info")
        .style(Style::default());

    let inner = modal.inner(area);
    f.render_widget(modal, area);

    let stock = &app.stock_modal.stock;
    let text = format!(
        "Ticker: {}, Price: {}",
        stock.ticker.as_deref().unwrap_or("-"),
        stock.price.map(|p| format!("{:.2}", p)).unwrap_or("-".into())
    );
    f.render_widget(Paragraph::new(text), inner);
}

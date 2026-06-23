use std::io;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

use ratatui::widgets::Table;
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Cell, Paragraph, Row},
};

use sqlx::postgres::PgPoolOptions;
use tokio::sync::mpsc;

use yfinance::{fetch::fetch_recent, models::QuoteRecord};

enum AppEvent {
    PageLoaded(Vec<QuoteRecord>),
    Error(String),
}

struct App {
    input: String,
    should_quit: bool,
    rows: Vec<QuoteRecord>,
    status: String,
}

impl App {
    fn new() -> Self {
        Self {
            input: String::new(),
            should_quit: false,
            rows: Vec::new(),
            status: "loading...".to_string(),
        }
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::PageLoaded(rows) => {
                self.status = format!("Loaded {} rows", rows.len());
                self.rows = rows;
            }
            AppEvent::Error(e) => {
                self.status = format!("Error: {}", e);
            }
        }
    }
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let database_url = dotenvy::var("DATABASE_URL").unwrap_or_else(|_| {
        eprintln!("DATABASE_URL not set in environment");
        std::process::exit(1);
    });

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("failed to connecto db");

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    {
        let pool = pool.clone();
        let tx = event_tx.clone();
        tokio::spawn(async move {
            match fetch_recent(&pool, 200).await {
                Ok(rows) => {
                    let _ = tx.send(AppEvent::PageLoaded(rows));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!(
                        "Failed to fetch recent quotes: {}",
                        e
                    )));
                }
            }
        });
    }
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    let result = run(&mut terminal, &mut app, &mut event_rx).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    event_rx: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> io::Result<()> {
    loop {
        while let Ok(event) = event_rx.try_recv() {
            app.handle_event(event);
        }

        terminal.draw(|f| draw(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Esc => app.should_quit = true,
                KeyCode::Char(c) => app.input.push(c),
                KeyCode::Backspace => {
                    app.input.pop();
                }
                KeyCode::Enter => {
                    // Spawn a fetch using app.input
                    app.input.clear();
                }
                _ => {}
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

fn draw(f: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(f.area());

    let left = outer[0];
    let right = outer[1];

    // Left split - query and log
    let left_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(left);

    let input_area = left_split[0];
    let log_area = left_split[1];

    // Input plane
    let input_block = Block::default()
        .title("Query")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let input_widget = Paragraph::new(app.input.as_str()).block(input_block);
    f.render_widget(input_widget, input_area);

    // Log pane
    let log_block = Block::default()
        .title("Logs")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(log_block, log_area);

    draw_quotes_table(f, right, app);

    f.set_cursor_position((input_area.x + app.input.len() as u16 + 1, input_area.y + 1));
}

fn draw_quotes_table(f: &mut Frame, area: ratatui::layout::Rect, app: &App) {
    let header = Row::new(vec![
        "ID",
        "Ticker",
        "Price",
        "Prev Close",
        "Volume",
        "As Of",
    ])
    .style(Style::default().add_modifier(Modifier::BOLD))
    .bottom_margin(1);

    let rows = app.rows.iter().map(|q| {
        let price = q
            .price
            .map(|p| format!("{p:.2}"))
            .unwrap_or_else(|| "-".to_string());
        let prev = q
            .previous_close
            .map(|p| format!("{p:.2}"))
            .unwrap_or_else(|| "-".to_string());
        let vol = q
            .day_volume
            .map(|v| format!("{v:.0}"))
            .unwrap_or_else(|| "-".to_string());
        let as_of = q
            .as_of
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "-".to_string());
        Row::new(vec![
            Cell::from(
                q.id.map(|id| id.to_string())
                    .unwrap_or_else(|| "-".to_string()),
            ),
            Cell::from(q.name.clone().unwrap()),
            Cell::from(price),
            Cell::from(prev),
            Cell::from(vol),
            Cell::from(as_of),
        ])
    });

    let widths = [
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(10),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(20),
    ];

    let title = format!("Quote({})", app.status);
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().title(title).borders(Borders::ALL))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_widget(table, area);
}

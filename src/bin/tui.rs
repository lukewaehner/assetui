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

use yfinance::{
    fetch::{fetch_quote_and_store, fetch_recent, fetch_sorted},
    models::QuoteRecord,
    sort::{SortMode, SortOrder},
};

enum AppEvent {
    PageLoaded(Vec<QuoteRecord>),
    FetchSpawned(String),
    FetchCompleted(QuoteRecord),
    ChangeSortMode(SortMode),
    ChangeSortOrder(SortOrder),
    LogLine(String),
    Error(String),
}

struct App {
    input_mode: InputMode,
    should_quit: bool,
    db_display: DbDisplay,
    logs: Vec<String>,
    pool: sqlx::PgPool,
    event_tx: mpsc::UnboundedSender<AppEvent>,
}

struct InputMode {
    // Whether keystrokes are interpreted as commands or text input for fetching / searching / etc
    input: String,
    toggled: bool,
}

struct DbDisplay {
    rows: Vec<QuoteRecord>,
    status: String,
    sort_mode: SortMode,
    sort_order: SortOrder,
}

impl App {
    fn new(pool: sqlx::PgPool, event_tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        Self {
            input_mode: InputMode {
                input: String::new(),
                toggled: false,
            },
            should_quit: false,
            db_display: DbDisplay {
                rows: Vec::new(),
                status: String::from("Loading..."),
                sort_mode: SortMode::ById,
                sort_order: SortOrder::Descending,
            },
            logs: Vec::new(),
            pool,
            event_tx,
        }
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::PageLoaded(rows) => {
                self.db_display.status = format!("Loaded {} rows", rows.len());
                self.db_display.rows = rows;
            }
            AppEvent::FetchSpawned(symbol) => {
                self.db_display.status = format!("fetching {symbol}…");
                self.logs.push(format!("[INFO] fetching {symbol}"));
            }
            AppEvent::FetchCompleted(record) => {
                let name = record.name.clone().unwrap_or_else(|| "?".to_string());
                self.db_display.status = format!("stored {name}");
                self.logs.push(format!("[SUCCESS] stored {name}"));
                // Newest on top, matching the fetch_recent ordering.
                self.db_display.rows.insert(0, record);
            }
            AppEvent::LogLine(line) => {
                self.logs.push(line);
            }
            AppEvent::Error(e) => {
                self.db_display.status = format!("Error: {e}");
                self.logs.push(format!("[ERROR] {e}"));
            }
            AppEvent::ChangeSortMode(mode) => {
                self.db_display.sort_mode = mode;
                self.spawn_reload();
            }
            AppEvent::ChangeSortOrder(order) => {
                self.db_display.sort_order = order;
                self.spawn_reload();
            }
        }
    }

    // Re-query the DB with the current sort mode/order and deliver the result
    // back through the event channel, matching how every other fetch is wired.
    fn spawn_reload(&self) {
        let pool = self.pool.clone();
        let tx = self.event_tx.clone();
        let mode = self.db_display.sort_mode;
        let order = self.db_display.sort_order;
        tokio::spawn(async move {
            match fetch_sorted(&pool, mode, order, 200).await {
                Ok(rows) => {
                    let _ = tx.send(AppEvent::PageLoaded(rows));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to sort quotes: {e}")));
                }
            }
        });
    }

    fn handle_command_key(&mut self, key: char) {
        match key.to_ascii_lowercase() {
            'q' => self.should_quit = true,
            'i' => self.input_mode.toggled = !self.input_mode.toggled,
            'd' | 'p' | 'c' | 'v' | 'a' | 'n' => {
                let mode = match key.to_ascii_lowercase() {
                    'd' => SortMode::ById,
                    'p' => SortMode::ByPrice,
                    'c' => SortMode::ByPrevClose,
                    'v' => SortMode::ByVolume,
                    'a' => SortMode::ByAsOf,
                    'n' => SortMode::ByName,
                    _ => unreachable!(),
                };
                self.handle_event(AppEvent::ChangeSortMode(mode));
            }
            'o' => {
                let order = match self.db_display.sort_order {
                    SortOrder::Ascending => SortOrder::Descending,
                    SortOrder::Descending => SortOrder::Ascending,
                };
                self.handle_event(AppEvent::ChangeSortOrder(order));
            }
            _ => {}
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

    let mut app = App::new(pool, event_tx);
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
                KeyCode::Esc => app.input_mode.toggled = false,
                // Input mode so we handle text input for fetch
                KeyCode::Char(c) if app.input_mode.toggled => app.input_mode.input.push(c),
                // Command mode
                KeyCode::Char(c) => app.handle_command_key(c),
                KeyCode::Backspace if app.input_mode.toggled => {
                    app.input_mode.input.pop();
                }
                KeyCode::Enter if app.input_mode.toggled => {
                    let symbol = app.input_mode.input.trim().to_uppercase();
                    app.input_mode.input.clear();
                    if !symbol.is_empty() {
                        let tx = app.event_tx.clone();
                        let pool = app.pool.clone();
                        tokio::spawn(async move {
                            let _ = tx.send(AppEvent::FetchSpawned(symbol.clone()));
                            match fetch_quote_and_store(&pool, &symbol).await {
                                Ok(Some(record)) => {
                                    let _ = tx.send(AppEvent::FetchCompleted(record));
                                }
                                Ok(None) => {
                                    let _ = tx.send(AppEvent::LogLine(format!(
                                        "no quote found for {symbol}"
                                    )));
                                }
                                Err(e) => {
                                    let _ = tx.send(AppEvent::Error(format!("{symbol}: {e}")));
                                }
                            }
                        });
                    }
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
    let input_widget = Paragraph::new(app.input_mode.input.as_str()).block(input_block);
    f.render_widget(input_widget, input_area);

    // Log pane - show the most recent lines that fit inside the borders.
    let log_block = Block::default()
        .title("Logs")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let visible = log_area.height.saturating_sub(2) as usize;
    let start = app.logs.len().saturating_sub(visible);
    let log_text = app.logs[start..].join("\n");
    let log_widget = Paragraph::new(log_text).block(log_block);
    f.render_widget(log_widget, log_area);

    draw_quotes_table(f, right, app);

    f.set_cursor_position((
        input_area.x + app.input_mode.input.len() as u16 + 1,
        input_area.y + 1,
    ));
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

    let rows = app.db_display.rows.iter().map(|q| {
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

    let title = format!("Quote({})", app.db_display.status);
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().title(title).borders(Borders::ALL))
        .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    f.render_widget(table, area);
}

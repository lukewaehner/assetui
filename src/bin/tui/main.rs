//! TUI binary entry point.
//!
//! Sets up the Postgres pool, initialises the ratatui terminal, seeds the
//! initial quote page, then drives the event loop until the user quits.

mod app;
mod draw;

use std::io;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};

use ratatui::{Terminal, backend::CrosstermBackend};

use sqlx::postgres::PgPoolOptions;
use tokio::sync::mpsc;

use yfinance::fetch::fetch_sorted;
use yfinance::sort::{SortMode, SortOrder};

use app::{App, AppEvent};

#[tokio::main]
async fn main() -> io::Result<()> {
    let database_url = dotenvy::var("DATABASE_URL").map_err(|_| {
        io::Error::new(io::ErrorKind::NotFound, "DATABASE_URL not set in environment")
    })?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("failed to connect to db");

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    {
        let pool = pool.clone();
        let tx = event_tx.clone();
        tokio::spawn(async move {
            match fetch_sorted(&pool, SortMode::ById, SortOrder::Descending, 200).await {
                Ok(rows) => {
                    let _ = tx.send(AppEvent::PageLoaded(rows));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to fetch quotes: {e}")));
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

/// Main event loop.
///
/// Drains pending [`AppEvent`]s, toggles the cursor blink, redraws the frame,
/// then polls for terminal input - all within a 100 ms tick.  Exits when
/// [`App::should_quit`] is set.
async fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    event_rx: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> io::Result<()> {
    loop {
        while let Ok(event) = event_rx.try_recv() {
            app.handle_event(event);
        }

        let now = Instant::now();
        if now.duration_since(app.last_blink) >= Duration::from_millis(500) {
            app.blink_state = !app.blink_state;
            app.last_blink = now;
        }

        terminal.draw(|f| draw::draw(f, app))?;

        if event::poll(std::time::Duration::from_millis(100))?
            && let Event::Key(key) = event::read()?
        {
            match key.code {
                KeyCode::Esc => {
                    if app.stock_modal.visible {
                        app.stock_modal.visible = false;
                    } else {
                        app.input_mode.toggled = false;
                    }
                }
                KeyCode::Char(c) if app.input_mode.toggled => app.input_mode.input.push(c),
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
                            match yfinance::fetch::fetch_quote_and_store(&pool, &symbol).await {
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

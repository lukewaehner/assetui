//! TUI binary entry point.
//!
//! Sets up the Postgres pool, initialises the ratatui terminal, seeds the
//! initial quote page, then drives the event loop until the user quits.

mod app;
mod draw;
mod theme;

use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event};

use ratatui::Terminal;

use sqlx::postgres::PgPoolOptions;
use tokio::sync::mpsc;

use app::{App, AppEvent};

/// How long each loop iteration waits for terminal input before redrawing.
const TICK_RATE: Duration = Duration::from_millis(100);

#[tokio::main]
async fn main() -> io::Result<()> {
    let database_url = dotenvy::var("DATABASE_URL").map_err(|_| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "DATABASE_URL not set in environment",
        )
    })?;

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("failed to connect to db");

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    let mut app = App::new(pool, event_tx);
    app.spawn_reload();
    app.spawn_theme_watcher();

    // Sets up the terminal (raw mode + alternate screen) and installs a panic
    // hook that restores it, so a panic doesn't leave the shell garbled.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &mut event_rx).await;
    ratatui::restore();
    result
}

/// Main event loop.
///
/// Drains pending [`AppEvent`]s, advances time-based state, redraws the
/// frame, then polls for terminal input - all within a [`TICK_RATE`] tick.
/// Exits when [`App::should_quit`] is set.
async fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    event_rx: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> io::Result<()>
where
    io::Error: From<B::Error>,
{
    let mut last_tick = Instant::now();
    loop {
        while let Ok(event) = event_rx.try_recv() {
            app.handle_event(event);
        }

        app.tick(last_tick.elapsed());
        last_tick = Instant::now();

        terminal.draw(|f| draw::draw(f, app))?;

        if event::poll(TICK_RATE)?
            && let Event::Key(key) = event::read()?
        {
            app.handle_key(key.code);
        }

        if app.should_quit {
            if let Some(handle) = app.stream_handle.take() {
                handle.stop().await;
            }
            return Ok(());
        }
    }
}

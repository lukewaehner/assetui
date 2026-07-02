//! TUI binary entry point.

mod app;
mod draw;

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

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &mut event_rx).await;
    ratatui::restore();
    result
}

/// Main event loop.
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
            return Ok(());
        }
    }
}

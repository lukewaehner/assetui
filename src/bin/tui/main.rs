//! TUI binary entry point.
//!
//! Sets up the Postgres pool, initialises the ratatui terminal, seeds the
//! initial quote page, then drives the event loop until the user quits.

use std::io;
use std::time::Duration;

use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::Terminal;
use tokio::sync::mpsc;
use yfinance::AppError;
use yfinance::db::connection::{DEFAULT_MAX_CONNECTIONS, setup_pool};
use yfinance_rs::YfClient;

use app::{App, AppEvent, table_page_size};

mod app;
mod draw;
mod theme;

/// Cadence of time-based updates (cursor blink, notification animations,
/// price-flash expiry).  Input and async events are handled the moment they
/// arrive via `tokio::select!`, so this does not bound input latency.
const TICK_RATE: Duration = Duration::from_millis(100);

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let database_url = dotenvy::var("DATABASE_URL")?;
    let pool = setup_pool(&database_url, DEFAULT_MAX_CONNECTIONS).await?;

    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    let mut app = App::new(pool, YfClient::default(), event_tx);
    app.spawn_reload();
    app.spawn_theme_watcher();

    // Sets up the terminal (raw mode + alternate screen) and installs a panic
    // hook that restores it, so a panic doesn't leave the shell garbled.
    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app, &mut event_rx).await;
    ratatui::restore();
    Ok(result?)
}

/// Main event loop.
///
/// `tokio::select!`s over three sources - terminal input, [`AppEvent`]s from
/// async tasks, and a [`TICK_RATE`] interval for time-based state - and only
/// redraws when one of them changed something visible, so an idle app costs
/// near-zero CPU.  Exits when [`App::should_quit`] is set or stdin closes.
async fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    event_rx: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> io::Result<()>
where
    io::Error: From<B::Error>,
{
    let mut input_events = EventStream::new();
    let mut tick = tokio::time::interval(TICK_RATE);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // First frame before any event arrives.
    app.set_page_size(table_page_size(terminal.size()?.height));
    terminal.draw(|f| draw::draw(f, app))?;

    loop {
        let mut redraw = true;
        tokio::select! {
            maybe_event = input_events.next() => match maybe_event {
                Some(Ok(Event::Key(key))) => app.handle_key(key.code),
                // Resize is handled by the redraw itself (set_page_size);
                // other events (focus, mouse) change nothing but are cheap.
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e),
                // Input stream closed (e.g. detached terminal): quit cleanly.
                None => break,
            },
            Some(event) = event_rx.recv() => {
                app.handle_event(event);
                // Drain any burst (e.g. a flurry of stream ticks) so one
                // redraw covers all of it.
                while let Ok(event) = event_rx.try_recv() {
                    app.handle_event(event);
                }
            },
            _ = tick.tick() => {
                redraw = app.tick(TICK_RATE);
            },
        }

        if app.should_quit {
            break;
        }

        if redraw {
            // Page size follows the terminal height; doing this here (not in
            // the draw call) keeps stream-resubscription side effects out of
            // the render path.
            app.set_page_size(table_page_size(terminal.size()?.height));
            terminal.draw(|f| draw::draw(f, app))?;
        }
    }

    if let Some(handle) = app.stream_handle.take() {
        handle.stop().await;
    }
    Ok(())
}

//! TUI event loop and entry point.

use std::io;
use std::time::Duration;

use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::Terminal;
use tokio::sync::mpsc;
use yfinance_rs::YfClient;

use super::app::{App, AppEvent, table_page_size};
use super::draw;
use crate::AppError;

/// Cadence of time-based updates (cursor blink, notification animations, price-flash expiry).
const TICK_RATE: Duration = Duration::from_millis(100);

/// Runs the interactive TUI to completion, returning once the user quits or stdin closes.
pub async fn run(pool: sqlx::PgPool, client: YfClient) -> Result<(), AppError> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();

    let mut app = App::new(pool, client, event_tx);
    app.spawn_reload();
    app.spawn_load_watchlist();
    app.spawn_load_alerts();
    app.spawn_load_portfolio();
    app.portfolio_modal.loading = true;
    app.spawn_load_portfolio_history();
    app.spawn_theme_watcher();

    let mut terminal = ratatui::init();
    let result = run_loop(&mut terminal, &mut app, &mut event_rx).await;
    ratatui::restore();
    Ok(result?)
}

/// Runs the event loop to completion, then always runs teardown before returning.
async fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    event_rx: &mut mpsc::UnboundedReceiver<AppEvent>,
) -> io::Result<()>
where
    io::Error: From<B::Error>,
{
    let outcome = drive_event_loop(terminal, app, event_rx).await;

    if let Some(handle) = app.stream_handle.take() {
        handle.stop().await;
    }

    if let Some((market_value, cost_basis)) = app.persistable_totals_now() {
        let day = chrono::Local::now().date_naive();
        if let Err(e) =
            crate::db::portfolio_history::upsert_day(&app.services.pool, day, market_value, cost_basis)
                .await
        {
            tracing::warn!("final portfolio history sample failed: {e}");
        }
    }

    outcome
}

async fn drive_event_loop<B: ratatui::backend::Backend>(
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

    app.set_page_size(table_page_size(terminal.size()?.height));
    terminal.draw(|f| draw::draw(f, app))?;

    loop {
        let mut redraw = true;
        tokio::select! {
            maybe_event = input_events.next() => match maybe_event {
                Some(Ok(Event::Key(key))) => app.handle_key(key.code),
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e),
                None => break,
            },
            Some(event) = event_rx.recv() => {
                app.handle_event(event);
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
            app.set_page_size(table_page_size(terminal.size()?.height));
            terminal.draw(|f| draw::draw(f, app))?;
        }
    }

    Ok(())
}

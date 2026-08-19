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

/// Bound on the final, awaited history write during shutdown.
const SHUTDOWN_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

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
        let write = crate::db::portfolio_history::upsert_day(
            &app.services.pool,
            day,
            market_value,
            cost_basis,
        );
        match tokio::time::timeout(SHUTDOWN_WRITE_TIMEOUT, write).await {
            Ok(Err(e)) => tracing::warn!("final portfolio history sample failed: {e}"),
            Err(_) => tracing::warn!(
                "final portfolio history sample timed out after {SHUTDOWN_WRITE_TIMEOUT:?}"
            ),
            Ok(Ok(())) => {}
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

#[cfg(test)]
mod tests {
    use std::cell::Cell as CallCounter;

    use ratatui::backend::{Backend, ClearType, WindowSize};
    use ratatui::buffer::Cell;
    use ratatui::layout::{Position, Size};
    use yfinance_rs::YfClient;

    use super::*;
    use crate::db::portfolio_history::load_history;
    use crate::models::QuoteRecord;
    use crate::portfolio::Holding;

    /// A `Backend` whose `size()` succeeds exactly once.
    struct FailingSizeBackend {
        size_calls: CallCounter<u32>,
    }

    impl FailingSizeBackend {
        fn new() -> Self {
            Self {
                size_calls: CallCounter::new(0),
            }
        }
    }

    impl Backend for FailingSizeBackend {
        type Error = io::Error;

        fn draw<'a, I>(&mut self, _content: I) -> io::Result<()>
        where
            I: Iterator<Item = (u16, u16, &'a Cell)>,
        {
            Ok(())
        }

        fn hide_cursor(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn show_cursor(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn get_cursor_position(&mut self) -> io::Result<Position> {
            Ok(Position::ORIGIN)
        }

        fn set_cursor_position<P: Into<Position>>(&mut self, _position: P) -> io::Result<()> {
            Ok(())
        }

        fn clear(&mut self) -> io::Result<()> {
            Ok(())
        }

        fn clear_region(&mut self, _clear_type: ClearType) -> io::Result<()> {
            Ok(())
        }

        fn size(&self) -> io::Result<Size> {
            let calls = self.size_calls.get();
            self.size_calls.set(calls + 1);
            if calls == 0 {
                Ok(Size::new(80, 24))
            } else {
                Err(io::Error::other("backend size unavailable"))
            }
        }

        fn window_size(&mut self) -> io::Result<WindowSize> {
            Ok(WindowSize {
                columns_rows: Size::new(80, 24),
                pixels: Size::new(0, 0),
            })
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[sqlx::test]
    #[ignore = "requires a controlling tty for crossterm's EventStream; run with `cargo test -- --ignored` from a real terminal, or `script -q /dev/null cargo test -- --ignored <name>`"]
    async fn run_loop_writes_the_final_history_sample_when_the_loop_exits_via_a_question_mark(
        pool: sqlx::PgPool,
    ) {
        // arrange
        let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
        let mut app = App::new(pool.clone(), YfClient::default(), tx);
        app.holdings = vec![Holding {
            ticker: "AAPL".to_string(),
            shares: 2.0,
            avg_cost: 100.0,
        }];
        app.market.db_display.all_rows = vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: Some(150.0),
            ..Default::default()
        }];
        app.has_live_prices = true;

        let backend = FailingSizeBackend::new();
        let mut terminal =
            Terminal::new(backend).expect("Terminal::new should succeed on the first size() call");

        // act
        let result = run_loop(&mut terminal, &mut app, &mut rx).await;

        // assert
        assert!(
            result.is_err(),
            "expected the size() failure to propagate as an Err"
        );
        let history = load_history(&pool)
            .await
            .expect("load_history should succeed");
        assert_eq!(
            history,
            vec![(chrono::Local::now().date_naive(), 300.0, 200.0)]
        );
    }
}

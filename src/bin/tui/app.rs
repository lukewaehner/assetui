//! Application state and event handling for the TUI binary.
//!
//! The event loop in `main.rs` owns the terminal and translates raw key events
//! into [`AppEvent`] variants.  [`App::handle_event`] processes those events
//! and mutates state; [`draw`](super::draw::draw) then renders the current
//! state on the next frame.

use std::time::{Duration, Instant};

use ratatui::widgets::TableState;
use ratatui_notifications::{
    Animation, Anchor, AutoDismiss, Level, Notification, Notifications, SlideDirection,
};
use tokio::sync::mpsc;

use yfinance::{
    fetch::{fetch_analysis, fetch_sorted},
    models::{QuoteRecord, QuoteRecordAnalysis},
    sort::{SortMode, SortOrder},
};

/// Messages sent from async tasks back to the main event loop.
pub enum AppEvent {
    /// Initial page load or a re-fetch triggered by a sort change.
    PageLoaded(Vec<QuoteRecord>),
    /// A fetch task has been spawned for the given symbol.
    FetchSpawned(String),
    /// A quote was fetched and stored; the record is ready for display.
    FetchCompleted(QuoteRecord),
    /// Analyst consensus and price targets loaded for the selected stock.
    StockAnalysisReady(QuoteRecordAnalysis),
    /// User pressed a sort-column key.
    ChangeSortMode(SortMode),
    /// User toggled sort direction.
    ChangeSortOrder(SortOrder),
    /// Informational message for the log panel.
    LogLine(String),
    /// Error message; also shown in the status bar.
    Error(String),
}

/// Top-level application state.
pub struct App {
    /// Ticker input box state (current text and whether the box is focused).
    pub input_mode: InputMode,
    /// Set to `true` when the user presses `q`; the event loop exits on the
    /// next iteration.
    pub should_quit: bool,
    /// The quotes table and its associated sort/selection state.
    pub db_display: DbDisplay,
    /// Lines shown in the log panel (most recent at the bottom).
    pub logs: Vec<String>,
    /// Shared database connection pool passed to async tasks.
    pub pool: sqlx::PgPool,
    /// Channel used by async tasks to push [`AppEvent`]s back to the loop.
    pub event_tx: mpsc::UnboundedSender<AppEvent>,
    /// Current state of the blinking cursor in the input box.
    pub blink_state: bool,
    /// Timestamp of the last blink toggle, used to drive the 500 ms interval.
    pub last_blink: Instant,
    /// Overlay modal showing detailed info for the selected stock.
    pub stock_modal: StockInfoModal,
    /// Slide-in error notifications shown top-right.
    pub notifications: Notifications,
}

/// State for the ticker input box.
pub struct InputMode {
    /// Text the user has typed so far.
    pub input: String,
    /// Whether the input box is currently focused (i.e. typing goes here).
    pub toggled: bool,
}

/// State for the quotes table on the right side of the layout.
pub struct DbDisplay {
    /// Rows currently visible in the table.
    pub rows: Vec<QuoteRecord>,
    /// Ratatui widget state tracking the selected row index.
    pub table_state: TableState,
    /// Short status message shown in the table's title bar.
    pub status: String,
    /// Column the table is currently sorted by.
    pub sort_mode: SortMode,
    /// Direction of the current sort.
    pub sort_order: SortOrder,
}

/// State for the stock-detail overlay modal.
pub struct StockInfoModal {
    /// The quote row that was selected when `?` was pressed.
    pub stock: QuoteRecord,
    /// Analyst data fetched in the background; `None` while loading.
    pub analysis: Option<QuoteRecordAnalysis>,
    /// Whether the modal is currently visible.
    pub visible: bool,
}

impl App {
    /// Creates a new `App` with sensible defaults.  The table starts sorted by
    /// `id DESC` to show the most recently stored quotes first.
    pub fn new(pool: sqlx::PgPool, event_tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        Self {
            input_mode: InputMode {
                input: String::new(),
                toggled: false,
            },
            should_quit: false,
            db_display: DbDisplay {
                rows: Vec::new(),
                status: String::from("Loading..."),
                table_state: TableState::default(),
                sort_mode: SortMode::ById,
                sort_order: SortOrder::Descending,
            },
            logs: Vec::new(),
            pool,
            event_tx,
            blink_state: true,
            last_blink: Instant::now(),
            stock_modal: StockInfoModal {
                stock: QuoteRecord::default(),
                analysis: None,
                visible: false,
            },
            notifications: Notifications::new(),
        }
    }

    /// Applies an [`AppEvent`] to the application state.
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::PageLoaded(rows) => {
                self.db_display.status = String::new();
                self.db_display.rows = rows;
                if !self.db_display.rows.is_empty()
                    && self.db_display.table_state.selected().is_none()
                {
                    self.db_display.table_state.select(Some(0));
                }
            }
            AppEvent::FetchSpawned(symbol) => {
                self.db_display.status = format!("fetching {symbol}…");
                self.logs.push(format!("[INFO] fetching {symbol}"));
            }
            AppEvent::FetchCompleted(record) => {
                let name = record.ticker.as_deref().unwrap_or("?");
                self.db_display.status = format!("stored {name}");
                self.logs.push(format!("[SUCCESS] stored {name}"));
                self.db_display.rows.insert(0, record);
            }
            AppEvent::LogLine(line) => {
                self.logs.push(line);
            }
            AppEvent::Error(e) => {
                self.db_display.status = format!("Error: {e}");
                self.logs.push(format!("[ERROR] {e}"));
                if let Ok(notif) = Notification::new(e)
                    .title("Error")
                    .level(Level::Error)
                    .anchor(Anchor::TopRight)
                    .animation(Animation::Slide)
                    .slide_direction(SlideDirection::FromRight)
                    .auto_dismiss(AutoDismiss::After(Duration::from_secs(5)))
                    .build()
                {
                    let _ = self.notifications.add(notif);
                }
            }
            AppEvent::ChangeSortMode(mode) => {
                self.logs.push(format!("[SORT] mode → {mode:?}"));
                self.db_display.sort_mode = mode;
                self.spawn_reload();
            }
            AppEvent::ChangeSortOrder(order) => {
                self.logs.push(format!("[SORT] order → {order:?}"));
                self.db_display.sort_order = order;
                self.spawn_reload();
            }
            AppEvent::StockAnalysisReady(analysis) => {
                self.stock_modal.analysis = Some(analysis);
            }
        }
    }

    /// Spawns a task to re-fetch the current page with the active sort applied,
    /// then sends a [`AppEvent::PageLoaded`] back when done.
    pub fn spawn_reload(&self) {
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

    /// Routes a single command key to the appropriate state change.
    ///
    /// Only called when the input box is not focused; while focused, characters
    /// are appended to [`InputMode::input`] directly in the event loop.
    pub fn handle_command_key(&mut self, key: char) {
        match key.to_ascii_lowercase() {
            'q' => self.should_quit = true,
            'i' => self.input_mode.toggled = !self.input_mode.toggled,
            'd' => self.handle_event(AppEvent::ChangeSortMode(SortMode::ById)),
            'p' => self.handle_event(AppEvent::ChangeSortMode(SortMode::ByPrice)),
            'c' => self.handle_event(AppEvent::ChangeSortMode(SortMode::ByPrevClose)),
            'v' => self.handle_event(AppEvent::ChangeSortMode(SortMode::ByVolume)),
            'a' => self.handle_event(AppEvent::ChangeSortMode(SortMode::ByAsOf)),
            'n' => self.handle_event(AppEvent::ChangeSortMode(SortMode::ByName)),
            't' => self.handle_event(AppEvent::ChangeSortMode(SortMode::ByTicker)),
            'j' if !self.db_display.rows.is_empty() => {
                let len = self.db_display.rows.len();
                let i = self
                    .db_display
                    .table_state
                    .selected()
                    .map(|i| if i >= len - 1 { 0 } else { i + 1 })
                    .unwrap_or(0);
                self.db_display.table_state.select(Some(i));
            }
            'k' if !self.db_display.rows.is_empty() => {
                let len = self.db_display.rows.len();
                let i = self
                    .db_display
                    .table_state
                    .selected()
                    .map(|i| if i == 0 { len - 1 } else { i - 1 })
                    .unwrap_or(0);
                self.db_display.table_state.select(Some(i));
            }
            'o' => {
                let order = match self.db_display.sort_order {
                    SortOrder::Ascending => SortOrder::Descending,
                    SortOrder::Descending => SortOrder::Ascending,
                };
                self.handle_event(AppEvent::ChangeSortOrder(order));
            }
            '?' => {
                if let Some(i) = self.db_display.table_state.selected()
                    && let Some(row) = self.db_display.rows.get(i)
                {
                    let stock = row.clone();
                    let ticker = stock.ticker.clone();
                    self.stock_modal.stock = stock;
                    self.stock_modal.analysis = None;
                    self.stock_modal.visible = true;
                    if let Some(t) = ticker.as_deref() {
                        self.spawn_analysis(t);
                    }
                }
            }
            _ => {}
        }
    }

    /// Spawns a background task to fetch analyst data for `symbol` and sends
    /// the result back as [`AppEvent::StockAnalysisReady`].
    fn spawn_analysis(&self, symbol: &str) {
        let tx = self.event_tx.clone();
        let symbol = symbol.to_owned();
        tokio::spawn(async move {
            match fetch_analysis(&symbol).await {
                Ok(a) => {
                    let _ = tx.send(AppEvent::StockAnalysisReady(a));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("analysis {symbol}: {e}")));
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    use yfinance::models::QuoteRecord;
    use yfinance::sort::{SortMode, SortOrder};

    fn make_test_app() -> (App, mpsc::UnboundedReceiver<AppEvent>) {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        (App::new(pool, tx), rx)
    }

    #[tokio::test]
    async fn test_app_new_default_state() {
        let (app, _rx) = make_test_app();
        assert!(!app.should_quit);
        assert!(!app.input_mode.toggled);
        assert!(app.db_display.rows.is_empty());
        assert!(app.logs.is_empty());
        assert!(!app.stock_modal.visible);
        assert_eq!(app.db_display.sort_mode, SortMode::ById);
        assert_eq!(app.db_display.sort_order, SortOrder::Descending);
    }

    #[tokio::test]
    async fn test_handle_event_page_loaded() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord::default()]));
        assert_eq!(app.db_display.rows.len(), 1);
        assert!(app.db_display.status.is_empty());
        assert_eq!(app.db_display.table_state.selected(), Some(0));
    }

    #[tokio::test]
    async fn test_handle_event_fetch_spawned() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::FetchSpawned("AAPL".to_string()));
        assert!(app.db_display.status.contains("AAPL"));
        assert_eq!(app.logs.len(), 1);
    }

    #[tokio::test]
    async fn test_handle_event_fetch_completed() {
        let (mut app, _rx) = make_test_app();
        let record = QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        };
        app.handle_event(AppEvent::FetchCompleted(record));
        assert_eq!(app.db_display.rows.len(), 1);
        assert_eq!(app.logs.len(), 1);
        assert!(app.logs[0].contains("AAPL"));
    }

    #[tokio::test]
    async fn test_handle_event_log_line() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::LogLine("hello".to_string()));
        assert_eq!(app.logs.last(), Some(&"hello".to_string()));
    }

    #[tokio::test]
    async fn test_handle_event_error() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::Error("oops".to_string()));
        assert!(app.db_display.status.contains("oops"));
        assert!(app.logs.iter().any(|l| l.contains("oops")));
    }

    #[tokio::test]
    async fn test_command_key_q_quits() {
        let (mut app, _rx) = make_test_app();
        app.handle_command_key('q');
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn test_command_key_i_toggles_input() {
        let (mut app, _rx) = make_test_app();
        app.handle_command_key('i');
        assert!(app.input_mode.toggled);
        app.handle_command_key('i');
        assert!(!app.input_mode.toggled);
    }

    #[tokio::test]
    async fn test_command_key_navigation_j_wraps() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![
            QuoteRecord::default(),
            QuoteRecord::default(),
            QuoteRecord::default(),
        ]));
        assert_eq!(app.db_display.table_state.selected(), Some(0));
        app.handle_command_key('j');
        assert_eq!(app.db_display.table_state.selected(), Some(1));
        app.handle_command_key('j');
        assert_eq!(app.db_display.table_state.selected(), Some(2));
        app.handle_command_key('j'); // wraps back to 0
        assert_eq!(app.db_display.table_state.selected(), Some(0));
    }

    #[tokio::test]
    async fn test_command_key_navigation_k_wraps() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![
            QuoteRecord::default(),
            QuoteRecord::default(),
            QuoteRecord::default(),
        ]));
        assert_eq!(app.db_display.table_state.selected(), Some(0));
        app.handle_command_key('k'); // wraps from 0 to 2
        assert_eq!(app.db_display.table_state.selected(), Some(2));
    }

    #[tokio::test]
    async fn test_command_key_o_toggles_sort_order() {
        let (mut app, _rx) = make_test_app();
        assert_eq!(app.db_display.sort_order, SortOrder::Descending);
        app.handle_command_key('o');
        assert_eq!(app.db_display.sort_order, SortOrder::Ascending);
        app.handle_command_key('o');
        assert_eq!(app.db_display.sort_order, SortOrder::Descending);
    }

    #[tokio::test]
    async fn test_command_key_sort_by_price() {
        let (mut app, _rx) = make_test_app();
        assert_eq!(app.db_display.sort_mode, SortMode::ById);
        app.handle_command_key('p');
        assert_eq!(app.db_display.sort_mode, SortMode::ByPrice);
    }
}

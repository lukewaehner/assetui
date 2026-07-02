//! Application state and event handling for the TUI binary.
//!
//! The event loop in `main.rs` owns the terminal and forwards raw key events
//! to [`App::handle_key`].  Async tasks report back through [`AppEvent`]
//! variants which [`App::handle_event`] applies to state;
//! [`draw`](super::draw::draw) then renders the current state on the next
//! frame.

use std::ops::Range;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use ratatui::widgets::TableState;
use ratatui_notifications::{
    Anchor, Animation, AutoDismiss, Level, Notification, Notifications, SlideDirection,
};
use tokio::sync::mpsc;

use yfinance::{
    fetch::{fetch_analysis, fetch_quote_and_store, fetch_sorted},
    models::{QuoteRecord, QuoteRecordAnalysis},
    sort::{SortMode, SortOrder},
};

/// Maximum number of rows fetched from the database per reload.
const PAGE_FETCH_LIMIT: i64 = 200;

/// Interval at which the input-box cursor toggles visibility.
const BLINK_INTERVAL: Duration = Duration::from_millis(500);

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
    /// Timestamp of the last blink toggle, used to drive the blink interval.
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
    /// All rows fetched from the database; the visible page is a window into
    /// this, derived from `page` and `page_size` via [`Self::window_range`].
    pub rows: Vec<QuoteRecord>,
    /// Ratatui widget state tracking the selected row index.
    pub table_state: TableState,
    /// Short status message shown in the table's title bar.
    pub status: String,
    /// Column the table is currently sorted by.
    pub sort_mode: SortMode,
    /// Direction of the current sort.
    pub sort_order: SortOrder,
    /// Current page index (0-based).
    pub page: usize,
    /// Number of rows per page.
    pub page_size: usize,
}

impl DbDisplay {
    /// Returns the total number of pages given the current row count and page size.
    pub fn total_pages(&self) -> usize {
        if self.rows.is_empty() {
            1
        } else {
            self.rows.len().div_ceil(self.page_size)
        }
    }

    /// Returns the index range of `rows` visible on the current page.
    pub fn window_range(&self) -> Range<usize> {
        let start = (self.page * self.page_size).min(self.rows.len());
        let end = (start + self.page_size).min(self.rows.len());
        start..end
    }

    /// Returns the slice of `rows` visible on the current page.
    pub fn window(&self) -> &[QuoteRecord] {
        &self.rows[self.window_range()]
    }

    /// Returns the number of rows visible on the current page.
    pub fn window_len(&self) -> usize {
        self.window_range().len()
    }
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

/// Maps a sort-column key to its [`SortMode`], or `None` for non-sort keys.
fn sort_mode_for_key(key: char) -> Option<SortMode> {
    Some(match key {
        'd' => SortMode::ById,
        't' => SortMode::ByTicker,
        'n' => SortMode::ByName,
        'p' => SortMode::ByPrice,
        'c' => SortMode::ByPrevClose,
        'v' => SortMode::ByVolume,
        'a' => SortMode::ByAsOf,
        _ => return None,
    })
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
                page: 0,
                page_size: 25,
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

    /// Advances time-based state: toggles the cursor blink when its interval
    /// elapses and ticks the notification animations.
    pub fn tick(&mut self, elapsed: Duration) {
        let now = Instant::now();
        if now.duration_since(self.last_blink) >= BLINK_INTERVAL {
            self.blink_state = !self.blink_state;
            self.last_blink = now;
        }
        self.notifications.tick(elapsed);
    }

    /// Applies an [`AppEvent`] to the application state.
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::PageLoaded(rows) => {
                self.db_display.status = String::new();
                self.db_display.rows = rows;
                self.db_display.page = 0;
                self.reset_selection();
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
                self.reset_selection();
            }
            AppEvent::LogLine(line) => {
                self.logs.push(line);
            }
            AppEvent::Error(e) => self.push_error(e),
            AppEvent::StockAnalysisReady(analysis) => {
                self.stock_modal.analysis = Some(analysis);
            }
        }
    }

    /// Records an error in the status bar and log panel, and raises a
    /// slide-in notification.
    fn push_error(&mut self, e: String) {
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

    /// Routes a raw terminal key press to the appropriate state change.
    ///
    /// While the input box is focused, printable characters edit the input
    /// text; otherwise they are treated as command keys.
    pub fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                if self.stock_modal.visible {
                    self.stock_modal.visible = false;
                } else {
                    self.input_mode.toggled = false;
                }
            }
            KeyCode::Char(c) if self.input_mode.toggled => self.input_mode.input.push(c),
            KeyCode::Char(c) => self.handle_command_key(c),
            KeyCode::Backspace if self.input_mode.toggled => {
                self.input_mode.input.pop();
            }
            KeyCode::Enter if self.input_mode.toggled => self.submit_input(),
            _ => {}
        }
    }

    /// Routes a single command key to the appropriate state change.
    ///
    /// Only called when the input box is not focused; while focused, characters
    /// are appended to [`InputMode::input`] instead.
    pub fn handle_command_key(&mut self, key: char) {
        let key = key.to_ascii_lowercase();
        if let Some(mode) = sort_mode_for_key(key) {
            self.set_sort_mode(mode);
            return;
        }
        match key {
            'q' => self.should_quit = true,
            'i' => self.input_mode.toggled = !self.input_mode.toggled,
            'j' => self.move_selection(1),
            'k' => self.move_selection(-1),
            // Paginate: h = prev page, l = next page
            'h' => self.paginate(-1),
            'l' => self.paginate(1),
            'o' => self.toggle_sort_order(),
            '?' => self.open_stock_modal(),
            _ => {}
        }
    }

    /// Takes the current input-box text as a ticker symbol and spawns a fetch
    /// for it.  Empty input is ignored.
    fn submit_input(&mut self) {
        let symbol = self.input_mode.input.trim().to_uppercase();
        self.input_mode.input.clear();
        if !symbol.is_empty() {
            self.spawn_fetch(symbol);
        }
    }

    /// Spawns a task to fetch and store a quote for `symbol`, reporting
    /// progress and the result back as [`AppEvent`]s.
    fn spawn_fetch(&self, symbol: String) {
        let tx = self.event_tx.clone();
        let pool = self.pool.clone();
        tokio::spawn(async move {
            let _ = tx.send(AppEvent::FetchSpawned(symbol.clone()));
            match fetch_quote_and_store(&pool, &symbol).await {
                Ok(Some(record)) => {
                    let _ = tx.send(AppEvent::FetchCompleted(record));
                }
                Ok(None) => {
                    let _ = tx.send(AppEvent::LogLine(format!("no quote found for {symbol}")));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("{symbol}: {e}")));
                }
            }
        });
    }

    /// Sets the sort column and re-fetches the table.
    fn set_sort_mode(&mut self, mode: SortMode) {
        self.logs.push(format!("[SORT] mode → {mode:?}"));
        self.db_display.sort_mode = mode;
        self.spawn_reload();
    }

    /// Flips the sort direction and re-fetches the table.
    fn toggle_sort_order(&mut self) {
        let order = match self.db_display.sort_order {
            SortOrder::Ascending => SortOrder::Descending,
            SortOrder::Descending => SortOrder::Ascending,
        };
        self.logs.push(format!("[SORT] order → {order:?}"));
        self.db_display.sort_order = order;
        self.spawn_reload();
    }

    /// Moves to the next (`delta > 0`) or previous page, clamped to the valid
    /// page range.
    fn paginate(&mut self, delta: i32) {
        let total = self.db_display.total_pages();
        let new_page = if delta > 0 {
            (self.db_display.page + 1).min(total.saturating_sub(1))
        } else {
            self.db_display.page.saturating_sub(1)
        };
        if new_page != self.db_display.page {
            self.db_display.page = new_page;
            self.reset_selection();
        }
    }

    /// Moves the table selection by `delta`, wrapping at the ends of the
    /// current page.  Does nothing when the page is empty.
    fn move_selection(&mut self, delta: i32) {
        let len = self.db_display.window_len();
        if len == 0 {
            return;
        }
        let i = self
            .db_display
            .table_state
            .selected()
            .map(|i| {
                if delta > 0 {
                    if i >= len - 1 { 0 } else { i + 1 }
                } else if i == 0 {
                    len - 1
                } else {
                    i - 1
                }
            })
            .unwrap_or(0);
        self.db_display.table_state.select(Some(i));
    }

    /// Opens the stock-detail modal for the selected row and starts fetching
    /// its analyst data in the background.
    fn open_stock_modal(&mut self) {
        let Some(stock) = self
            .db_display
            .table_state
            .selected()
            .and_then(|i| self.db_display.window().get(i))
            .cloned()
        else {
            return;
        };
        let ticker = stock.ticker.clone();
        self.stock_modal.stock = stock;
        self.stock_modal.analysis = None;
        self.stock_modal.visible = true;
        if let Some(t) = ticker.as_deref() {
            self.spawn_analysis(t);
        }
    }

    /// Updates `page_size` to `size` and shifts the visible page so the
    /// globally-selected row stays on screen.  Called every frame from the
    /// draw function so that terminal resizes are reflected immediately.
    pub fn set_page_size(&mut self, size: usize) {
        let size = size.max(1);
        if self.db_display.page_size == size {
            return;
        }
        // Remember which global row index is currently selected so we can
        // keep it visible after the page boundaries shift.
        let global_idx = self
            .db_display
            .table_state
            .selected()
            .map(|i| self.db_display.page * self.db_display.page_size + i);

        self.db_display.page_size = size;

        // Jump to the page that contains the previously-selected row.
        let target_page = global_idx.map(|gi| gi / size).unwrap_or(0);
        self.db_display.page = target_page.min(self.db_display.total_pages().saturating_sub(1));

        // Restore the local selection inside the new window.
        let win_len = self.db_display.window_len();
        if win_len == 0 {
            self.db_display.table_state.select(None);
        } else {
            let local = global_idx.map(|gi| gi % size).unwrap_or(0).min(win_len - 1);
            self.db_display.table_state.select(Some(local));
        }
    }

    /// Selects row 0 in the current window, or clears selection if empty.
    fn reset_selection(&mut self) {
        if self.db_display.window_len() == 0 {
            self.db_display.table_state.select(None);
        } else {
            self.db_display.table_state.select(Some(0));
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
            match fetch_sorted(&pool, mode, order, PAGE_FETCH_LIMIT).await {
                Ok(rows) => {
                    let _ = tx.send(AppEvent::PageLoaded(rows));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::Error(format!("Failed to fetch quotes: {e}")));
                }
            }
        });
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

    fn n_records(n: usize) -> Vec<QuoteRecord> {
        (0..n).map(|_| QuoteRecord::default()).collect()
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
        app.handle_event(AppEvent::PageLoaded(n_records(3)));
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
        app.handle_event(AppEvent::PageLoaded(n_records(3)));
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

    #[tokio::test]
    async fn test_window_follows_page() {
        let (mut app, _rx) = make_test_app();
        // Default page_size is 25, so 30 rows span two pages: 25 + 5.
        app.handle_event(AppEvent::PageLoaded(n_records(30)));
        assert_eq!(app.db_display.total_pages(), 2);
        assert_eq!(app.db_display.window_len(), 25);
        app.handle_command_key('l');
        assert_eq!(app.db_display.page, 1);
        assert_eq!(app.db_display.window_len(), 5);
    }

    #[tokio::test]
    async fn test_paginate_clamps_at_bounds() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(n_records(30)));
        app.handle_command_key('h'); // already on first page
        assert_eq!(app.db_display.page, 0);
        app.handle_command_key('l');
        app.handle_command_key('l'); // already on last page
        assert_eq!(app.db_display.page, 1);
    }
}

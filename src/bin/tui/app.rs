//! Application state and event handling for the TUI binary.
//!
//! The event loop in `main.rs` owns the terminal and forwards raw key events
//! to [`App::handle_key`].  Async tasks report back through [`AppEvent`]
//! variants which [`App::handle_event`] applies to state;
//! [`draw`](super::draw::draw) then renders the current state on the next
//! frame.

use std::collections::{HashMap, VecDeque};
use std::ops::Range;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use ratatui::widgets::TableState;
use ratatui_notifications::{
    Anchor, Animation, AutoDismiss, Level, Notification, Notifications, SlideDirection,
};
use tokio::sync::mpsc;

use super::theme::{Appearance, Theme};
use yfinance::{
    cli::parse_tickers,
    models::{FLASH_TTL, QuoteRecord, QuoteRecordAnalysis, QuoteTick},
    sort::{SortMode, SortOrder},
};
use yfinance_rs::{Candle, StreamHandle, YfClient};

/// Async task spawners (`spawn_*`, `resubscribe_stream`) live in a submodule to
/// keep this file focused on synchronous state, input, and event handling.
mod tasks;

/// Maximum number of rows fetched from the database per reload.
const PAGE_FETCH_LIMIT: i64 = 200;

/// Interval at which the input-box cursor toggles visibility.
const BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// Maximum number of lines retained in the log panel; older lines are
/// dropped so a long session doesn't grow memory without bound.
const MAX_LOG_LINES: usize = 500;

/// Rows visible in the quotes table for a terminal of `terminal_height` rows.
///
/// Overhead: 1 status bar + 2 table borders + 1 header row + 1 header
/// bottom-margin = 5 rows.  Kept next to [`App::set_page_size`] so the event
/// loop (which sizes the page before each draw) and the draw layout can't
/// drift apart.
pub fn table_page_size(terminal_height: u16) -> usize {
    terminal_height.saturating_sub(5).max(1) as usize
}

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
    /// Stock price history loaded for the selected stock
    ChartDataReady(Vec<Candle>),
    /// Informational message for the log panel.
    LogLine(String),
    /// A quote update is returned from the stream.
    QuoteTick(QuoteTick),
    /// A stream was started for the current window of tickers
    StreamStarted(StreamHandle),
    /// The macOS system appearance changed; swap palettes.
    ThemeChanged(Appearance),
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
    /// Lines shown in the log panel (most recent at the bottom), capped at
    /// [`MAX_LOG_LINES`].
    pub logs: VecDeque<String>,
    /// Shared database connection pool passed to async tasks.
    pub pool: sqlx::PgPool,
    /// Shared Yahoo Finance client (and its HTTP connection pool), cloned
    /// into every async fetch/stream task.
    pub client: YfClient,
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
    /// The system appearance the current [`theme`](Self::theme) was derived
    /// from; seeds the watcher and is updated on each `ThemeChanged`.
    pub appearance: Appearance,
    /// Colour palette used by every draw call.
    pub theme: Theme,
    /// Handler for the live quotes stream
    pub stream_handle: Option<StreamHandle>,
    /// Subscribed symbols - starts as an empty vec, fills when subscribed
    pub subscribed_symbols: Vec<String>,
    /// Per-ticker price-flash state (signed price delta + timestamp), keyed by
    /// uppercase ticker. Written on each `QuoteTick`; read by the draw layer.
    /// Expired entries are evicted opportunistically in [`QuoteTick::record_flash`].
    pub row_flash_map: HashMap<String, (f64, Instant)>,
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

    /// Returns a vector of ticker symbols for the visible rows in a window
    pub fn window_tickers(&self) -> Vec<String> {
        let mut syms: Vec<String> = self
            .window()
            .iter()
            .filter_map(|r| r.ticker.as_deref())
            .map(str::to_ascii_uppercase)
            .collect();
        // Normalize
        syms.sort();
        syms.dedup();
        syms
    }
}

/// State for the stock-detail overlay modal.
pub struct StockInfoModal {
    /// The quote row that was selected when `?` was pressed.
    pub stock: QuoteRecord,
    /// Analyst data fetched in the background; `None` while loading.
    pub analysis: Option<QuoteRecordAnalysis>,
    /// Whether the info modal is currently visible.
    pub info_visible: bool,
    /// Whether the chart modal is visible
    pub chart_visible: bool,
    /// Stock price history fetched in the bg, None while loading
    pub chart_data: Option<Vec<Candle>>,
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
    // ---- Construction & lifecycle ----

    /// Creates a new `App` with sensible defaults.  The table starts sorted by
    /// `id DESC` to show the most recently stored quotes first.
    pub fn new(
        pool: sqlx::PgPool,
        client: YfClient,
        event_tx: mpsc::UnboundedSender<AppEvent>,
    ) -> Self {
        let appearance = super::theme::detect_appearance();
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
            logs: VecDeque::new(),
            pool,
            client,
            event_tx,
            blink_state: true,
            last_blink: Instant::now(),
            stock_modal: StockInfoModal {
                stock: QuoteRecord::default(),
                analysis: None,
                info_visible: false,
                chart_visible: false,
                chart_data: None,
            },
            notifications: Notifications::new(),
            appearance,
            theme: Theme::for_appearance(appearance),
            stream_handle: None,
            subscribed_symbols: Vec::new(),
            row_flash_map: HashMap::new(),
        }
    }

    /// Advances time-based state: toggles the cursor blink when its interval
    /// elapses and ticks the notification animations.
    ///
    /// Returns `true` when something visible changed (a blink toggle while
    /// the input box is focused, an active notification animation, or a live
    /// price flash that needs to expire), so the event loop can skip redraws
    /// on idle ticks.
    pub fn tick(&mut self, elapsed: Duration) -> bool {
        let now = Instant::now();
        let mut blinked = false;
        if now.duration_since(self.last_blink) >= BLINK_INTERVAL {
            self.blink_state = !self.blink_state;
            self.last_blink = now;
            blinked = true;
        }
        self.notifications.tick(elapsed);

        // Evict expired price flashes here (not just on the next tick's
        // `record_flash`) so `had_flashes` forces one final redraw that
        // repaints the rows in their normal colours.
        let had_flashes = !self.row_flash_map.is_empty();
        self.row_flash_map
            .retain(|_, (_, ts)| ts.elapsed() < FLASH_TTL);

        (blinked && self.input_mode.toggled) || self.notifications.has_notification() || had_flashes
    }

    /// Appends a line to the log panel, dropping the oldest line once the
    /// [`MAX_LOG_LINES`] cap is reached.
    fn push_log(&mut self, line: String) {
        if self.logs.len() >= MAX_LOG_LINES {
            self.logs.pop_front();
        }
        self.logs.push_back(line);
    }

    // ---- Event handling ----

    /// Applies an [`AppEvent`] to the application state.
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::PageLoaded(rows) => {
                self.db_display.status = String::new();
                self.db_display.rows = rows;
                self.db_display.page = 0;
                self.reset_selection();
                self.resubscribe_stream();
            }
            AppEvent::FetchSpawned(symbol) => {
                self.db_display.status = format!("fetching {symbol}…");
                self.push_log(format!("[INFO] fetching {symbol}"));
            }
            AppEvent::FetchCompleted(record) => {
                let name = record.ticker.as_deref().unwrap_or("?");
                self.db_display.status = format!("stored {name}");
                self.push_log(format!("[SUCCESS] stored {name}"));
                self.db_display.rows.insert(0, record);
                self.reset_selection();
            }
            AppEvent::LogLine(line) => {
                self.push_log(line);
            }
            AppEvent::Error(e) => self.push_error(e),
            AppEvent::StockAnalysisReady(analysis) => {
                self.stock_modal.analysis = Some(analysis);
            }
            AppEvent::ChartDataReady(data) => {
                self.stock_modal.chart_data = Some(data);
            }
            AppEvent::QuoteTick(tick) => {
                // Applies to the visible window only (the stream subscribes to
                // the visible tickers) and records the price flash from the
                // row's pre-update value in the same pass.  A tick for a
                // ticker not on screen is a silent no-op.
                let range = self.db_display.window_range();
                tick.apply(&mut self.db_display.rows[range], &mut self.row_flash_map);
            }
            AppEvent::StreamStarted(handle) => {
                self.stream_handle = Some(handle);
            }
            AppEvent::ThemeChanged(appearance) => {
                self.appearance = appearance;
                self.theme = Theme::for_appearance(appearance);
            }
        }
    }

    /// Records an error in the status bar and log panel, and raises a
    /// slide-in notification.
    fn push_error(&mut self, e: String) {
        self.db_display.status = format!("Error: {e}");
        self.push_log(format!("[ERROR] {e}"));
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

    // ---- Input & key handling ----

    /// Routes a raw terminal key press to the appropriate state change.
    ///
    /// While the input box is focused, printable characters edit the input
    /// text; otherwise they are treated as command keys.
    pub fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                if self.stock_modal.info_visible || self.stock_modal.chart_visible {
                    self.stock_modal.info_visible = false;
                    self.stock_modal.chart_visible = false;
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
            KeyCode::Enter => self.open_chart_modal(),
            _ => {}
        }
    }

    /// Routes a single command key to the appropriate state change.
    ///
    /// Only called when the input box is not focused; while focused, characters
    /// are appended to [`InputMode::input`] instead.
    pub fn handle_command_key(&mut self, key: char) {
        if self.stock_modal.info_visible || self.stock_modal.chart_visible {
            // Disable all command keys while a modal is open
            // Unless we want modal-specific commands eventually
            // For now, '?' is allowed to switch to the info modal from the chart modal, and vice
            // versa
            if key == '?' {
                self.open_info_modal()
            }
            return;
        }
        let key = key.to_ascii_lowercase();
        // Return early if the key corresponds to a sort order
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
            '?' => self.open_info_modal(),
            _ => {}
        }
    }

    /// Parses the input-box text as comma-separated ticker symbols (same
    /// normalisation as the CLI) and spawns a fetch for each.  Empty input is
    /// ignored.
    fn submit_input(&mut self) {
        let symbols = parse_tickers(&self.input_mode.input);
        self.input_mode.input.clear();
        for symbol in symbols {
            self.spawn_fetch(symbol);
        }
    }

    // ---- Navigation, sort & selection ----

    /// Sets the sort column and re-fetches the table.
    fn set_sort_mode(&mut self, mode: SortMode) {
        self.push_log(format!("[SORT] mode → {mode:?}"));
        self.db_display.sort_mode = mode;
        self.spawn_reload();
    }

    /// Flips the sort direction and re-fetches the table.
    fn toggle_sort_order(&mut self) {
        let order = match self.db_display.sort_order {
            SortOrder::Ascending => SortOrder::Descending,
            SortOrder::Descending => SortOrder::Ascending,
        };
        self.push_log(format!("[SORT] order → {order:?}"));
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
            self.resubscribe_stream();
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
        self.resubscribe_stream();
    }

    /// Selects row 0 in the current window, or clears selection if empty.
    fn reset_selection(&mut self) {
        if self.db_display.window_len() == 0 {
            self.db_display.table_state.select(None);
        } else {
            self.db_display.table_state.select(Some(0));
        }
    }

    // ---- Modals ----

    /// Opens the stock-detail modal for the selected row and starts fetching
    /// its analyst data in the background.
    fn open_info_modal(&mut self) {
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
        self.stock_modal.chart_visible = false;
        self.stock_modal.info_visible = true;

        let has_analysis = self
            .stock_modal
            .analysis
            .as_ref()
            .is_some_and(|a| a.ticker == ticker);

        if !has_analysis && let Some(t) = ticker.as_deref() {
            self.stock_modal.analysis = None;
            self.spawn_analysis(t);
        }
    }

    /// Opens the stock-chart modal for the selected row and starts fetching
    /// its pricing data in the background.
    fn open_chart_modal(&mut self) {
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
        // Keep any cached analysis: the chart modal never reads it, and
        // `open_info_modal` reuses it (guarded by a ticker match) so returning
        // to the info modal doesn't trigger a redundant refetch.
        self.stock_modal.info_visible = false;
        self.stock_modal.chart_visible = true;
        if let Some(t) = ticker.as_deref() {
            self.spawn_chart_data(t);
        }
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
        (App::new(pool, YfClient::default(), tx), rx)
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
        assert!(!app.stock_modal.info_visible);
        assert!(!app.stock_modal.chart_visible);
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
        assert_eq!(app.logs.back(), Some(&"hello".to_string()));
    }

    #[tokio::test]
    async fn test_handle_event_error() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::Error("oops".to_string()));
        assert!(app.db_display.status.contains("oops"));
        assert!(app.logs.iter().any(|l| l.contains("oops")));
    }

    fn analysis_for(ticker: &str) -> QuoteRecordAnalysis {
        QuoteRecordAnalysis {
            ticker: Some(ticker.to_string()),
            recommendation_summary: None,
            price_target: None,
        }
    }

    #[tokio::test]
    async fn test_open_info_reuses_matching_analysis() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        // Analysis already on hand for the selected ticker.
        app.stock_modal.analysis = Some(analysis_for("AAPL"));
        app.open_info_modal();
        // Should NOT be wiped/refetched.
        assert!(app.stock_modal.analysis.is_some());
    }

    #[tokio::test]
    async fn test_analysis_survives_chart_visit_for_same_ticker() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.analysis = Some(analysis_for("AAPL"));
        // Visit the chart modal for the same ticker, then return to info.
        app.open_chart_modal();
        app.open_info_modal();
        // The cached analysis should still be reused, not refetched.
        assert!(app.stock_modal.analysis.is_some());
    }

    #[tokio::test]
    async fn test_open_info_refetches_on_ticker_mismatch() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        // Stale analysis for a different ticker.
        app.stock_modal.analysis = Some(analysis_for("TSLA"));
        app.open_info_modal();
        assert!(app.stock_modal.analysis.is_none());
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

//! Application state and event handling for the TUI binary.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ops::Range;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::TableState;
use ratatui_notifications::{
    Anchor, Animation, AutoDismiss, Level, Notification, Notifications, SlideDirection,
};
use tokio::sync::mpsc;

use super::theme::{Appearance, Theme};
use crate::{
    cli::parse_tickers,
    models::{
        Alert, FLASH_TTL, OptionChain, QuoteRecord, QuoteRecordAnalysis,
        QuoteRecordStatistics, QuoteTick, StockNews, Ticker, crosses, parse_alert_entry,
    },
    portfolio::{Holding, parse_holding_entry},
    sort::{SortMode, SortOrder},
};
use yfinance_rs::{Candle, StreamHandle, YfClient};

mod tasks;

/// Maximum number of rows fetched from the database per reload.
const PAGE_FETCH_LIMIT: i64 = 200;

/// Interval at which the input-box cursor toggles visibility.
const BLINK_INTERVAL: Duration = Duration::from_millis(500);

/// How often the Market dashboard cubes re-snapshot their data.
const DASHBOARD_REFRESH: Duration = Duration::from_secs(3);

/// How many portfolio-value samples the dashboard sparkline retains.
const PORTFOLIO_SERIES_CAP: usize = 120;

/// Maximum number of lines retained in the log panel.
const MAX_LOG_LINES: usize = 500;

const TOAST_ANCHOR: Anchor = Anchor::TopRight;
const TOAST_ANIMATION: Animation = Animation::Slide;
const TOAST_SLIDE_DIRECTION: SlideDirection = SlideDirection::FromRight;
const TOAST_DISMISS_AFTER: Duration = Duration::from_secs(5);

/// The number of equity sectors the treemap fetches (the full `EquitySector` set).
const SECTOR_COUNT: usize = 11;

pub fn table_page_size(terminal_height: u16) -> usize {
    terminal_height.saturating_sub(5).max(1) as usize
}

/// Messages sent from async tasks back to the main event loop.
pub enum AppEvent {
    /// Initial page load or a re-fetch triggered by a sort change.
    PageLoaded(Vec<QuoteRecord>),
    FetchSpawned(String),
    /// A quote was fetched and stored.
    FetchCompleted(QuoteRecord),
    /// The watchlist returned successfully from db.
    WatchlistLoaded(Vec<String>),
    /// Analyst consensus and price targets loaded for the selected stock.
    StockAnalysisReady(QuoteRecordAnalysis),
    /// Key statistics (valuation, dividend, risk metrics) loaded for the selected stock.
    StockStatisticsReady(QuoteRecordStatistics),
    /// Recent news headlines loaded for the selected stock.
    StockNewsReady(StockNews),
    /// Stock price history loaded for the selected stock.
    ChartDataReady(Vec<Candle>),
    /// Options chain (expirations + contracts) loaded for the selected stock.
    OptionChainReady(OptionChain),
    /// Active price alerts loaded from the database at startup.
    AlertsLoaded(Vec<Alert>),
    /// Portfolio holdings loaded from the database at startup.
    PortfolioLoaded(Vec<Holding>),
    /// Informational message for the log panel.
    LogLine(String),
    /// A quote update is returned from the stream.
    QuoteTick(QuoteTick),
    /// A stream was started for the current window of tickers.
    StreamStarted(StreamHandle),
    /// The macOS system appearance changed.
    ThemeChanged(Appearance),
    /// One sector's screener results, mapped to treemap tiles.
    // TODO(sectors-render): drop this #[allow] once the Sectors page wires up
    #[allow(dead_code)]
    SectorDataReady {
        /// Which sector these tiles belong to.
        sector: yfinance_rs::screener::EquitySector,
        /// The sector's tiles (already sorted/truncated).
        tiles: Vec<crate::sectors::SectorTile>,
    },
    Error(String),
}

/// Why a quote fetch was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchOrigin {
    /// A ticker the user explicitly asked to fetch.
    UserRequest,
    /// A background top-up of a held ticker's price.
    HoldingPrice,
}

fn fetch_failure_event(origin: FetchOrigin, symbol: &str, err: &str) -> AppEvent {
    match origin {
        FetchOrigin::UserRequest => AppEvent::Error(format!("{symbol}: {err}")),
        FetchOrigin::HoldingPrice => {
            AppEvent::LogLine(format!("[WARN] price for {symbol} unavailable: {err}"))
        }
    }
}

/// The top-level page currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    /// The quotes table, input box, logs, and detail modals.
    #[default]
    Market,
    /// User-owned data management: holdings and alerts.
    Manage,
    Sectors,
}

/// The single overlay currently on top of the page, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overlay {
    /// No overlay.
    #[default]
    None,
    /// The stock-detail info modal (analyst data, statistics, news).
    Info,
    /// The price-history chart modal.
    Chart,
    /// The options-chain modal.
    Options,
    /// The keybinding help modal.
    Help,
}

/// The interaction mode reflected in the status bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusMode {
    /// The help overlay is open.
    Help,
    /// The Manage page's inline entry line is being typed.
    ManageInput,
    /// The Manage page is focused with no inline entry active.
    Manage,
    /// The Sectors page is focused with no modal open.
    Sectors,
    /// The options-chain modal is open.
    Options,
    /// The info or chart modal is open.
    View,
    /// The fuzzy filter is focused for typing.
    Search,
    /// The ticker input box is focused.
    Input,
    /// A fuzzy filter is active but not focused.
    Filter,
    /// The default Market browsing mode.
    Normal,
}

impl StatusMode {
    /// The short chip label shown at the left of the status bar.
    pub fn label(&self) -> &'static str {
        match self {
            StatusMode::Help => "HELP",
            StatusMode::ManageInput | StatusMode::Manage => "MANAGE",
            StatusMode::Sectors => "SECTORS",
            StatusMode::Options | StatusMode::View => "VIEW",
            StatusMode::Search => "SEARCH",
            StatusMode::Input => "INPUT",
            StatusMode::Filter => "FILTER",
            StatusMode::Normal => "NORMAL",
        }
    }
}

/// Which section of the Manage page currently holds the `j`/`k` cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ManageSection {
    #[default]
    Holdings,
    /// The active-alerts list.
    Alerts,
}

impl ManageSection {
    /// Number of rows this section renders, read from its backing list.
    fn item_count(&self, app: &App) -> usize {
        match self {
            ManageSection::Holdings => app.holdings.len(),
            ManageSection::Alerts => app.alerts.len(),
        }
    }

    /// The inline-input kind used when composing an entry for this section.
    fn input_kind(&self) -> ManageInputKind {
        match self {
            ManageSection::Holdings => ManageInputKind::Holding,
            ManageSection::Alerts => ManageInputKind::Alert,
        }
    }
}

/// Which kind of entry the Manage-page inline input is composing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManageInputKind {
    /// A `TICKER SHARES COST` holding entry.
    Holding,
    /// A `TICKER >150` / `TICKER <100` alert entry.
    Alert,
}

/// The Manage page's inline entry line, shown when adding or editing.
pub struct ManageInput {
    /// Whether the buffer is a holding or an alert spec.
    pub kind: ManageInputKind,
    /// The text typed so far.
    pub buffer: String,
}

/// Market-page state: the ticker input box, the quotes table, and the log panel.
pub struct MarketState {
    /// Ticker input box state (current text and whether the box is focused).
    pub input_mode: InputMode,
    /// The quotes table and its associated sort/selection state.
    pub db_display: DbDisplay,
    /// Lines shown in the log panel (most recent at the bottom), capped at [`MAX_LOG_LINES`].
    pub logs: VecDeque<String>,
    /// Throttled snapshot the dashboard cubes render from, so they don't jitter on every quote tick.
    pub dashboard: DashboardSnapshot,
    /// Time accumulated since [`Self::dashboard`] last refreshed.
    pub dashboard_age: Duration,
    pub portfolio_value_series: VecDeque<f64>,
}

/// A point-in-time copy of the inputs the Market dashboard cubes render from.
#[derive(Default)]
pub struct DashboardSnapshot {
    pub rows: Vec<QuoteRecord>,
    /// Holdings as of the last refresh.
    pub holdings: Vec<Holding>,
    /// Active alerts as of the last refresh.
    pub alerts: Vec<Alert>,
    /// Whether the live quote stream was running at the last refresh.
    pub stream_live: bool,
    /// Number of streamed symbols at the last refresh.
    pub tracked: usize,
}

pub struct ManageState {
    /// Which Manage-page section the `j`/`k` cursor is in.
    pub section: ManageSection,
    /// Selected row index within the focused Manage section.
    pub selected: usize,
    /// The Manage page's inline add/edit line, when active.
    pub input: Option<ManageInput>,
}

pub struct SectorState {
    /// The treemap, filled in progressively as each sector's screener returns.
    pub map: crate::sectors::SectorMap,
    /// True while at least one sector query is outstanding.
    pub loading: bool,
    /// Selected tile index into the flattened tile order (see `draw_sectors_page`).
    pub selected: usize,
    /// Set once the first fetch is kicked off, so page entry doesn't refetch every frame.
    pub requested: bool,
    /// The body area `draw_sectors_page` last laid the treemap into.
    pub last_area: Rect,
}

impl Default for SectorState {
    fn default() -> Self {
        Self {
            map: crate::sectors::SectorMap::default(),
            loading: false,
            selected: 0,
            requested: false,
            last_area: Rect::new(0, 0, 80, 24),
        }
    }
}

/// Top-level application state.
pub struct App {
    /// Market-page state: input box, quotes table, and log panel.
    pub market: MarketState,
    /// Set to `true` when the user presses `q`.
    pub should_quit: bool,
    /// Shared handles (pool, client, event channel) cloned into async tasks.
    pub services: Services,
    /// Transient animation state advanced by [`App::tick`].
    pub animations: Animations,
    /// Overlay modal showing detailed info for the selected stock.
    pub stock_modal: StockInfoModal,
    /// The single overlay currently on top of the page, if any.
    pub overlay: Overlay,
    /// Slide-in error notifications shown top-right.
    pub notifications: Notifications,
    /// The system appearance the current [`theme`](Self::theme) was derived from.
    pub appearance: Appearance,
    /// Colour palette used by every draw call.
    pub theme: Theme,
    /// Handler for the live quotes stream.
    pub stream_handle: Option<StreamHandle>,
    /// Subscribed symbols - starts as an empty vec, fills when subscribed.
    pub subscribed_symbols: Vec<String>,
    /// Active price alerts (`triggered_at IS NULL`), the source of truth for tick evaluation and rendering.
    pub alerts: Vec<Alert>,
    pub holdings: Vec<Holding>,
    /// The top-level page currently displayed.
    pub page: Page,
    /// Manage-page state: section focus, selected row, and inline add/edit line.
    pub manage: ManageState,
    /// Sectors-page state: the treemap and its selection.
    pub sectors: SectorState,
    pub show_logs: bool,
}

/// Shared handles cloned into every async fetch/stream task.
pub struct Services {
    /// Shared database connection pool passed to async tasks.
    pub pool: sqlx::PgPool,
    pub client: YfClient,
    /// Channel used by async tasks to push [`AppEvent`]s back to the loop.
    pub event_tx: mpsc::UnboundedSender<AppEvent>,
}

/// Transient animation state advanced by [`App::tick`].
pub struct Animations {
    /// Current state of the blinking cursor in the input box.
    pub blink_state: bool,
    /// Timestamp of the last blink toggle, used to drive the blink interval.
    pub last_blink: Instant,
    /// Per-ticker price-flash state (signed price delta + timestamp), keyed by canonical [`Ticker`].
    pub row_flash_map: HashMap<Ticker, (f64, Instant)>,
}

/// What the input box is currently doing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum InputActivity {
    /// Box unfocused.
    #[default]
    Idle,
    /// Typing comma-separated tickers to fetch.
    Ticker,
    /// Typing a fuzzy-filter query, box focused.
    Fuzzy,
    FuzzyApplied,
}

/// State for the ticker input box: the typed text plus what it currently means.
pub struct InputMode {
    /// Text the user has typed so far.
    pub input: String,
    /// The box's current activity.
    pub activity: InputActivity,
}

impl InputMode {
    /// Whether the box is focused (printable keys edit [`Self::input`]).
    pub fn is_focused(&self) -> bool {
        matches!(self.activity, InputActivity::Ticker | InputActivity::Fuzzy)
    }

    /// Whether a fuzzy filter is active (being typed or already applied).
    pub fn is_fuzzy(&self) -> bool {
        matches!(self.activity, InputActivity::Fuzzy | InputActivity::FuzzyApplied)
    }
}

/// State for the quotes table on the right side of the layout.
pub struct DbDisplay {
    pub rows: Vec<QuoteRecord>,
    /// Master list of every row from the last database load, unaffected by filtering.
    pub all_rows: Vec<QuoteRecord>,
    /// Watchlist hashset of tracked tickers.
    pub watchlist: HashSet<String>,
    pub watchlist_only: bool,
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

    /// Returns a vector of ticker symbols for the visible rows in a window.
    pub fn window_tickers(&self) -> Vec<String> {
        let mut syms: Vec<String> = self
            .window()
            .iter()
            .filter_map(|r| r.ticker.as_deref())
            .map(str::to_ascii_uppercase)
            .collect();
        syms.sort();
        syms.dedup();
        syms
    }

    fn on_watchlist(&self, row: &QuoteRecord) -> bool {
        if let Some(ticker) = row.ticker.as_deref() {
            self.watchlist.contains(&ticker.to_ascii_uppercase())
        } else {
            false
        }
    }
}

/// State for the stock-detail overlay modals.
pub struct StockInfoModal {
    /// The quote row that was selected when `?` was pressed.
    pub stock: QuoteRecord,
    /// Analyst data fetched in the background.
    pub analysis: Option<QuoteRecordAnalysis>,
    /// Key statistics fetched in the background.
    pub statistics: Option<QuoteRecordStatistics>,
    /// Recent news headlines fetched in the background.
    pub news: Option<StockNews>,
    /// Stock price history fetched in the bg, None while loading.
    pub chart_data: Option<Vec<Candle>>,
    pub option_chain: Option<OptionChain>,
    /// Index into the chain's `expirations` currently shown in the modal.
    pub selected_expiry: usize,
}

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
    /// Creates a new `App` with sensible defaults.
    pub fn new(
        pool: sqlx::PgPool,
        client: YfClient,
        event_tx: mpsc::UnboundedSender<AppEvent>,
    ) -> Self {
        let appearance = super::theme::detect_appearance();
        Self {
            market: MarketState {
                input_mode: InputMode {
                    input: String::new(),
                    activity: InputActivity::Idle,
                },
                db_display: DbDisplay {
                    rows: Vec::new(),
                    all_rows: Vec::new(),
                    status: String::from("Loading..."),
                    table_state: TableState::default(),
                    watchlist: HashSet::new(),
                    watchlist_only: false,
                    sort_mode: SortMode::ById,
                    sort_order: SortOrder::Descending,
                    page: 0,
                    page_size: 25,
                },
                logs: VecDeque::new(),
                dashboard: DashboardSnapshot::default(),
                dashboard_age: DASHBOARD_REFRESH,
                portfolio_value_series: VecDeque::new(),
            },
            should_quit: false,
            services: Services {
                pool,
                client,
                event_tx,
            },
            animations: Animations {
                blink_state: true,
                last_blink: Instant::now(),
                row_flash_map: HashMap::new(),
            },
            stock_modal: StockInfoModal {
                stock: QuoteRecord::default(),
                analysis: None,
                statistics: None,
                news: None,
                chart_data: None,
                option_chain: None,
                selected_expiry: 0,
            },
            overlay: Overlay::None,
            notifications: Notifications::new(),
            appearance,
            theme: Theme::for_appearance(appearance),
            stream_handle: None,
            subscribed_symbols: Vec::new(),
            alerts: Vec::new(),
            holdings: Vec::new(),
            page: Page::Market,
            manage: ManageState {
                section: ManageSection::Holdings,
                selected: 0,
                input: None,
            },
            sectors: SectorState::default(),
            show_logs: false,
        }
    }

    pub fn tick(&mut self, elapsed: Duration) -> bool {
        let now = Instant::now();
        let mut blinked = false;
        if now.duration_since(self.animations.last_blink) >= BLINK_INTERVAL {
            self.animations.blink_state = !self.animations.blink_state;
            self.animations.last_blink = now;
            blinked = true;
        }
        self.notifications.tick(elapsed);

        let had_flashes = !self.animations.row_flash_map.is_empty();
        self.animations
            .row_flash_map
            .retain(|_, (_, ts)| ts.elapsed() < FLASH_TTL);

        self.market.dashboard_age += elapsed;
        let dashboard_refreshed = self.market.dashboard_age >= DASHBOARD_REFRESH;
        if dashboard_refreshed {
            self.refresh_dashboard();
            self.market.dashboard_age = Duration::ZERO;
        }

        (blinked && self.market.input_mode.is_focused())
            || self.notifications.has_notification()
            || had_flashes
            || (dashboard_refreshed && self.page == Page::Market && !self.show_logs)
    }

    /// Snapshots the current dashboard inputs into [`MarketState::dashboard`].
    fn refresh_dashboard(&mut self) {
        let snapshot = DashboardSnapshot {
            rows: self.market.db_display.all_rows.clone(),
            holdings: self.holdings.clone(),
            alerts: self.alerts.clone(),
            stream_live: self.stream_handle.is_some(),
            tracked: self.subscribed_symbols.len(),
        };
        self.market.dashboard = snapshot;
        self.sample_portfolio_value();
    }

    fn sample_portfolio_value(&mut self) {
        let positions = crate::portfolio::portfolio_positions(
            &self.holdings,
            &self.market.db_display.all_rows,
        );
        if !positions.iter().any(|p| p.market_value.is_some()) {
            return;
        }
        let value = crate::portfolio::summarize(&positions).total_market_value;
        let series = &mut self.market.portfolio_value_series;
        series.push_back(value);
        while series.len() > PORTFOLIO_SERIES_CAP {
            series.pop_front();
        }
    }

    pub fn status_mode(&self) -> (StatusMode, &'static str) {
        if self.overlay == Overlay::Help {
            (StatusMode::Help, "esc close")
        } else if self.manage.input.is_some() {
            (
                StatusMode::ManageInput,
                "type entry · enter save · esc cancel",
            )
        } else if self.page == Page::Manage {
            (
                StatusMode::Manage,
                "j/k move · a add · e edit · d delete · tab market · q quit",
            )
        } else if self.overlay == Overlay::Options {
            (StatusMode::Options, "h/l expiry · esc close")
        } else if self.overlay == Overlay::Info || self.overlay == Overlay::Chart {
            (StatusMode::View, "s info · enter chart · esc close")
        } else if self.page == Page::Sectors {
            (
                StatusMode::Sectors,
                "h/j/k/l move · enter chart · r refresh · tab cycle · q quit",
            )
        } else if self.market.input_mode.is_focused() && self.market.input_mode.is_fuzzy() {
            (StatusMode::Search, "type to filter · enter apply · esc cancel")
        } else if self.market.input_mode.is_focused() {
            (StatusMode::Input, "type ticker · enter fetch · esc done")
        } else if self.market.input_mode.is_fuzzy() {
            (
                StatusMode::Filter,
                "j/k move · s info · enter chart · w ★ · / new search · esc clear · ? help",
            )
        } else {
            (
                StatusMode::Normal,
                "j/k move · h/l page · s info · enter chart · x options · tab manage · w ★ · i query · / search · o order · ? help · q quit",
            )
        }
    }

    /// Appends a line to the log panel, dropping the oldest line once the [`MAX_LOG_LINES`] cap is reached.
    fn push_log(&mut self, line: String) {
        if self.market.logs.len() >= MAX_LOG_LINES {
            self.market.logs.pop_front();
        }
        self.market.logs.push_back(line);
    }

    /// Applies an [`AppEvent`] to the application state.
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::PageLoaded(rows) => {
                self.market.db_display.status = String::new();
                self.market.db_display.all_rows = rows;
                self.apply_filter();
                self.resubscribe_stream();
            }
            AppEvent::FetchSpawned(symbol) => {
                self.market.db_display.status = format!("fetching {symbol}…");
                self.push_log(format!("[INFO] fetching {symbol}"));
            }
            AppEvent::FetchCompleted(record) => {
                let name = record.ticker.as_deref().unwrap_or("?");
                self.market.db_display.status = format!("stored {name}");
                self.push_log(format!("[SUCCESS] stored {name}"));
                self.market.db_display.all_rows.insert(0, record);
                self.apply_filter();
            }
            AppEvent::LogLine(line) => {
                self.push_log(line);
            }
            AppEvent::WatchlistLoaded(tickers) => {
                self.market.db_display.watchlist = tickers.into_iter().collect();
            }
            AppEvent::AlertsLoaded(alerts) => {
                self.alerts = alerts;
            }
            AppEvent::PortfolioLoaded(holdings) => {
                self.holdings = holdings;
                self.fetch_unpriced_holdings();
                self.resubscribe_stream();
            }
            AppEvent::Error(e) => self.push_error(e),
            AppEvent::StockAnalysisReady(analysis) => {
                self.stock_modal.analysis = Some(analysis);
            }
            AppEvent::StockStatisticsReady(stats) => {
                self.stock_modal.statistics = Some(stats);
            }
            AppEvent::StockNewsReady(news) => {
                self.stock_modal.news = Some(news);
            }
            AppEvent::ChartDataReady(data) => {
                self.stock_modal.chart_data = Some(data);
            }
            AppEvent::OptionChainReady(mut chain) => {
                chain.underlying_price = self.stock_modal.stock.price;
                let last = chain.expirations.len().saturating_sub(1);
                self.stock_modal.selected_expiry = self.stock_modal.selected_expiry.min(last);
                self.stock_modal.option_chain = Some(chain);
            }
            AppEvent::QuoteTick(tick) => {
                self.materialize_held_ticker_row(&tick);
                tick.apply(&mut self.market.db_display.all_rows, &mut HashMap::new());
                let range = self.market.db_display.window_range();
                tick.apply(
                    &mut self.market.db_display.rows[range],
                    &mut self.animations.row_flash_map,
                );
                self.evaluate_alerts(&tick);
            }
            AppEvent::StreamStarted(handle) => {
                self.stream_handle = Some(handle);
            }
            AppEvent::ThemeChanged(appearance) => {
                self.appearance = appearance;
                self.theme = Theme::for_appearance(appearance);
            }
            AppEvent::SectorDataReady { sector, tiles } => {
                self.sectors.map.upsert(sector, tiles);
                if self.sectors.map.groups.len() >= SECTOR_COUNT {
                    self.sectors.loading = false;
                }
            }
        }
    }

    /// Builds a slide-in toast and mirrors it into the log panel.
    fn push_toast(&mut self, level: Level, title: &'static str, log_prefix: &str, msg: String) {
        self.push_log(format!("[{log_prefix}] {msg}"));
        if let Ok(notif) = Notification::new(msg)
            .title(title)
            .level(level)
            .anchor(TOAST_ANCHOR)
            .animation(TOAST_ANIMATION)
            .slide_direction(TOAST_SLIDE_DIRECTION)
            .auto_dismiss(AutoDismiss::After(TOAST_DISMISS_AFTER))
            .build()
        {
            let _ = self.notifications.add(notif);
        }
    }

    /// Records an error in the status bar and log panel, and raises a slide-in notification.
    fn push_error(&mut self, e: String) {
        self.market.db_display.status = format!("Error: {e}");
        self.push_toast(Level::Error, "Error", "ERROR", e);
    }

    fn push_alert_toast(&mut self, msg: String) {
        self.push_toast(Level::Warn, "Alert", "ALERT", msg);
    }

    /// Pushes a portfolio-action toast (an `Info`-level confirmation) and mirrors it into the log panel.
    fn push_portfolio_toast(&mut self, msg: String) {
        self.push_toast(Level::Info, "Portfolio", "PORTFOLIO", msg);
    }

    fn evaluate_alerts(&mut self, tick: &QuoteTick) {
        let (Some(ticker), Some(price)) = (tick.ticker.as_deref(), tick.price) else {
            return;
        };

        let mut fired: Vec<Alert> = Vec::new();
        self.alerts.retain(|a| {
            let hit = a.ticker.eq_ignore_ascii_case(ticker) && crosses(a.direction, a.threshold, price);
            if hit {
                fired.push(a.clone());
            }
            !hit
        });

        let now = chrono::Utc::now();
        for alert in fired {
            let dir = alert.direction.label();
            self.push_alert_toast(format!(
                "{} crossed {dir} {:.2} → {price:.2}",
                alert.ticker, alert.threshold
            ));
            self.spawn_mark_alert_triggered(alert.id, now);
        }
    }

    /// Routes a raw terminal key press, dispatching by the current page.
    pub fn handle_key(&mut self, code: KeyCode) {
        if matches!(code, KeyCode::Tab | KeyCode::BackTab) && self.manage.input.is_none() {
            match code {
                KeyCode::Tab => self.next_page(),
                _ => self.prev_page(),
            }
            return;
        }
        match self.page {
            Page::Market => self.handle_market_key(code),
            Page::Manage => self.handle_manage_key(code),
            Page::Sectors => self.handle_sectors_key(code),
        }
    }

    /// Advances the page forward: Market → Manage → Sectors → Market.
    fn next_page(&mut self) {
        self.page = match self.page {
            Page::Market => Page::Manage,
            Page::Manage => Page::Sectors,
            Page::Sectors => Page::Market,
        };
        self.on_page_entered();
    }

    /// Advances the page backward: the reverse of [`Self::next_page`].
    fn prev_page(&mut self) {
        self.page = match self.page {
            Page::Market => Page::Sectors,
            Page::Manage => Page::Market,
            Page::Sectors => Page::Manage,
        };
        self.on_page_entered();
    }

    /// Runs page-entry side effects.
    fn on_page_entered(&mut self) {
        if self.page == Page::Sectors && !self.sectors.requested {
            self.sectors.requested = true;
            self.sectors.loading = true;
            self.spawn_sector_map();
        }
    }

    fn handle_sectors_key(&mut self, code: KeyCode) {
        use super::draw::{Dir, flatten_tiles, nearest_in_direction};

        if self.overlay != Overlay::None {
            if code == KeyCode::Esc {
                self.overlay = Overlay::None;
            }
            return;
        }

        match code {
            KeyCode::Char('r') => {
                self.sectors.loading = true;
                self.spawn_sector_map();
            }
            KeyCode::Enter => self.open_sector_modal(),
            KeyCode::Char('h')
            | KeyCode::Left
            | KeyCode::Char('l')
            | KeyCode::Right
            | KeyCode::Char('j')
            | KeyCode::Down
            | KeyCode::Char('k')
            | KeyCode::Up => {
                let dir = match code {
                    KeyCode::Char('h') | KeyCode::Left => Dir::Left,
                    KeyCode::Char('l') | KeyCode::Right => Dir::Right,
                    KeyCode::Char('j') | KeyCode::Down => Dir::Down,
                    _ => Dir::Up,
                };
                let tiles = flatten_tiles(&self.sectors.map, self.sectors.last_area);
                self.sectors.selected = nearest_in_direction(&tiles, self.sectors.selected, dir);
            }
            _ => {}
        }
    }

    fn open_sector_modal(&mut self) {
        use super::draw::flatten_tiles;

        let tiles = flatten_tiles(&self.sectors.map, self.sectors.last_area);
        let Some((_, ticker)) = tiles.get(self.sectors.selected) else {
            return;
        };
        let ticker = ticker.clone();
        self.stock_modal.stock = QuoteRecord {
            ticker: Some(ticker.clone()),
            ..Default::default()
        };
        self.overlay = Overlay::Chart;
        self.spawn_chart_data(&ticker);
    }

    fn manage_move(&mut self, delta: isize) {
        match self.manage.section {
            ManageSection::Holdings => {
                if delta > 0 {
                    if self.manage.selected + 1 < self.manage.section.item_count(self) {
                        self.manage.selected += 1;
                    } else {
                        self.manage.section = ManageSection::Alerts;
                        self.manage.selected = 0;
                    }
                } else if self.manage.selected > 0 {
                    self.manage.selected -= 1;
                }
            }
            ManageSection::Alerts => {
                if delta > 0 {
                    if self.manage.selected + 1 < self.manage.section.item_count(self) {
                        self.manage.selected += 1;
                    }
                } else if self.manage.selected > 0 {
                    self.manage.selected -= 1;
                } else {
                    self.manage.section = ManageSection::Holdings;
                    self.manage.selected = ManageSection::Holdings.item_count(self).saturating_sub(1);
                }
            }
        }
    }

    fn handle_manage_key(&mut self, code: KeyCode) {
        if let Some(input) = self.manage.input.as_mut() {
            match code {
                KeyCode::Char(c) => input.buffer.push(c),
                KeyCode::Backspace => {
                    input.buffer.pop();
                }
                KeyCode::Enter => self.submit_manage_input(),
                KeyCode::Esc => self.manage.input = None,
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Esc => self.page = Page::Market,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.open_help_modal(),
            KeyCode::Char('j') | KeyCode::Down => self.manage_move(1),
            KeyCode::Char('k') | KeyCode::Up => self.manage_move(-1),
            KeyCode::Char('a') => self.start_manage_add(),
            KeyCode::Char('e') => self.start_manage_edit(),
            KeyCode::Char('d') => self.delete_selected_manage(),
            _ => {}
        }
    }

    /// Opens an inline add line for the focused section (empty buffer).
    fn start_manage_add(&mut self) {
        self.manage.input = Some(ManageInput {
            kind: self.manage.section.input_kind(),
            buffer: String::new(),
        });
    }

    /// Opens an inline edit line pre-filled from the selected holding (`TICKER SHARES COST`).
    fn start_manage_edit(&mut self) {
        if self.manage.section != ManageSection::Holdings {
            return;
        }
        let Some(h) = self.holdings.get(self.manage.selected) else {
            return;
        };
        self.manage.input = Some(ManageInput {
            kind: self.manage.section.input_kind(),
            buffer: format!("{} {} {}", h.ticker, h.shares, h.avg_cost),
        });
    }

    /// Cancels any active inline entry.
    fn cancel_manage_input(&mut self) {
        self.manage.input = None;
    }

    /// Parses and applies the inline entry.
    fn submit_manage_input(&mut self) {
        let Some(input) = self.manage.input.as_ref() else {
            return;
        };
        match input.kind {
            ManageInputKind::Holding => match parse_holding_entry(&input.buffer) {
                Ok(holding) => {
                    self.upsert_holding_in_memory(holding.clone());
                    self.push_portfolio_toast(format!(
                        "Saved {}: {} @ ${:.2}",
                        holding.ticker, holding.shares, holding.avg_cost
                    ));
                    self.fetch_holding_price(&holding.ticker);
                    self.spawn_upsert_holding(holding);
                    self.resubscribe_stream();
                    self.cancel_manage_input();
                }
                Err(e) => self.push_error(format!("holding: {e}")),
            },
            ManageInputKind::Alert => match parse_alert_entry(&input.buffer) {
                Ok((ticker, direction, threshold)) => {
                    self.alerts.push(Alert {
                        id: 0,
                        ticker: ticker.clone(),
                        direction,
                        threshold,
                        created_at: chrono::Utc::now(),
                        triggered_at: None,
                    });
                    let dir = direction.label();
                    self.push_alert_toast(format!("Alert: {ticker} {dir} {threshold:.2}"));
                    self.spawn_add_alert(ticker, direction, threshold);
                    self.cancel_manage_input();
                }
                Err(e) => self.push_error(format!("alert: {e}")),
            },
        }
    }

    fn delete_selected_manage(&mut self) {
        match self.manage.section {
            ManageSection::Holdings => {
                if self.manage.selected < self.manage.section.item_count(self) {
                    let removed = self.holdings.remove(self.manage.selected);
                    self.push_portfolio_toast(format!("Removed {}", removed.ticker));
                    self.spawn_remove_holding(removed.ticker);
                    self.clamp_manage_selected();
                }
            }
            ManageSection::Alerts => {
                if self.manage.selected < self.manage.section.item_count(self) {
                    let removed = self.alerts.remove(self.manage.selected);
                    self.push_portfolio_toast(format!("Removed alert {}", removed.ticker));
                    self.spawn_remove_alert(removed.id);
                    self.clamp_manage_selected();
                }
            }
        }
    }

    /// Clamps `manage_selected` to the focused section's bounds after a removal.
    fn clamp_manage_selected(&mut self) {
        let len = self.manage.section.item_count(self);
        if self.manage.selected >= len {
            self.manage.selected = len.saturating_sub(1);
        }
    }

    fn handle_market_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                if self.overlay != Overlay::None {
                    self.overlay = Overlay::None;
                } else if self.market.input_mode.is_fuzzy() {
                    self.cancel_fuzzy_search();
                } else {
                    self.market.input_mode.activity = InputActivity::Idle;
                }
            }
            KeyCode::Char(c) if self.market.input_mode.is_focused() => {
                self.market.input_mode.input.push(c);
                if self.market.input_mode.is_fuzzy() {
                    self.apply_filter();
                }
            }
            KeyCode::Char(c) => self.handle_command_key(c),
            KeyCode::Backspace if self.market.input_mode.is_focused() => {
                self.market.input_mode.input.pop();
                if self.market.input_mode.is_fuzzy() {
                    self.apply_filter();
                }
            }
            KeyCode::Enter if self.market.input_mode.is_focused() => {
                if self.market.input_mode.is_fuzzy() {
                    self.accept_fuzzy_search();
                } else {
                    self.submit_input();
                }
            }
            KeyCode::Enter => self.open_chart_modal(),
            _ => {}
        }
    }

    /// Routes a single command key to the appropriate state change.
    pub fn handle_command_key(&mut self, key: char) {
        match self.overlay {
            Overlay::Help => return,
            Overlay::Options => {
                match key {
                    'h' => self.cycle_expiry(-1),
                    'l' => self.cycle_expiry(1),
                    _ => {}
                }
                return;
            }
            Overlay::Info | Overlay::Chart => {
                if key == 's' {
                    self.open_info_modal();
                }
                return;
            }
            Overlay::None => {}
        }
        if let Some(mode) = sort_mode_for_key(key) {
            self.set_sort_mode(mode);
            return;
        }
        match key {
            'q' => self.should_quit = true,
            'o' => self.toggle_sort_order(),
            's' => self.open_info_modal(),
            '?' => self.open_help_modal(),
            'w' => self.toggle_watchlist(),
            'x' => self.open_options_modal(),
            'L' => self.show_logs = !self.show_logs,
            'W' => {
                self.market.db_display.watchlist_only = !self.market.db_display.watchlist_only;
                self.apply_filter();
                self.resubscribe_stream();
            }
            'i' => {
                if self.market.input_mode.is_focused() {
                    self.market.input_mode.activity = InputActivity::Idle;
                } else {
                    if self.market.input_mode.is_fuzzy() {
                        self.cancel_fuzzy_search();
                    }
                    self.market.input_mode.activity = InputActivity::Ticker;
                }
            }
            '/' => self.start_fuzzy_search(),
            'j' => self.move_selection(1),
            'k' => self.move_selection(-1),
            'h' => self.paginate(-1),
            'l' => self.paginate(1),
            _ => {}
        }
    }

    fn submit_input(&mut self) {
        let symbols = parse_tickers(&self.market.input_mode.input);
        self.market.input_mode.input.clear();
        for symbol in symbols {
            self.spawn_fetch(symbol, FetchOrigin::UserRequest);
        }
    }

    fn upsert_holding_in_memory(&mut self, holding: Holding) {
        if let Some(existing) = self
            .holdings
            .iter_mut()
            .find(|h| h.ticker.eq_ignore_ascii_case(&holding.ticker))
        {
            *existing = holding;
        } else {
            self.holdings.push(holding);
        }
    }

    /// Whether `all_rows` holds a priced quote for `ticker` (case-insensitive).
    fn has_priced_row(&self, ticker: &str) -> bool {
        self.market.db_display.all_rows.iter().any(|r| {
            r.ticker
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(ticker))
                && r.price.is_some()
        })
    }

    /// Uppercased, deduped tickers of every current holding.
    fn held_tickers(&self) -> Vec<String> {
        let mut syms: Vec<String> = self
            .holdings
            .iter()
            .map(|h| h.ticker.to_ascii_uppercase())
            .collect();
        syms.sort();
        syms.dedup();
        syms
    }

    /// The full symbol set the live stream must cover: the visible quotes window plus every held ticker.
    fn stream_symbols(&self) -> Vec<String> {
        let mut syms = self.market.db_display.window_tickers();
        syms.extend(self.held_tickers());
        syms.sort();
        syms.dedup();
        syms
    }

    /// Held tickers the portfolio can't value yet.
    fn unpriced_holdings(&self) -> Vec<String> {
        self.held_tickers()
            .into_iter()
            .filter(|t| !self.has_priced_row(t))
            .collect()
    }

    fn fetch_unpriced_holdings(&self) {
        for ticker in self.unpriced_holdings() {
            self.spawn_fetch(ticker, FetchOrigin::HoldingPrice);
        }
    }

    fn materialize_held_ticker_row(&mut self, tick: &QuoteTick) {
        let Some(ticker) = tick.ticker.as_deref() else {
            return;
        };
        let held = self
            .holdings
            .iter()
            .any(|h| h.ticker.eq_ignore_ascii_case(ticker));
        let already_present = self.market.db_display.all_rows.iter().any(|r| {
            r.ticker
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(ticker))
        });
        if !held || already_present {
            return;
        }
        self.market.db_display.all_rows.push(QuoteRecord {
            ticker: Some(ticker.to_ascii_uppercase()),
            ..Default::default()
        });
    }

    fn fetch_holding_price(&self, ticker: &str) {
        if !self.has_priced_row(ticker) {
            self.spawn_fetch(ticker.to_ascii_uppercase(), FetchOrigin::HoldingPrice);
        }
    }

    fn selected_ticker(&self) -> Option<String> {
        let selected_idx = self.market.db_display.table_state.selected()?;
        let window = self.market.db_display.window();
        window.get(selected_idx).and_then(|row| row.ticker.clone())
    }

    fn toggle_watchlist(&mut self) {
        let Some(ticker) = self.selected_ticker() else {
            return;
        };

        if self.market.db_display.watchlist.remove(&ticker) {
            self.spawn_remove_from_watchlist(ticker);
        } else {
            self.market.db_display.watchlist.insert(ticker.clone());
            self.spawn_save_to_watchlist(ticker);
        }

        if self.market.db_display.watchlist_only {
            self.apply_filter();
            self.resubscribe_stream();
        }
    }

    /// Sets the sort column and re-fetches the table.
    fn set_sort_mode(&mut self, mode: SortMode) {
        self.push_log(format!("[SORT] mode → {mode:?}"));
        self.market.db_display.sort_mode = mode;
        self.spawn_reload();
    }

    /// Flips the sort direction and re-fetches the table.
    fn toggle_sort_order(&mut self) {
        let order = match self.market.db_display.sort_order {
            SortOrder::Ascending => SortOrder::Descending,
            SortOrder::Descending => SortOrder::Ascending,
        };
        self.push_log(format!("[SORT] order → {order:?}"));
        self.market.db_display.sort_order = order;
        self.spawn_reload();
    }

    /// Moves to the next (`delta > 0`) or previous page, clamped to the valid page range.
    fn paginate(&mut self, delta: i32) {
        let total = self.market.db_display.total_pages();
        let new_page = if delta > 0 {
            (self.market.db_display.page + 1).min(total.saturating_sub(1))
        } else {
            self.market.db_display.page.saturating_sub(1)
        };
        if new_page != self.market.db_display.page {
            self.market.db_display.page = new_page;
            self.reset_selection();
            self.resubscribe_stream();
        }
    }

    /// Moves the table selection by `delta`, wrapping at the ends of the current page.
    fn move_selection(&mut self, delta: i32) {
        let len = self.market.db_display.window_len();
        if len == 0 {
            return;
        }
        let i = self
            .market.db_display
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
        self.market.db_display.table_state.select(Some(i));
    }

    fn start_fuzzy_search(&mut self) {
        self.market.input_mode.activity = InputActivity::Fuzzy;
        self.market.input_mode.input.clear();
        self.apply_filter();
    }

    fn accept_fuzzy_search(&mut self) {
        if self.market.input_mode.input.trim().is_empty() {
            self.cancel_fuzzy_search();
            return;
        }
        self.market.input_mode.activity = InputActivity::FuzzyApplied;
        self.resubscribe_stream();
    }

    /// Cancels fuzzy search (Esc): clears the query and restores the full table.
    fn cancel_fuzzy_search(&mut self) {
        self.market.input_mode.activity = InputActivity::Idle;
        self.market.input_mode.input.clear();
        self.apply_filter();
        self.resubscribe_stream();
    }

    fn apply_filter(&mut self) {
        let query = &self.market.input_mode.input;
        let mut rows: Vec<QuoteRecord> = if self.market.input_mode.is_fuzzy() {
            self.market.db_display
                .all_rows
                .iter()
                .filter(|r| r.matches_query(query))
                .cloned()
                .collect()
        } else {
            self.market.db_display.all_rows.clone()
        };
        if self.market.db_display.watchlist_only {
            rows.retain(|r| self.market.db_display.on_watchlist(r));
        }
        self.market.db_display.rows = rows;
        self.market.db_display.page = 0;
        self.reset_selection();
    }

    pub fn set_page_size(&mut self, size: usize) {
        let size = size.max(1);
        if self.market.db_display.page_size == size {
            return;
        }
        let global_idx = self
            .market.db_display
            .table_state
            .selected()
            .map(|i| self.market.db_display.page * self.market.db_display.page_size + i);

        self.market.db_display.page_size = size;

        let target_page = global_idx.map(|gi| gi / size).unwrap_or(0);
        self.market.db_display.page = target_page.min(self.market.db_display.total_pages().saturating_sub(1));

        let win_len = self.market.db_display.window_len();
        if win_len == 0 {
            self.market.db_display.table_state.select(None);
        } else {
            let local = global_idx.map(|gi| gi % size).unwrap_or(0).min(win_len - 1);
            self.market.db_display.table_state.select(Some(local));
        }
        self.resubscribe_stream();
    }

    /// Selects row 0 in the current window, or clears selection if empty.
    fn reset_selection(&mut self) {
        if self.market.db_display.window_len() == 0 {
            self.market.db_display.table_state.select(None);
        } else {
            self.market.db_display.table_state.select(Some(0));
        }
    }

    fn open_info_modal(&mut self) {
        let Some(stock) = self
            .market.db_display
            .table_state
            .selected()
            .and_then(|i| self.market.db_display.window().get(i))
            .cloned()
        else {
            return;
        };
        let ticker = stock.ticker.clone();
        self.stock_modal.stock = stock;
        self.overlay = Overlay::Info;

        let has_analysis = self
            .stock_modal
            .analysis
            .as_ref()
            .is_some_and(|a| a.ticker == ticker);

        if !has_analysis && let Some(t) = ticker.as_deref() {
            self.stock_modal.analysis = None;
            self.spawn_analysis(t);
        }

        let has_statistics = self
            .stock_modal
            .statistics
            .as_ref()
            .is_some_and(|s| s.ticker == ticker);

        if !has_statistics && let Some(t) = ticker.as_deref() {
            self.stock_modal.statistics = None;
            self.spawn_statistics(t);
        }

        let has_news = self
            .stock_modal
            .news
            .as_ref()
            .is_some_and(|n| n.ticker == ticker);

        if !has_news && let Some(t) = ticker.as_deref() {
            self.stock_modal.news = None;
            self.spawn_news(t);
        }
    }

    fn open_chart_modal(&mut self) {
        let Some(stock) = self
            .market.db_display
            .table_state
            .selected()
            .and_then(|i| self.market.db_display.window().get(i))
            .cloned()
        else {
            return;
        };
        let ticker = stock.ticker.clone();
        self.stock_modal.stock = stock;
        self.overlay = Overlay::Chart;
        if let Some(t) = ticker.as_deref() {
            self.spawn_chart_data(t);
        }
    }

    /// Opens the options-chain modal for the selected row and starts fetching its chain in the background.
    fn open_options_modal(&mut self) {
        let Some(stock) = self
            .market.db_display
            .table_state
            .selected()
            .and_then(|i| self.market.db_display.window().get(i))
            .cloned()
        else {
            return;
        };
        let ticker = stock.ticker.clone();
        let spot = stock.price;
        self.stock_modal.stock = stock;
        self.overlay = Overlay::Options;

        let has_chain = self
            .stock_modal
            .option_chain
            .as_ref()
            .is_some_and(|c| Some(&c.underlying) == ticker.as_ref());

        if has_chain {
            if let Some(chain) = self.stock_modal.option_chain.as_mut() {
                chain.underlying_price = spot;
            }
        } else if let Some(t) = ticker.as_deref() {
            self.stock_modal.option_chain = None;
            self.stock_modal.selected_expiry = 0;
            self.spawn_option_chain(t);
        }
    }

    fn cycle_expiry(&mut self, delta: isize) {
        let Some(chain) = self.stock_modal.option_chain.as_ref() else {
            return;
        };
        if chain.expirations.is_empty() {
            return;
        }
        let last = (chain.expirations.len() - 1) as isize;
        let current = self.stock_modal.selected_expiry as isize;
        let next = (current + delta).clamp(0, last);
        if next == current {
            return;
        }
        let next = next as usize;
        let symbol = chain.underlying.clone();
        let expiry = chain.expirations[next];
        self.stock_modal.selected_expiry = next;
        self.stock_modal.option_chain = None;
        self.spawn_option_chain_for_expiry(&symbol, expiry);
    }

    /// Opens the keybinding help overlay, closing any stock modal so the two never stack.
    fn open_help_modal(&mut self) {
        self.overlay = Overlay::Help;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::AlertDirection;
    use crate::models::QuoteRecord;
    use crate::sort::{SortMode, SortOrder};
    use tokio::sync::mpsc;

    fn make_test_app() -> (App, mpsc::UnboundedReceiver<AppEvent>) {
        let pool = sqlx::PgPool::connect_lazy("postgres://localhost/nonexistent_test_db").unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        (App::new(pool, YfClient::default(), tx), rx)
    }

    fn n_records(n: usize) -> Vec<QuoteRecord> {
        (0..n).map(|_| QuoteRecord::default()).collect()
    }

    fn named(ticker: &str, name: &str) -> QuoteRecord {
        QuoteRecord {
            ticker: Some(ticker.to_string()),
            name: Some(name.to_string()),
            ..Default::default()
        }
    }

    /// Loads three named quotes and returns the app ready for search tests.
    fn app_with_three_stocks() -> App {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![
            named("AAPL", "Apple Inc."),
            named("TSLA", "Tesla Inc."),
            named("NVDA", "NVIDIA Corp."),
        ]));
        app
    }

    fn type_str(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle_key(KeyCode::Char(c));
        }
    }

    #[tokio::test]
    async fn test_app_new_default_state() {
        let (app, _rx) = make_test_app();
        assert!(!app.should_quit);
        assert!(!app.market.input_mode.is_focused());
        assert!(app.market.db_display.rows.is_empty());
        assert!(app.market.logs.is_empty());
        assert_eq!(app.overlay, Overlay::None);
        assert_eq!(app.market.db_display.sort_mode, SortMode::ById);
        assert_eq!(app.market.db_display.sort_order, SortOrder::Descending);
    }

    #[tokio::test]
    async fn test_sector_data_ready_upserts_group() {
        use crate::sectors::SectorTile;
        use yfinance_rs::screener::EquitySector;

        let (mut app, _rx) = make_test_app();
        let tiles = vec![
            SectorTile { ticker: "AAPL".into(), name: "Apple".into(), market_cap: 100.0, day_pct: 1.0 },
            SectorTile { ticker: "MSFT".into(), name: "Microsoft".into(), market_cap: 50.0, day_pct: -0.5 },
        ];

        app.handle_event(AppEvent::SectorDataReady {
            sector: EquitySector::Technology,
            tiles,
        });

        assert_eq!(app.sectors.map.groups.len(), 1);
        let group = &app.sectors.map.groups[0];
        assert_eq!(group.sector, EquitySector::Technology);
        assert_eq!(group.total_market_cap, 150.0);
        assert_eq!(group.tiles.len(), 2);
    }

    #[tokio::test]
    async fn test_sector_data_ready_with_empty_tiles_forms_a_counting_group() {
        use yfinance_rs::screener::EquitySector;
        let (mut app, _rx) = make_test_app();

        app.handle_event(AppEvent::SectorDataReady { sector: EquitySector::Energy, tiles: Vec::new() });

        assert_eq!(app.sectors.map.groups.len(), 1, "empty sector still forms a group");
        assert_eq!(app.sectors.map.groups[0].total_market_cap, 0.0);
    }

    #[tokio::test]
    async fn test_sectors_enter_opens_modal_for_selected_tile() {
        use crate::sectors::SectorTile;
        use yfinance_rs::screener::EquitySector;

        let (mut app, _rx) = make_test_app();
        app.page = Page::Sectors;
        app.handle_event(AppEvent::SectorDataReady {
            sector: EquitySector::Technology,
            tiles: vec![SectorTile {
                ticker: "AAPL".into(),
                name: "Apple".into(),
                market_cap: 100.0,
                day_pct: 1.0,
            }],
        });
        app.sectors.selected = 0;

        app.handle_key(KeyCode::Enter);

        assert_eq!(app.overlay, Overlay::Chart);
        assert_eq!(app.stock_modal.stock.ticker.as_deref(), Some("AAPL"));
    }

    #[tokio::test]
    async fn status_mode_is_normal_with_hint_for_default_market_state() {
        // arrange
        let (app, _rx) = make_test_app();

        // act
        let (mode, hint) = app.status_mode();

        // assert
        assert_eq!(
            (mode, mode.label(), hint),
            (
                StatusMode::Normal,
                "NORMAL",
                "j/k move · h/l page · s info · enter chart · x options · tab manage · w ★ · i query · / search · o order · ? help · q quit"
            )
        );
    }

    #[tokio::test]
    async fn status_mode_is_sectors_with_its_own_hint_on_the_sectors_page() {
        // arrange
        let (mut app, _rx) = make_test_app();
        app.page = Page::Sectors;

        // act
        let (mode, hint) = app.status_mode();

        // assert
        assert_eq!(
            (mode, mode.label(), hint),
            (
                StatusMode::Sectors,
                "SECTORS",
                "h/j/k/l move · enter chart · r refresh · tab cycle · q quit"
            )
        );
    }

    #[tokio::test]
    async fn status_mode_is_view_with_options_hint_when_options_overlay_open() {
        // arrange
        let (mut app, _rx) = make_test_app();
        app.overlay = Overlay::Options;

        // act
        let (mode, hint) = app.status_mode();

        // assert
        assert_eq!(
            (mode, mode.label(), hint),
            (StatusMode::Options, "VIEW", "h/l expiry · esc close")
        );
    }

    #[tokio::test]
    async fn status_mode_help_overlay_takes_precedence_over_manage_page() {
        // arrange
        let (mut app, _rx) = make_test_app();
        app.page = Page::Manage;
        app.overlay = Overlay::Help;

        // act
        let (mode, hint) = app.status_mode();

        // assert
        assert_eq!((mode, mode.label(), hint), (StatusMode::Help, "HELP", "esc close"));
    }

    #[tokio::test]
    async fn test_handle_event_page_loaded() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord::default()]));
        assert_eq!(app.market.db_display.rows.len(), 1);
        assert!(app.market.db_display.status.is_empty());
        assert_eq!(app.market.db_display.table_state.selected(), Some(0));
    }

    #[tokio::test]
    async fn test_handle_event_fetch_spawned() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::FetchSpawned("AAPL".to_string()));
        assert!(app.market.db_display.status.contains("AAPL"));
        assert_eq!(app.market.logs.len(), 1);
    }

    #[tokio::test]
    async fn test_handle_event_fetch_completed() {
        let (mut app, _rx) = make_test_app();
        let record = QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        };
        app.handle_event(AppEvent::FetchCompleted(record));
        assert_eq!(app.market.db_display.rows.len(), 1);
        assert_eq!(app.market.logs.len(), 1);
        assert!(app.market.logs[0].contains("AAPL"));
    }

    #[tokio::test]
    async fn test_handle_event_log_line() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::LogLine("hello".to_string()));
        assert_eq!(app.market.logs.back(), Some(&"hello".to_string()));
    }

    #[tokio::test]
    async fn test_handle_event_error() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::Error("oops".to_string()));
        assert!(app.market.db_display.status.contains("oops"));
        assert!(app.market.logs.iter().any(|l| l.contains("oops")));
    }

    fn analysis_for(ticker: &str) -> QuoteRecordAnalysis {
        QuoteRecordAnalysis {
            ticker: Some(ticker.to_string()),
            recommendation_summary: None,
            price_target: None,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_open_info_reuses_matching_analysis() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.analysis = Some(analysis_for("AAPL"));
        app.open_info_modal();
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
        app.open_chart_modal();
        app.open_info_modal();
        assert!(app.stock_modal.analysis.is_some());
    }

    #[tokio::test]
    async fn test_open_info_refetches_on_ticker_mismatch() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.analysis = Some(analysis_for("TSLA"));
        app.open_info_modal();
        assert!(app.stock_modal.analysis.is_none());
    }

    fn statistics_for(ticker: &str) -> QuoteRecordStatistics {
        QuoteRecordStatistics {
            ticker: Some(ticker.to_string()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_open_info_reuses_matching_statistics() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.statistics = Some(statistics_for("AAPL"));
        app.open_info_modal();
        assert!(app.stock_modal.statistics.is_some());
    }

    #[tokio::test]
    async fn test_open_info_refetches_statistics_on_ticker_mismatch() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.statistics = Some(statistics_for("TSLA"));
        app.open_info_modal();
        assert!(app.stock_modal.statistics.is_none());
    }

    #[tokio::test]
    async fn test_statistics_survive_chart_visit_for_same_ticker() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.statistics = Some(statistics_for("AAPL"));
        app.open_chart_modal();
        app.open_info_modal();
        assert!(app.stock_modal.statistics.is_some());
    }

    #[tokio::test]
    async fn test_statistics_ready_event_sets_modal_statistics() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::StockStatisticsReady(statistics_for("AAPL")));
        assert_eq!(
            app.stock_modal.statistics.and_then(|s| s.ticker).as_deref(),
            Some("AAPL")
        );
    }

    fn news_for(ticker: &str) -> StockNews {
        StockNews {
            ticker: Some(ticker.to_string()),
            items: Vec::new(),
        }
    }

    #[tokio::test]
    async fn test_open_info_reuses_matching_news() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.news = Some(news_for("AAPL"));
        app.open_info_modal();
        assert!(app.stock_modal.news.is_some());
    }

    #[tokio::test]
    async fn test_open_info_refetches_news_on_ticker_mismatch() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.news = Some(news_for("TSLA"));
        app.open_info_modal();
        assert!(app.stock_modal.news.is_none());
    }

    #[tokio::test]
    async fn test_news_survive_chart_visit_for_same_ticker() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.news = Some(news_for("AAPL"));
        app.open_chart_modal();
        app.open_info_modal();
        assert!(app.stock_modal.news.is_some());
    }

    #[tokio::test]
    async fn test_news_ready_event_sets_modal_news() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::StockNewsReady(news_for("AAPL")));
        assert_eq!(
            app.stock_modal.news.and_then(|n| n.ticker).as_deref(),
            Some("AAPL")
        );
    }

    fn chain_for(ticker: &str) -> OptionChain {
        OptionChain {
            underlying: ticker.to_string(),
            ..Default::default()
        }
    }

    fn chain_with_expirations(ticker: &str, n: usize) -> OptionChain {
        let base = chrono::DateTime::from_timestamp(1_768_521_600, 0).unwrap();
        OptionChain {
            underlying: ticker.to_string(),
            expirations: (0..n)
                .map(|i| base + chrono::Duration::days(i as i64))
                .collect(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_option_chain_ready_stores_chain() {
        let (mut app, _rx) = make_test_app();

        app.handle_event(AppEvent::OptionChainReady(chain_for("AAPL")));

        assert_eq!(
            app.stock_modal.option_chain.as_ref().map(|c| c.underlying.as_str()),
            Some("AAPL")
        );
    }

    #[tokio::test]
    async fn test_option_chain_ready_preserves_in_range_selection() {
        let (mut app, _rx) = make_test_app();
        app.stock_modal.selected_expiry = 2;

        app.handle_event(AppEvent::OptionChainReady(chain_with_expirations("AAPL", 5)));

        assert_eq!(app.stock_modal.selected_expiry, 2);
    }

    #[tokio::test]
    async fn test_option_chain_ready_clamps_out_of_range_selection() {
        let (mut app, _rx) = make_test_app();
        app.stock_modal.selected_expiry = 9;

        app.handle_event(AppEvent::OptionChainReady(chain_with_expirations("AAPL", 3)));

        assert_eq!(app.stock_modal.selected_expiry, 2);
    }

    #[tokio::test]
    async fn test_option_chain_ready_fills_underlying_price_from_selected_quote() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: Some(150.0),
            ..Default::default()
        }]));
        app.open_options_modal();

        app.handle_event(AppEvent::OptionChainReady(chain_for("AAPL")));

        assert_eq!(
            app.stock_modal.option_chain.and_then(|c| c.underlying_price),
            Some(150.0)
        );
    }

    #[tokio::test]
    async fn test_command_key_x_opens_options_modal() {
        let mut app = app_with_three_stocks();

        app.handle_command_key('x');

        assert_eq!(app.overlay, Overlay::Options);
    }

    #[tokio::test]
    async fn test_open_options_modal_replaces_other_overlay() {
        let mut app = app_with_three_stocks();
        app.overlay = Overlay::Help;

        app.open_options_modal();

        assert_eq!(app.overlay, Overlay::Options);
    }

    #[tokio::test]
    async fn test_esc_closes_options_modal() {
        let mut app = app_with_three_stocks();
        app.handle_command_key('x');
        assert_eq!(app.overlay, Overlay::Options);

        app.handle_key(KeyCode::Esc);

        assert_eq!(app.overlay, Overlay::None);
    }

    #[tokio::test]
    async fn test_open_options_reuses_matching_chain() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.option_chain = Some(chain_for("AAPL"));

        app.open_options_modal();

        assert!(app.stock_modal.option_chain.is_some());
    }

    #[tokio::test]
    async fn test_open_options_refetches_chain_on_ticker_mismatch() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            ..Default::default()
        }]));
        app.stock_modal.option_chain = Some(chain_for("TSLA"));

        app.open_options_modal();

        assert!(app.stock_modal.option_chain.is_none());
    }

    #[tokio::test]
    async fn test_cycle_expiry_advances_and_refetches() {
        let mut app = app_with_three_stocks();
        app.overlay = Overlay::Options;
        app.stock_modal.option_chain = Some(chain_with_expirations("AAPL", 5));

        app.handle_command_key('l');

        assert_eq!(app.stock_modal.selected_expiry, 1);
        assert!(app.stock_modal.option_chain.is_none());
    }

    #[tokio::test]
    async fn test_cycle_expiry_clamps_at_first() {
        let mut app = app_with_three_stocks();
        app.overlay = Overlay::Options;
        app.stock_modal.option_chain = Some(chain_with_expirations("AAPL", 5));

        app.handle_command_key('h');

        assert_eq!(app.stock_modal.selected_expiry, 0);
        assert!(app.stock_modal.option_chain.is_some());
    }

    #[tokio::test]
    async fn test_cycle_expiry_clamps_at_last() {
        let mut app = app_with_three_stocks();
        app.overlay = Overlay::Options;
        app.stock_modal.selected_expiry = 4;
        app.stock_modal.option_chain = Some(chain_with_expirations("AAPL", 5));

        app.handle_command_key('l');

        assert_eq!(app.stock_modal.selected_expiry, 4);
        assert!(app.stock_modal.option_chain.is_some());
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
        assert!(app.market.input_mode.is_focused());
        app.handle_command_key('i');
        assert!(!app.market.input_mode.is_focused());
    }

    #[tokio::test]
    async fn test_command_key_navigation_j_wraps() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(n_records(3)));
        assert_eq!(app.market.db_display.table_state.selected(), Some(0));
        app.handle_command_key('j');
        assert_eq!(app.market.db_display.table_state.selected(), Some(1));
        app.handle_command_key('j');
        assert_eq!(app.market.db_display.table_state.selected(), Some(2));
        app.handle_command_key('j');
        assert_eq!(app.market.db_display.table_state.selected(), Some(0));
    }

    #[tokio::test]
    async fn test_command_key_navigation_k_wraps() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(n_records(3)));
        assert_eq!(app.market.db_display.table_state.selected(), Some(0));
        app.handle_command_key('k');
        assert_eq!(app.market.db_display.table_state.selected(), Some(2));
    }

    #[tokio::test]
    async fn test_command_key_o_toggles_sort_order() {
        let (mut app, _rx) = make_test_app();
        assert_eq!(app.market.db_display.sort_order, SortOrder::Descending);
        app.handle_command_key('o');
        assert_eq!(app.market.db_display.sort_order, SortOrder::Ascending);
        app.handle_command_key('o');
        assert_eq!(app.market.db_display.sort_order, SortOrder::Descending);
    }

    #[tokio::test]
    async fn test_command_key_sort_by_price() {
        let (mut app, _rx) = make_test_app();
        assert_eq!(app.market.db_display.sort_mode, SortMode::ById);
        app.handle_command_key('p');
        assert_eq!(app.market.db_display.sort_mode, SortMode::ByPrice);
    }

    #[tokio::test]
    async fn test_window_follows_page() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(n_records(30)));
        assert_eq!(app.market.db_display.total_pages(), 2);
        assert_eq!(app.market.db_display.window_len(), 25);
        app.handle_command_key('l');
        assert_eq!(app.market.db_display.page, 1);
        assert_eq!(app.market.db_display.window_len(), 5);
    }

    #[tokio::test]
    async fn test_paginate_clamps_at_bounds() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(n_records(30)));
        app.handle_command_key('h');
        assert_eq!(app.market.db_display.page, 0);
        app.handle_command_key('l');
        app.handle_command_key('l');
        assert_eq!(app.market.db_display.page, 1);
    }

    #[tokio::test]
    async fn test_fuzzy_search_filters_live() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('/'));
        assert!(app.market.input_mode.is_focused() && app.market.input_mode.is_fuzzy());
        assert_eq!(app.market.db_display.rows.len(), 3, "empty query shows all");

        type_str(&mut app, "apl");
        assert_eq!(app.market.db_display.rows.len(), 1);
        assert_eq!(app.market.db_display.rows[0].ticker.as_deref(), Some("AAPL"));
        assert_eq!(app.market.db_display.all_rows.len(), 3, "master list untouched");
        assert_eq!(app.market.db_display.table_state.selected(), Some(0));
    }

    #[tokio::test]
    async fn test_fuzzy_search_backspace_widens() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('/'));
        type_str(&mut app, "tsla");
        assert_eq!(app.market.db_display.rows.len(), 1);
        app.handle_key(KeyCode::Backspace);
        app.handle_key(KeyCode::Backspace);
        app.handle_key(KeyCode::Backspace);
        app.handle_key(KeyCode::Backspace);
        assert_eq!(app.market.db_display.rows.len(), 3, "empty query shows all");
    }

    #[tokio::test]
    async fn test_fuzzy_search_enter_keeps_filter() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('/'));
        type_str(&mut app, "inc");
        assert_eq!(app.market.db_display.rows.len(), 2, "Apple Inc. + Tesla Inc.");

        app.handle_key(KeyCode::Enter);
        assert!(!app.market.input_mode.is_focused(), "input box loses focus");
        assert!(app.market.input_mode.is_fuzzy(), "filter stays active");
        assert_eq!(app.market.db_display.rows.len(), 2);

        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.market.db_display.table_state.selected(), Some(1));
    }

    #[tokio::test]
    async fn test_fuzzy_search_esc_restores_all_rows() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('/'));
        type_str(&mut app, "aapl");
        assert_eq!(app.market.db_display.rows.len(), 1);
        app.handle_key(KeyCode::Esc);
        assert!(!app.market.input_mode.is_fuzzy());
        assert!(app.market.input_mode.input.is_empty());
        assert_eq!(app.market.db_display.rows.len(), 3);

        app.handle_key(KeyCode::Char('/'));
        type_str(&mut app, "aapl");
        app.handle_key(KeyCode::Enter);
        app.handle_key(KeyCode::Esc);
        assert!(!app.market.input_mode.is_fuzzy());
        assert_eq!(app.market.db_display.rows.len(), 3);
    }

    #[tokio::test]
    async fn test_fuzzy_search_empty_enter_cancels() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Enter);
        assert!(!app.market.input_mode.is_fuzzy());
        assert!(!app.market.input_mode.is_focused());
        assert_eq!(app.market.db_display.rows.len(), 3);
    }

    #[tokio::test]
    async fn test_i_key_clears_accepted_filter() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('/'));
        type_str(&mut app, "aapl");
        app.handle_key(KeyCode::Enter);

        app.handle_key(KeyCode::Char('i'));
        assert!(app.market.input_mode.is_focused(), "input box focused for a fetch");
        assert!(!app.market.input_mode.is_fuzzy());
        assert!(app.market.input_mode.input.is_empty());
        assert_eq!(app.market.db_display.rows.len(), 3, "filter cleared");
    }

    #[tokio::test]
    async fn test_page_load_reapplies_filter() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('/'));
        type_str(&mut app, "aapl");
        app.handle_key(KeyCode::Enter);

        app.handle_event(AppEvent::PageLoaded(vec![
            named("AAPL", "Apple Inc."),
            named("TSLA", "Tesla Inc."),
        ]));
        assert_eq!(app.market.db_display.all_rows.len(), 2);
        assert_eq!(app.market.db_display.rows.len(), 1, "filter survives the reload");
        assert_eq!(app.market.db_display.rows[0].ticker.as_deref(), Some("AAPL"));
    }

    #[tokio::test]
    async fn test_tick_updates_master_list_while_filtered() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('/'));
        type_str(&mut app, "aapl");
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.market.db_display.rows.len(), 1, "TSLA is hidden");

        app.handle_event(AppEvent::QuoteTick(QuoteTick {
            ticker: Some("TSLA".to_string()),
            price: Some(456.0),
            previous_close: None,
            day_volume: None,
            as_of: None,
        }));

        app.handle_key(KeyCode::Esc);
        let tsla = app
            .market.db_display
            .rows
            .iter()
            .find(|r| r.ticker.as_deref() == Some("TSLA"))
            .expect("TSLA back after clearing");
        assert_eq!(tsla.price, Some(456.0), "tick reached the master list");
    }

    #[tokio::test]
    async fn test_app_new_starts_with_empty_watchlist() {
        let (app, _rx) = make_test_app();
        assert!(app.market.db_display.watchlist.is_empty());
        assert!(!app.market.db_display.watchlist_only);
    }

    #[tokio::test]
    async fn test_handle_event_watchlist_loaded() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::WatchlistLoaded(vec![
            "AAPL".to_string(),
            "TSLA".to_string(),
        ]));
        assert_eq!(app.market.db_display.watchlist.len(), 2);
        assert!(app.market.db_display.watchlist.contains("AAPL"));
        assert!(app.market.db_display.watchlist.contains("TSLA"));
    }

    /// The set collapses duplicate tickers coming back from the db.
    #[tokio::test]
    async fn test_watchlist_loaded_dedups() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::WatchlistLoaded(vec![
            "AAPL".to_string(),
            "AAPL".to_string(),
        ]));
        assert_eq!(app.market.db_display.watchlist.len(), 1);
        assert!(app.market.db_display.watchlist.contains("AAPL"));
    }

    #[tokio::test]
    async fn test_watchlist_loaded_replaces_previous() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::WatchlistLoaded(vec!["AAPL".to_string()]));
        app.handle_event(AppEvent::WatchlistLoaded(vec!["TSLA".to_string()]));
        assert_eq!(app.market.db_display.watchlist.len(), 1);
        assert!(app.market.db_display.watchlist.contains("TSLA"));
        assert!(!app.market.db_display.watchlist.contains("AAPL"));
    }

    #[tokio::test]
    async fn test_watchlist_loaded_empty_clears() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::WatchlistLoaded(vec!["AAPL".to_string()]));
        app.handle_event(AppEvent::WatchlistLoaded(vec![]));
        assert!(app.market.db_display.watchlist.is_empty());
    }

    fn alert_for(id: i32, ticker: &str) -> crate::models::Alert {
        crate::models::Alert {
            id,
            ticker: ticker.to_string(),
            direction: crate::models::AlertDirection::Above,
            threshold: 100.0,
            created_at: chrono::Utc::now(),
            triggered_at: None,
        }
    }

    #[tokio::test]
    async fn test_alerts_loaded_populates() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::AlertsLoaded(vec![
            alert_for(1, "AAPL"),
            alert_for(2, "TSLA"),
        ]));
        assert_eq!(app.alerts.len(), 2);
    }

    #[tokio::test]
    async fn test_alerts_loaded_replaces_previous() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::AlertsLoaded(vec![alert_for(1, "AAPL")]));
        app.handle_event(AppEvent::AlertsLoaded(vec![alert_for(2, "TSLA")]));
        assert_eq!(app.alerts.len(), 1);
        assert_eq!(app.alerts[0].ticker, "TSLA");
    }

    fn holding_for(ticker: &str) -> Holding {
        Holding {
            ticker: ticker.to_string(),
            shares: 10.0,
            avg_cost: 100.0,
        }
    }

    #[tokio::test]
    async fn test_portfolio_loaded_populates() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PortfolioLoaded(vec![
            holding_for("AAPL"),
            holding_for("TSLA"),
        ]));
        assert_eq!(app.holdings.len(), 2);
    }

    #[tokio::test]
    async fn test_portfolio_loaded_replaces_previous() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PortfolioLoaded(vec![holding_for("AAPL")]));
        app.handle_event(AppEvent::PortfolioLoaded(vec![holding_for("TSLA")]));
        assert_eq!(app.holdings.len(), 1);
        assert_eq!(app.holdings[0].ticker, "TSLA");
    }

    #[tokio::test]
    async fn test_portfolio_loaded_empty_clears() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PortfolioLoaded(vec![holding_for("AAPL")]));
        app.handle_event(AppEvent::PortfolioLoaded(vec![]));
        assert!(app.holdings.is_empty());
    }

    fn price_tick(ticker: &str, price: Option<f64>) -> crate::models::QuoteTick {
        crate::models::QuoteTick {
            ticker: Some(ticker.to_string()),
            price,
            previous_close: None,
            day_volume: None,
            as_of: None,
        }
    }

    #[tokio::test]
    async fn test_crossing_tick_fires_and_removes_only_matching_alert() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::AlertsLoaded(vec![
            alert_for(1, "AAPL"),
            alert_for(2, "TSLA"),
        ]));

        app.handle_event(AppEvent::QuoteTick(price_tick("AAPL", Some(150.0))));

        assert_eq!(app.alerts.len(), 1, "the crossed alert should be removed");
        assert_eq!(app.alerts[0].ticker, "TSLA", "unrelated alert stays");
    }

    #[tokio::test]
    async fn test_non_crossing_tick_leaves_alert() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::AlertsLoaded(vec![alert_for(1, "AAPL")]));

        app.handle_event(AppEvent::QuoteTick(price_tick("AAPL", Some(99.0))));

        assert_eq!(app.alerts.len(), 1);
    }

    #[tokio::test]
    async fn test_priceless_tick_is_noop() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::AlertsLoaded(vec![alert_for(1, "AAPL")]));

        app.handle_event(AppEvent::QuoteTick(price_tick("AAPL", None)));

        assert_eq!(app.alerts.len(), 1);
    }

    #[tokio::test]
    async fn test_tick_for_other_ticker_does_not_fire() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::AlertsLoaded(vec![alert_for(1, "AAPL")]));

        app.handle_event(AppEvent::QuoteTick(price_tick("TSLA", Some(9_999.0))));

        assert_eq!(app.alerts.len(), 1, "AAPL alert must not fire on a TSLA tick");
    }

    #[tokio::test]
    async fn test_tick_updates_held_position_pnl() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: Some(120.0),
            previous_close: Some(118.0),
            ..Default::default()
        }]));
        app.handle_event(AppEvent::PortfolioLoaded(vec![Holding {
            ticker: "AAPL".to_string(),
            shares: 10.0,
            avg_cost: 100.0,
        }]));

        app.handle_event(AppEvent::QuoteTick(price_tick("AAPL", Some(150.0))));

        let positions =
            crate::portfolio::portfolio_positions(&app.holdings, &app.market.db_display.all_rows);
        assert_eq!(positions[0].market_value, Some(1500.0));
        assert_eq!(positions[0].unrealized_pnl, Some(500.0));
    }

    #[tokio::test]
    async fn test_tick_for_unheld_ticker_does_not_change_portfolio() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: Some(120.0),
            previous_close: Some(118.0),
            ..Default::default()
        }]));
        app.handle_event(AppEvent::PortfolioLoaded(vec![Holding {
            ticker: "AAPL".to_string(),
            shares: 10.0,
            avg_cost: 100.0,
        }]));

        app.handle_event(AppEvent::QuoteTick(price_tick("TSLA", Some(999.0))));

        let positions =
            crate::portfolio::portfolio_positions(&app.holdings, &app.market.db_display.all_rows);
        assert_eq!(positions[0].market_value, Some(1200.0), "AAPL still valued at its own price");
    }

    #[tokio::test]
    async fn test_held_ticker_off_page_is_subscribed_on_load() {
        let (mut app, _rx) = make_test_app();
        // arrange
        app.handle_event(AppEvent::PageLoaded(vec![named("AAPL", "Apple Inc.")]));

        // act
        app.handle_event(AppEvent::PortfolioLoaded(vec![holding_for("MSFT")]));

        // assert
        assert!(
            app.subscribed_symbols.contains(&"MSFT".to_string()),
            "held ticker must be streamed even when off-page"
        );
    }

    #[tokio::test]
    async fn test_live_tick_prices_held_ticker_with_no_row() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![named("AAPL", "Apple Inc.")]));
        app.handle_event(AppEvent::PortfolioLoaded(vec![holding_for("MSFT")]));

        app.handle_event(AppEvent::QuoteTick(price_tick("MSFT", Some(200.0))));

        let positions =
            crate::portfolio::portfolio_positions(&app.holdings, &app.market.db_display.all_rows);
        assert_eq!(positions[0].market_value, Some(2000.0));
    }

    #[tokio::test]
    async fn test_live_tick_for_unheld_ticker_creates_no_row() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![named("AAPL", "Apple Inc.")]));
        app.handle_event(AppEvent::PortfolioLoaded(vec![holding_for("AAPL")]));

        app.handle_event(AppEvent::QuoteTick(price_tick("MSFT", Some(200.0))));

        assert!(
            !app.market.db_display
                .all_rows
                .iter()
                .any(|r| r.ticker.as_deref() == Some("MSFT")),
            "an unheld ticker's tick must not conjure a row"
        );
    }

    #[tokio::test]
    async fn test_added_holding_is_subscribed_to_the_live_stream() {
        let (mut app, _rx) = make_test_app();
        app.page = Page::Manage;

        app.handle_key(KeyCode::Char('a'));
        type_str(&mut app, "AAPL 10 150");
        app.handle_key(KeyCode::Enter);

        assert!(
            app.subscribed_symbols.contains(&"AAPL".to_string()),
            "a newly added holding must be streamed for live P&L"
        );
    }

    #[test]
    fn test_user_fetch_failure_raises_an_error_notification() {
        let ev = fetch_failure_event(FetchOrigin::UserRequest, "AAPL", "currency unresolved");

        match ev {
            AppEvent::Error(msg) => {
                assert!(msg.contains("AAPL"));
                assert!(msg.contains("currency unresolved"));
            }
            _ => panic!("user fetch failure must be an error notification"),
        }
    }

    #[test]
    fn test_holding_price_fetch_failure_is_logged_not_toasted() {
        let ev = fetch_failure_event(FetchOrigin::HoldingPrice, "AAPL", "currency unresolved");

        match ev {
            AppEvent::LogLine(msg) => {
                assert!(msg.contains("AAPL"));
                assert!(msg.contains("currency unresolved"));
            }
            _ => panic!("holding-price fetch failure must be logged, not an error toast"),
        }
    }

    #[tokio::test]
    async fn test_unpriced_holdings_lists_only_held_tickers_without_a_price() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![QuoteRecord {
            ticker: Some("AAPL".to_string()),
            price: Some(120.0),
            ..Default::default()
        }]));
        app.handle_event(AppEvent::PortfolioLoaded(vec![
            holding_for("AAPL"),
            holding_for("MSFT"),
        ]));

        assert_eq!(app.unpriced_holdings(), vec!["MSFT".to_string()]);
    }

    #[tokio::test]
    async fn test_tab_cycles_forward_through_all_pages() {
        let (mut app, _rx) = make_test_app();
        assert_eq!(app.page, Page::Market, "starts on Market");

        app.handle_key(KeyCode::Tab);
        assert_eq!(app.page, Page::Manage);

        app.handle_key(KeyCode::Tab);
        assert_eq!(app.page, Page::Sectors);

        app.handle_key(KeyCode::Tab);
        assert_eq!(app.page, Page::Market, "wraps back to Market");
    }

    #[tokio::test]
    async fn test_backtab_cycles_backward_through_all_pages() {
        let (mut app, _rx) = make_test_app();

        app.handle_key(KeyCode::BackTab);
        assert_eq!(app.page, Page::Sectors, "back from Market wraps to Sectors");

        app.handle_key(KeyCode::BackTab);
        assert_eq!(app.page, Page::Manage);

        app.handle_key(KeyCode::BackTab);
        assert_eq!(app.page, Page::Market);
    }

    #[tokio::test]
    async fn test_manage_nav_flows_across_sections() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PortfolioLoaded(vec![
            holding_for("AAPL"),
            holding_for("TSLA"),
        ]));
        app.handle_event(AppEvent::AlertsLoaded(vec![alert_for(1, "NVDA")]));
        app.page = Page::Manage;

        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.manage.section, ManageSection::Holdings);
        assert_eq!(app.manage.selected, 1);

        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.manage.section, ManageSection::Alerts);
        assert_eq!(app.manage.selected, 0);

        app.handle_key(KeyCode::Char('k'));
        assert_eq!(app.manage.section, ManageSection::Holdings);
        assert_eq!(app.manage.selected, 1);
    }

    #[tokio::test]
    async fn test_manage_section_item_count_reads_backing_list() {
        // arrange
        let (mut app, _rx) = make_test_app();
        app.holdings = vec![holding_for("AAPL"), holding_for("TSLA")];
        app.alerts = vec![alert_for(1, "NVDA")];

        // act
        assert_eq!(ManageSection::Holdings.item_count(&app), 2);
        assert_eq!(ManageSection::Alerts.item_count(&app), 1);
    }

    #[test]
    fn test_manage_section_input_kind_maps_each_section() {
        // arrange
        assert_eq!(ManageSection::Holdings.input_kind(), ManageInputKind::Holding);
        assert_eq!(ManageSection::Alerts.input_kind(), ManageInputKind::Alert);
    }

    #[tokio::test]
    async fn test_manage_add_holding_saves_and_closes_input() {
        let (mut app, _rx) = make_test_app();
        app.page = Page::Manage;

        app.handle_key(KeyCode::Char('a'));
        assert!(app.manage.input.is_some(), "a opens the inline entry");
        type_str(&mut app, "AAPL 10 150");
        app.handle_key(KeyCode::Enter);

        assert_eq!(app.holdings.len(), 1);
        assert_eq!(app.holdings[0].ticker, "AAPL");
        assert_eq!(app.holdings[0].shares, 10.0);
        assert_eq!(app.holdings[0].avg_cost, 150.0);
        assert!(app.manage.input.is_none(), "input closes on success");
    }

    #[tokio::test]
    async fn test_manage_edit_prefills_selected_holding() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PortfolioLoaded(vec![Holding {
            ticker: "AAPL".to_string(),
            shares: 5.0,
            avg_cost: 100.0,
        }]));
        app.page = Page::Manage;

        app.handle_key(KeyCode::Char('e'));

        assert_eq!(app.manage.input.as_ref().map(|i| i.buffer.as_str()), Some("AAPL 5 100"));
    }

    #[tokio::test]
    async fn test_manage_edit_saves_in_place() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PortfolioLoaded(vec![Holding {
            ticker: "AAPL".to_string(),
            shares: 5.0,
            avg_cost: 100.0,
        }]));
        app.page = Page::Manage;

        app.handle_key(KeyCode::Char('e'));
        app.manage.input.as_mut().unwrap().buffer = "AAPL 20 200".to_string();
        app.handle_key(KeyCode::Enter);

        assert_eq!(app.holdings.len(), 1, "edit replaces, not duplicates");
        assert_eq!(app.holdings[0].shares, 20.0);
        assert_eq!(app.holdings[0].avg_cost, 200.0);
    }

    #[tokio::test]
    async fn test_manage_delete_holding() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PortfolioLoaded(vec![
            holding_for("AAPL"),
            holding_for("TSLA"),
        ]));
        app.page = Page::Manage;

        app.handle_key(KeyCode::Char('d'));

        assert_eq!(app.holdings.len(), 1);
        assert_eq!(app.holdings[0].ticker, "TSLA", "the selected (first) holding is removed");
    }

    #[tokio::test]
    async fn test_manage_invalid_holding_keeps_input_open() {
        let (mut app, _rx) = make_test_app();
        app.page = Page::Manage;

        app.handle_key(KeyCode::Char('a'));
        type_str(&mut app, "garbage");
        app.handle_key(KeyCode::Enter);

        assert!(app.holdings.is_empty());
        assert!(app.manage.input.is_some(), "a bad entry keeps the line open");
    }

    #[tokio::test]
    async fn test_manage_esc_cancels_input() {
        let (mut app, _rx) = make_test_app();
        app.page = Page::Manage;

        app.handle_key(KeyCode::Char('a'));
        type_str(&mut app, "AAP");
        app.handle_key(KeyCode::Esc);

        assert!(app.manage.input.is_none());
        assert_eq!(app.page, Page::Manage, "esc cancels the input, not the page");
    }

    #[tokio::test]
    async fn test_manage_add_alert_saves_and_closes_input() {
        let (mut app, _rx) = make_test_app();
        app.page = Page::Manage;
        app.manage.section = ManageSection::Alerts;

        app.handle_key(KeyCode::Char('a'));
        type_str(&mut app, "AAPL >150");
        app.handle_key(KeyCode::Enter);

        assert_eq!(app.alerts.len(), 1);
        assert_eq!(app.alerts[0].ticker, "AAPL");
        assert_eq!(app.alerts[0].direction, AlertDirection::Above);
        assert_eq!(app.alerts[0].threshold, 150.0);
        assert!(app.manage.input.is_none());
    }

    #[tokio::test]
    async fn test_manage_delete_alert() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::AlertsLoaded(vec![
            alert_for(1, "AAPL"),
            alert_for(2, "TSLA"),
        ]));
        app.page = Page::Manage;
        app.manage.section = ManageSection::Alerts;

        app.handle_key(KeyCode::Char('d'));

        assert_eq!(app.alerts.len(), 1);
        assert_eq!(app.alerts[0].ticker, "TSLA");
    }

    #[tokio::test]
    async fn test_manage_invalid_alert_keeps_input_open() {
        let (mut app, _rx) = make_test_app();
        app.page = Page::Manage;
        app.manage.section = ManageSection::Alerts;

        app.handle_key(KeyCode::Char('a'));
        type_str(&mut app, "nope");
        app.handle_key(KeyCode::Enter);

        assert!(app.alerts.is_empty());
        assert!(app.manage.input.is_some());
    }

    #[tokio::test]
    async fn test_manage_page_renders_without_panicking() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = app_with_three_stocks();
        app.page = Page::Manage;
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

        terminal
            .draw(|f| crate::tui::draw::draw(f, &mut app))
            .expect("empty Manage page should render");

        app.handle_event(AppEvent::PortfolioLoaded(vec![holding_for("AAPL")]));
        app.handle_event(AppEvent::AlertsLoaded(vec![alert_for(1, "AAPL")]));
        terminal
            .draw(|f| crate::tui::draw::draw(f, &mut app))
            .expect("populated Manage page should render");
    }

    #[tokio::test]
    async fn test_market_dashboard_renders_cubes_then_logs_on_toggle() {
        use ratatui::{Terminal, backend::TestBackend};

        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![
            quote("AAPL", 110.0, 100.0),
            quote("TSLA", 90.0, 100.0),
        ]));
        app.handle_event(AppEvent::PortfolioLoaded(vec![holding_for("AAPL")]));
        app.handle_event(AppEvent::AlertsLoaded(vec![alert_for(1, "AAPL")]));
        app.tick(DASHBOARD_REFRESH);
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

        terminal
            .draw(|f| crate::tui::draw::draw(f, &mut app))
            .expect("Market dashboard should render");
        let dashboard = buffer_text(&terminal);
        assert!(dashboard.contains("portfolio"), "portfolio cube is drawn");
        assert!(dashboard.contains("movers"), "movers cube is drawn");
        assert!(dashboard.contains("alerts"), "alerts cube is drawn");
        assert!(dashboard.contains("breadth"), "breadth cube is drawn");
        assert!(dashboard.contains("most active"), "most-active cube is drawn");
        assert!(dashboard.contains("status"), "status cube is drawn");

        app.handle_key(KeyCode::Char('L'));
        terminal
            .draw(|f| crate::tui::draw::draw(f, &mut app))
            .expect("log panel should render after toggle");
        assert!(buffer_text(&terminal).contains("logs"), "logs panel is drawn");
    }

    fn buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    /// The value a dashboard cube would show for the sole holding.
    fn dashboard_market_value(app: &App) -> Option<f64> {
        crate::portfolio::portfolio_positions(
            &app.market.dashboard.holdings,
            &app.market.dashboard.rows,
        )[0]
            .market_value
    }

    #[tokio::test]
    async fn test_dashboard_snapshot_primes_on_first_tick() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![quote("AAPL", 110.0, 100.0)]));
        assert!(app.market.dashboard.rows.is_empty(), "empty before any tick");

        app.tick(Duration::from_millis(100));

        assert_eq!(app.market.dashboard.rows.len(), 1, "first tick primes the snapshot");
    }

    #[tokio::test]
    async fn test_dashboard_holds_value_until_refresh_interval() {
        let (mut app, _rx) = make_test_app();
        app.handle_event(AppEvent::PageLoaded(vec![quote("AAPL", 100.0, 100.0)]));
        app.handle_event(AppEvent::PortfolioLoaded(vec![holding_for("AAPL")]));
        app.tick(DASHBOARD_REFRESH);
        assert_eq!(dashboard_market_value(&app), Some(1000.0));

        app.handle_event(AppEvent::QuoteTick(price_tick("AAPL", Some(150.0))));
        assert_eq!(
            app.market.db_display.all_rows[0].price,
            Some(150.0),
            "live data updates immediately"
        );

        app.tick(Duration::from_secs(1));
        assert_eq!(
            dashboard_market_value(&app),
            Some(1000.0),
            "cube holds until the interval elapses"
        );

        app.tick(Duration::from_secs(2));
        assert_eq!(dashboard_market_value(&app), Some(1500.0), "10 shares * $150");
    }

    #[tokio::test]
    async fn test_dashboard_refresh_samples_priced_portfolio_value() {
        let (mut app, _rx) = make_test_app();
        app.holdings = vec![holding_for("AAPL")];
        app.handle_event(AppEvent::PageLoaded(vec![quote("AAPL", 110.0, 100.0)]));

        app.tick(Duration::from_millis(100));

        assert_eq!(app.market.portfolio_value_series.len(), 1);
        assert_eq!(app.market.portfolio_value_series[0], 1100.0);
    }

    #[tokio::test]
    async fn test_dashboard_refresh_skips_unpriced_portfolio() {
        let (mut app, _rx) = make_test_app();
        app.holdings = vec![holding_for("AAPL")];

        app.tick(Duration::from_millis(100));

        assert!(app.market.portfolio_value_series.is_empty());
    }

    #[tokio::test]
    async fn test_portfolio_value_series_caps_and_drops_oldest() {
        let (mut app, _rx) = make_test_app();
        app.market.portfolio_value_series = (0..PORTFOLIO_SERIES_CAP as u64)
            .map(|i| i as f64)
            .collect();
        app.holdings = vec![holding_for("AAPL")];
        app.handle_event(AppEvent::PageLoaded(vec![quote("AAPL", 110.0, 100.0)]));

        app.tick(Duration::from_millis(100));

        assert_eq!(app.market.portfolio_value_series.len(), PORTFOLIO_SERIES_CAP);
        assert_eq!(app.market.portfolio_value_series.front(), Some(&1.0));
        assert_eq!(app.market.portfolio_value_series.back(), Some(&1100.0));
    }

    #[tokio::test]
    async fn test_question_mark_opens_help_modal() {
        let (mut app, _rx) = make_test_app();
        app.handle_key(KeyCode::Char('?'));
        assert_eq!(app.overlay, Overlay::Help);
    }

    #[tokio::test]
    async fn test_esc_closes_help_modal() {
        let (mut app, _rx) = make_test_app();
        app.handle_key(KeyCode::Char('?'));
        app.handle_key(KeyCode::Esc);
        assert_eq!(app.overlay, Overlay::None);
    }

    #[tokio::test]
    async fn test_s_opens_info_modal() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('s'));
        assert_eq!(app.overlay, Overlay::Info);
    }

    #[tokio::test]
    async fn test_help_modal_disables_command_keys() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Char('?'));
        app.handle_key(KeyCode::Char('s'));
        assert_eq!(app.overlay, Overlay::Help);
    }

    #[tokio::test]
    async fn test_s_switches_from_chart_to_info() {
        let mut app = app_with_three_stocks();
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.overlay, Overlay::Chart);
        app.handle_key(KeyCode::Char('s'));
        assert_eq!(app.overlay, Overlay::Info);
    }

    /// Builds a priced quote for movers/alert-proximity tests.
    fn quote(ticker: &str, price: f64, previous_close: f64) -> QuoteRecord {
        QuoteRecord {
            ticker: Some(ticker.to_string()),
            price: Some(price),
            previous_close: Some(previous_close),
            ..Default::default()
        }
    }

    #[test]
    fn test_day_breadth_counts_advancers_and_decliners() {
        let rows = vec![
            quote("AAPL", 110.0, 100.0),
            quote("MSFT", 105.0, 100.0),
            quote("TSLA", 90.0, 100.0),
            quote("KO", 100.0, 100.0),
            named("NVDA", "NVIDIA Corp."),
        ];

        assert_eq!(crate::tui::draw::day_breadth(&rows), (2, 1));
    }

    #[test]
    fn test_most_active_picks_highest_volume() {
        let rows = vec![
            QuoteRecord {
                ticker: Some("AAPL".to_string()),
                day_volume: Some(62_400_000.0),
                ..Default::default()
            },
            QuoteRecord {
                ticker: Some("TSLA".to_string()),
                day_volume: Some(18_000_000.0),
                ..Default::default()
            },
        ];

        assert_eq!(
            crate::tui::draw::most_active(&rows),
            Some(("AAPL".to_string(), 62_400_000.0))
        );
    }

    #[test]
    fn test_most_active_is_none_without_positive_volume() {
        let rows = vec![
            QuoteRecord {
                ticker: Some("AAPL".to_string()),
                day_volume: Some(0.0),
                ..Default::default()
            },
            quote("TSLA", 90.0, 100.0),
        ];

        assert!(crate::tui::draw::most_active(&rows).is_none());
    }

    #[test]
    fn test_nearest_alert_picks_closest_to_threshold() {
        let rows = vec![quote("AAPL", 149.0, 148.0), quote("TSLA", 90.0, 92.0)];
        let alerts = vec![
            crate::models::Alert {
                id: 1,
                ticker: "AAPL".to_string(),
                direction: crate::models::AlertDirection::Above,
                threshold: 150.0,
                created_at: chrono::Utc::now(),
                triggered_at: None,
            },
            crate::models::Alert {
                id: 2,
                ticker: "TSLA".to_string(),
                direction: crate::models::AlertDirection::Below,
                threshold: 100.0,
                created_at: chrono::Utc::now(),
                triggered_at: None,
            },
        ];

        let nearest = crate::tui::draw::nearest_alert(&alerts, &rows).unwrap();

        assert_eq!(nearest.ticker, "AAPL");
    }

    #[test]
    fn test_nearest_alert_skips_unpriced_tickers() {
        let rows = vec![quote("TSLA", 90.0, 92.0)];
        let alerts = vec![
            alert_for(1, "AAPL"),
            crate::models::Alert {
                id: 2,
                ticker: "TSLA".to_string(),
                direction: crate::models::AlertDirection::Below,
                threshold: 100.0,
                created_at: chrono::Utc::now(),
                triggered_at: None,
            },
        ];

        let nearest = crate::tui::draw::nearest_alert(&alerts, &rows).unwrap();

        assert_eq!(nearest.ticker, "TSLA", "only the priced alert is rankable");
    }

    #[test]
    fn test_nearest_alert_is_none_when_no_alert_is_priced() {
        let alerts = vec![alert_for(1, "AAPL")];

        assert!(crate::tui::draw::nearest_alert(&alerts, &[]).is_none());
    }

    #[tokio::test]
    async fn test_l_toggles_logs_panel() {
        let (mut app, _rx) = make_test_app();
        assert!(!app.show_logs, "dashboard is the default view");

        app.handle_key(KeyCode::Char('L'));
        assert!(app.show_logs, "L shows the logs");

        app.handle_key(KeyCode::Char('L'));
        assert!(!app.show_logs, "L again returns to the dashboard");
    }
}

use std::time::Instant;

use ratatui::widgets::TableState;
use tokio::sync::mpsc;

use yfinance::{
    fetch::fetch_sorted,
    models::QuoteRecord,
    sort::{SortMode, SortOrder},
};

pub enum AppEvent {
    PageLoaded(Vec<QuoteRecord>),
    FetchSpawned(String),
    FetchCompleted(QuoteRecord),
    ChangeSortMode(SortMode),
    ChangeSortOrder(SortOrder),
    LogLine(String),
    Error(String),
}

pub struct App {
    pub input_mode: InputMode,
    pub should_quit: bool,
    pub db_display: DbDisplay,
    pub logs: Vec<String>,
    pub pool: sqlx::PgPool,
    pub event_tx: mpsc::UnboundedSender<AppEvent>,
    pub blink_state: bool,
    pub last_blink: Instant,
    pub stock_modal: StockInfoModal,
}

pub struct InputMode {
    pub input: String,
    pub toggled: bool,
}

pub struct DbDisplay {
    pub rows: Vec<QuoteRecord>,
    pub table_state: TableState,
    pub status: String,
    pub sort_mode: SortMode,
    pub sort_order: SortOrder,
}

pub struct StockInfoModal {
    pub stock: QuoteRecord,
    pub visible: bool,
}

impl App {
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
                visible: false,
            },
        }
    }

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
                let name = record.ticker.clone().unwrap_or_else(|| "?".to_string());
                self.db_display.status = format!("stored {name}");
                self.logs.push(format!("[SUCCESS] stored {name}"));
                // Newest on top, matching the fetch_recent ordering.
                self.db_display.rows.insert(0, record);
            }
            AppEvent::LogLine(line) => {
                self.logs.push(line);
            }
            AppEvent::Error(e) => {
                self.db_display.status = format!("Error: {e}");
                self.logs.push(format!("[ERROR] {e}"));
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
        }
    }

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

    pub fn handle_command_key(&mut self, key: char) {
        match key.to_ascii_lowercase() {
            'q' => self.should_quit = true,
            'i' => self.input_mode.toggled = !self.input_mode.toggled,
            'd' | 'p' | 'c' | 'v' | 'a' | 'n' => {
                let mode = match key.to_ascii_lowercase() {
                    'd' => SortMode::ById,
                    'p' => SortMode::ByPrice,
                    'c' => SortMode::ByPrevClose,
                    'v' => SortMode::ByVolume,
                    'a' => SortMode::ByAsOf,
                    'n' => SortMode::ByTicker,
                    _ => unreachable!(),
                };
                self.handle_event(AppEvent::ChangeSortMode(mode));
            }
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
                    self.stock_modal.stock = QuoteRecord {
                        id: row.id,
                        ticker: row.ticker.clone(),
                        price: row.price,
                        previous_close: row.previous_close,
                        day_volume: row.day_volume,
                        as_of: row.as_of,
                    };
                    self.stock_modal.visible = true;
                }
            }
            _ => {}
        }
    }
}

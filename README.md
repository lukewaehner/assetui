# assetui

A Rust tool for pulling stock quotes from Yahoo Finance and keeping a local history in Postgres. Two ways to use it: a simple CLI for batch operations and a ratatui TUI for browsing what you've collected.

## What it does

Every fetch stores a timestamped snapshot of a quote (price, previous close, volume) in Postgres. Duplicate fetches - same ticker at the same `as_of` timestamp - are silently ignored, so you can run it on a schedule without cluttering the table.

The TUI loads your most recent 200 quotes on startup, pages through them, and lets you sort by any column. Four modals cover the selected stock: an info modal (press `s`) with analyst consensus and price targets, a chart modal (press `Enter`) that plots recent price history, an options modal (press `x`) that shows the option chain for a selectable expiry with side-by-side calls and puts, each contract's implied volatility, and a Black-Scholes theoretical price and Greeks, and an earnings modal (press `e`) with quarterly EPS estimates vs actuals and their surprise, a revenue trend, and the next scheduled report date. While you're browsing, the rows on the current page update live from Yahoo's streaming feed - price cells flash green or red in the tick direction - and the colour theme follows your macOS light/dark setting.

## Setup

You need Postgres running and [sqlx-cli](https://github.com/launchbr/sqlx-cli) installed for the migrations.

Create an empty database, and add the database connection string to a `.env` file at the project root

```
DATABASE_URL=postgres://<user>:<pass>@localhost/<database_name>
```

Put that in a `.env` file you just created at the project root, then optionally run migrations (running the app will run migrations):

```sh
sqlx migrate run
```

Build the binary:

```sh
cargo build --release
```

### Using the bundled Postgres

If you'd rather not install Postgres, `compose.yaml` brings one up:

```sh
docker compose up --wait
```

It publishes on **5433**, not 5432, so it can't collide with a Postgres.app or
Homebrew server already holding the default port:

```
DATABASE_URL=postgres://yfinance:yfinance@localhost:5433/yfinance_dev
```

### Tests

```sh
cargo test
```

The suite needs a reachable `DATABASE_URL` whose role can create databases:
`#[sqlx::test]` provisions a fresh database per test, applies `migrations/`, and
drops it afterwards, so tests are isolated and order-independent.

Building does *not* need a database. The one compile-time-checked query
(`sqlx::query!`) resolves against the committed `.sqlx/` query data, so
`SQLX_OFFLINE=true cargo build` works from a clean checkout. If you add or edit
a `query!`, regenerate it:

```sh
cargo sqlx prepare -- --all-targets
```

CI (`.github/workflows/ci.yml`) gates on `cargo fmt --check`, `cargo clippy -D
warnings`, `cargo sqlx prepare --check`, and the full suite against a Postgres
service container.

## TUI

The TUI is the default: run with no subcommand to launch it.

```sh
cargo run
```

### Keys

| Key | Action |
|-----|--------|
| `i` | Toggle ticker input (type a symbol, press Enter to fetch) |
| `f` | Add or remove a ticker to the favorites watchlist  |
| `/` | Fuzzy search: filter the table by ticker or company name |
| `j` / `k` | Navigate rows |
| `h` / `l` | Previous / next page |
| `s` | Open the info modal |
| `x` | Open the options modal (chain, IV, and Greeks) for the selected row |
| `e` | Open the earnings modal (EPS surprise, revenue trend, next report) for the selected row |
| `Tab` | Open the page picker (Market / Manage / Sectors / News / Optimizer) |
| `r` | On the Optimizer page: refetch history and recompute |
| `?` | Open the help modal |
| `Enter` | Open the chart modal (price history) for the selected row |
| `Esc` | Close modal, cancel search, or exit input mode |
| `o` | Toggle sort direction |
| `d` / `t` / `n` | Sort by ID / Ticker / Name |
| `p` / `c` / `v` / `a` | Sort by Price / Prev Close / Volume / As Of |
| `q` | Quit |

`?` and `Enter` work whether or not a modal is already open, so you can flip straight between the info and chart views without closing first. `Esc` closes whichever modal is showing.

### The Manage page

Press `Tab` to switch from the Market page (quotes) to the Manage page, where you keep your holdings and alerts. It has two sections — Holdings and Alerts — navigated together with `j`/`k`; the section your cursor is in is where actions apply. Press `a` to add (an inline line accepts the whole entry — `AAPL 10 150.50` for a holding, `AAPL >150` for an alert), `e` to edit the selected holding, and `d` to delete the selected row. `Tab` (or `Esc`) returns to the Market page.

**Holdings** show one row per position — shares, average cost, current price, market value, unrealized P&L (\$ and %), and day-change — with a totals line, coloured green/red by gain/loss. Prices come from the quotes already in memory, so a holding whose ticker isn't loaded shows `-` rather than a misleading `$0`, and P&L updates live as ticks land. This is a manual, display-only tracker — no brokerage connection, unrealized P&L only, single-currency.

**Alerts** fire once when a live tick crosses the threshold (`>150` / `<100`): a toast pops and the alert clears itself. Armed tickers show a `!` in the Market gutter. Two limitations: alerts are evaluated only against the live stream, which covers the tickers on the **current page**, so an off-screen ticker's alert won't fire until it's visible and streaming; and ticks only flow while the market is open.

### The Optimizer page

The **Optimizer** page turns your holdings into a mean-variance analysis: a Monte Carlo cloud of thousands of random long-only portfolios plotted in risk/return space, with the analytical tangency (max-Sharpe, `★`) and minimum-variance (`◇`) portfolios overlaid and your current allocation (`●`) marked, plus a correlation heatmap across your holdings. All the math is a pure, dependency-free core (hand-rolled covariance inverse and a seeded, reproducible sampler).

### Fuzzy search

Press `/` and start typing to filter the table live. Matching is a case-insensitive subsequence over both the ticker and the company name - `apl` finds **A**A**PL**, `nvc` finds **NV**IDIA **C**orp. - and the matched characters are highlighted in the table. Press `Enter` to keep the filter applied while you navigate, sort, or open modals over the narrowed rows (the status bar switches to `FILTER` and shows `matched/total`); press `Esc` at any point to clear it. Changing the sort re-fetches from the database but keeps your filter applied.

The left panel shows what you're typing and a running log of fetch activity. The table border turns cyan to show which side is active, and the status bar tracks the quote count and current page. Fetch and stream errors surface as slide-in notifications in the top-right corner.

### Live quotes

While the TUI is open it subscribes to a live quote stream for the tickers on the current page and updates their rows in place - price, previous close, volume, and timestamp - as ticks arrive. Each update briefly flashes the price cell green or red in the direction of the tick; once the flash fades, the cell colour falls back to price versus previous close. Paginating or changing the sort re-subscribes to the newly visible symbols, and the stream is stopped cleanly when you quit.

Updates are display-only; streamed ticks are not written to Postgres. Ticks only flow while the market is open, so nights, weekends, and holidays show no live movement. The log panel prints `[STREAM] connected: N tickers` when the stream attaches, so you can tell "connected but idle" apart from "not started".

### Appearance

On macOS the TUI follows your system appearance: it picks a light or dark palette at launch and switches automatically within a couple of seconds when you toggle the OS between Light and Dark - no key or restart needed. On other platforms it defaults to the dark palette.

## CLI

For batch operations, pass a subcommand instead of launching the TUI:

```sh
cargo run -- fetch-and-store --ticker AAPL,MSFT,GOOG   # fetch quotes in parallel and store them
cargo run -- dump-to-csv                               # write the quotes table to quotes_dump.csv
cargo run -- pull-from-db                              # print a formatted table of everything stored
```

Tickers can also be passed as `-t`. Log verbosity is controlled via `RUST_LOG` (defaults to `info`); the TUI captures its own logs in-app instead.

## Project layout

```
src/
  lib.rs              library root
  main.rs             binary entry point (TUI default + CLI subcommands)
  models.rs           QuoteRecord, QuoteRecordAnalysis, QuoteTick
  fetch.rs            Yahoo Finance API calls
  run.rs              parallel fetch pipeline (CLI path)
  search.rs           fuzzy subsequence matching
  sort.rs             SortMode / SortOrder enums
  cli/                stdin prompts and comfy-table rendering
  db/                 Postgres pool setup and queries
  tui/                ratatui interface (default mode)
    run.rs                 terminal setup and event loop
    app.rs, app/tasks.rs   state, key handling, background tasks
    draw.rs                table, modals, chart, and status-bar rendering
    stream.rs              live quote stream adapter (websocket + fallback)
    theme.rs               light/dark palettes and macOS appearance detection
migrations/           sqlx migration files
```

## Notes

The Yahoo Finance data comes from [yfinance-rs](https://crates.io/crates/yfinance-rs). It's not an official API, so availability depends on what Yahoo exposes. Extended-hours quotes may have `None` fields depending on market status.

Price cells are coloured in both interfaces: green when above the previous close, red when below. In the TUI a live tick briefly overrides that with a flash in the tick's direction, and the info modal also shows the comparison numerically.

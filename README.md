# yfinance

A Rust tool for pulling stock quotes from Yahoo Finance and keeping a local history in Postgres. Two ways to use it: a simple CLI for batch operations and a ratatui TUI for browsing what you've collected.

## What it does

Every fetch stores a timestamped snapshot of a quote (price, previous close, volume) in Postgres. Duplicate fetches - same ticker at the same `as_of` timestamp - are silently ignored, so you can run it on a schedule without cluttering the table.

The TUI loads your most recent 200 quotes on startup, pages through them, and lets you sort by any column. Two modals cover the selected stock: an info modal (press `?`) with analyst consensus and price targets, and a chart modal (press `Enter`) that plots recent price history. While you're browsing, the rows on the current page update live from Yahoo's streaming feed - price cells flash green or red in the tick direction - and the colour theme follows your macOS light/dark setting.

## Setup

You need Postgres running and [sqlx-cli](https://github.com/launchbr/sqlx-cli) installed for the migrations.

```
DATABASE_URL=postgres://user:pass@localhost/yfinance
```

Put that in a `.env` file at the project root, then run migrations:

```sh
sqlx migrate run
```

Build both binaries:

```sh
cargo build --release
```

## CLI

```sh
cargo run
```

Prompts you to pick a mode:

1. **Fetch and store** - enter comma-separated tickers (e.g. `AAPL,MSFT,GOOG`), fetches them in parallel
2. **Dump to CSV** - writes the full quotes table to `quotes_dump_YYYYMMDDHHMMSS.csv`
3. **Pull from DB** - prints a formatted table of everything stored

Log verbosity is controlled via `RUST_LOG` (defaults to `info`).

## TUI

```sh
cargo run --bin tui
```

### Keys

| Key | Action |
|-----|--------|
| `i` | Toggle ticker input (type a symbol, press Enter to fetch) |
| `/` | Fuzzy search: filter the table by ticker or company name |
| `j` / `k` | Navigate rows |
| `h` / `l` | Previous / next page |
| `?` | Open the info modal (analyst consensus + price targets) |
| `Enter` | Open the chart modal (price history) for the selected row |
| `Esc` | Close modal, cancel search, or exit input mode |
| `o` | Toggle sort direction |
| `d` / `t` / `n` | Sort by ID / Ticker / Name |
| `p` / `c` / `v` / `a` | Sort by Price / Prev Close / Volume / As Of |
| `q` | Quit |

`?` and `Enter` work whether or not a modal is already open, so you can flip straight between the info and chart views without closing first. `Esc` closes whichever modal is showing.

### Fuzzy search

Press `/` and start typing to filter the table live. Matching is a case-insensitive subsequence over both the ticker and the company name - `apl` finds **A**A**PL**, `nvc` finds **NV**IDIA **C**orp. - and the matched characters are highlighted in the table. Press `Enter` to keep the filter applied while you navigate, sort, or open modals over the narrowed rows (the status bar switches to `FILTER` and shows `matched/total`); press `Esc` at any point to clear it. Changing the sort re-fetches from the database but keeps your filter applied.

The left panel shows what you're typing and a running log of fetch activity. The table border turns cyan to show which side is active, and the status bar tracks the quote count and current page. Fetch and stream errors surface as slide-in notifications in the top-right corner.

### Live quotes

While the TUI is open it subscribes to a live quote stream for the tickers on the current page and updates their rows in place - price, previous close, volume, and timestamp - as ticks arrive. Each update briefly flashes the price cell green or red in the direction of the tick; once the flash fades, the cell colour falls back to price versus previous close. Paginating or changing the sort re-subscribes to the newly visible symbols, and the stream is stopped cleanly when you quit.

Updates are display-only; streamed ticks are not written to Postgres. Ticks only flow while the market is open, so nights, weekends, and holidays show no live movement. The log panel prints `[STREAM] connected: N tickers` when the stream attaches, so you can tell "connected but idle" apart from "not started".

### Appearance

On macOS the TUI follows your system appearance: it picks a light or dark palette at launch and switches automatically within a couple of seconds when you toggle the OS between Light and Dark - no key or restart needed. On other platforms it defaults to the dark palette.

## Project layout

```
src/
  lib.rs              library root
  main.rs             CLI binary entry point
  models.rs           QuoteRecord, QuoteRecordAnalysis, QuoteTick
  fetch.rs            Yahoo Finance API calls
  stream.rs           live quote stream adapter (websocket + fallback)
  run.rs              parallel fetch pipeline (CLI path)
  sort.rs             SortMode / SortOrder enums
  cli/                stdin prompts and comfy-table rendering
  db/                 Postgres pool setup and queries
  bin/tui/            ratatui TUI binary
    app.rs, app/tasks.rs   state, key handling, background tasks
    draw.rs                table, modals, chart, and status-bar rendering
    theme.rs               light/dark palettes and macOS appearance detection
migrations/           sqlx migration files
```

## Notes

The Yahoo Finance data comes from [yfinance-rs](https://crates.io/crates/yfinance-rs). It's not an official API, so availability depends on what Yahoo exposes. Extended-hours quotes may have `None` fields depending on market status.

Price cells are coloured in both interfaces: green when above the previous close, red when below. In the TUI a live tick briefly overrides that with a flash in the tick's direction, and the info modal also shows the comparison numerically.

-- Store portfolio money and share counts as NUMERIC rather than DOUBLE PRECISION.
--
-- These four columns are different in kind from `quotes.price` and friends.
-- Quote columns hold market data that arrives from Yahoo as floats, so `f64` is
-- the honest representation. These hold a cost basis the user typed and totals
-- accumulated from repeated addition — where binary floating point drifts and,
-- once written, drifts permanently. A real row from this table read
-- `market_value = 2811.8999999999996`.
--
-- Scale 8 is deliberate: enough for fractional-share brokerages (which quote to
-- 6-8 places) with room to spare, while still rounding away float noise. The
-- USING casts inherit that rounding, so the value above becomes 2811.90000000
-- rather than being carried over verbatim.
--
-- Precision 20 leaves 12 integer digits — a trillion dollars — which is not a
-- limit worth worrying about for a single-user tracker.
--
-- The Rust side converts at the database boundary only (see `db::numeric`); the
-- analytics core stays on `f64`.
ALTER TABLE portfolio_holdings
    ALTER COLUMN shares TYPE NUMERIC(20, 8) USING shares::numeric,
    ALTER COLUMN avg_cost TYPE NUMERIC(20, 8) USING avg_cost::numeric;

ALTER TABLE portfolio_value_history
    ALTER COLUMN market_value TYPE NUMERIC(20, 8) USING market_value::numeric,
    ALTER COLUMN cost_basis TYPE NUMERIC(20, 8) USING cost_basis::numeric;

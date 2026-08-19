-- Drop the per-column indexes on `quotes`, which no query can use.
--
-- `20260629000001_index_all_quote_fields.sql` added one index per sortable
-- column on the theory that the table's sort modes would use them. They can't:
-- every page query wraps a `DISTINCT ON (ticker) ... ORDER BY ticker, as_of DESC`
-- subquery and sorts *its output*, so the ordering is applied to a derived table
-- rather than to `quotes`. An index on `quotes.price` is unreachable from there
-- at any table size.
--
-- Verified rather than assumed, on a seeded copy with 400k rows over 2000
-- tickers (large enough that the planner prefers indexes where it can use one):
--
--   * All five sort modes (ticker/name/price/previous_close/volume/as_of) plan
--     as `Index Scan using quotes_ticker_as_of_idx -> Unique -> top-N heapsort`.
--     None of the dropped indexes appears in any plan.
--   * The other queries — the retention prune's DELETE, the sparkline series,
--     and the per-ticker latest lookup — all resolve to
--     `quotes_ticker_as_of_idx` too.
--   * `pg_stat_user_indexes` on a real database showed 0 scans for the price,
--     previous_close, and day_volume indexes.
--
-- What they did cost: ~40% on write throughput (50k-row insert, 760ms -> 455ms
-- median) and ~23 MB of index for a 400k-row table. Every fetch pays that.
--
-- `quotes_as_of_idx` goes too. It had accumulated scans on a live database, but
-- no current query can use it — those predate the DISTINCT ON rewrite, when the
-- page query ordered `quotes` directly. If a cross-ticker `ORDER BY as_of` query
-- reappears, add it back then.
--
-- Kept: `quotes_ticker_as_of_idx` (serves every query), the
-- `quotes_ticker_as_of_key` unique constraint (dedup on insert), and the
-- primary key.
DROP INDEX IF EXISTS quotes_name_idx;
DROP INDEX IF EXISTS quotes_price_idx;
DROP INDEX IF EXISTS quotes_previous_close_idx;
DROP INDEX IF EXISTS quotes_day_volume_idx;
DROP INDEX IF EXISTS quotes_as_of_idx;

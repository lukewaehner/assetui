use yfinance_rs::screener::EquitySector;

/// How many stocks (top by market cap) each sector keeps for the treemap.
pub const SECTOR_TOP_N: usize = 20;

/// One stock's tile in the treemap: sized by `market_cap`, colored by `day_pct`.
#[derive(Debug, Clone, PartialEq)]
pub struct SectorTile {
    /// Ticker symbol (e.g. `"AAPL"`).
    pub ticker: String,
    /// Company display name.
    pub name: String,
    /// Market capitalisation in dollars.
    pub market_cap: f64,
    /// Regular-market day change percent.
    pub day_pct: f64,
}

/// One sector's stocks, sized by the sum of their market caps.
#[derive(Debug, Clone, PartialEq)]
pub struct SectorGroup {
    /// Which of the 11 equity sectors this group is.
    pub sector: EquitySector,
    /// Tiles, sorted biggest market cap first, truncated to [`SECTOR_TOP_N`].
    pub tiles: Vec<SectorTile>,
    pub total_market_cap: f64,
}

/// The full treemap: one group per sector that has loaded.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SectorMap {
    /// Loaded sector groups, in arrival order.
    pub groups: Vec<SectorGroup>,
}

/// Builds a treemap tile from the raw optional fields a screener row carries.
pub fn make_tile(
    ticker: Option<String>,
    name: Option<String>,
    market_cap: Option<f64>,
    day_pct: Option<f64>,
) -> Option<SectorTile> {
    let ticker = ticker?;
    let market_cap = market_cap.filter(|v| v.is_finite())?;
    let day_pct = day_pct.filter(|v| v.is_finite())?;
    Some(SectorTile {
        ticker,
        name: name.unwrap_or_default(),
        market_cap,
        day_pct,
    })
}

/// Sorts tiles biggest market cap first and truncates to `limit`.
pub fn rank_tiles(mut tiles: Vec<SectorTile>, limit: usize) -> Vec<SectorTile> {
    tiles.sort_by(|a, b| {
        b.market_cap
            .partial_cmp(&a.market_cap)
            .expect("finite market_cap")
    });
    tiles.truncate(limit);
    tiles
}

impl SectorMap {
    /// Inserts or replaces the group for `sector`, recomputing its total market cap.
    pub fn upsert(&mut self, sector: EquitySector, tiles: Vec<SectorTile>) {
        let total_market_cap = tiles.iter().map(|t| t.market_cap).sum();
        let group = SectorGroup {
            sector,
            tiles,
            total_market_cap,
        };
        match self.groups.iter_mut().find(|g| g.sector == sector) {
            Some(existing) => *existing = group,
            None => self.groups.push(group),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(ticker: &str, market_cap: f64) -> SectorTile {
        SectorTile {
            ticker: ticker.into(),
            name: format!("{ticker} Inc."),
            market_cap,
            day_pct: 0.0,
        }
    }

    #[test]
    fn make_tile_builds_a_tile_when_all_fields_present() {
        let t = make_tile(Some("AAPL".into()), Some("Apple".into()), Some(100.0), Some(1.5));
        assert_eq!(
            t,
            Some(SectorTile { ticker: "AAPL".into(), name: "Apple".into(), market_cap: 100.0, day_pct: 1.5 })
        );
    }

    #[test]
    fn make_tile_is_none_when_market_cap_missing() {
        assert!(make_tile(Some("AAPL".into()), Some("Apple".into()), None, Some(1.5)).is_none());
    }

    #[test]
    fn make_tile_is_none_when_change_missing() {
        assert!(make_tile(Some("AAPL".into()), Some("Apple".into()), Some(100.0), None).is_none());
    }

    #[test]
    fn make_tile_is_none_when_ticker_missing() {
        assert!(make_tile(None, Some("Apple".into()), Some(100.0), Some(1.5)).is_none());
    }

    #[test]
    fn make_tile_is_none_on_non_finite_values() {
        assert!(make_tile(Some("X".into()), None, Some(f64::NAN), Some(1.0)).is_none());
        assert!(make_tile(Some("X".into()), None, Some(100.0), Some(f64::INFINITY)).is_none());
    }

    #[test]
    fn rank_tiles_sorts_by_market_cap_desc() {
        let tiles = vec![tile("SMALL", 10.0), tile("BIG", 100.0), tile("MID", 50.0)];
        let ranked = rank_tiles(tiles, 10);
        let tickers: Vec<&str> = ranked.iter().map(|t| t.ticker.as_str()).collect();
        assert_eq!(tickers, vec!["BIG", "MID", "SMALL"]);
    }

    #[test]
    fn rank_tiles_truncates_to_limit() {
        let tiles = vec![tile("A", 100.0), tile("B", 90.0), tile("C", 80.0)];
        let ranked = rank_tiles(tiles, 2);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].ticker, "A");
        assert_eq!(ranked[1].ticker, "B");
    }

    #[test]
    fn rank_tiles_empty_input_is_empty() {
        assert!(rank_tiles(Vec::new(), 10).is_empty());
    }
}

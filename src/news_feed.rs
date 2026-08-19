use std::collections::HashSet;

use crate::models::quote::NewsItem;

#[cfg(test)]
use chrono::DateTime;

/// How many watchlist tickers (sorted) the feed fetches news for.
pub const NEWS_FEED_MAX_TICKERS: usize = 6;

/// One headline tagged with the watchlist ticker it was fetched for.
#[derive(Debug, Clone)]
pub struct FeedItem {
    pub ticker: String,
    pub item: NewsItem,
}

/// The shared feed plus its load bookkeeping.
#[derive(Default)]
pub struct NewsFeedState {
    /// Merged headlines, most-recent first.
    pub items: Vec<FeedItem>,
    /// True while any per-ticker fetch is outstanding.
    pub loading: bool,
    /// Set once the first fetch has been kicked off.
    pub requested: bool,
    /// Outstanding per-ticker fetches.
    pub pending: usize,
}

pub fn feed_source_tickers(watchlist: &HashSet<String>) -> Vec<String> {
    let mut tickers: Vec<String> = watchlist.iter().cloned().collect();
    tickers.sort();
    tickers.truncate(NEWS_FEED_MAX_TICKERS);
    tickers
}

pub fn merge_feed(
    mut existing: Vec<FeedItem>,
    ticker: &str,
    items: Vec<NewsItem>,
) -> Vec<FeedItem> {
    for item in items {
        let key = feed_dedup_key(&item);
        if existing.iter().any(|f| feed_dedup_key(&f.item) == key) {
            continue;
        }
        existing.push(FeedItem {
            ticker: ticker.to_string(),
            item,
        });
    }
    existing.sort_by_key(|item| std::cmp::Reverse(item.item.published_at));
    existing
}

/// Dedup key for a headline: its link when usable, else its lowercased title.
fn feed_dedup_key(item: &NewsItem) -> String {
    match &item.link {
        Some(link) if !link.is_empty() => link.clone(),
        _ => item.title.to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(title: &str, link: Option<&str>, secs: Option<i64>) -> NewsItem {
        NewsItem {
            title: title.to_string(),
            publisher: None,
            published_at: secs.map(|s| DateTime::from_timestamp(s, 0).unwrap()),
            link: link.map(str::to_string),
        }
    }

    #[test]
    fn feed_source_tickers_sorts_and_caps() {
        let wl: HashSet<String> = ["MSFT", "AAPL", "TSLA", "NVDA", "AMZN", "GOOG", "META"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let got = feed_source_tickers(&wl);
        assert_eq!(got, vec!["AAPL", "AMZN", "GOOG", "META", "MSFT", "NVDA"]);
    }

    #[test]
    fn merge_feed_tags_items_with_their_ticker() {
        let out = merge_feed(
            Vec::new(),
            "AAPL",
            vec![item("A", Some("http://a"), Some(1))],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].ticker, "AAPL");
    }

    #[test]
    fn merge_feed_dedupes_by_link() {
        let existing = merge_feed(
            Vec::new(),
            "AAPL",
            vec![item("Story", Some("http://x"), Some(2))],
        );
        let out = merge_feed(
            existing,
            "MSFT",
            vec![item("Same story, other title", Some("http://x"), Some(3))],
        );
        assert_eq!(out.len(), 1, "same link is one item");
    }

    #[test]
    fn merge_feed_dedupes_by_title_when_link_absent() {
        let existing = merge_feed(Vec::new(), "AAPL", vec![item("Repeat", None, Some(2))]);
        let out = merge_feed(existing, "MSFT", vec![item("repeat", None, Some(3))]);
        assert_eq!(out.len(), 1, "case-insensitive title dedupe when no link");
    }

    #[test]
    fn merge_feed_sorts_most_recent_first() {
        let out = merge_feed(
            Vec::new(),
            "AAPL",
            vec![
                item("old", Some("o"), Some(10)),
                item("new", Some("n"), Some(20)),
            ],
        );
        assert_eq!(out[0].item.title, "new");
        assert_eq!(out[1].item.title, "old");
    }

    #[test]
    fn merge_feed_puts_undated_items_last() {
        let out = merge_feed(
            Vec::new(),
            "AAPL",
            vec![
                item("undated", Some("u"), None),
                item("dated", Some("d"), Some(5)),
            ],
        );
        assert_eq!(out[0].item.title, "dated");
        assert_eq!(out[1].item.title, "undated");
    }
}

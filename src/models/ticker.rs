//! A ticker symbol in canonical (uppercased) form.

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ticker(String);

impl Ticker {
    /// Builds a `Ticker`, ASCII-uppercasing `symbol`.
    pub fn new(symbol: &str) -> Self {
        Self(symbol.to_ascii_uppercase())
    }

    /// The canonical (uppercased) symbol.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Ticker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Splits a Manage-page entry into its leading ticker token.
pub(crate) fn split_ticker_entry(input: &str) -> Option<(String, &str)> {
    let (ticker, rest) = input.trim().split_once(char::is_whitespace)?;
    Some((ticker.to_uppercase(), rest))
}

#[cfg(test)]
mod tests {
    use super::Ticker;
    use std::collections::HashMap;

    #[test]
    fn test_new_uppercases_symbol() {
        // arrange
        let ticker = Ticker::new("aapl");

        // assert
        assert_eq!(ticker.as_str(), "AAPL");
    }

    /// Two tickers built from differently-cased spellings are equal.
    #[test]
    fn test_equality_is_case_insensitive_by_construction() {
        assert_eq!(Ticker::new("aapl"), Ticker::new("AAPL"));
    }

    #[test]
    fn test_as_str_round_trips_uppercase_input() {
        assert_eq!(Ticker::new("MSFT").as_str(), "MSFT");
    }

    #[test]
    fn test_display_renders_uppercased_symbol() {
        assert_eq!(format!("{}", Ticker::new("nvda")), "NVDA");
    }

    #[test]
    fn test_new_does_not_trim_whitespace() {
        assert_eq!(Ticker::new(" aapl ").as_str(), " AAPL ");
    }

    #[test]
    fn test_usable_as_case_insensitive_map_key() {
        // arrange
        let mut map: HashMap<Ticker, i32> = HashMap::new();
        map.insert(Ticker::new("aapl"), 1);

        // act
        assert_eq!(map.get(&Ticker::new("AAPL")), Some(&1));
    }

    #[test]
    fn test_split_ticker_entry_uppercases_and_returns_rest() {
        assert_eq!(
            super::split_ticker_entry("  aapl 10 150 "),
            Some(("AAPL".to_string(), "10 150"))
        );
    }

    #[test]
    fn test_split_ticker_entry_without_separator_is_none() {
        assert_eq!(super::split_ticker_entry("  aapl  "), None);
    }
}

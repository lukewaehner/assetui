//! Ticker-symbol parsing for the CLI binary.

pub fn parse_tickers(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::parse_tickers;

    #[test]
    fn test_parse_tickers_normalises_case_and_skips_blanks() {
        assert_eq!(
            parse_tickers("aapl,msft, ,goog"),
            vec!["AAPL", "MSFT", "GOOG"]
        );
    }

    #[test]
    fn test_parse_tickers_empty_input() {
        assert!(parse_tickers("").is_empty());
        assert!(parse_tickers(" , ,").is_empty());
    }

    #[test]
    fn test_parse_tickers_single_symbol_trimmed() {
        assert_eq!(parse_tickers("  nvda \n"), vec!["NVDA"]);
    }
}

//! Interactive stdin prompts for the CLI binary.

use std::io::{self, BufRead};

use crate::AppError;

/// The three operations the CLI binary supports.
pub enum Mode {
    /// Fetch live quotes from Yahoo Finance and write them to the database.
    FetchAndStore,
    /// Serialise every row in the quotes table to a timestamped CSV file.
    DumpToCsv,
    /// Read quotes from the database and print them as a formatted table.
    PullFromDb,
}

/// Reads one line from stdin, treating end-of-input (closed stdin, Ctrl-D) as
/// an `UnexpectedEof` error rather than an empty line, so prompt loops
/// terminate instead of spinning forever.
fn read_line() -> io::Result<String> {
    let mut line = String::new();
    if io::stdin().lock().read_line(&mut line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "stdin closed before input was complete",
        ));
    }
    Ok(line)
}

/// Splits comma-separated input into normalised ticker symbols: trimmed,
/// uppercased, with empty entries discarded.
pub fn parse_tickers(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Prompts the user to pick one of the three [`Mode`] options and returns
/// the selection.  Loops until a valid option is entered; returns an error
/// if stdin closes first.
pub fn select_mode() -> Result<Mode, AppError> {
    println!("Select mode:");
    println!("1. Fetch and store quotes");
    println!("2. Dump quotes table to CSV");
    println!("3. Pull quotes from DB and display");
    loop {
        match read_line()?.trim() {
            "1" => return Ok(Mode::FetchAndStore),
            "2" => return Ok(Mode::DumpToCsv),
            "3" => return Ok(Mode::PullFromDb),
            _ => println!("Invalid selection, please enter 1, 2, or 3:"),
        }
    }
}

/// Prompts the user to enter comma-separated ticker symbols and confirms
/// when they are finished.
///
/// Tickers are normalised via [`parse_tickers`].  The loop continues asking
/// "are you done?" until the user answers `y` or `yes`, so multiple rounds of
/// entry are possible in one session.  Returns an error if stdin closes.
pub fn pick_tickers() -> Result<Vec<String>, AppError> {
    let mut stocks: Vec<String> = Vec::new();
    loop {
        println!("Enter stock tickers separated by commas (e.g. AAPL,MSFT,GOOG):");
        let entered = parse_tickers(&read_line()?);
        if entered.is_empty() {
            println!("No valid tickers entered, please try again.");
            continue;
        }
        stocks.extend(entered);
        println!("Are you done entering tickers? (y/n):");
        if matches!(read_line()?.trim().to_lowercase().as_str(), "y" | "yes") {
            return Ok(stocks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Mode, parse_tickers};

    #[test]
    fn test_mode_variants_exist() {
        let _fetch = Mode::FetchAndStore;
        let _dump = Mode::DumpToCsv;
        let _pull = Mode::PullFromDb;
    }

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

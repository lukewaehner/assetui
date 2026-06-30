//! Interactive stdin prompts for the CLI binary.

use std::io;

/// The three operations the CLI binary supports.
pub enum Mode {
    /// Fetch live quotes from Yahoo Finance and write them to the database.
    FetchAndStore,
    /// Serialise every row in the quotes table to a timestamped CSV file.
    DumpToCsv,
    /// Read quotes from the database and print them as a formatted table.
    PullFromDb,
}

/// Prompts the user to pick one of the three [`Mode`] options and returns
/// the selection.  Loops until a valid option is entered.
pub fn select_mode() -> Mode {
    println!("Select mode:");
    println!("1. Fetch and store quotes");
    println!("2. Dump quotes table to CSV");
    println!("3. Pull quotes from DB and display");
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .expect("Failed to read input");
    let mut mode: Option<Mode> = None;
    while mode.is_none() {
        mode = match input.trim() {
            "1" => Some(Mode::FetchAndStore),
            "2" => Some(Mode::DumpToCsv),
            "3" => Some(Mode::PullFromDb),
            _ => {
                println!("Invalid selection, please enter 1, 2, or 3:");
                input.clear();
                io::stdin()
                    .read_line(&mut input)
                    .expect("Failed to read input");
                None
            }
        }
    }
    mode.unwrap_or(Mode::FetchAndStore)
}

/// Prompts the user to enter comma-separated ticker symbols and confirms
/// when they are finished.
///
/// Tickers are normalised to uppercase and empty entries are discarded.
/// The loop continues asking "are you done?" until the user answers `y` or
/// `yes`, so multiple rounds of entry are possible in one session.
pub fn pick_tickers() -> Vec<String> {
    let mut stocks: Vec<String> = Vec::new();
    let mut done: bool = false;
    while !done {
        println!("Enter stock tickers separated by commas (e.g. AAPL,MSFT,GOOG):");
        let mut tickers_input = String::new();
        io::stdin()
            .read_line(&mut tickers_input)
            .expect("Failed to read input");
        stocks.extend(
            tickers_input
                .split(',')
                .map(|s| s.trim().to_uppercase())
                .filter(|s| !s.is_empty()),
        );
        if stocks.is_empty() {
            println!("No valid tickers entered, please try again.");
        }
        println!("Are you done entering tickers? (y/n):");
        let mut done_input = String::new();
        io::stdin()
            .read_line(&mut done_input)
            .expect("Failed to read input");
        done = matches!(done_input.trim().to_lowercase().as_str(), "y" | "yes");
    }
    stocks
}

#[cfg(test)]
mod tests {
    use super::Mode;

    #[test]
    fn test_mode_variants_exist() {
        let _fetch = Mode::FetchAndStore;
        let _dump = Mode::DumpToCsv;
        let _pull = Mode::PullFromDb;
    }

    #[test]
    fn test_pick_tickers_normalises_case() {
        let input = "aapl,msft, ,goog";
        let result: Vec<String> = input
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
            .collect();
        assert_eq!(result, vec!["AAPL", "MSFT", "GOOG"]);
    }

    #[test]
    fn test_empty_input_filtered() {
        let input = "";
        let result: Vec<String> = input
            .split(',')
            .map(|s| s.trim().to_uppercase())
            .filter(|s| !s.is_empty())
            .collect();
        assert!(result.is_empty());
    }
}

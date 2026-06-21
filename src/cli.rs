use std::io;

pub enum Mode {
    FetchAndStore,
    DumpToCsv,
    PullFromDb,
}

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

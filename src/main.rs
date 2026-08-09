use clap::{Parser, Subcommand};

use asfiled::edgar::tickers;
use asfiled::edgar::EdgarClient;
use asfiled::Config;

#[derive(Parser)]
#[command(
    name = "asfiled",
    version,
    about = "The open point-in-time database of public company data"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Fetch the registrant universe (ticker ↔ CIK mapping) from EDGAR.
    Tickers,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> asfiled::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Tickers => {
            let client = EdgarClient::new(Config::load()?)?;
            let universe = tickers::fetch(&client)?;
            println!("{} registrants", universe.len());
            for r in universe.iter().take(10) {
                println!(
                    "  {:>10}  {:<6}  {}  [{}]",
                    r.cik,
                    r.ticker,
                    r.name,
                    r.exchange.as_deref().unwrap_or("-")
                );
            }
        }
    }
    Ok(())
}

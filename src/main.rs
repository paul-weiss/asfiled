use clap::{Parser, Subcommand};

use asfiled::edgar::{facts, submissions, tickers, EdgarClient};
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
    /// Show a company's metadata, filing history, and XBRL fact counts.
    Company {
        /// Ticker symbol or bare CIK number.
        id: String,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn resolve_cik(client: &EdgarClient, id: &str) -> asfiled::Result<Option<u64>> {
    if let Ok(cik) = id.parse::<u64>() {
        return Ok(Some(cik));
    }
    let ticker = id.to_uppercase();
    Ok(tickers::fetch(client)?
        .into_iter()
        .find(|r| r.ticker == ticker)
        .map(|r| r.cik))
}

fn run() -> asfiled::Result<()> {
    let cli = Cli::parse();
    let client = EdgarClient::new(Config::load()?)?;
    match cli.command {
        Command::Tickers => {
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
        Command::Company { id } => {
            let Some(cik) = resolve_cik(&client, &id)? else {
                eprintln!("no registrant matching {id:?}");
                std::process::exit(1);
            };
            let Some((meta, filings)) = submissions::fetch(&client, cik, true)? else {
                eprintln!("no submissions document for CIK {cik}");
                std::process::exit(1);
            };
            println!("{} (CIK {})", meta.name, meta.cik);
            if let Some(sic) = &meta.sic_description {
                println!("  industry: {sic}");
            }
            println!(
                "  {} filings, {} to {}",
                filings.len(),
                filings
                    .iter()
                    .map(|f| f.filed_date)
                    .min()
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
                filings
                    .iter()
                    .map(|f| f.filed_date)
                    .max()
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
            );

            let all_facts = facts::fetch(&client, cik)?;
            let concepts: std::collections::HashSet<_> =
                all_facts.iter().map(|f| f.concept.as_str()).collect();
            println!(
                "  {} XBRL facts across {} concepts",
                all_facts.len(),
                concepts.len()
            );
            if let Some(latest) = all_facts
                .iter()
                .filter(|f| {
                    f.concept == "Revenues"
                        || f.concept == "RevenueFromContractWithCustomerExcludingAssessedTax"
                })
                .max_by_key(|f| (f.period_end, f.filed_date))
            {
                println!(
                    "  latest revenue: {:.1}B {} for period ending {} (filed {})",
                    latest.value / 1e9,
                    latest.unit,
                    latest.period_end,
                    latest.filed_date
                );
            }
        }
    }
    Ok(())
}

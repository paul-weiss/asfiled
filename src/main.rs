use clap::{Parser, Subcommand};
use duckdb::types::ValueRef;

use asfiled::edgar::{facts, submissions, tickers, EdgarClient};
use asfiled::store::Store;
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
    /// Ingest companies into the point-in-time store.
    Ingest {
        /// Ticker symbols or bare CIK numbers.
        ids: Vec<String>,
    },
    /// Run read-only SQL against the store. `facts_asof(DATE '...')` is the
    /// sanctioned path for historical queries.
    Query { sql: String },
    /// Export the store as Parquet + manifest for publication.
    Publish {
        /// Output directory.
        #[arg(long, default_value = "data/publish")]
        out: std::path::PathBuf,
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
    let config = Config::load()?;
    match cli.command {
        Command::Tickers => {
            let client = EdgarClient::new(config)?;
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
            let client = EdgarClient::new(config)?;
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
        }
        Command::Ingest { ids } => {
            let db_path = config.db_path.clone();
            let client = EdgarClient::new(config)?;
            let mut store = Store::open(&db_path)?;

            let universe = tickers::fetch(&client)?;
            store.put_registrants(&universe)?;
            println!("universe: {} registrants", universe.len());

            for id in &ids {
                let Some(cik) = resolve_cik(&client, id)? else {
                    eprintln!("skipping {id:?}: no matching registrant");
                    continue;
                };
                let Some((meta, filings)) = submissions::fetch(&client, cik, true)? else {
                    eprintln!("skipping {id:?}: no submissions document");
                    continue;
                };
                let company_facts = facts::fetch(&client, cik)?;
                let name = meta.name.clone();
                let (n_filings, n_facts) = (filings.len(), company_facts.len());
                store.put_company(&meta, &filings, &company_facts)?;
                println!("{name}: {n_filings} filings, {n_facts} facts");
            }

            let (companies, filings, all_facts) = store.counts()?;
            println!("store: {companies} companies, {filings} filings, {all_facts} facts");
        }
        Command::Query { sql } => {
            let store = Store::open(&config.db_path)?;
            let conn = store.connection();
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query([])?;
            let mut printed_header = false;
            let mut count = 0usize;
            while let Some(row) = rows.next()? {
                let stmt = row.as_ref();
                if !printed_header {
                    println!("{}", stmt.column_names().join("\t"));
                    printed_header = true;
                }
                let ncols = stmt.column_count();
                let mut cells = Vec::with_capacity(ncols);
                for i in 0..ncols {
                    cells.push(format_value(row.get_ref(i)?));
                }
                println!("{}", cells.join("\t"));
                count += 1;
            }
            eprintln!("({count} rows)");
        }
        Command::Publish { out } => {
            let store = Store::open(&config.db_path)?;
            let published = asfiled::publish::publish(&store, &out)?;
            for (table, rows) in &published {
                println!("{}: {} rows", table, rows);
            }
            println!("published to {}", out.display());
        }
    }
    Ok(())
}

fn format_value(value: ValueRef<'_>) -> String {
    use duckdb::types::Value;
    match Value::from(value) {
        Value::Null => "NULL".to_string(),
        Value::Text(s) => s,
        Value::Boolean(b) => b.to_string(),
        Value::TinyInt(v) => v.to_string(),
        Value::SmallInt(v) => v.to_string(),
        Value::Int(v) => v.to_string(),
        Value::BigInt(v) => v.to_string(),
        Value::UTinyInt(v) => v.to_string(),
        Value::USmallInt(v) => v.to_string(),
        Value::UInt(v) => v.to_string(),
        Value::UBigInt(v) => v.to_string(),
        Value::Float(v) => v.to_string(),
        Value::Double(v) => v.to_string(),
        // DATE arrives as days since the Unix epoch.
        Value::Date32(days) => chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
            .unwrap()
            .checked_add_signed(chrono::Duration::days(days as i64))
            .map(|d| d.to_string())
            .unwrap_or_else(|| days.to_string()),
        Value::Timestamp(_, micros) => chrono::DateTime::from_timestamp_micros(micros)
            .map(|dt| dt.naive_utc().to_string())
            .unwrap_or_else(|| micros.to_string()),
        other => format!("{other:?}"),
    }
}

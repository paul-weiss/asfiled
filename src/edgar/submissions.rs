//! Per-company filing index from the EDGAR submissions API.
//!
//! The response holds company metadata plus `filings.recent`, a set of
//! parallel arrays (one array per column, not one object per filing). Older
//! filings are paged out into shard files listed under `filings.files`; a
//! full history needs those too.
//!
//! This is where `filed_date` enters the system — the seed of every
//! point-in-time guarantee downstream.

use std::collections::HashMap;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::edgar::client::{EdgarClient, FetchPolicy};
use crate::edgar::urls;
use crate::error::Error;
use crate::Result;

/// Companies file continuously, so the live document is refetched twice a day.
const MAX_AGE: Duration = Duration::from_secs(12 * 60 * 60);

/// A name the registrant filed under, and the window it was valid for. EDGAR
/// publishes this as `formerNames`, which is what makes a company's *name*
/// point-in-time rather than whatever it is called today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormerName {
    pub name: String,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyMeta {
    pub cik: u64,
    pub name: String,
    pub sic: Option<String>,
    pub sic_description: Option<String>,
    pub fiscal_year_end: Option<String>,
    pub state_of_incorporation: Option<String>,
    pub tickers: Vec<String>,
    pub exchanges: Vec<String>,
    pub former_names: Vec<FormerName>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filing {
    pub accession: String,
    pub cik: u64,
    pub form: String,
    pub filed_date: NaiveDate,
    pub period_of_report: Option<NaiveDate>,
    pub acceptance_datetime: Option<DateTime<Utc>>,
    pub primary_document: Option<String>,
    pub primary_doc_desc: Option<String>,
    pub is_xbrl: bool,
    pub size_bytes: Option<u64>,
    /// 8-K item list, e.g. `["4.02", "9.01"]` — sorted, deduplicated.
    pub items: Vec<String>,
}

#[derive(Deserialize)]
struct Payload {
    cik: Value,
    #[serde(default)]
    name: String,
    #[serde(default)]
    sic: Option<String>,
    #[serde(default, rename = "sicDescription")]
    sic_description: Option<String>,
    #[serde(default, rename = "fiscalYearEnd")]
    fiscal_year_end: Option<String>,
    #[serde(default, rename = "stateOfIncorporation")]
    state_of_incorporation: Option<String>,
    #[serde(default)]
    tickers: Vec<String>,
    #[serde(default)]
    exchanges: Vec<String>,
    #[serde(default, rename = "formerNames")]
    former_names: Vec<FormerNameSpec>,
    #[serde(default)]
    filings: Filings,
}

#[derive(Deserialize, Default)]
struct FormerNameSpec {
    #[serde(default)]
    name: String,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
}

#[derive(Deserialize, Default)]
struct Filings {
    #[serde(default)]
    recent: FilingTable,
    #[serde(default)]
    files: Vec<ShardRef>,
}

#[derive(Deserialize, Default)]
struct ShardRef {
    #[serde(default)]
    name: String,
}

/// The parallel-array table, kept schemaless: columns come and go across
/// EDGAR revisions, and a missing column must degrade to `None` per filing
/// rather than fail the document.
#[derive(Deserialize, Default)]
pub struct FilingTable(HashMap<String, Vec<Value>>);

impl FilingTable {
    fn col(&self, name: &str, i: usize) -> Option<&Value> {
        self.0.get(name).and_then(|values| values.get(i))
    }

    fn col_str(&self, name: &str, i: usize) -> Option<&str> {
        self.col(name, i)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }
}

fn as_date(value: Option<&str>) -> Option<NaiveDate> {
    value.and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok())
}

fn as_datetime(value: Option<&str>) -> Option<DateTime<Utc>> {
    value
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|dt| dt.with_timezone(&Utc))
}

/// Split the comma-separated 8-K item list, e.g. `"4.02,9.01"`.
fn parse_items(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    let mut items: Vec<String> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    items.sort();
    items.dedup();
    items
}

fn parse_meta(url: &str, payload: &Payload) -> Result<CompanyMeta> {
    // `cik` arrives as a number in submissions documents but as a string in
    // some shards' metadata — accept both.
    let cik = match &payload.cik {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
    .ok_or_else(|| Error::Json {
        url: url.to_string(),
        source: serde::de::Error::custom("cik is neither number nor numeric string"),
    })?;
    Ok(CompanyMeta {
        cik,
        name: payload.name.clone(),
        sic: payload.sic.clone().filter(|s| !s.is_empty()),
        sic_description: payload.sic_description.clone().filter(|s| !s.is_empty()),
        fiscal_year_end: payload.fiscal_year_end.clone().filter(|s| !s.is_empty()),
        state_of_incorporation: payload
            .state_of_incorporation
            .clone()
            .filter(|s| !s.is_empty()),
        tickers: payload.tickers.clone(),
        exchanges: payload.exchanges.clone(),
        // Timestamps arrive as ISO datetimes; only the date matters.
        former_names: payload
            .former_names
            .iter()
            .filter(|f| !f.name.is_empty())
            .map(|f| FormerName {
                name: f.name.clone(),
                from: f
                    .from
                    .as_deref()
                    .and_then(|d| as_date(Some(&d[..10.min(d.len())]))),
                to: f
                    .to
                    .as_deref()
                    .and_then(|d| as_date(Some(&d[..10.min(d.len())]))),
            })
            .collect(),
    })
}

/// Convert the parallel-array filing table into `Filing` records. Rows with
/// an unparseable filing date are skipped — a filing we cannot date cannot
/// participate in point-in-time queries.
pub fn parse_filing_table(cik: u64, table: &FilingTable) -> Vec<Filing> {
    let Some(accessions) = table.0.get("accessionNumber") else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(accessions.len());
    for (i, accession) in accessions.iter().enumerate() {
        let Some(accession) = accession.as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some(filed_date) = as_date(table.col_str("filingDate", i)) else {
            eprintln!("skipping {accession}: unparseable filingDate");
            continue;
        };
        let is_xbrl = [table.col("isXBRL", i), table.col("isInlineXBRL", i)]
            .iter()
            .flatten()
            .any(|v| v.as_u64() == Some(1) || v.as_bool() == Some(true));
        out.push(Filing {
            accession: accession.to_string(),
            cik,
            form: table.col_str("form", i).unwrap_or("").to_string(),
            filed_date,
            period_of_report: as_date(table.col_str("reportDate", i)),
            acceptance_datetime: as_datetime(table.col_str("acceptanceDateTime", i)),
            primary_document: table.col_str("primaryDocument", i).map(str::to_string),
            primary_doc_desc: table
                .col_str("primaryDocDescription", i)
                .map(str::to_string),
            is_xbrl,
            size_bytes: table.col("size", i).and_then(Value::as_u64),
            items: parse_items(table.col_str("items", i)),
        });
    }
    out
}

pub fn parse(
    url: &str,
    raw: &[u8],
    full_history_shards: &mut Vec<String>,
) -> Result<(CompanyMeta, Vec<Filing>)> {
    let payload: Payload = serde_json::from_slice(raw).map_err(|source| Error::Json {
        url: url.to_string(),
        source,
    })?;
    let meta = parse_meta(url, &payload)?;
    let filings = parse_filing_table(meta.cik, &payload.filings.recent);
    full_history_shards.extend(
        payload
            .filings
            .files
            .iter()
            .map(|s| s.name.clone())
            .filter(|n| !n.is_empty()),
    );
    Ok((meta, filings))
}

/// Fetch a company's metadata and filing history. With `full_history`, shard
/// files are fetched too — they cover closed date ranges and never change
/// once written, so they cache forever.
pub fn fetch(
    client: &EdgarClient,
    cik: u64,
    full_history: bool,
) -> Result<Option<(CompanyMeta, Vec<Filing>)>> {
    let url = urls::submissions(cik);
    let policy = FetchPolicy {
        max_age: Some(MAX_AGE),
        allow_404: true,
        ..FetchPolicy::default()
    };
    let Some(resp) = client.get(&url, policy)? else {
        return Ok(None);
    };

    let mut shards = Vec::new();
    let (meta, mut filings) = parse(&url, &resp.body, &mut shards)?;

    if full_history {
        for name in shards {
            let shard_url = urls::submissions_shard(&name);
            let shard_policy = FetchPolicy {
                allow_404: true,
                ..FetchPolicy::default()
            };
            let Some(shard) = client.get(&shard_url, shard_policy)? else {
                eprintln!("submissions shard {name} missing for CIK {cik}");
                continue;
            };
            let table: FilingTable =
                serde_json::from_slice(&shard.body).map_err(|source| Error::Json {
                    url: shard_url.clone(),
                    source,
                })?;
            filings.extend(parse_filing_table(meta.cik, &table));
        }
    }

    Ok(Some((meta, filings)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"{
        "cik": 320193,
        "name": "Apple Inc.",
        "sic": "3571",
        "sicDescription": "Electronic Computers",
        "fiscalYearEnd": "0927",
        "stateOfIncorporation": "CA",
        "tickers": ["AAPL"],
        "exchanges": ["Nasdaq"],
        "filings": {
            "recent": {
                "accessionNumber": ["0000320193-24-000123", "0000320193-24-000100", "bad-row"],
                "filingDate": ["2024-11-01", "2024-08-02", "not-a-date"],
                "form": ["10-K", "10-Q", "8-K"],
                "reportDate": ["2024-09-28", "2024-06-29", ""],
                "acceptanceDateTime": ["2024-11-01T18:03:41.000Z", "", ""],
                "isXBRL": [1, 0, 0],
                "isInlineXBRL": [0, 1, 0],
                "size": [1234567, null, 100],
                "items": ["", "", "4.02,9.01, 4.02"]
            },
            "files": [{"name": "CIK0000320193-submissions-001.json"}]
        }
    }"#;

    #[test]
    fn parses_meta_and_parallel_arrays() {
        let mut shards = Vec::new();
        let (meta, filings) = parse("test://subs", SAMPLE, &mut shards).unwrap();
        assert_eq!(meta.cik, 320193);
        assert_eq!(meta.tickers, vec!["AAPL"]);
        assert!(meta.former_names.is_empty());
        assert_eq!(shards, vec!["CIK0000320193-submissions-001.json"]);

        // The unparseable-date row is skipped.
        assert_eq!(filings.len(), 2);
        let k = &filings[0];
        assert_eq!(k.form, "10-K");
        assert_eq!(k.filed_date, NaiveDate::from_ymd_opt(2024, 11, 1).unwrap());
        assert_eq!(k.period_of_report, NaiveDate::from_ymd_opt(2024, 9, 28));
        assert!(k.acceptance_datetime.is_some());
        assert!(k.is_xbrl);
        assert_eq!(k.size_bytes, Some(1234567));

        // Inline XBRL alone still counts as XBRL.
        assert!(filings[1].is_xbrl);
    }

    #[test]
    fn items_are_split_sorted_deduplicated() {
        assert_eq!(parse_items(Some("4.02,9.01, 4.02")), vec!["4.02", "9.01"]);
        assert!(parse_items(None).is_empty());
        assert!(parse_items(Some("")).is_empty());
    }

    /// A renamed registrant keeps the same identifier, so the name is the
    /// only thing that moved. Facebook became Meta on the same filer.
    #[test]
    fn former_names_are_parsed_with_their_windows() {
        let raw = br#"{"cik": 1326801, "name": "Meta Platforms, Inc.",
            "formerNames": [{"name": "Facebook Inc",
              "from": "2005-05-06T04:00:00.000Z", "to": "2021-10-27T04:00:00.000Z"}],
            "filings": {}}"#;
        let mut shards = Vec::new();
        let (meta, _) = parse("test://subs", raw, &mut shards).unwrap();
        assert_eq!(meta.former_names.len(), 1);
        assert_eq!(meta.former_names[0].name, "Facebook Inc");
        assert_eq!(
            meta.former_names[0].from,
            NaiveDate::from_ymd_opt(2005, 5, 6)
        );
        assert_eq!(
            meta.former_names[0].to,
            NaiveDate::from_ymd_opt(2021, 10, 27)
        );
    }

    #[test]
    fn string_cik_is_accepted() {
        let raw = br#"{"cik": "320193", "filings": {}}"#;
        let mut shards = Vec::new();
        let (meta, filings) = parse("test://subs", raw, &mut shards).unwrap();
        assert_eq!(meta.cik, 320193);
        assert!(filings.is_empty());
    }
}

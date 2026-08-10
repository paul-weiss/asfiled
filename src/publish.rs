//! Publish the store as versioned Parquet + a manifest.
//!
//! The published dataset *is* the public interface: the browser screener,
//! notebooks, and the MCP server all read these files. Parquet is the most
//! tradfi-legible interchange there is — "query it with anything."
//!
//! The point-in-time macros are not carried by Parquet; consumers either use
//! the shipped views (browser app, MCP) or reimplement the documented
//! semantics. The manifest records exactly what this snapshot contains.

use std::path::Path;

use serde::Serialize;

use crate::store::Store;
use crate::Result;

const TABLES: [&str; 4] = ["registrants", "companies", "filings", "facts"];

#[derive(Serialize)]
struct Manifest {
    format_version: u32,
    generated_at_utc: String,
    tables: Vec<TableEntry>,
    /// The SQL consumers need to rebuild the safe read path over facts.
    facts_asof_semantics: &'static str,
}

#[derive(Serialize)]
struct TableEntry {
    name: String,
    file: String,
    rows: u64,
}

pub fn publish(store: &Store, out_dir: &Path) -> Result<Vec<(String, u64)>> {
    std::fs::create_dir_all(out_dir).map_err(|source| crate::Error::Io {
        path: out_dir.to_path_buf(),
        source,
    })?;
    let conn = store.connection();

    let mut published = Vec::new();
    for table in TABLES {
        let file = out_dir.join(format!("{table}.parquet"));
        // COPY re-exports the whole table each run: snapshots are cheap, and
        // whole-file replacement keeps consumers' caching simple (ETag flips
        // exactly when data changed).
        conn.execute_batch(&format!(
            "COPY (SELECT * FROM {table}) TO '{}' (FORMAT PARQUET, COMPRESSION ZSTD);",
            file.display()
        ))?;
        let rows: u64 =
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
        published.push((table.to_string(), rows));
    }

    let manifest = Manifest {
        format_version: 1,
        generated_at_utc: chrono::Utc::now().to_rfc3339(),
        tables: published
            .iter()
            .map(|(name, rows)| TableEntry {
                name: name.clone(),
                file: format!("{name}.parquet"),
                rows: *rows,
            })
            .collect(),
        facts_asof_semantics: "For each (cik, taxonomy, concept, unit, period_start, \
             period_end), keep the row with the greatest filed_date <= your as-of date, \
             breaking ties on accession (descending). Restatements filed after the as-of \
             date must remain invisible.",
    };
    let manifest_path = out_dir.join("manifest.json");
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).expect("manifest serializes"),
    )
    .map_err(|source| crate::Error::Io {
        path: manifest_path,
        source,
    })?;

    Ok(published)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publishes_parquet_and_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let published = publish(&store, dir.path()).unwrap();
        assert_eq!(published.len(), 4);
        assert!(dir.path().join("facts.parquet").exists());
        let manifest: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("manifest.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["format_version"], 1);
        assert_eq!(manifest["tables"].as_array().unwrap().len(), 4);
    }

    /// The published Parquet must round-trip through DuckDB unchanged.
    #[test]
    fn published_facts_are_readable() {
        use crate::edgar::submissions::CompanyMeta;

        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::open_in_memory().unwrap();
        let meta = CompanyMeta {
            cik: 1,
            name: "Test Co".into(),
            sic: None,
            sic_description: None,
            fiscal_year_end: Some("1231".into()),
            state_of_incorporation: None,
            tickers: vec![],
            exchanges: vec![],
        };
        let fact = crate::edgar::facts::Fact {
            cik: 1,
            taxonomy: "us-gaap".into(),
            concept: "Assets".into(),
            unit: "USD".into(),
            period_start: chrono::NaiveDate::from_ymd_opt(2020, 12, 31).unwrap(),
            period_end: chrono::NaiveDate::from_ymd_opt(2020, 12, 31).unwrap(),
            is_instant: true,
            fiscal_year: None,
            fiscal_period: None,
            form: Some("10-K".into()),
            accession: "acc-1".into(),
            filed_date: chrono::NaiveDate::from_ymd_opt(2021, 2, 15).unwrap(),
            frame: None,
            value: 42.0,
        };
        store.put_company(&meta, &[], &[fact]).unwrap();
        publish(&store, dir.path()).unwrap();

        let reader = Store::open_in_memory().unwrap();
        let value: f64 = reader
            .connection()
            .query_row(
                &format!(
                    "SELECT value FROM '{}' WHERE concept = 'Assets'",
                    dir.path().join("facts.parquet").display()
                ),
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(value, 42.0);
    }
}

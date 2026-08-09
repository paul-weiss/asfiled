//! The point-in-time store.
//!
//! DuckDB, embedded. Facts are append-only observations keyed by the filing
//! that reported them; restatements are additional rows, never updates. The
//! `facts_asof(d)` macro is the *only* sanctioned read path for historical
//! queries: it answers "what was the latest knowable value of each
//! concept-period as of date d" — which makes look-ahead bias structurally
//! impossible rather than a matter of query discipline.
//!
//! Ingestion is idempotent per company: re-ingesting deletes that company's
//! rows and appends fresh. Combined with the client's disk cache, a full
//! rebuild is deterministic and network-free.

use std::path::Path;

use duckdb::{params, Connection};

use crate::edgar::facts::Fact;
use crate::edgar::submissions::{CompanyMeta, Filing};
use crate::edgar::tickers::Registrant;
use crate::normalize::{concept_map, fiscal};
use crate::Result;

fn kind_str(kind: fiscal::Kind) -> &'static str {
    match kind {
        fiscal::Kind::Instant => "instant",
        fiscal::Kind::Quarter => "quarter",
        fiscal::Kind::Half => "half",
        fiscal::Kind::NineMonth => "nine_month",
        fiscal::Kind::Annual => "annual",
        fiscal::Kind::Other => "other",
    }
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS registrants (
    cik       UBIGINT NOT NULL,
    name      VARCHAR NOT NULL,
    ticker    VARCHAR NOT NULL,
    exchange  VARCHAR
);

CREATE TABLE IF NOT EXISTS companies (
    cik                    UBIGINT PRIMARY KEY,
    name                   VARCHAR NOT NULL,
    sic                    VARCHAR,
    sic_description        VARCHAR,
    fiscal_year_end        VARCHAR,
    state_of_incorporation VARCHAR
);

CREATE TABLE IF NOT EXISTS filings (
    accession             VARCHAR NOT NULL,
    cik                   UBIGINT NOT NULL,
    form                  VARCHAR NOT NULL,
    filed_date            DATE NOT NULL,
    period_of_report      DATE,
    acceptance_datetime   TIMESTAMP,
    primary_document      VARCHAR,
    is_xbrl               BOOLEAN NOT NULL,
    size_bytes            UBIGINT,
    -- Comma-joined 8-K item list, e.g. '4.02,9.01'. A list type would be
    -- cleaner; a flat string survives Parquet/WASM round trips unchanged.
    items                 VARCHAR NOT NULL
);

CREATE TABLE IF NOT EXISTS facts (
    cik          UBIGINT NOT NULL,
    taxonomy     VARCHAR NOT NULL,
    concept      VARCHAR NOT NULL,
    unit         VARCHAR NOT NULL,
    period_start DATE NOT NULL,
    period_end   DATE NOT NULL,
    is_instant   BOOLEAN NOT NULL,
    fiscal_year  BIGINT,
    fiscal_period VARCHAR,
    form         VARCHAR,
    accession    VARCHAR NOT NULL,
    filed_date   DATE NOT NULL,   -- the knowability boundary
    value        DOUBLE NOT NULL,
    -- Derived period identity (normalize::fiscal). The raw XBRL fy/fp
    -- columns describe the *filing*, not the fact, so queries should use
    -- these instead: period_kind ('quarter','annual','half','nine_month',
    -- 'instant','other'), and for panel-worthy periods, the fiscal year and
    -- period resolved against the company's own year-end.
    period_kind  VARCHAR NOT NULL,
    fy_derived   INTEGER,
    fp_derived   VARCHAR
);

-- The safe read path. For each concept-period, the observation with the
-- latest filed_date <= d wins: at date d you would know a restatement filed
-- before d, and cannot know one filed after. Ties (same-day amendments)
-- break on accession for determinism.
CREATE OR REPLACE MACRO facts_asof(d) AS TABLE
    SELECT * EXCLUDE (rn) FROM (
        SELECT *, row_number() OVER (
            PARTITION BY cik, taxonomy, concept, unit, period_start, period_end
            ORDER BY filed_date DESC, accession DESC
        ) AS rn
        FROM facts
        WHERE filed_date <= d
    ) WHERE rn = 1;
"#;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| crate::Error::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Self::install_concept_map(&conn)?;
        Ok(Self { conn })
    }

    /// (Re)install the canonical concept map and the `fundamentals_asof`
    /// view. The template is compiled into the binary, so the table always
    /// reflects the code that wrote it — replaced on every open, never
    /// migrated.
    fn install_concept_map(conn: &Connection) -> Result<()> {
        let map = concept_map::default()?;
        conn.execute_batch(
            "CREATE OR REPLACE TABLE concept_map (
                item      VARCHAR NOT NULL,
                label     VARCHAR NOT NULL,
                statement VARCHAR NOT NULL,
                kind      VARCHAR NOT NULL,
                unit      VARCHAR NOT NULL,
                tag       VARCHAR NOT NULL,
                tag_rank  INTEGER NOT NULL
            );",
        )?;
        {
            let mut app = conn.appender("concept_map")?;
            for item in &map.items {
                for (rank, tag) in item.tags.iter().enumerate() {
                    app.append_row(params![
                        item.key,
                        item.label,
                        item.statement,
                        item.kind,
                        item.unit,
                        tag,
                        rank as i32
                    ])?;
                }
            }
        }
        // Canonical items over the safe read path: for each company-item-
        // period, the best-ranked tag that carried a value wins. Everything
        // inherits facts_asof's knowability guarantee.
        conn.execute_batch(
            "CREATE OR REPLACE MACRO fundamentals_asof(d) AS TABLE
                SELECT * EXCLUDE (rn) FROM (
                    SELECT f.cik, m.item, m.label, m.statement, m.kind,
                           f.fy_derived, f.fp_derived,
                           f.period_start, f.period_end, f.filed_date,
                           f.accession, f.value,
                           row_number() OVER (
                               PARTITION BY f.cik, m.item, f.fy_derived, f.fp_derived
                               ORDER BY m.tag_rank, f.period_end DESC,
                                        f.filed_date DESC, f.accession DESC
                           ) AS rn
                    FROM facts_asof(d) f
                    JOIN concept_map m
                      ON f.taxonomy = 'us-gaap'
                     AND f.concept = m.tag
                     AND f.unit = m.unit
                    WHERE f.fy_derived IS NOT NULL
                ) WHERE rn = 1;",
        )?;
        Ok(())
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Replace the registrant universe snapshot.
    pub fn put_registrants(&mut self, registrants: &[Registrant]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute_batch("DELETE FROM registrants;")?;
        {
            let mut app = tx.appender("registrants")?;
            for r in registrants {
                app.append_row(params![r.cik, r.name, r.ticker, r.exchange])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Idempotently store one company's metadata, filings, and facts.
    pub fn put_company(
        &mut self,
        meta: &CompanyMeta,
        filings: &[Filing],
        facts: &[Fact],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM companies WHERE cik = ?", params![meta.cik])?;
        tx.execute("DELETE FROM filings WHERE cik = ?", params![meta.cik])?;
        tx.execute("DELETE FROM facts WHERE cik = ?", params![meta.cik])?;

        tx.execute(
            "INSERT INTO companies VALUES (?, ?, ?, ?, ?, ?)",
            params![
                meta.cik,
                meta.name,
                meta.sic,
                meta.sic_description,
                meta.fiscal_year_end,
                meta.state_of_incorporation
            ],
        )?;

        {
            let mut app = tx.appender("filings")?;
            for f in filings {
                app.append_row(params![
                    f.accession,
                    f.cik,
                    f.form,
                    f.filed_date,
                    f.period_of_report,
                    f.acceptance_datetime.map(|dt| dt.naive_utc()),
                    f.primary_document,
                    f.is_xbrl,
                    f.size_bytes,
                    f.items.join(",")
                ])?;
            }
        }

        {
            let fye = meta.fiscal_year_end.as_deref();
            let mut app = tx.appender("facts")?;
            for f in facts {
                let kind = fiscal::classify(f.period_start, f.period_end, f.is_instant);
                let resolved = match kind {
                    fiscal::Kind::Instant => fiscal::resolve_instant(f.period_end, fye),
                    _ => fiscal::resolve(f.period_end, fye, kind),
                };
                app.append_row(params![
                    f.cik,
                    f.taxonomy,
                    f.concept,
                    f.unit,
                    f.period_start,
                    f.period_end,
                    f.is_instant,
                    f.fiscal_year,
                    f.fiscal_period,
                    f.form,
                    f.accession,
                    f.filed_date,
                    f.value,
                    kind_str(kind),
                    resolved.map(|r| r.fiscal_year),
                    resolved.map(|r| r.fiscal_period.as_str())
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn counts(&self) -> Result<(u64, u64, u64)> {
        let row = self.conn.query_row(
            "SELECT (SELECT count(*) FROM companies),
                    (SELECT count(*) FROM filings),
                    (SELECT count(*) FROM facts)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        Ok(row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn fact(concept: &str, filed: &str, accession: &str, value: f64) -> Fact {
        Fact {
            cik: 1,
            taxonomy: "us-gaap".into(),
            concept: concept.into(),
            unit: "USD".into(),
            period_start: d("2020-01-01"),
            period_end: d("2020-12-31"),
            is_instant: false,
            fiscal_year: Some(2020),
            fiscal_period: Some("FY".into()),
            form: Some("10-K".into()),
            accession: accession.into(),
            filed_date: d(filed),
            frame: None,
            value,
        }
    }

    fn meta() -> CompanyMeta {
        CompanyMeta {
            cik: 1,
            name: "Test Co".into(),
            sic: None,
            sic_description: None,
            fiscal_year_end: None,
            state_of_incorporation: None,
            tickers: vec!["TST".into()],
            exchanges: vec![],
        }
    }

    /// The flagship invariant: a restatement filed in 2023 must be invisible
    /// to a query as-of 2021, and authoritative as-of 2024.
    #[test]
    fn facts_asof_respects_knowability() {
        let mut store = Store::open_in_memory().unwrap();
        let original = fact("Revenues", "2021-02-15", "acc-original", 100.0);
        let restated = fact("Revenues", "2023-06-01", "acc-restated", 80.0);
        store
            .put_company(&meta(), &[], &[original, restated])
            .unwrap();

        let value_asof = |as_of: &str| -> f64 {
            store
                .connection()
                .query_row(
                    "SELECT value FROM facts_asof(?) WHERE concept = 'Revenues'",
                    params![d(as_of)],
                    |r| r.get(0),
                )
                .unwrap()
        };

        assert_eq!(value_asof("2021-12-31"), 100.0); // restatement not yet knowable
        assert_eq!(value_asof("2024-01-01"), 80.0); // restatement knowable

        // Before anything was filed, the period does not exist at all.
        let rows: u64 = store
            .connection()
            .query_row(
                "SELECT count(*) FROM facts_asof(?)",
                params![d("2020-12-31")],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn reingest_is_idempotent() {
        let mut store = Store::open_in_memory().unwrap();
        let f = fact("Assets", "2021-02-15", "acc-1", 500.0);
        store
            .put_company(&meta(), &[], std::slice::from_ref(&f))
            .unwrap();
        store.put_company(&meta(), &[], &[f]).unwrap();

        let (companies, _, facts) = store.counts().unwrap();
        assert_eq!(companies, 1);
        assert_eq!(facts, 1);
    }

    /// Derived period identity comes from the period dates and the company's
    /// own year-end — never from the filing's fy/fp tags.
    #[test]
    fn derived_fiscal_columns_are_populated() {
        let mut store = Store::open_in_memory().unwrap();
        let mut m = meta();
        m.fiscal_year_end = Some("1231".into());
        // Deliberately wrong filing-level tags: the 10-K claims fy=2022, but
        // the period itself is calendar 2020.
        let mut f = fact("Revenues", "2021-02-15", "acc-1", 100.0);
        f.fiscal_year = Some(2022);
        store.put_company(&m, &[], &[f]).unwrap();

        let (fy, fp, kind): (i32, String, String) = store
            .connection()
            .query_row(
                "SELECT fy_derived, fp_derived, period_kind FROM facts",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(fy, 2020);
        assert_eq!(fp, "FY");
        assert_eq!(kind, "annual");
    }

    /// fundamentals_asof maps tags to canonical items by priority: when both
    /// the ASC 606 revenue tag and the legacy Revenues tag carry a value for
    /// the same period, the 606 tag wins; a same-tag value in the wrong unit
    /// never matches.
    #[test]
    fn fundamentals_prefers_priority_tag_and_respects_unit() {
        let mut store = Store::open_in_memory().unwrap();
        let mut m = meta();
        m.fiscal_year_end = Some("1231".into());

        let legacy = fact("Revenues", "2021-02-15", "acc-1", 90.0);
        let mut asc606 = fact(
            "RevenueFromContractWithCustomerExcludingAssessedTax",
            "2021-02-15",
            "acc-1",
            100.0,
        );
        asc606.concept = "RevenueFromContractWithCustomerExcludingAssessedTax".into();
        let mut wrong_unit = fact("Revenues", "2021-02-15", "acc-1", 7.0);
        wrong_unit.unit = "shares".into();

        store
            .put_company(&m, &[], &[legacy, asc606, wrong_unit])
            .unwrap();

        let (item, value): (String, f64) = store
            .connection()
            .query_row(
                "SELECT item, value FROM fundamentals_asof(DATE '2022-01-01')
                 WHERE item = 'revenue'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(item, "revenue");
        assert_eq!(value, 100.0); // ASC 606 tag outranks legacy; shares row ignored
    }

    #[test]
    fn filings_round_trip() {
        let mut store = Store::open_in_memory().unwrap();
        let filing = Filing {
            accession: "acc-8k".into(),
            cik: 1,
            form: "8-K".into(),
            filed_date: d("2022-03-01"),
            period_of_report: None,
            acceptance_datetime: None,
            primary_document: None,
            primary_doc_desc: None,
            is_xbrl: false,
            size_bytes: Some(1000),
            items: vec!["4.02".into(), "9.01".into()],
        };
        store.put_company(&meta(), &[filing], &[]).unwrap();

        let items: String = store
            .connection()
            .query_row(
                "SELECT items FROM filings WHERE accession = 'acc-8k'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(items, "4.02,9.01");
    }
}

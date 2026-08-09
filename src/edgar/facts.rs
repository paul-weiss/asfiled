//! Structured XBRL financials from the EDGAR company-facts API.
//!
//! Each fact carries the accession and filing date that reported it, which is
//! what makes point-in-time reconstruction possible. Restatements arrive as
//! additional observations of an already-reported period; they are kept,
//! never merged.
//!
//! Caveat worth knowing: this API returns *numeric* facts only. Text facts
//! such as `dei:EntityFilerCategory` are absent and have to come from the
//! filing documents themselves.

use std::collections::HashSet;
use std::time::Duration;

use chrono::NaiveDate;
use serde_json::Value;

use crate::edgar::client::{EdgarClient, FetchPolicy};
use crate::edgar::urls;
use crate::error::Error;
use crate::Result;

const MAX_AGE: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Debug, Clone, PartialEq)]
pub struct Fact {
    pub cik: u64,
    /// Taxonomy namespace, e.g. `us-gaap`, `dei`, `ifrs-full`.
    pub taxonomy: String,
    pub concept: String,
    pub unit: String,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    /// Instant facts (balance-sheet items) have `period_start == period_end`.
    pub is_instant: bool,
    pub fiscal_year: Option<i64>,
    pub fiscal_period: Option<String>,
    pub form: Option<String>,
    pub accession: String,
    /// The date this observation was filed — the knowability boundary.
    pub filed_date: NaiveDate,
    pub frame: Option<String>,
    pub value: f64,
}

fn as_date(value: Option<&str>) -> Option<NaiveDate> {
    value.and_then(|v| NaiveDate::parse_from_str(v, "%Y-%m-%d").ok())
}

/// Parse a company-facts document.
///
/// The `cik` argument is authoritative, because the payload cannot be
/// trusted to identify itself: closed-end funds and other non-operating
/// registrants return documents with `entityName` and `facts` but no `cik`
/// key at all. We asked for a specific company, so we already know which one
/// it is.
pub fn parse(url: &str, raw: &[u8], cik: u64) -> Result<Vec<Fact>> {
    let payload: Value = serde_json::from_slice(raw).map_err(|source| Error::Json {
        url: url.to_string(),
        source,
    })?;

    let mut out = Vec::new();
    // The API occasionally repeats an observation with a differing `frame`
    // annotation; one observation per accession per concept-period wins.
    let mut seen: HashSet<(String, String, String, NaiveDate, NaiveDate, String)> = HashSet::new();

    let Some(taxonomies) = payload.get("facts").and_then(Value::as_object) else {
        return Ok(out);
    };
    for (taxonomy, concepts) in taxonomies {
        let Some(concepts) = concepts.as_object() else {
            continue;
        };
        for (concept, node) in concepts {
            let Some(units) = node.get("units").and_then(Value::as_object) else {
                continue;
            };
            for (unit, observations) in units {
                let Some(observations) = observations.as_array() else {
                    continue;
                };
                for obs in observations {
                    let (Some(period_end), Some(filed_date), Some(accession), Some(value)) = (
                        as_date(obs.get("end").and_then(Value::as_str)),
                        as_date(obs.get("filed").and_then(Value::as_str)),
                        obs.get("accn").and_then(Value::as_str),
                        obs.get("val").and_then(Value::as_f64),
                    ) else {
                        continue;
                    };

                    let start_raw = obs.get("start").and_then(Value::as_str);
                    let is_instant = start_raw.is_none();
                    let period_start = if is_instant {
                        period_end
                    } else {
                        match as_date(start_raw) {
                            Some(start) => start,
                            None => continue,
                        }
                    };

                    let key = (
                        taxonomy.clone(),
                        concept.clone(),
                        unit.clone(),
                        period_start,
                        period_end,
                        accession.to_string(),
                    );
                    if !seen.insert(key) {
                        continue;
                    }

                    out.push(Fact {
                        cik,
                        taxonomy: taxonomy.clone(),
                        concept: concept.clone(),
                        unit: unit.clone(),
                        period_start,
                        period_end,
                        is_instant,
                        fiscal_year: obs.get("fy").and_then(Value::as_i64),
                        fiscal_period: obs.get("fp").and_then(Value::as_str).map(str::to_string),
                        form: obs.get("form").and_then(Value::as_str).map(str::to_string),
                        accession: accession.to_string(),
                        filed_date,
                        frame: obs.get("frame").and_then(Value::as_str).map(str::to_string),
                        value,
                    });
                }
            }
        }
    }
    Ok(out)
}

/// Fetch all XBRL facts for a company. `Ok(None)`-style absence is folded to
/// an empty vec: shell companies, foreign private issuers filing 20-F, and
/// registrants predating XBRL have no company-facts document, and that is
/// ordinary.
pub fn fetch(client: &EdgarClient, cik: u64) -> Result<Vec<Fact>> {
    let url = urls::company_facts(cik);
    let policy = FetchPolicy {
        max_age: Some(MAX_AGE),
        allow_404: true,
        ..FetchPolicy::default()
    };
    match client.get(&url, policy)? {
        None => Ok(Vec::new()),
        Some(resp) => parse(&url, &resp.body, cik),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = br#"{
        "entityName": "Apple Inc.",
        "facts": {
            "us-gaap": {
                "Revenues": {
                    "units": {
                        "USD": [
                            {"start": "2023-10-01", "end": "2024-09-28", "val": 391035000000,
                             "accn": "0000320193-24-000123", "fy": 2024, "fp": "FY",
                             "form": "10-K", "filed": "2024-11-01", "frame": "CY2024"},
                            {"start": "2023-10-01", "end": "2024-09-28", "val": 391035000000,
                             "accn": "0000320193-24-000123", "fy": 2024, "fp": "FY",
                             "form": "10-K", "filed": "2024-11-01"},
                            {"end": "2024-09-28", "val": 1, "accn": "x", "filed": "bad-date"}
                        ]
                    }
                },
                "Assets": {
                    "units": {
                        "USD": [
                            {"end": "2024-09-28", "val": 364980000000,
                             "accn": "0000320193-24-000123", "fy": 2024, "fp": "FY",
                             "form": "10-K", "filed": "2024-11-01"}
                        ]
                    }
                }
            }
        }
    }"#;

    #[test]
    fn parses_duration_and_instant_facts() {
        let facts = parse("test://facts", SAMPLE, 320193).unwrap();
        // Duplicate observation deduplicated; bad-date row skipped.
        assert_eq!(facts.len(), 2);

        let revenue = facts.iter().find(|f| f.concept == "Revenues").unwrap();
        assert!(!revenue.is_instant);
        assert_eq!(revenue.value, 391035000000.0);
        assert_eq!(
            revenue.filed_date,
            NaiveDate::from_ymd_opt(2024, 11, 1).unwrap()
        );

        let assets = facts.iter().find(|f| f.concept == "Assets").unwrap();
        assert!(assets.is_instant);
        assert_eq!(assets.period_start, assets.period_end);
    }

    #[test]
    fn supplied_cik_is_authoritative_when_payload_has_none() {
        // No top-level cik key — common for closed-end funds.
        let facts = parse("test://facts", SAMPLE, 999).unwrap();
        assert!(facts.iter().all(|f| f.cik == 999));
    }

    #[test]
    fn empty_or_missing_facts_is_ordinary() {
        let facts = parse("test://facts", br#"{"entityName": "Shell Co"}"#, 1).unwrap();
        assert!(facts.is_empty());
    }
}

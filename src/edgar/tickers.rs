//! The registrant universe, from EDGAR's ticker-to-CIK mapping.
//!
//! This is the starting universe only — exchange-listed registrants with a
//! ticker. Screening filters apply much later, at query time, not here.
//! Ingesting broadly and filtering late means a filter change does not
//! require a re-ingest.

use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::edgar::client::{EdgarClient, FetchPolicy};
use crate::edgar::urls;
use crate::error::Error;
use crate::Result;

/// The mapping changes slowly; a day-old copy is fine.
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registrant {
    pub cik: u64,
    pub name: String,
    pub ticker: String,
    pub exchange: Option<String>,
}

/// `company_tickers_exchange.json` is column-oriented: a `fields` array of
/// names and a `data` array of rows.
#[derive(Deserialize)]
struct Payload {
    fields: Vec<String>,
    data: Vec<Vec<Value>>,
}

pub fn parse(url: &str, raw: &[u8]) -> Result<Vec<Registrant>> {
    let payload: Payload = serde_json::from_slice(raw).map_err(|source| Error::Json {
        url: url.to_string(),
        source,
    })?;
    let idx = |name: &str| payload.fields.iter().position(|f| f == name);
    let (Some(cik_i), Some(name_i), Some(ticker_i)) = (idx("cik"), idx("name"), idx("ticker"))
    else {
        return Err(Error::Json {
            url: url.to_string(),
            source: serde::de::Error::custom("missing cik/name/ticker fields"),
        });
    };
    let exchange_i = idx("exchange");

    let mut out = Vec::with_capacity(payload.data.len());
    for row in &payload.data {
        let (Some(cik), Some(name), Some(ticker)) = (
            row.get(cik_i).and_then(Value::as_u64),
            row.get(name_i).and_then(Value::as_str),
            row.get(ticker_i).and_then(Value::as_str),
        ) else {
            continue; // malformed row: skip rather than fail the universe
        };
        let exchange = exchange_i
            .and_then(|i| row.get(i))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        out.push(Registrant {
            cik,
            name: name.to_string(),
            ticker: ticker.to_string(),
            exchange,
        });
    }
    Ok(out)
}

pub fn fetch(client: &EdgarClient) -> Result<Vec<Registrant>> {
    let url = urls::company_tickers();
    let resp = client
        .get(&url, FetchPolicy::max_age(MAX_AGE))?
        .expect("company tickers endpoint always exists");
    parse(&url, &resp.body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_column_oriented_payload() {
        let raw = br#"{
            "fields": ["cik", "name", "ticker", "exchange"],
            "data": [
                [320193, "Apple Inc.", "AAPL", "Nasdaq"],
                [789019, "MICROSOFT CORP", "MSFT", ""],
                [null, "Broken Row", "X", "NYSE"]
            ]
        }"#;
        let out = parse("test://tickers", raw).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[0],
            Registrant {
                cik: 320193,
                name: "Apple Inc.".into(),
                ticker: "AAPL".into(),
                exchange: Some("Nasdaq".into()),
            }
        );
        // Empty exchange string becomes None.
        assert_eq!(out[1].exchange, None);
    }

    #[test]
    fn missing_fields_is_an_error() {
        let raw = br#"{"fields": ["cik"], "data": []}"#;
        assert!(parse("test://tickers", raw).is_err());
    }
}

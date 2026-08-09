//! EDGAR endpoint construction.
//!
//! Note the two hosts: bulk JSON APIs live on data.sec.gov, archives and
//! index files on www.sec.gov. Both are covered by the same fair-access rate
//! limit.

use chrono::{Datelike, NaiveDate};

pub const DATA_HOST: &str = "https://data.sec.gov";
pub const WWW_HOST: &str = "https://www.sec.gov";

/// CIKs are zero-padded to ten digits in EDGAR's JSON API paths.
pub fn cik10(cik: u64) -> String {
    format!("{cik:010}")
}

pub fn company_tickers() -> String {
    format!("{WWW_HOST}/files/company_tickers_exchange.json")
}

pub fn submissions(cik: u64) -> String {
    format!("{DATA_HOST}/submissions/CIK{}.json", cik10(cik))
}

/// Older filings are paged out of the main submissions file into shards.
pub fn submissions_shard(filename: &str) -> String {
    format!("{DATA_HOST}/submissions/{filename}")
}

pub fn company_facts(cik: u64) -> String {
    format!("{DATA_HOST}/api/xbrl/companyfacts/CIK{}.json", cik10(cik))
}

pub fn daily_index(day: NaiveDate) -> String {
    let quarter = (day.month() - 1) / 3 + 1;
    format!(
        "{WWW_HOST}/Archives/edgar/daily-index/{}/QTR{}/master.{}.idx",
        day.year(),
        quarter,
        day.format("%Y%m%d")
    )
}

pub fn filing_index(cik: u64, accession: &str) -> String {
    let bare = accession.replace('-', "");
    format!("{WWW_HOST}/Archives/edgar/data/{cik}/{bare}/{accession}-index.htm")
}

pub fn filing_document(cik: u64, accession: &str, document: &str) -> String {
    let bare = accession.replace('-', "");
    format!("{WWW_HOST}/Archives/edgar/data/{cik}/{bare}/{document}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cik_is_zero_padded_to_ten() {
        assert_eq!(cik10(320193), "0000320193");
        assert_eq!(
            company_facts(320193),
            "https://data.sec.gov/api/xbrl/companyfacts/CIK0000320193.json"
        );
    }

    #[test]
    fn daily_index_encodes_quarter_and_date() {
        let day = NaiveDate::from_ymd_opt(2026, 8, 7).unwrap();
        assert_eq!(
            daily_index(day),
            "https://www.sec.gov/Archives/edgar/daily-index/2026/QTR3/master.20260807.idx"
        );
        let jan = NaiveDate::from_ymd_opt(2025, 1, 2).unwrap();
        assert!(daily_index(jan).contains("/2025/QTR1/master.20250102.idx"));
    }

    #[test]
    fn filing_urls_strip_accession_dashes_in_path_only() {
        let url = filing_index(320193, "0000320193-24-000123");
        assert_eq!(
            url,
            "https://www.sec.gov/Archives/edgar/data/320193/000032019324000123/0000320193-24-000123-index.htm"
        );
    }
}

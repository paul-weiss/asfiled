//! asfiled — the open point-in-time database of public company data.
//!
//! Every fact carries the date it became knowable; every query runs as-of a
//! date. This crate is the ingestion and normalization backend: it fetches
//! public filings (SEC EDGAR first), normalizes them into a point-in-time
//! schema, and publishes versioned Parquet for the browser screener, the
//! English-query layer, and the MCP server.

pub mod config;
pub mod edgar;
pub mod error;
pub mod normalize;
pub mod store;

pub use config::Config;
pub use error::Error;

pub type Result<T> = std::result::Result<T, Error>;

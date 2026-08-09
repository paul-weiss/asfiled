//! SEC EDGAR access: endpoint construction, the rate-limited disk-cached
//! client every request goes through, and per-dataset fetchers.

pub mod client;
pub mod tickers;
pub mod urls;

pub use client::{EdgarClient, FetchPolicy};

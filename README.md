# asfiled

[![CI](https://github.com/paul-weiss/asfiled/actions/workflows/ci.yml/badge.svg)](https://github.com/paul-weiss/asfiled/actions/workflows/ci.yml)
[![Security audit](https://github.com/paul-weiss/asfiled/actions/workflows/audit.yml/badge.svg)](https://github.com/paul-weiss/asfiled/actions/workflows/audit.yml)
[![Release](https://github.com/paul-weiss/asfiled/actions/workflows/release.yml/badge.svg)](https://github.com/paul-weiss/asfiled/releases)
[![Site](https://github.com/paul-weiss/asfiled/actions/workflows/site.yml/badge.svg)](https://paul-weiss.github.io/asfiled/)
[![Dependencies](https://deps.rs/repo/github/paul-weiss/asfiled/status.svg)](https://deps.rs/repo/github/paul-weiss/asfiled)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**The open point-in-time database of public company data.**
*As it was filed, when it was knowable.*

> **Status: early.** The design below is settled; the code is landing
> milestone by milestone. **[Documentation →](https://paul-weiss.github.io/asfiled/)**

## What this is

Free screeners and datasets quietly use restated, look-ahead-biased
financials: the numbers you see for 2020 are the numbers as *revised* in
2023, not the numbers anyone could have known in 2020. That makes them
unusable for honest backtesting, research, or fraud analysis.

asfiled is built on one rule, enforced by construction rather than
convention: **every fact carries the date it became knowable, and every query
runs as-of a date.** Restatements are visible *events* in the data — they are
never silently applied backwards.

Three surfaces, one dataset:

1. **Screener** — a dense, finviz-style table over US public-company
   fundamentals and flows, running entirely in your browser (DuckDB-WASM over
   published Parquet; no backend, no account). The defining control is the
   **as-of date**: set it to 2019-06-30 and you see exactly what was knowable
   then.
2. **English queries** — ask in plain language ("companies with rising
   receivables and falling revenue, as of 2021"); a language model translates
   your question into SQL over the safe views, and **the SQL is always shown**
   — auditable, editable, yours. Bring your own Anthropic API key; queries go
   directly from your browser to the API.
3. **MCP server** — the same dataset exposed to AI agents through the Model
   Context Protocol, with the same point-in-time guarantees. Runs locally
   against the published data.

## Data

All sources are free, public, and redistributable — primarily SEC EDGAR:

- **XBRL company facts** — all fundamentals, as filed
- **Forms 3/4/5** — insider transactions
- **8-K events** — restatements (Item 4.02), auditor changes, material events
- **13F-HR** — institutional holdings
- **DEF 14A** — executive compensation

Planned additions: FINRA short interest and Reg SHO volume, SEC
fails-to-deliver, and macro context from FRED/Treasury/BLS.

**Deliberately absent: prices.** Equity price data is exchange-licensed, and
every "free" source is a terms-of-service gray zone. asfiled is
fundamentals-and-flows only, and says so plainly — a price plug-in interface
may come later for data you license yourself.

## Design principles

1. **Point-in-time by construction.** The query layer exposes only safe views
   over as-reported facts with knowable-at dates. You *cannot* write a
   look-ahead-biased query — neither can the language model.
2. **As-reported, never silently restated.**
3. **Show the SQL.** Every English query displays its generated SQL.
4. **Static-first.** Data ships as versioned Parquet; the browser does the
   compute. The hosted site is just the reference deployment — clone the repo
   and run the whole thing yourself.
5. **Not investment advice.** asfiled is a data tool. Nothing here is a
   recommendation.

## Architecture

```
SEC EDGAR (+ FINRA, FRED, …)
        │  ingest & normalize (Rust)
        ▼
point-in-time schema ── facts keyed by accession, filed_at, knowable_at
        │  publish
        ▼
versioned Parquet + manifest (static hosting)
        │
        ├── Screener UI (DuckDB-WASM, in-browser)
        ├── English queries (Claude, BYOK, structured outputs)
        └── MCP server (local, read-only, safe views)
```

## Roadmap

- **M1** — ingestion, normalization, point-in-time schema, safe views
- **M2** — published Parquet + the in-browser screener *(public launch)*
- **M3** — English queries
- **M4** — MCP server
- **M5** — short interest, insider/13F depth, macro columns

## License

[Apache-2.0](LICENSE)

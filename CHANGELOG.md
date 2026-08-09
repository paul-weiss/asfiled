# Changelog

All notable changes to asfiled. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [SemVer](https://semver.org/).

## [Unreleased]

### Added

- Per-company filing index from the submissions API: metadata, the
  parallel-array filing table, and full-history shards. This is where
  `filed_date` — the knowability boundary — enters the system.
- XBRL company facts: every numeric fact with its accession and filing date;
  restatements kept as additional observations, never merged.
- `asfiled company <ticker|cik>` CLI command.
- The point-in-time store (embedded DuckDB): append-only facts keyed by the
  filing that reported them, and the `facts_asof(d)` table macro — the single
  sanctioned read path, under which look-ahead bias is structurally
  impossible. Ingestion is idempotent per company (delete-then-append in a
  transaction, bulk-loaded via the Appender API).
- `asfiled ingest <ids...>` and `asfiled query <sql>` CLI commands.
- Fiscal calendar alignment: `period_kind` / `fy_derived` / `fp_derived`
  derived from period dates and each company's own year-end, replacing the
  misleading filing-level `fy`/`fp` tags.
- The canonical concept map (`config/concepts.toml`, compiled in): ~19
  screener items over priority-ordered us-gaap tags, with
  `CostsAndExpenses` rejected as a cost tag at load. `fundamentals_asof(d)`
  resolves tags to items on top of the safe read path — milestone M1
  complete.
- Documentation site (mdBook → GitHub Pages).

## [0.1.0] — 2026-08-09

### Added

- Rate-limited (8 req/s, under the SEC 10 req/s fair-access ceiling),
  disk-cached EDGAR client with retry/backoff. Cache is gzip body + JSON
  metadata, write-then-rename, keyed by URL SHA-1 — full rebuilds need no
  network.
- EDGAR endpoint construction for both hosts (data.sec.gov, www.sec.gov).
- Registrant universe fetcher (ticker ↔ CIK ↔ exchange mapping).
- `asfiled tickers` CLI command.
- CI (fmt, clippy, tests), weekly RustSec security audit, tagged-release
  binaries for Linux and macOS.

[0.1.0]: https://github.com/paul-weiss/asfiled/releases/tag/v0.1.0

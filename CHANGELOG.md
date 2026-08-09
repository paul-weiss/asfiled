# Changelog

All notable changes to asfiled. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
versioning follows [SemVer](https://semver.org/).

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

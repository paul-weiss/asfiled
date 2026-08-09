# Roadmap

| Milestone | Scope | Status |
|---|---|---|
| **M1 — Rust core** | EDGAR ingestion, point-in-time store, `facts_asof`, fiscal normalization, canonical concept map | in progress — concept map remaining |
| **M2 — Data + screener** | Published Parquet + manifest, daily refresh, in-browser screener (DuckDB-WASM) with the as-of control | next — UI design iteration underway |
| **M3 — English queries** | Natural language → auditable SQL over the safe views (bring your own Anthropic key) | planned |
| **M4 — MCP server** | The dataset as an MCP endpoint for AI agents | planned |
| **M5 — More sources** | Short interest, insider forms, 13F, macro context | planned |

Design decisions and their reasoning live in the repository:
[README](https://github.com/paul-weiss/asfiled#readme) ·
[CHANGELOG](https://github.com/paul-weiss/asfiled/blob/main/CHANGELOG.md) ·
[CONTRIBUTING](https://github.com/paul-weiss/asfiled/blob/main/CONTRIBUTING.md)

# CLI

One binary: `asfiled`. Configuration comes from the environment or a `.env`
file in the working directory (real environment variables win).

| Variable | Required | Default |
|---|---|---|
| `ASFILED_SEC_USER_AGENT` | **yes** — SEC access policy | none, refuses to run |
| `ASFILED_DATA_DIR` | no | `data/` |
| `ASFILED_DB` | no | `data/asfiled.duckdb` |

## Commands

### `asfiled tickers`

Fetch the registrant universe and print a summary.

### `asfiled company <ticker|cik>`

Show one company's metadata, filing history span, and XBRL fact counts.

```text
$ asfiled company AAPL
Apple Inc. (CIK 320193)
  industry: Electronic Computers
  2238 filings, 1994-01-26 to 2026-07-31
  25135 XBRL facts across 505 concepts
```

### `asfiled ingest <ids...>`

Ingest companies (tickers or CIKs) into the point-in-time store. Idempotent —
re-ingesting a company replaces its rows atomically.

```text
$ asfiled ingest AAPL MSFT NVDA
universe: 10398 registrants
Apple Inc.: 2238 filings, 25135 facts
MICROSOFT CORP: 4481 filings, 32671 facts
NVIDIA CORP: 2462 filings, 26903 facts
store: 3 companies, 9181 filings, 84709 facts
```

### `asfiled query <sql>`

Run SQL against the store and print tab-separated results.
`facts_asof(DATE '...')` is the sanctioned path for historical queries.

```text
$ asfiled query "SELECT c.name, round(f.value/1e9,1) AS revenue_b, f.filed_date
                 FROM facts_asof(DATE '2020-06-30') f
                 JOIN companies c USING (cik)
                 WHERE f.concept IN ('Revenues','RevenueFromContractWithCustomerExcludingAssessedTax')
                   AND f.fp_derived = 'FY'
                 QUALIFY row_number() OVER (PARTITION BY f.cik ORDER BY f.fy_derived DESC) = 1
                 ORDER BY revenue_b DESC"
name            revenue_b   filed_date
Apple Inc.      260.2       2019-10-31
MICROSOFT CORP  125.8       2019-08-01
NVIDIA CORP     10.9        2020-02-20
```

Three companies, one date, and the answer is exactly what the market knew on
2020-06-30 — including NVIDIA at $10.9B.

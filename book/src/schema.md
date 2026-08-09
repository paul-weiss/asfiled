# The schema

The store is embedded DuckDB; the published dataset is the same tables as
Parquet. Four tables, one macro.

## `registrants`

The current ticker ↔ CIK ↔ exchange snapshot (~10,400 rows).

| Column | Type | Notes |
|---|---|---|
| `cik` | UBIGINT | SEC Central Index Key |
| `name` | VARCHAR | |
| `ticker` | VARCHAR | |
| `exchange` | VARCHAR | nullable |

## `companies`

One row per ingested company.

| Column | Type | Notes |
|---|---|---|
| `cik` | UBIGINT | primary key |
| `name` | VARCHAR | |
| `sic`, `sic_description` | VARCHAR | industry classification |
| `fiscal_year_end` | VARCHAR | `MMDD`, as reported to EDGAR |
| `state_of_incorporation` | VARCHAR | |

## `filings`

The full filing index, back to the beginning of a company's EDGAR history.

| Column | Type | Notes |
|---|---|---|
| `accession` | VARCHAR | globally unique filing id |
| `cik` | UBIGINT | |
| `form` | VARCHAR | `10-K`, `8-K`, `4`, … |
| `filed_date` | DATE | |
| `period_of_report` | DATE | nullable |
| `acceptance_datetime` | TIMESTAMP | nullable |
| `is_xbrl` | BOOLEAN | |
| `items` | VARCHAR | comma-joined 8-K items, e.g. `4.02,9.01` |

## `facts`

Every numeric XBRL observation — the heart of the dataset. Append-only;
restatements are additional rows.

| Column | Type | Notes |
|---|---|---|
| `cik` | UBIGINT | |
| `taxonomy` | VARCHAR | `us-gaap`, `dei`, `ifrs-full`, … |
| `concept` | VARCHAR | XBRL tag |
| `unit` | VARCHAR | `USD`, `shares`, `USD/shares`, … |
| `period_start`, `period_end` | DATE | equal for instants |
| `is_instant` | BOOLEAN | balance-sheet items |
| `fiscal_year`, `fiscal_period` | | ⚠️ raw filing tags — do not use for period identity |
| `form` | VARCHAR | reporting form |
| `accession` | VARCHAR | reporting filing |
| `filed_date` | DATE | **the knowability boundary** |
| `value` | DOUBLE | |
| `period_kind` | VARCHAR | derived: `quarter` / `half` / `nine_month` / `annual` / `instant` / `other` |
| `fy_derived` | INTEGER | derived fiscal year (nullable) |
| `fp_derived` | VARCHAR | derived `Q1`–`Q4` / `FY` / `H1` / `9M` (nullable) |

## `facts_asof(d)` — the safe read path

A table macro. For each concept-period, returns the observation with the
latest `filed_date` on or before `d`. See
[The point-in-time model](point-in-time.md) for semantics and guarantees.

```sql
SELECT * FROM facts_asof(DATE '2019-06-30')
WHERE concept = 'Assets' AND fp_derived = 'Q2'
```

# Introduction

**asfiled** is the open point-in-time database of public company data —
*as it was filed, when it was knowable.*

Free screeners and datasets quietly use restated, look-ahead-biased
financials: the numbers you see for 2020 are the numbers as revised in 2023,
not the numbers anyone could have known in 2020. That makes them unusable for
honest backtesting, research, or fraud analysis.

asfiled is built on one rule, enforced by construction rather than
convention: **every fact carries the date it became knowable, and every query
runs as-of a date.** Restatements are visible *events* in the data — they are
never silently applied backwards.

## The three surfaces

| Surface | What it is |
|---|---|
| **Screener** | A dense, sortable table over US public-company fundamentals and flows, running entirely in your browser — no backend, no account. The defining control is the as-of date. |
| **English queries** | Ask in plain language; a language model translates your question into SQL over the safe views. The SQL is always shown — auditable, editable, yours. |
| **MCP server** | The same dataset exposed to AI agents through the Model Context Protocol, with the same point-in-time guarantees. |

## What asfiled is not

- **Not a price feed.** Equity prices are exchange-licensed; asfiled is
  fundamentals-and-flows only, and says so plainly.
- **Not investment advice.** asfiled is a data tool.
- **Not a silently-cleaned dataset.** Where source data is missing or
  ambiguous (and in SEC filings, it often is), asfiled surfaces that as a
  coverage fact rather than papering over it.

## Quick start

```sh
export ASFILED_SEC_USER_AGENT="Your Name you@example.com"  # SEC access policy
asfiled ingest AAPL MSFT NVDA
asfiled query "SELECT c.name, round(f.value/1e9,1) AS revenue_b
               FROM facts_asof(DATE '2020-06-30') f
               JOIN companies c USING (cik)
               WHERE f.concept = 'Revenues' AND f.fp_derived = 'FY'"
```

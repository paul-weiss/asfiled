# The point-in-time model

Everything in asfiled follows from one question: **what did the market know,
and when did it know it?**

## Knowability

Every XBRL observation EDGAR serves carries the accession number and filing
date of the filing that reported it. asfiled preserves both on every fact:
`filed_date` is the *knowability boundary* — before that date, the value did
not publicly exist.

A company's history is therefore not one row per period. It is one row per
**observation** of a period: the original 10-K number, the comparative
re-reported a year later, the restated figure from an amended filing. All are
kept; none are merged.

## The safe read path: `facts_asof(d)`

```sql
SELECT * FROM facts_asof(DATE '2020-06-30') WHERE concept = 'Revenues'
```

For each concept-period, `facts_asof(d)` returns the observation with the
latest `filed_date` on or before `d`:

- A restatement filed **before** `d` wins — at date `d` you would have known it.
- A restatement filed **after** `d` is invisible — at date `d` you could not have.
- A period first reported after `d` does not exist at all.

Ties (same-day amendments) break on accession number for determinism. This is
the *only* sanctioned read path for historical queries — under it, look-ahead
bias is structurally impossible, for you and for any language model writing
SQL on your behalf.

## Derived period identity

Two XBRL traps make the raw data misleading, and asfiled corrects both at
ingest:

1. **`fy`/`fp` describe the filing, not the fact.** A 10-K for FY2020 tags
   its FY2018 comparatives `fy=2020, fp=FY`. Using those fields as period
   identity misfiles years of history.
2. **Year-ends are company-specific.** Apple ends its year in late September;
   52/53-week filers drift by days annually.

Every fact therefore carries three derived columns, computed from the period
dates and the company's own fiscal year-end:

| Column | Meaning |
|---|---|
| `period_kind` | `quarter`, `half`, `nine_month`, `annual`, `instant`, or `other` |
| `fy_derived` | Fiscal year, SEC convention (labelled by the calendar year it ends in) |
| `fp_derived` | `Q1`–`Q4`, `FY`, or a cumulative stub (`H1`, `9M`) |

Filter on these — never on the raw `fy`/`fp` tags.

## What point-in-time is not

asfiled does not pretend to reconstruct information channels beyond filings:
press releases, earnings calls, and news are out of scope. "Knowable" means
*knowable from EDGAR* — the strictest, most reproducible definition available
from public data.

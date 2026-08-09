# Querying from anything

asfiled's public interface is deliberately boring: **Parquet**. Every tool in
the modern data stack reads it natively, so nobody is locked into our engine
choices.

> The published-dataset pipeline lands with milestone M2. The examples below
> show the intended shapes; today the same queries run locally via
> `asfiled query`.

## pandas / polars

```python
import pandas as pd
facts = pd.read_parquet("https://asfiled.io/data/facts.parquet")
```

```python
import polars as pl
facts = pl.scan_parquet("https://asfiled.io/data/facts.parquet")
```

## DuckDB (anywhere)

```sql
SELECT * FROM 'https://asfiled.io/data/facts.parquet' LIMIT 10;
```

## Notebooks

Example notebooks (point-in-time backtest hygiene, restatement analysis)
ship in the repository's `examples/` directory with M2.

## MCP (AI agents)

The MCP server exposes the dataset to Claude and other agents with the same
safe-view guarantees — an agent cannot write a look-ahead query any more than
you can. Lands with milestone M4.

## A note on reimplementing `facts_asof`

If you query the raw Parquet directly, the point-in-time discipline is yours
to uphold. The reference semantics: for each
`(cik, taxonomy, concept, unit, period_start, period_end)`, keep the row with
the greatest `filed_date <= your_as_of_date`, breaking ties on `accession`.
Or use the shipped views and don't think about it.

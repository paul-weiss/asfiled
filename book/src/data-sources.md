# Data sources

All sources are free, public, and redistributable. Every source is fetched
through one rate-limited, disk-cached client (8 req/s, under the SEC's
10 req/s fair-access ceiling) — a full rebuild needs no network.

## Live today — SEC EDGAR

| Dataset | Endpoint | Refresh |
|---|---|---|
| Registrant universe (ticker ↔ CIK ↔ exchange) | `company_tickers_exchange.json` | daily |
| Company metadata + filing index | `data.sec.gov/submissions/` (+ history shards) | twice daily |
| XBRL company facts (all numeric fundamentals) | `data.sec.gov/api/xbrl/companyfacts/` | twice daily |

## Planned

| Dataset | Why |
|---|---|
| Forms 3/4/5 (insider transactions) | Insider buying/selling as screener columns |
| 8-K events (incl. Item 4.02 restatement notices) | Event flags — partially present already via the filing index |
| 13F-HR (institutional holdings) | Ownership and quarter-over-quarter flows |
| DEF 14A (executive compensation) | Governance columns |
| FINRA short interest + Reg SHO daily volume | Short positioning |
| SEC fails-to-deliver | Settlement stress |
| FRED / Treasury / BLS | Macro context columns |
| FDIC call reports | Bank fundamentals EDGAR normalizes poorly |

## Deliberately absent

**Prices.** Equity price data is exchange-licensed, and every "free" source
is a terms-of-service gray zone. asfiled is fundamentals-and-flows only. A
plug-in interface for price data you license yourself may come later.

## SEC access etiquette

The SEC requires a declared User-Agent with a real contact address. asfiled
refuses to run without one:

```sh
export ASFILED_SEC_USER_AGENT="Your Name you@example.com"
```

Requests are rate-limited in one place, cached aggressively (immutable
archive documents cache forever), and bulk endpoints are preferred over
crawling.

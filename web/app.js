// asfiled screener — DuckDB-WASM over the published Parquet, entirely
// in-browser. The safe read path (facts_asof / fundamentals_asof) is
// recreated here with the same SQL the Rust store installs; the manifest
// documents the semantics for anyone reimplementing them elsewhere.

import * as duckdb from "https://cdn.jsdelivr.net/npm/@duckdb/duckdb-wasm@1.29.0/+esm";

const DATA_BASE = new URL("../data/", import.meta.url);
const TABLES = ["registrants", "companies", "filings", "facts"];

// Same definitions as the Rust store (src/store/mod.rs). If these drift,
// the browser and the CLI disagree — treat as a bug.
const MACROS = `
CREATE OR REPLACE MACRO facts_asof(d) AS TABLE
  SELECT * EXCLUDE (rn) FROM (
    SELECT *, row_number() OVER (
      PARTITION BY cik, taxonomy, concept, unit, period_start, period_end
      ORDER BY filed_date DESC, accession DESC
    ) AS rn
    FROM facts
    WHERE filed_date <= d
  ) WHERE rn = 1;

CREATE OR REPLACE MACRO fundamentals_asof(d) AS TABLE
  SELECT * EXCLUDE (rn) FROM (
    SELECT f.cik, m.item, f.fy_derived, f.fp_derived,
           f.period_end, f.filed_date, f.value,
           row_number() OVER (
             PARTITION BY f.cik, m.item, f.fy_derived, f.fp_derived
             ORDER BY m.tag_rank, f.period_end DESC, f.filed_date DESC, f.accession DESC
           ) AS rn
    FROM facts_asof(d) f
    JOIN concept_map m
      ON f.taxonomy = 'us-gaap' AND f.concept = m.tag AND f.unit = m.unit
    WHERE f.fy_derived IS NOT NULL
  ) WHERE rn = 1;
`;

// The concept map rides in facts.parquet's sibling — but v1 publishes only
// the four tables, so the map is inlined here from config/concepts.toml.
// Kept to the items this screener displays.
const CONCEPT_MAP = [
  ["revenue", ["RevenueFromContractWithCustomerExcludingAssessedTax", "RevenueFromContractWithCustomerIncludingAssessedTax", "Revenues", "SalesRevenueNet", "SalesRevenueGoodsNet", "SalesRevenueServicesNet"], "USD"],
  ["cost_of_revenue", ["CostOfRevenue", "CostOfGoodsAndServicesSold", "CostOfGoodsSold", "CostOfServices"], "USD"],
  ["gross_profit", ["GrossProfit"], "USD"],
  ["operating_income", ["OperatingIncomeLoss"], "USD"],
  ["net_income", ["NetIncomeLoss", "ProfitLoss", "NetIncomeLossAvailableToCommonStockholdersBasic"], "USD"],
  ["assets", ["Assets"], "USD"],
  ["equity", ["StockholdersEquity", "StockholdersEquityIncludingPortionAttributableToNoncontrollingInterest"], "USD"],
  ["long_term_debt", ["LongTermDebtNoncurrent", "LongTermDebt"], "USD"],
  ["operating_cash_flow", ["NetCashProvidedByUsedInOperatingActivities", "NetCashProvidedByUsedInOperatingActivitiesContinuingOperations"], "USD"],
];

const COLUMNS = [
  { key: "ticker", label: "Ticker", align: "l" },
  { key: "name", label: "Company", align: "l" },
  { key: "industry", label: "Industry", align: "l" },
  { key: "fy", label: "FY" },
  { key: "revenue", label: "Revenue", fmt: money, bar: true },
  { key: "rev_yoy", label: "Rev YoY", fmt: pct, signed: true },
  { key: "gross_mgn", label: "Gross mgn", fmt: pct },
  { key: "op_mgn", label: "Op mgn", fmt: pct, signed: true },
  { key: "net_income", label: "Net income", fmt: money, signed: true },
  { key: "ocf", label: "OCF", fmt: money, signed: true },
  { key: "debt_eq", label: "Debt/Eq", fmt: ratio },
  { key: "filed", label: "Filed", fmt: (v) => v ?? "—", cls: () => "dim" },
];

const state = {
  conn: null,
  rows: [],
  sortKey: "revenue",
  sortDir: -1,
};

function money(v) {
  if (v == null) return "—";
  const abs = Math.abs(v);
  const s = abs >= 1e9 ? `$${(abs / 1e9).toFixed(1)}B` : `$${(abs / 1e6).toFixed(0)}M`;
  return v < 0 ? `−${s}` : s;
}
function pct(v) {
  if (v == null) return "—";
  const s = `${Math.abs(v * 100).toFixed(1)}%`;
  return v < 0 ? `−${s}` : `+${s}`;
}
function ratio(v) {
  return v == null ? "—" : v.toFixed(2);
}

function screenerSql(asOf, minRev, industry) {
  const industryFilter = industry
    ? `AND c.sic_description = '${industry.replaceAll("'", "''")}'`
    : "";
  return `
WITH wide AS (
  -- Flows live on the FY duration; balance-sheet levels are instants that
  -- resolve to the fiscal Q4 close of the same year.
  SELECT cik, fy_derived AS fy,
         max(CASE WHEN item = 'revenue' AND fp_derived = 'FY' THEN value END) AS revenue,
         max(CASE WHEN item = 'cost_of_revenue' AND fp_derived = 'FY' THEN value END) AS cogs,
         max(CASE WHEN item = 'gross_profit' AND fp_derived = 'FY' THEN value END) AS gross_profit,
         max(CASE WHEN item = 'operating_income' AND fp_derived = 'FY' THEN value END) AS op_income,
         max(CASE WHEN item = 'net_income' AND fp_derived = 'FY' THEN value END) AS net_income,
         max(CASE WHEN item = 'operating_cash_flow' AND fp_derived = 'FY' THEN value END) AS ocf,
         max(CASE WHEN item = 'long_term_debt' AND fp_derived = 'Q4' THEN value END) AS lt_debt,
         max(CASE WHEN item = 'equity' AND fp_derived = 'Q4' THEN value END) AS equity,
         max(CASE WHEN item = 'revenue' AND fp_derived = 'FY' THEN filed_date END) AS revenue_filed
  FROM fundamentals_asof(DATE '${asOf}')
  WHERE fp_derived IN ('FY', 'Q4')
  GROUP BY cik, fy_derived
),
seq AS (
  SELECT *, lag(revenue) OVER (PARTITION BY cik ORDER BY fy) AS prev_revenue,
         row_number() OVER (PARTITION BY cik ORDER BY fy DESC) AS recency
  FROM wide
)
SELECT c.cik,
       coalesce(r.ticker, '#' || c.cik) AS ticker,
       c.name,
       coalesce(c.sic_description, '—') AS industry,
       s.fy,
       s.revenue,
       s.revenue / nullif(s.prev_revenue, 0) - 1 AS rev_yoy,
       coalesce(s.gross_profit, s.revenue - s.cogs) / nullif(s.revenue, 0) AS gross_mgn,
       s.op_income / nullif(s.revenue, 0) AS op_mgn,
       s.net_income,
       s.ocf,
       s.lt_debt / nullif(s.equity, 0) AS debt_eq,
       strftime(s.revenue_filed, '%Y-%m-%d') AS filed
FROM seq s
JOIN companies c USING (cik)
LEFT JOIN (SELECT cik, min(ticker) AS ticker FROM registrants GROUP BY cik) r USING (cik)
WHERE s.recency = 1 AND s.revenue >= ${Number(minRev)} ${industryFilter}
ORDER BY s.revenue DESC`;
}

async function init() {
  const bundle = await duckdb.selectBundle(duckdb.getJsDelivrBundles());
  const worker = await duckdb.createWorker(bundle.mainWorker);
  const db = new duckdb.AsyncDuckDB(
    new duckdb.ConsoleLogger(duckdb.LogLevel.WARNING),
    worker
  );
  await db.instantiate(bundle.mainModule, bundle.pthreadWorker);
  state.conn = await db.connect();

  const manifest = await (await fetch(new URL("manifest.json", DATA_BASE))).json();
  for (const t of TABLES) {
    const buf = await (await fetch(new URL(`${t}.parquet`, DATA_BASE))).arrayBuffer();
    await db.registerFileBuffer(`${t}.parquet`, new Uint8Array(buf));
    await state.conn.query(`CREATE VIEW ${t} AS SELECT * FROM '${t}.parquet'`);
  }

  // Concept map + safe views, mirroring the Rust store.
  await state.conn.query(
    "CREATE TABLE concept_map (item VARCHAR, tag VARCHAR, unit VARCHAR, tag_rank INTEGER)"
  );
  const rows = CONCEPT_MAP.flatMap(([item, tags, unit]) =>
    tags.map((tag, rank) => `('${item}', '${tag}', '${unit}', ${rank})`)
  );
  await state.conn.query(`INSERT INTO concept_map VALUES ${rows.join(",")}`);
  await state.conn.query(MACROS);

  await refreshTiles(manifest);
  await populateIndustries();
  wireControls();
  await runScreen();
}

async function refreshTiles(manifest) {
  const one = async (sql) =>
    Number((await state.conn.query(sql)).toArray()[0]?.n ?? 0);
  document.getElementById("t-universe").textContent = (
    await one("SELECT count(DISTINCT cik) AS n FROM registrants")
  ).toLocaleString();
  document.getElementById("t-companies").textContent = (
    await one("SELECT count(*) AS n FROM companies")
  ).toLocaleString();
  document.getElementById("t-facts").textContent = (
    await one("SELECT count(*) AS n FROM facts")
  ).toLocaleString();
  const generated = manifest?.generated_at_utc?.slice(0, 10) ?? "—";
  document.getElementById("t-generated").textContent = `as-filed observations · refreshed ${generated}`;
  document.getElementById("pandas-snippet").textContent =
    `pd.read_parquet("${new URL("facts.parquet", DATA_BASE).href}")`;
}

async function populateIndustries() {
  const result = await state.conn.query(
    "SELECT DISTINCT sic_description AS d FROM companies WHERE d IS NOT NULL ORDER BY 1"
  );
  const select = document.getElementById("f-industry");
  for (const row of result.toArray()) {
    const opt = document.createElement("option");
    opt.value = row.d;
    opt.textContent = row.d;
    select.appendChild(opt);
  }
}

function controls() {
  return {
    asOf: document.getElementById("asof-date").value,
    minRev: document.getElementById("f-minrev").value,
    industry: document.getElementById("f-industry").value,
  };
}

async function runScreen() {
  const { asOf, minRev, industry } = controls();
  if (!/^\d{4}-\d{2}-\d{2}$/.test(asOf)) return;
  const sql = screenerSql(asOf, minRev, industry);
  document.getElementById("sql-text").textContent = sql.trim();

  const started = performance.now();
  try {
    const result = await state.conn.query(sql);
    state.rows = result.toArray().map((r) => ({ ...r }));
    const ms = Math.round(performance.now() - started);
    document.getElementById("sql-meta").textContent =
      `single SELECT · safe views only · ${ms} ms in your browser`;
    document.getElementById("timing").textContent = `${ms} ms`;
    render();
  } catch (err) {
    document.getElementById("grid-body").innerHTML =
      `<tr><td class="l loading" colspan="12">Query failed: ${String(err).slice(0, 200)}</td></tr>`;
  }
}

function render() {
  const { sortKey, sortDir } = state;
  const rows = [...state.rows].sort((a, b) => {
    const av = a[sortKey], bv = b[sortKey];
    if (av == null) return 1;
    if (bv == null) return -1;
    return (av < bv ? -1 : av > bv ? 1 : 0) * sortDir;
  });

  const head = document.getElementById("grid-head");
  head.innerHTML = COLUMNS.map(
    (c) =>
      `<th class="${c.align ?? ""}" data-key="${c.key}">${c.label}` +
      (c.key === sortKey ? ` <span class="arr">${sortDir < 0 ? "▼" : "▲"}</span>` : "") +
      "</th>"
  ).join("");
  head.querySelectorAll("th").forEach((th) =>
    th.addEventListener("click", () => {
      const key = th.dataset.key;
      if (state.sortKey === key) state.sortDir *= -1;
      else Object.assign(state, { sortKey: key, sortDir: -1 });
      render();
    })
  );

  const maxRev = Math.max(...rows.map((r) => r.revenue ?? 0), 1);
  const body = document.getElementById("grid-body");
  if (!rows.length) {
    const asOf = document.getElementById("asof-date").value;
    const msg =
      asOf < "2009-07-22"
        ? `${asOf} predates structured XBRL: the SEC mandate phased in from 2009, and the earliest as-filed observation in this dataset is <b>2009-07-22</b>. The filings themselves exist in EDGAR — what didn't exist yet was machine-readable facts. That gap is real history, so asfiled shows it rather than papering over it.`
        : `No companies match — widen the filters or move the as-of date later.`;
    body.innerHTML = `<tr><td class="l loading" colspan="12">${msg}</td></tr>`;
  } else {
    body.innerHTML = rows
      .map((r) => {
        const cells = COLUMNS.map((c) => {
          const v = r[c.key];
          let text = c.fmt ? c.fmt(v) : String(v ?? "—");
          let cls = c.align === "l" ? "l" : "";
          if (c.key === "ticker") cls += " tick";
          if (c.key === "name") cls += " co";
          if (c.key === "industry") cls += " sec";
          if (c.signed && v != null) cls += v < 0 ? " neg" : " pos";
          if (c.cls) cls += " " + c.cls(v);
          if (c.bar && v != null) {
            const w = Math.max(1, Math.round((v / maxRev) * 120));
            text = `<span class="bar" style="width:${w}px"></span>${text}`;
          }
          const tip =
            c.key === "name" || c.key === "industry"
              ? ` title="${String(v ?? "").replace(/"/g, "&quot;")}"`
              : "";
          return `<td class="${cls.trim()}"${tip}>${text}</td>`;
        });
        return `<tr>${cells.join("")}</tr>`;
      })
      .join("");
  }
  const { minRev, industry } = controls();
  const filterDesc = [
    industry && `industry: ${industry}`,
    Number(minRev) > 0 && `revenue > $${Number(minRev) / 1e9}B`,
  ]
    .filter(Boolean)
    .join(" · ");
  document.getElementById("rowcount").textContent =
    `${rows.length} compan${rows.length === 1 ? "y" : "ies"}` +
    (filterDesc ? ` · ${filterDesc}` : "") +
    ` · sorted by ${sortKey}`;
}

function wireControls() {
  document.getElementById("asof-date").addEventListener("change", runScreen);
  document.getElementById("f-minrev").addEventListener("change", runScreen);
  document.getElementById("f-industry").addEventListener("change", runScreen);
  document.getElementById("presets").addEventListener("click", (e) => {
    const chip = e.target.closest("button.chip");
    if (!chip) return;
    const d = chip.dataset.date === "today"
      ? new Date().toISOString().slice(0, 10)
      : chip.dataset.date;
    document.getElementById("asof-date").value = d;
    runScreen();
  });
  document.getElementById("scenario").addEventListener("change", (e) => {
    if (!e.target.value) return;
    document.getElementById("asof-date").value = e.target.value;
    runScreen();
  });
  const toggle = document.getElementById("toggle-sql");
  const panel = document.getElementById("sql-panel");
  toggle.addEventListener("click", () => {
    const open = panel.hidden;
    panel.hidden = !open;
    toggle.textContent = open ? "Hide SQL" : "Show SQL";
    toggle.setAttribute("aria-expanded", String(open));
  });
}

// Accounts are optional infrastructure: the API exists on the hosted
// deployment, not on static mirrors. The control only appears when
// /api/me answers in the shape our server speaks.
async function initAccount() {
  const box = document.getElementById("account");
  const action = document.getElementById("account-action");
  const emailEl = document.getElementById("account-email");
  let me;
  try {
    const resp = await fetch("/api/me");
    if (resp.status !== 200 && resp.status !== 401) return;
    me = await resp.json();
    if (typeof me.signed_in !== "boolean") return;
  } catch {
    return;
  }
  // The hosted app requires sign-in; the server already redirects page
  // loads, this covers session expiry while the tab is open.
  if (!me.signed_in) {
    window.location.href = "/signin";
    return;
  }
  box.hidden = false;
  action.textContent = "Sign out";
  emailEl.textContent = me.email;

  action.addEventListener("click", async (e) => {
    e.preventDefault();
    await fetch("/api/auth/logout", { method: "POST" });
    window.location.href = "/signin";
  });
}

initAccount();

init().catch((err) => {
  document.getElementById("grid-body").innerHTML =
    `<tr><td class="l loading" colspan="12">Failed to start: ${String(err).slice(0, 300)}</td></tr>`;
});

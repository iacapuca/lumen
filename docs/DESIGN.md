# Lumen — Architecture & Design

> **Status:** living design document. The contract crates (`lumen-shared`,
> `lumen-artifact`) are implemented and tested; the surrounding crates are
> scaffolded against that contract. Where this document describes behaviour that
> is not yet coded, it is marked *(designed)*. Everything in §3, §5 and §11 that
> describes types, the container, the id scheme and the credit constants is *as
> built* — taken from the code, not aspiration.

---

## Table of contents

1.  [Overview & philosophy](#1-overview--philosophy)
2.  [Overall architecture](#2-overall-architecture)
3.  [Rust workspace layout](#3-rust-workspace-layout)
4.  [Compiler design](#4-compiler-design)
5.  [Artifact format — the `.lumen` DEP container](#5-artifact-format--the-lumen-dep-container)
6.  [Runtime request flow](#6-runtime-request-flow)
7.  [Semantic-layer abstraction](#7-semantic-layer-abstraction)
8.  [HTML rendering pipeline](#8-html-rendering-pipeline)
9.  [Authentication & multi-tenancy model](#9-authentication--multi-tenancy-model)
10. [Caching strategy](#10-caching-strategy)
11. [Pricing model](#11-pricing-model)
12. [Local development](#12-local-development)
13. [Cube.dev setup with TPC-H sample data](#13-cubedev-setup-with-tpc-h-sample-data)
14. [MVP implementation plan](#14-mvp-implementation-plan)
15. [Future evolution toward an edge-native analytics runtime](#15-future-evolution-toward-an-edge-native-analytics-runtime)

---

## 1. Overview & philosophy

### 1.1 What Lumen is

Lumen is an **embedding runtime** for analytics. It is *not* a BI tool. It does
not own metrics, it does not model your warehouse, it has no drag-and-drop
explorer, and it deliberately ships no "explore" surface. It sits **on top of a
semantic layer** (Cube.dev OSS for the MVP) and does exactly four things:

1. **Compile** a dashboard definition into an immutable, content-addressed
   artifact.
2. **Serve** that artifact as embeddable HTML, per request, with fresh data.
3. **Authenticate** and isolate by tenant, forwarding a row-level-security
   context to the semantic layer untouched.
4. **Meter** every render and bill on usage, not seats.

The analogy that drives every decision: **Vercel for dashboards.** Vercel did
not invent a new way to write React; it invented a *build pipeline* that
compiles a site into static + edge artifacts and a *runtime* that serves them
fast, everywhere, per request. Lumen is the same shape applied to analytics: a
compiler that turns `dashboards/sales.json` into `sales.lumen`, and an edge-ready
runtime that serves `GET /embed/dashboard/sales` by binding fresh numbers into
an already-compiled plan.

> A *lumen* is the SI unit of luminous flux — the measure of rendered light
> output. The product is billed per **render** (per emitted lumen), not per
> seat. The name is the pricing model.

### 1.2 The thesis: a dashboard should be compiled, not interpreted

Traditional embedded BI **interprets** a dashboard on every open:

```
open dashboard ──► parse JSON spec ──► run layout engine ──► resolve metrics
              ──► plan queries ──► build viz pipeline ──► fetch data ──► draw
```

Every one of those steps except "fetch data" is **pure function of the
dashboard definition** — it produces the identical output whether the dashboard
is opened once or a million times. Re-running it per request is waste: CPU,
latency, and a fat client bundle that re-derives layout in the browser.

Lumen's thesis is that the expensive work belongs at **change time**, not at
**open time**:

```
                       CHANGE TIME (once, when the dashboard is edited)
 dashboard.json ─► [ COMPILER ] ─► sales.lumen  (Dashboard Execution Plan)
                       │
                       ├─ parse + validate the spec
                       ├─ resolve auto-flow layout into absolute grid cells
                       ├─ lower each widget into a neutral Query IR
                       ├─ deduplicate queries by content hash
                       ├─ choose a renderer per widget & precompile its spec
                       └─ estimate credits, content-hash the whole plan
 ────────────────────────────────────────────────────────────────────────────
                       OPEN TIME (per request, cheap)
 GET /embed/.. ─► [ RUNTIME ] ─► HTML
                       │
                       ├─ authenticate JWT, resolve tenant + RLS context
                       ├─ load the DEP (cached, content-hash keyed)
                       ├─ execute DISTINCT queries (cached by TTL)
                       ├─ bind fresh rows into the precompiled specs
                       └─ assemble HTML, set ETag, meter the render
```

Nothing in the runtime path compiles a layout, plans a query, or sniffs a chart
type. The runtime is a *binder*: it joins fresh `Data` onto a frozen plan and
emits bytes.

### 1.3 The DEP is the differentiator

The compiler's output is **not HTML**. It is a *plan*, in exactly the sense that
a SQL planner emits a query plan or a bundler emits a bundle. We call it the
**Dashboard Execution Plan (DEP)**: a single packed `.lumen` file containing the
manifest, the resolved layout, the deduplicated queries, the precompiled chart
specs, and the compiled theme. It is:

- **Content-addressed** — its identity *is* the `blake3` hash of its bytes. An
  edit produces a new hash and therefore a new object; old objects are never
  mutated.
- **Immutable & cacheable** — `Cache-Control: public, max-age=31536000,
  immutable`; one file = one CDN object = one atomic fetch.
- **Renderer-agnostic** — each chart spec carries a `renderer` tag; the runtime
  dispatches with a `match`, never a heuristic.
- **Semantic-layer-agnostic** — queries are a neutral IR; nothing Cube-specific
  is frozen into the plan. Swapping Cube for dbt's Semantic Layer changes the
  *adapter*, not a single byte of any existing DEP.

The browser, at the end of all this, receives only three things: a **compiled
layout**, **fresh metric JSON**, and a **tiny runtime** (and the runtime only
when an interactive chart is present). No layout compiler ships to the client.
No query planner ships to the client. That is the whole product in one sentence.

### 1.4 Design tenets (the opinions we will defend)

| # | Tenet | Consequence |
|---|-------|-------------|
| T1 | Compile once, bind many | All layout/query/chart derivation is compile-time. The runtime never interprets the spec. |
| T2 | Content addressing is identity | `blake3` of bytes is the id, the cache key, the ETag, the CDN object name. Determinism is mandatory. |
| T3 | The semantic layer owns truth | Lumen never computes a metric. It forwards a query and an RLS context; the warehouse computes. |
| T4 | Fail closed on tenancy | Every result/HTML cache key carries `tenant_id` **and** `sc_hash`. No path constructs a key without both. |
| T5 | Pay for light emitted | Usage-priced compute credits. A warm render is cheap; a cold heavy query costs more. Never seat-priced. |
| T6 | The client gets data, not code | KPIs/lines are server-rendered SVG/HTML (0 KB JS). Only interactive charts ship JS, and only the data is fresh. |

---

## 2. Overall architecture

### 2.1 Component diagram

```
                          ┌──────────────────────────────────────────────┐
   author writes          │                  COMPILE TIME                 │
   dashboards/sales.json ─┼─► lumen-cli compile ─► lumen-compiler         │
                          │                          │                    │
                          │              uses lumen-shared (Query IR,     │
                          │              ChartSpec) + lumen-artifact       │
                          │                          │                    │
                          │                          ▼                    │
                          │                  sales.lumen  (DEP)           │
                          └──────────────────────────┬───────────────────┘
                                                     │  publish (content-hash keyed)
                                                     ▼
                                            ┌──────────────────┐
                                            │   DEP store      │  S3 / Postgres / FS
                                            │ id → content_hash│
                                            └─────────┬────────┘
   ┌──────────────────────────────────────────────────┼──────────────────────────────┐
   │                      REQUEST TIME (lumen-runtime, Axum)                           │
   │                                                   │                               │
   │  embed (iframe)                                   ▼                               │
   │  GET /embed/dashboard/sales      ┌─────────────────────────────┐                  │
   │  Authorization: <embed JWT> ────►│ lumen-auth                  │                  │
   │                                  │  verify HS256, build        │                  │
   │                                  │  Principal{tenant, sc, …}   │                  │
   │                                  └──────────────┬──────────────┘                  │
   │                                                 ▼                                 │
   │                            ┌──────────────────────────────────┐                  │
   │   CACHE A  (Arc<Dep>) ◄────┤ load DEP by id → content_hash    │                  │
   │   content-hash keyed       └──────────────┬───────────────────┘                  │
   │                                           ▼                                       │
   │                            ┌──────────────────────────────────┐                  │
   │   CACHE B  (QueryResult)◄──┤ for each DISTINCT query:         │                  │
   │   q:{tenant}:{sc}:{qhash}  │   hit → use; miss → lumen-semantic├──► Cube.dev      │
   │   TTL                      │   (Cube adapter, mints Cube JWT, │   /cubejs-api/v1 │
   │                            │    forwards securityContext)     │◄── warehouse     │
   │                            └──────────────┬───────────────────┘                  │
   │                                           ▼                                       │
   │                            ┌──────────────────────────────────┐                  │
   │   CACHE C  (HTML/ETag) ◄───┤ lumen-renderer: bind Data into   │                  │
   │   optional, private        │   precompiled specs → HTML       │                  │
   │                            └──────────────┬───────────────────┘                  │
   │                                           ▼                                       │
   │                            ┌──────────────────────────────────┐                  │
   │                            │ async lumen meter → Postgres     │                  │
   │                            │ (dashboard_render + per-query)   │                  │
   │                            └──────────────────────────────────┘                  │
   │                                           ▼                                       │
   │                                  200 text/html + ETag                             │
   │                                  Cache-Control: private                           │
   └───────────────────────────────────────────────────────────────────────────────────┘
                                               │
                                               ▼
                          browser: compiled layout + fresh JSON + (maybe) runtime.js
```

### 2.2 The two timelines

The single most important property of the architecture is the **clean split
between the two timelines**, and the fact that **the DEP is the only thing that
crosses between them**.

| | Compile time | Request time |
|---|---|---|
| Trigger | dashboard edit / `lumen compile` | end-user opens the embed |
| Frequency | rare (per change) | hot (per view) |
| Work | parse, validate, layout, lower to IR, dedup, choose renderer, precompile specs, hash | auth, load DEP, execute distinct queries, bind, assemble, meter |
| Cost driver | author count / edit frequency | view count × query coldness |
| Output | `sales.lumen` (DEP) | HTML |
| Determinism | byte-deterministic (same input → same hash) | data-fresh, layout-frozen |
| Where it runs | CI / build box / `lumen-cli` | runtime nodes (later: the edge) |

Everything in the left column is a pure function of the dashboard definition.
Everything in the right column is a pure function of *(DEP, principal, fresh
data)*. The DEP is the membrane. This is what makes Lumen edge-portable later
(§15): the request-time column has no compiler in it, so it can run in a
constrained WASM sandbox at a CDN POP.

---

## 3. Rust workspace layout

A Cargo workspace (`resolver = "2"`, `edition = 2021`) of eight crates. The
dependency direction is strictly downward: `lumen-shared` is the pinned contract
every other crate compiles against, and nothing depends on the runtime.

### 3.1 Crate responsibilities

| Crate | Responsibility | Status |
|-------|----------------|--------|
| **`lumen-shared`** | The pinned contract. Input model (`DashboardDef`, `WidgetDef`), neutral query IR (`Query`, `Data`), compiled model (`Manifest`, `Layout`, `Queries`, `Charts`, `ChartSpec`), ids, `SecurityContext`, metering (`MeterEvent`, credit constants), `LumenError`. Nothing Cube-specific. | **Implemented + tested** |
| **`lumen-artifact`** | Read/write the `.lumen` DEP container: the 64-byte header, section blobs, JSON TOC, `blake3` content hash, per-section integrity, forward-compatible reads. | **Implemented + tested** |
| **`lumen-semantic`** | The `SemanticLayer` trait + the Cube.dev adapter: translate the neutral `Query` IR to a Cube `/load` request, mint the Cube JWT from the security context, handle the `Continue wait` long-poll, parse string-encoded numerics. | Scaffolded *(designed)* |
| **`lumen-compiler`** | `DashboardDef` → DEP. Validate, resolve layout, lower widgets to `Query`, dedup by `QueryId`, choose a renderer per widget, precompile each `ChartSpec`, estimate credits, pack with `lumen-artifact`. | Scaffolded *(designed)* |
| **`lumen-renderer`** | DEP + `Data` → HTML. Askama shell, server-side SVG for KPI/line, ECharts data-island hydration stub for bar, theme inlining, data islands. | Scaffolded *(designed)* |
| **`lumen-auth`** | Verify the inbound embed JWT (HS256, `LUMEN_JWT_SECRET`), build the `Principal` (tenant, roles, permissions, `SecurityContext`), enforce the tenancy invariants, mint the one `cache_key` function. | Scaffolded *(designed)* |
| **`lumen-runtime`** | The Axum embedding server: `GET /embed/dashboard/{id}`, caches A/B/C (moka), metering channel + Postgres writer (sqlx), wiring of all the seams. | Scaffolded *(designed)* |
| **`lumen-cli`** | The `lumen` binary: `compile`, `serve`, `seed`. The single entry point for the local dev loop. | Scaffolded *(designed)* |

> **Why split `shared` from everything else?** Because the contract must be the
> stable, dependency-light core that the compiler and the runtime both agree on.
> `lumen-shared` pulls in only `serde`, `serde_json`, `blake3`, `hex`, `chrono`,
> `thiserror`. It has no async runtime, no HTTP client, no web framework. A DEP
> written by a compiler built against `lumen-shared@X` is readable by a runtime
> built against `lumen-shared@X`. That is the whole point of having a contract
> crate.

### 3.2 Dependency graph

```
                         ┌──────────────┐
                         │ lumen-shared │  (serde, blake3, hex, chrono, thiserror)
                         └──────┬───────┘
            ┌───────────────────┼───────────────────┬───────────────┐
            ▼                   ▼                   ▼               ▼
     ┌────────────┐     ┌──────────────┐     ┌───────────┐   ┌───────────┐
     │  artifact  │     │   semantic   │     │   auth    │   │ (compiler │
     │ (container)│     │ (Cube adapter│     │ (JWT/RLS) │   │  needs    │
     └─────┬──────┘     │  reqwest,jwt)│     └─────┬─────┘   │  artifact)│
           │            └──────┬───────┘           │         └───────────┘
           ├──────────────┐    │                   │
           ▼              ▼    │                   │
     ┌───────────┐  ┌──────────┴──┐                │
     │ compiler  │  │  renderer   │                │
     │  (dedup)  │  │ (askama,svg)│                │
     └─────┬─────┘  └──────┬──────┘                │
           │               │                       │
           └───────┬───────┴───────────┬───────────┘
                   ▼                   ▼
            ┌────────────────────────────────┐
            │          lumen-runtime          │  (axum, tower-http, sqlx, moka, uuid)
            │  depends on artifact, semantic, │
            │  compiler, renderer, auth       │
            └────────────────┬───────────────┘
                             ▼
                      ┌────────────┐
                      │ lumen-cli  │  (clap) → bin `lumen`
                      │ compile|   │   depends on shared, artifact, compiler,
                      │ serve|seed │   runtime, auth
                      └────────────┘
```

Observations that matter:

- **`lumen-artifact` depends only on `lumen-shared`.** The container does not
  know about Cube, auth, or HTTP. It serializes whatever typed sections it is
  handed. This is why a tool can read a `.lumen` file without linking the
  runtime.
- **`lumen-compiler` depends on `artifact` + `shared`, nothing else.** It needs
  no async runtime: compilation is pure CPU. (It does *not* execute queries; it
  only emits the plan to execute them.)
- **`lumen-runtime` is the only crate that fans in everything.** It is the
  composition root. The three swappable seams — `SemanticLayer`, the DEP store,
  the `Meter` — are wired here behind traits, so the Cube→dbt swap, the
  moka→Redis swap, and the FS→S3 swap are config changes, not rewrites.

### 3.3 Pinned dependency versions

From the workspace `Cargo.toml`. These are the actual pins; the research
verified them against crates.io as of June 2026.

| Dependency | Pin | Role |
|---|---|---|
| `tokio` | `1.52` (`full`) | async runtime |
| `axum` | `0.8` | embedding HTTP server (note: `{id}` path syntax, not `:id`) |
| `tower` / `tower-http` | `0.5` / `0.7` (`trace`,`cors`,`fs`) | middleware, static serving |
| `reqwest` | `0.13` (`json`,`rustls`) | Cube HTTP client |
| `async-trait` | `0.1` | `SemanticLayer` / `Meter` trait objects |
| `sqlx` | `0.9` (`postgres`,`runtime-tokio`,`tls-rustls`,`uuid`,`chrono`) | meter store |
| `serde` / `serde_json` | `1` | the entire contract is serde-defined |
| `blake3` | `1` | content addressing, ids, ETags |
| `hex` | `0.4` | hash hex encoding |
| `uuid` | `1` (`v4`,`serde`) | request/event ids |
| `chrono` | `0.4` (`serde`) | `DateTime<Utc>` ↔ `timestamptz` |
| `askama` | `0.16` | server HTML templates |
| `jsonwebtoken` | `10` | inbound embed JWT + outbound Cube JWT (HS256) |
| `moka` | `0.12` (`future`) | in-process caches A/B/C |
| `clap` | `4` (`derive`) | CLI |
| `thiserror` / `anyhow` | `2` / `1` | library errors / app boundary |
| `tracing` / `tracing-subscriber` | `0.1` / `0.3` (`env-filter`) | observability |

`[profile.release] lto = "thin"`. Frontend pins (ECharts `6.1.0`) live in
`frontend/`, not in Cargo.

---

## 4. Compiler design

`lumen-compiler` turns a `DashboardDef` (the JSON an author writes) into a DEP.
It is a pure, deterministic, CPU-only pipeline. Same input bytes → same output
bytes → same content hash.

### 4.1 The input model (`lumen-shared`)

The authored dashboard is intentionally flat and permissive; the compiler does
the validating:

```rust
pub struct DashboardDef {
    pub id: Option<String>,      // defaults to the file stem
    pub title: String,
    pub theme: String,           // default "light"
    pub layout: Vec<WidgetDef>,
}

pub struct WidgetDef {
    pub kind: WidgetKind,        // #[serde(rename="type")]: kpi|line_chart|bar_chart|table
    pub title: Option<String>,
    pub measure: Option<String>, // KPI: the single measure
    pub x: Option<String>,       // chart: x dimension (often a time dimension)
    pub y: Option<String>,       // chart: y measure
    pub granularity: Option<Granularity>, // time grouping for x
    pub format: Option<ValueFormat>,      // currency | number | percent
}
```

`dashboards/sales.json` is the canonical fixture:

```json
{
  "id": "sales",
  "title": "Sales Dashboard",
  "theme": "light",
  "layout": [
    { "type": "kpi",        "title": "Total Revenue",     "measure": "orders.revenue", "format": "currency" },
    { "type": "kpi",        "title": "Orders",            "measure": "orders.count",   "format": "number" },
    { "type": "line_chart", "title": "Revenue over time", "x": "orders.created_at", "y": "orders.revenue", "granularity": "month" },
    { "type": "bar_chart",  "title": "Revenue by status", "x": "orders.status",     "y": "orders.revenue" }
  ]
}
```

### 4.2 The pipeline

```
DashboardDef
   │  (1) validate
   ▼
[CompiledWidget]                 each widget: kind + role-checked members
   │  (2) lower to Query IR
   ▼
[(WidgetId, Query)]              widget → neutral semantic query
   │  (3) dedup by QueryId::of(&query)
   ▼
Queries { BTreeMap<QueryId, Query> }  + widget→queryId map
   │  (4) choose renderer + precompile ChartSpec per widget
   ▼
Charts  { BTreeMap<ChartId, ChartSpec> }
   │  (5) resolve auto-flow layout → absolute GridPos, build WidgetPlacement
   ▼
Layout  { grid, [WidgetPlacement{ id, kind, title, pos, query, chart, binding }] }
   │  (6) credit estimate + Manifest
   ▼
Manifest{ content_hash, source_hash, widgets, queries, renderers, required_credits, … }
   │  (7) pack with lumen-artifact DepWriter
   ▼
sales.lumen   (manifest|layout|queries|charts as JSON; theme as raw CSS)
```

**(1) Validate.** Reject a KPI with no `measure`; reject a chart with no `x`/`y`;
reject an unknown member shape. Errors are `LumenError::InvalidDashboard` /
`UnknownWidget`. Validation is the only place authoring mistakes surface — by the
time a DEP exists it is known-good.

**(2) Lower each widget to the neutral `Query` IR.** A KPI becomes
`{measures:[m]}`. A line chart becomes `{measures:[y], time_dimensions:[{x,
granularity}], order:[{x, asc}]}`. A bar chart becomes `{measures:[y],
dimensions:[x]}`. The IR is **semantic-layer-agnostic** — there is no Cube
vocabulary in it; the Cube adapter translates at execution time (§7).

```rust
pub struct Query {
    pub measures: Vec<String>,
    pub dimensions: Vec<String>,
    pub time_dimensions: Vec<TimeDimension>,   // {dimension, granularity, date_range?}
    pub filters: Vec<Filter>,                  // {member, op, values}
    pub order: Vec<Order>,                      // {field, dir}
    pub limit: Option<u32>,
}
```

Every field is `#[serde(default, skip_serializing_if = "…is_empty/is_none")]`.
This is load-bearing: it means **"absent" and "empty" serialize to the same
bytes**, so two queries that differ only in whether a default was written
explicitly hash identically.

**(3) Deduplicate by content hash.** This is the compiler's headline trick.

```rust
fn compile_queries(widgets: &[CompiledWidget]) -> (Queries, Vec<(WidgetId, QueryId)>) {
    let mut by_hash: BTreeMap<QueryId, Query> = BTreeMap::new();
    let mut wmap = Vec::new();
    for w in widgets {
        let q  = w.to_query();        // widget → neutral Query IR
        let id = QueryId::of(&q);      // blake3(canonical_json(q))[..16] → "q_<32hex>"
        by_hash.entry(id.clone()).or_insert(q);   // identical queries collapse
        wmap.push((w.id.clone(), id));
    }
    (Queries { schema: "lumen.dep.queries/1".into(), queries: by_hash }, wmap)
}
```

`QueryId::of` is `blake3` over the canonical JSON of the `Query`, truncated to
the first 16 bytes → `q_<32 hex>`. Two widgets whose queries are byte-identical
after canonicalization get the **same** id and share **one** execution at request
time. Dedup is **structural, not by widget type**: a KPI "total revenue"
(`{measures:[orders.revenue]}`) and a line "revenue by month"
(`{measures:[orders.revenue], time_dimensions:[…month]}`) differ in
`time_dimensions`, so they stay distinct — correctly. But a KPI and a sparkline
that both want "revenue by month" collapse to one query. The dedup unit is also
the **billing unit** (§11): you pay per distinct executed query, not per widget.

The map is a `BTreeMap<QueryId, Query>` precisely so the serialized `queries`
section is in sorted key order — deterministic bytes, deterministic content hash.

**(4) Choose a renderer and precompile the chart spec.** The renderer policy is
fixed for the MVP (per-dashboard default with per-widget override later):

| Widget kind | Renderer | `ChartSpec` variant | Client JS |
|---|---|---|---|
| `kpi` | **HTML** | `ChartSpec::Html { kind: Kpi, template, number_format, theme_ref }` | 0 KB |
| `line_chart` | **SVG** | `ChartSpec::Svg { kind: Line, encoding, geometry, palette, theme_ref }` | 0 KB |
| `bar_chart` | **ECharts** | `ChartSpec::Echarts { kind: Bar, option_template, theme_ref }` | runtime.js (shared) |
| `table` | **HTML** | `ChartSpec::Html { kind: Table, template, … }` | 0 KB |

The `ChartSpec` is `#[serde(tag = "renderer")]`, so the renderer choice is the
discriminator the runtime matches on. The spec is **result-shape-agnostic** — it
says *how to draw an axis / a bar / a number*, not *what the data is*. The
separate `Binding` (in `Layout`) says which result column lands on which channel.
That separation is what lets one query feed several widgets with different
bindings.

For an SVG line chart the compiler freezes the geometry and encoding:

```jsonc
"c_line_rev_month": {
  "renderer": "svg", "kind": "line",
  "encoding": { "x": { "scale": "time",   "format": "%b %Y", "grid": false },
                "y": { "scale": "linear", "format": "$,.0f", "grid": true } },
  "geometry": { "width": 640, "height": 320, "margin": [16,16,28,48], "curve": "monotone" },
  "palette": ["#2563eb"], "theme_ref": "light"
}
```

For an ECharts bar chart it freezes the `option` skeleton (no data):

```jsonc
"c_bar_rev_status": {
  "renderer": "echarts", "kind": "bar",
  "option_template": { "xAxis": { "type": "category" },
                       "yAxis": { "type": "value" },
                       "dataset": { "dimensions": ["x","y"], "source": [] },
                       "series": [ { "type": "bar", "encode": { "x": "x", "y": "y" } } ] },
  "theme_ref": "light"
}
```

**(5) Resolve the layout.** No layout engine runs per request. The compiler
takes the authored order and resolves the auto-flow grid into absolute cells,
producing one `WidgetPlacement` per widget:

```rust
pub struct WidgetPlacement {
    pub id: WidgetId,        // "w_0", positional, stable within a DEP
    pub kind: WidgetKind,
    pub title: String,
    pub pos: GridPos,        // { x, y, w, h } — absolute, resolved
    pub query: QueryId,      // → queries section
    pub chart: ChartId,      // → charts section
    pub binding: Binding,    // { value?, format?, x?, y? } → maps columns to channels
}
```

**(6) Estimate credits + build the manifest.** The compile-time estimate uses
the `distinct-query/v1` model: a floor of one `QUERY_BASE` credit per *distinct*
query (so dedup is visible to the buyer up front). The runtime computes the real
charge per request from the full formula (§11). The manifest records the content
hash, the source hash (hash of the input definition), widget/query ids, the set
of renderers used, and the runtime-asset pointer.

```rust
pub struct Manifest {
    pub schema: String,                 // "lumen.dep.manifest/1"
    pub dep_id: DashboardId,
    pub content_hash: Hash,             // "blake3:<64hex>"
    pub source_hash: Hash,
    pub title: String,
    pub theme: String,
    pub created_at: DateTime<Utc>,
    pub compiler: CompilerInfo,         // { name, version, semantic_adapter }
    pub format: FormatInfo,             // { container_ver, min_runtime }
    pub widgets: Vec<WidgetId>,
    pub queries: Vec<QueryId>,
    pub renderers: Vec<Renderer>,       // svg | echarts | html
    pub required_credits: CreditEstimate, // { model:"distinct-query/v1", estimate, queries }
    pub runtime_asset: RuntimeAsset,    // { url, integrity, embedded }
}
```

**(7) Pack.** Hand the four typed sections + the raw theme CSS to
`lumen-artifact`'s `DepWriter`:

```rust
let mut w = DepWriter::new();
w.add_json(section::MANIFEST, &manifest)?;
w.add_json(section::LAYOUT,   &layout)?;
w.add_json(section::QUERIES,  &queries)?;
w.add_json(section::CHARTS,   &charts)?;
w.add_raw (section::THEME,    theme_css_bytes);
let dep_bytes = w.finish();   // → sales.lumen
```

### 4.3 Worked example: `sales.json` → `sales.lumen`

Four widgets lower to four distinct queries (no two collapse here):

| Widget | Kind | Query IR (neutral) | `QueryId` | Renderer |
|---|---|---|---|---|
| `w_0` Total Revenue | kpi | `{measures:[orders.revenue]}` | `q_3a8f…` | html |
| `w_1` Orders | kpi | `{measures:[orders.count]}` | `q_77c0…` | html |
| `w_2` Revenue over time | line | `{measures:[orders.revenue], time_dimensions:[{orders.created_at, month}], order:[{orders.created_at, asc}]}` | `q_9bd4…` | svg |
| `w_3` Revenue by status | bar | `{measures:[orders.revenue], dimensions:[orders.status]}` | `q_2e51…` | echarts |

Result: `widgets.len() == 4`, `queries.len() == 4`,
`required_credits = { model:"distinct-query/v1", estimate:4, queries:4 }`,
`renderers = [html, svg, echarts]`. Had a second "revenue by month" widget
existed, it would reuse `q_9bd4…` → `queries.len()` would stay below
`widgets.len()` and the estimate would not rise. One DEP, three renderers, one
shared query map.

> Note on member names: `sales.json` uses `orders.revenue` for readability. In
> the TPC-H-backed model (§13) canonical revenue lives on `lineitem`
> (`lineitem.revenue`). The MVP either remaps the dashboard members to the
> TPC-H model (`lineitem.revenue`, `orders.order_date`, `orders.status`,
> `orders.count`) or exposes an `orders.revenue` convenience measure. Member
> names are part of the authored spec, not the engine.

---

## 5. Artifact format — the `.lumen` DEP container

This section is **as built**: it describes `lumen-artifact` exactly as
implemented and tested (`crates/artifact/src/lib.rs`).

### 5.1 Why a single packed file (and not a directory)

The README's expanded view (`manifest.json`, `layout.json`, `charts/*.chart`,
`theme.css`, `runtime.js`) is the **debug projection** only. The shipped artifact
is **one packed file**, `<id>-<hash>.lumen`. The reasons are not aesthetic:

- **One content hash.** The hash of the file *is* its identity, its CDN object
  name, its ETag, its edge-KV key. A directory has no single hash.
- **One atomic fetch.** The runtime (and later an edge node) does one GET / one
  read / one mmap, not N syscalls with partial-failure modes.
- **Random section access.** A TOC indexes sections by offset, so the runtime can
  read the manifest and plan queries **without touching** `theme` — something a
  tarball (no random index, padding) cannot do.
- **Write-once, read-many.** SQLite would be overkill; a tarball wastes space on
  padding; a directory loses atomicity. The custom frame is ~80 lines of Rust.

### 5.2 Container layout

```
┌── header (64 bytes, fixed) ─────────────────────────────────────────┐
│ off   field            type      value / meaning                     │
│ 0..4  magic            [u8;4]    b"LMN1"                              │
│ 4..6  container_ver    u16 LE    = 1  (format of THIS frame)         │
│ 6..8  flags            u16 LE    = 0  (reserved; e.g. sections_zstd) │
│ 8..16 toc_offset       u64 LE    byte offset of the TOC              │
│ 16..20 toc_len         u32 LE    byte length of the TOC              │
│ 20..22 section_count   u16 LE    number of sections                  │
│ 22..32 _reserved       [u8;10]   zero                                │
│ 32..64 content_hash    [u8;32]   blake3 over EVERYTHING after byte 64│
└─────────────────────────────────────────────────────────────────────┘
┌── sections (blobs, back to back, starting at byte 64) ──────────────┐
│ manifest (json) · layout (json) · queries (json) · charts (json)    │
│ · theme (raw css)            [MVP: JSON sections; MsgPack in prod]   │
└─────────────────────────────────────────────────────────────────────┘
┌── TOC (at toc_offset, JSON array, written LAST) ────────────────────┐
│ [ { name, codec, offset, len, hash[32] }, … ]                       │
└─────────────────────────────────────────────────────────────────────┘
```

```rust
pub const MAGIC: [u8; 4] = *b"LMN1";
pub const CONTAINER_VER: u16 = 1;
pub const HEADER_LEN: usize = 64;

pub enum Codec { Json, MsgPack, Raw }   // #[serde(rename_all="lowercase")]

pub struct TocEntry {
    pub name: String,    // "manifest" | "layout" | "queries" | "charts" | "theme"
    pub codec: Codec,
    pub offset: u64,     // from start of file
    pub len: u32,
    pub hash: [u8; 32],  // blake3 of THIS section's bytes (per-section integrity)
}
```

Key facts, exactly as coded:

- **The TOC is written last** so the writer can stream sections and then backfill
  offsets. `content_hash` is computed over `bytes[64..]` — i.e. **every section
  plus the TOC** — so the header is never self-referential.
- **The TOC is JSON**, and **MVP sections are JSON** (`Codec::Json`) for
  debuggability, with `theme` as `Codec::Raw`. The `Codec` enum **already
  includes `MsgPack`**: MessagePack is the **documented production codec** for
  `layout`/`queries`/`charts` (compact, field-name maps → forward-compatible).
  Switching a section's codec is a writer change; the reader dispatches on the
  TOC's `codec` field. We ship JSON first because a `.lumen` you can `xxd` and
  read is worth more than a few kilobytes during MVP.
- **Reads are O(header + TOC).** The reader owns the byte buffer (`Vec<u8>`); each
  section is a zero-copy slice into it (`&self.bytes[offset..offset+len]`). No
  layout parsing is needed to plan queries. *(mmap-backed zero-copy is a future
  option for the edge; the MVP owns the bytes.)*

### 5.3 Writer and reader

```rust
// WRITE
let mut w = DepWriter::new();
w.add_json("manifest", &manifest)?;   // canonical JSON section
w.add_raw ("theme",    css.into());    // raw bytes section
let bytes: Vec<u8> = w.finish();       // lays out sections, appends TOC, writes header

// READ
let dep = Dep::from_bytes(bytes)?;     // validates magic, version, content hash
let m   = dep.manifest()?;             // typed accessor over the shared model
let css = dep.theme_css();             // &[u8]
let names = dep.section_names();       // TOC-driven; tolerates unknown sections
let etag  = dep.content_hash_tagged(); // "blake3:<64hex>" — manifest / ETag / CDN key
```

`Dep::from_bytes` enforces, in order: minimum length (≥64), magic == `LMN1`,
`container_ver == 1` (else `UnsupportedVersion`), TOC bounds, and then it
**recomputes `blake3(bytes[64..])` and compares to the stored `content_hash`**,
returning `HashMismatch` on any corruption. The test suite proves all three
failure modes (`round_trip_and_hash_verify`, `corruption_is_detected`,
`rejects_non_lumen`).

### 5.4 Versioning — four independent axes

Do not conflate these. Each answers a different question.

| Axis | Where | Bumps when | On mismatch |
|---|---|---|---|
| `container_ver` | header (`u16`) | the binary frame itself changes | **hard refuse** (`UnsupportedVersion`) — it's the envelope |
| `schema` tag `lumen.dep.X/N` | each section (`"…/1"`) | a section's struct evolves incompatibly | minor add → load; major bump → refuse, ask for recompile |
| `compiler.version` | manifest | compiler logic changes | informational (traceability), never gating |
| `min_runtime` | manifest `FormatInfo` | the DEP needs runtime features ≥ N | runtime < `min_runtime` → refuse |

### 5.5 Forward-compatibility rules

Enforced in `lumen-artifact` + the serde derives in `lumen-shared`:

1. **Unknown sections in the TOC are ignored.** Iteration is TOC-driven and
   by-name, never positional, so a newer compiler can add a section (say
   `prefetch`) and an older runtime simply doesn't read it.
2. **Unknown fields are ignored.** We rely on `#[serde(default)]` /
   `skip_serializing_if` and deliberately **do not** use `deny_unknown_fields`,
   so an old runtime reads a newer additive DEP.
3. **Schema minor additions load; major bumps refuse.** `lumen.dep.layout/1`
   gaining an optional field still loads under `/1`. Going to `/2` is a signal
   to recompile.
4. **`container_ver` mismatch is fatal.** The frame is the one thing that cannot
   be forward-read.

### 5.6 Why content addressing here

Because the canonical serialization is deterministic (sorted-key `BTreeMap`s for
`queries`/`charts`, fixed struct field order, `skip_serializing_if` collapsing
empties), recompiling an unchanged dashboard produces **byte-identical** output
and therefore the **same hash** — no cache churn, no redundant CDN purge. An edit
produces a new hash and a new immutable object. The filename carries the short
hash (`sales-9f2c8a1d.lumen`); HTTP serves it `ETag: "blake3:…"`,
`Cache-Control: public, max-age=31536000, immutable`; the edge keys it by full
content hash so two dashboards compiling to identical plans share one cache
entry.

---

## 6. Runtime request flow

`lumen-runtime` serves one route that matters: `GET /embed/dashboard/{id}`. The
JWT arrives in `Authorization: Bearer <token>` or, for `iframe` embeds that can't
set headers, a signed query parameter. The flow is ordered, and the **cache
placement is the design** — A, B, C sit at three different lifetimes.

```
 1. Parse {id}. Reject malformed early → 400. No I/O yet.

 2. Verify the inbound embed JWT  [lumen-auth]
      - HS256, LUMEN_JWT_SECRET; verify sig + exp + nbf + iss + aud.
      - leeway ≤ 30s. INVARIANT: validate_exp = true, fail-closed → 401.

 3. Build the Principal (pure claim destructuring, no DB hit)
      Principal { tenant_id, sub, roles, permissions, security_context, sc_hash }
      - security_context taken VERBATIM from the claim.
      - sc_hash = blake3(canonical_json(security_context))[..16]  (computed ONCE)

 4. Authorize: principal.permissions ⊇ dashboard.required_permission? else 403.

 5. Load the DEP                         ┌──────────────── CACHE A ───────────────┐
      key = dep:{id}:{content_hash}      │ Arc<Dep>, content-hash keyed, process- │
      - resolve id → content_hash via a  │ lifetime. Immutable ⇒ no TTL. Pointer  │
        short-TTL pointer lookup.        │ id→hash cached ≤30s so republish is    │
      - miss → DepStore.get() → Dep::    │ picked up quickly.                     │
        from_bytes → insert Arc.         └────────────────────────────────────────┘

 6. dep.queries()  →  BTreeMap<QueryId, Query>   (distinct queries only)

 7. For each DISTINCT query (bounded concurrency, buffer_unordered(8)):
        qkey = q:{tenant_id}:{sc_hash}:{blake3(canonical_json(query))}
                                          ┌──────────── CACHE B ───────────────────┐
      a. lookup CACHE B                   │ QueryResult, TTL (default 60s, per-     │
      b. HIT  → emit MeterEvent{CacheHit} │ query override). The HOT path.         │
         MISS → SemanticLayer.execute(    │ key MUST carry tenant_id AND sc_hash.  │
                  query, security_context)└─────────────────────────────────────────┘
                 - Cube adapter mints Cube JWT (payload = securityContext),
                   POSTs /cubejs-api/v1/load, handles "Continue wait".
                 - time it → exec_time_ms; record Cube payload bytes.
                 - emit MeterEvent{SemanticCompute, exec_time_ms, bytes}
                 - emit MeterEvent{CacheMiss}
                 - insert into CACHE B under qkey with TTL.

 8. Bind: results: HashMap<QueryId, Data>. For each widget in dep.layout():
        data = &results[&w.query]          // O(1) fan-out, shared result
        spec = &dep.charts().specs[&w.chart]
        html_fragment = renderer::render_widget(spec, &w.binding, data, &w.pos)
      No layout compilation. No query planning. Just bind.

 9. Assemble HTML + ETag                  ┌──────────── CACHE C (optional) ─────────┐
      etag = blake3(content_hash ‖ sc_hash ‖ max(result_versions))                  │
      - If-None-Match == etag → 304 (no   │ html:{id}:{content_hash}:{tenant}:{sc}  │
        body; still meters a light render)│ short TTL = min(query TTLs). Off by MVP.│
      - else cache HTML under CACHE C.    └─────────────────────────────────────────┘

10. Emit MeterEvent{DashboardRender, bytes = len(html)}  (async, non-blocking).

11. Return 200 text/html
      ETag: "<etag>"
      Cache-Control: private          ← RLS-scoped; never public/CDN-shared
```

Cache lifetimes at a glance: **A** = process lifetime (busts on content-hash
change), **B** = seconds–minutes (the hot path), **C** = seconds (or off). The
critical property: **no step compiles a layout, plans a query, or sniffs a chart
type.** Step 8 is a `match` on the serde `renderer` tag and a column→channel
bind. That is the "compile, don't interpret" contract, enforced by the absence of
a compiler in the request path.

Defaults to ship: per-request total timeout 5s; per-Cube-call timeout 3s with one
jittered retry on 5xx/timeout; the meter emit is a non-blocking channel send that
never fails the request.

---

## 7. Semantic-layer abstraction

Lumen never computes a metric. It forwards a neutral query and an RLS context to
a semantic layer, which compiles SQL, runs it against the warehouse, and returns
rows. The seam is one trait.

### 7.1 The trait

```rust
#[async_trait]
pub trait SemanticLayer: Send + Sync {
    async fn execute(&self, q: &Query, sc: &SecurityContext)
        -> Result<Data, SemanticError>;
}
```

Two things are deliberate. First, the **signature requires `&SecurityContext`** —
you cannot call `execute` without passing the RLS context, so "forgot to forward
it" is a compile error, not a production leak (security invariant #2, §9).
Second, the argument is the **neutral `Query` IR**, not a Cube query. Nothing
Cube-specific is frozen anywhere upstream; the adapter is the only place Cube
vocabulary appears.

`Data` mirrors a semantic-layer result: rows keyed by full member name, plus an
opaque annotation map. Crucially, **numeric values may arrive as JSON strings**
(Cube returns `"700"`), so `Data` carries the coercion:

```rust
pub type Row = serde_json::Map<String, serde_json::Value>;
pub struct Data { pub rows: Vec<Row>, pub annotation: serde_json::Value }
impl Data { pub fn scalar(&self, member: &str) -> Option<f64> { … } } // parses "700" → 700.0
```

### 7.2 The Cube.dev adapter (`lumen-semantic`)

Targets **Cube Core (OSS) v1.6.64**. Specifics, all verified against the current
docs:

- **Endpoint.** `POST /cubejs-api/v1/load` with body `{"query": { … }}`,
  `Content-Type: application/json`. (GET with a URL-encoded `query` param also
  works; POST is preferred for a Rust client.) `CUBE_URL` defaults to
  `http://localhost:4000`.
- **Query translation.** The adapter maps the neutral IR onto the Cube query
  object. The IR was designed to be a near-1:1 lowering:

  | Neutral IR | Cube query field | Note |
  |---|---|---|
  | `measures: [String]` | `measures` | `"cube.member"` names |
  | `dimensions: [String]` | `dimensions` | |
  | `time_dimensions: [{dimension, granularity, date_range?}]` | `timeDimensions` | granularity ∈ second…year; `dateRange` ISO `[from,to]` or relative `"last 12 months"` |
  | `filters: [{member, op, values}]` | `filters` | `op`: equals/notEquals/gt/gte/lt/lte/contains/notContains/set/notSet/inDateRange/beforeDate/afterDate (camelCase matches Cube operators) |
  | `order: [{field, dir}]` | `order` | array form `[["m","asc"], …]` |
  | `limit: Option<u32>` | `limit` | |

- **Authentication & RLS.** Cube authenticates with a JWT in the `Authorization`
  header, **signed with `CUBEJS_API_SECRET` (HS256)**, whose **payload *is* the
  `securityContext`**. So the adapter mints a *fresh outbound Cube JWT per call*
  by signing the principal's `security_context` claim with the Cube secret. Cube
  then enforces row-level security server-side via `queryRewrite` /
  `COMPILE_CONTEXT` (e.g. inject a mandatory `allowed_stores` filter). Lumen never
  rewrites the context — it only forwards and hashes it.

  > **Two JWTs, do not confuse them.** The *inbound embed JWT* is verified by
  > `lumen-auth` with `LUMEN_JWT_SECRET` and carries tenant/roles/permissions +
  > `security_context`. The *outbound Cube JWT* is minted by `lumen-semantic`
  > with `CUBEJS_API_SECRET` and carries *only* the security context. Different
  > secret, different direction, different purpose.

- **The "Continue wait" long-poll.** When a query is still computing, Cube
  returns **HTTP 200** with body `{"error": "Continue wait"}`. This is **not** an
  error and **not** a cancellation — the database query is still running. The
  contract: **re-POST the identical request in a loop** until a normal
  `{data, annotation, …}` body returns. Subsequent identical calls are idempotent
  (they don't schedule new DB work). The adapter detects it as *HTTP 200 AND
  top-level `error == "Continue wait"`* and polls with small jittered backoff up
  to the per-call timeout. Cube holds each request up to `continueWaitTimeout`
  (default 10s) before returning the sentinel.

- **Response parsing.** `data[]` is rows keyed by member name; parse string
  numerics with `value_as_f64`; `annotation` carries `{title, shortTitle, type}`
  per member for formatting hints.

```rust
// shape of the adapter's execute()
async fn execute(&self, q: &Query, sc: &SecurityContext) -> Result<Data, SemanticError> {
    let cube_jwt = sign_hs256(sc.0.clone(), &self.cube_api_secret)?;     // payload = securityContext
    let body = json!({ "query": translate(q) });
    loop {
        let resp = self.http.post(format!("{}/cubejs-api/v1/load", self.base))
            .header(AUTHORIZATION, &cube_jwt).json(&body).send().await?;
        let v: serde_json::Value = resp.json().await?;
        if v.get("error").and_then(|e| e.as_str()) == Some("Continue wait") {
            sleep(backoff()).await; continue;        // idempotent re-poll
        }
        return Ok(parse_data(v));                    // { rows, annotation }
    }
}
```

### 7.3 Future adapters

Because the contract is the neutral IR and the trait, alternative backends are
*adapters*, not rewrites:

- **dbt Semantic Layer** — translate the IR to a `query_metrics` GraphQL/JDBC
  call; the security context maps to dbt's entity filters.
- **MetricFlow** — direct MetricFlow query API.
- **Custom REST** — for teams with a bespoke metrics service.

None of these touches the compiler, the DEP, the query ids, the dedup, or the
credit math. They implement one `async fn execute`.

---

## 8. HTML rendering pipeline

`lumen-renderer` turns a DEP plus a `HashMap<QueryId, Data>` into HTML. The
guiding rule (tenet T6): **the browser gets data, not code.** The renderer
dispatches on the `ChartSpec`'s serde `renderer` tag:

```rust
match spec {
    ChartSpec::Svg     { .. } => render_svg(spec, binding, data, pos),     // string of SVG
    ChartSpec::Html    { .. } => render_html(spec, binding, data, pos),    // Askama partial
    ChartSpec::Echarts { .. } => emit_hydration_stub(spec, binding, data), // <div data-chart> + JSON island
}
```

### 8.1 KPI → pure server HTML/SVG (0 KB JS)

A KPI is a number, an optional delta, and (optionally) an inline sparkline whose
points the **server** computes from the metric series. No library, no hydration,
instant paint:

```html
<article class="kpi">
  <h3>Total Revenue</h3>
  <strong>$1,284,003</strong>
  <span class="delta up">+6.6%</span>
  <svg viewBox="0 0 100 24" preserveAspectRatio="none">
    <polyline fill="none" stroke="currentColor" stroke-width="1.5" points="0,18 50,6 100,14"/>
  </svg>
</article>
```

`render_html` uses the `ChartSpec::Html { template, number_format, … }` and the
`Binding { value, format }` to format the scalar (`number_format` =
currency/number/percent). Zero client JS for this widget.

### 8.2 Line chart → server-rendered SVG (0 KB JS)

`render_svg` produces the **final SVG server-side** from the frozen
`encoding` + `geometry` + the fresh `Data`. The compiler froze the axes, scales,
margins, palette and curve; the renderer maps the data series to a `<path>` /
`<polyline>` and emits the axes/gridlines. Best for KPIs, sparklines, print, SEO,
and no-JS embeds. Still zero client JS.

### 8.3 Bar / interactive chart → ECharts data-island hydration

Interactive charts use **Apache ECharts 6.1.0** (tree-shaken: line + bar + SVG
renderer + grid/tooltip/dataset/dataZoom components ≈ 50–70 KB gz). The
compiled `option` skeleton (no data) is shipped **inside the DEP**; the fresh
metric rows are injected at request time into a **separate**
`<script type="application/json">` data island; a tiny `runtime.js` binds them
late. The two are never mixed:

```html
<div class="chart" data-chart="rev_status" style="width:100%;height:240px"></div>

<!-- COMPILED spec: frozen at build, shipped from the DEP. No data. -->
<script id="spec:rev_status" type="application/json">
{ "option": { "xAxis": { "type": "category" }, "yAxis": { "type": "value" },
              "dataset": { "dimensions": ["x","y"], "source": [] },
              "series": [ { "type": "bar", "encode": { "x": "x", "y": "y" } } ] } }
</script>

<!-- FRESH metric JSON: injected at request time, kept separate from the spec. -->
<script id="data:rev_status" type="application/json">
{ "rows": [["O",812003.42],["F",402551.10],["P",69448.77]] }
</script>
```

`frontend/runtime/runtime.js` (no React; ~50–70 KB gz tree-shaken) reads each
spec island and binds the matching data island late via `dataset.source`:

```js
import * as echarts from 'echarts/core';
import { LineChart, BarChart } from 'echarts/charts';
import { GridComponent, TooltipComponent, DatasetComponent, DataZoomComponent } from 'echarts/components';
import { SVGRenderer } from 'echarts/renderers';
echarts.use([LineChart, BarChart, GridComponent, TooltipComponent, DatasetComponent, DataZoomComponent, SVGRenderer]);

const island = id => JSON.parse(document.getElementById(id).textContent);
for (const el of document.querySelectorAll('[data-chart]')) {
  const key = el.dataset.chart;
  const { option } = island(`spec:${key}`);  // compiled spec from DEP
  const { rows }   = island(`data:${key}`);  // fresh data, separate island
  option.dataset.source = rows;              // <-- late binding
  const chart = echarts.init(el, null, { renderer: 'svg' });
  chart.setOption(option);
  addEventListener('resize', () => chart.resize());
}
```

Live updates (§15) reuse the same skeleton:
`chart.setOption({ dataset: { source: newRows } })` — ECharts diffs and animates.

### 8.4 The Askama shell and what reaches the browser

The page is an Askama template: the compiled grid shell (`grid.cols`,
`row_height`, `gap` from `Layout`), the theme CSS inlined from the DEP's `theme`
section, the per-widget SVG/HTML fragments, and — **only if at least one
`echarts` widget exists** — a single
`<script src="/_lumen/runtime-<hash>.js">` plus the inline data islands. The
runtime asset is referenced by content hash (`runtime_asset.url` + `integrity`
in the manifest) and served once from the CDN, deduped across every dashboard;
it is **not** embedded per DEP (unless a `self_contained` flag is set).

So the complete payload the browser receives is:

```
compiled layout shell  (frozen, from the DEP)
  + theme.css inline    (frozen, from the DEP)
  + per-widget SVG/HTML  (server-rendered, fresh numbers)
  + [ data islands + one runtime.js ]   only when an interactive chart is present
```

No layout compilation, no chart-type sniffing, no query planning in the browser.
A KPI-and-line dashboard ships **zero** JavaScript.

---

## 9. Authentication & multi-tenancy model

### 9.1 Claim shape

The inbound embed JWT (the host application mints it for its logged-in user):

```json
{
  "iss": "lumen-issuer",
  "aud": "lumen-embed",
  "sub": "user_8842",
  "exp": 1751212800,
  "nbf": 1751209200,
  "iat": 1751209200,
  "tenant_id": "acme",
  "roles": ["viewer", "finance"],
  "permissions": ["dashboard:revenue:read"],
  "security_context": {
    "tenant_id": "acme",
    "region": "us",
    "user_id": "user_8842",
    "allowed_stores": [12, 19, 44]
  }
}
```

```rust
struct Claims {
    iss: String, aud: String, sub: String,
    exp: i64, nbf: i64, iat: i64,
    tenant_id: String,
    #[serde(default)] roles: Vec<String>,
    #[serde(default)] permissions: Vec<String>,
    security_context: serde_json::Value,   // opaque to Lumen, meaningful to Cube
}

struct Principal {
    tenant_id: String,
    sub: String,
    permissions: HashSet<String>,
    security_context: SecurityContext,     // newtype over serde_json::Value
    sc_hash: String,                        // blake3(canonical_json)[..16], computed once
}
```

`SecurityContext` is a `#[serde(transparent)]` newtype over `serde_json::Value`;
its `sc_hash()` is `hex(blake3(canonical_json(value))[..16])` — a stable 32-hex
digest. Because `serde_json::Map` is a `BTreeMap` (sorted keys), two
logically-equal contexts hash identically and two differing ones never collide.

### 9.2 Verification (MVP: HS256 shared secret)

```rust
let mut v = Validation::new(Algorithm::HS256);
v.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
v.set_issuer(&["lumen-issuer"]);
v.set_audience(&["lumen-embed"]);
v.leeway = 30;            // small clock skew, not unbounded
v.validate_exp = true;    // INVARIANT
v.validate_nbf = true;
let data = decode::<Claims>(token, &DecodingKey::from_secret(secret), &v)?;
```

Signature, `exp`, `nbf`, `iss`, `aud` are checked **on every request**, fail
closed → 401. There is no "skip auth in dev" branch compiled into release builds.

**Path to production:** swap `HS256 + shared secret` → `RS256/ES256 + JWKS
endpoint + kid rotation`. Only the `Validation` / `DecodingKey` construction
changes; **the claim shape is stable**, so nothing downstream moves. Keep two
active secrets for zero-downtime rotation.

### 9.3 Security context → Cube

The `security_context` claim is forwarded **verbatim** as Cube's
`securityContext` (it becomes the payload of the outbound Cube JWT, §7.2). Cube's
`queryRewrite` / `COMPILE_CONTEXT` reference it to enforce row-level security at
the warehouse — e.g. inject a mandatory `allowed_stores IN (12,19,44)` filter.
**Lumen never parses, trusts, or rewrites it for filtering.** It only (a) hashes
it for cache keying and (b) forwards it. RLS lives in Cube; Lumen must not assume
it has filtered anything.

### 9.4 The security invariants (MUST hold — fail closed)

These are not guidelines. They are enforced structurally (one `cache_key`
function taking `&Principal`; a trait signature that requires the context) and
backed by unit tests.

1. **No cross-tenant cache hits.** Every layer-B and layer-C cache key contains
   **`tenant_id` AND `sc_hash`**. Keys are minted by exactly one
   `fn cache_key(&Principal, …)`; raw string concatenation for cache keys is
   forbidden. A missing tenant segment is a debug-panic / prod-500, never a silent
   global key. A unit test asserts that two principals differing only in
   `tenant_id` produce different keys for an identical query. *(The implemented
   test `sc_hash_isolates_tenants` already proves the `sc_hash` half of this.)*

2. **`securityContext` is always forwarded to Cube, verbatim, unmodified.** No
   query reaches a `SemanticLayer` without the request's `security_context`. The
   `execute(&self, q, sc: &SecurityContext)` signature makes "forgot to pass it"
   a **compile error**.

3. **JWT `exp` (and signature, `nbf`, `iss`, `aud`) verified on every request.**
   `validate_exp = true`, bounded leeway (≤30s), fail-closed 401. If a
   verified-claims cache is ever added, its TTL is capped at `min(30s, exp−now)`
   so it can never outlive the token.

4. **Security context participates in cache identity, not just tenant.** Two users
   in the same tenant with different RLS scopes (`allowed_stores`) must never share
   a layer-B/C entry — hence `sc_hash` is mandatory alongside `tenant_id`.
   Canonicalize before hashing so equal contexts coalesce and unequal ones never
   collide.

5. **HTML responses are `Cache-Control: private`.** RLS-scoped HTML must never
   land in a shared/CDN cache. ETags derive from
   `content_hash ‖ sc_hash ‖ result_version`, so a stale ETag can never serve
   another scope's body. The immutable-public caching applies to the DEP object,
   **never** to a rendered, tenant-scoped HTML response.

---

## 10. Caching strategy

Three layers at three lifetimes. The placement is the whole performance story:
the DEP is cached forever (it's immutable), query results are cached for seconds,
and rendered HTML is cached briefly or not at all.

### 10.1 (A) Compiled-DEP cache — long-lived, content-hash keyed

```
key:   dep:{dashboard_id}:{content_hash}
value: Arc<Dep>
store: in-process moka/LRU (MVP). Cap by count or weight.
TTL:   none — content-hash addressed ⇒ immutable. The only mutable bit is the
       pointer dashboard_id → content_hash, cached ≤30s.
```

Invalidation is implicit: publishing a new DEP changes `content_hash` → new key;
the old `Arc<Dep>` ages out by LRU. No explicit bust. The pointer flip
(`dashboard_id → new_hash`) is the one mutable thing; expire it in ≤30s or push
an invalidation event so a republish is picked up promptly.

### 10.2 (B) Semantic-query result cache — the hot path

```
key:   q:{tenant_id}:{sc_hash}:{query_hash}
       query_hash = blake3(canonical_json(Query))     (= the QueryId basis)
value: QueryResult { rows, annotation, computed_at, result_version }
store: in-process moka with TTL (MVP) → Redis (multi-instance / shared) later.
TTL:   default 60s, per-query override carried in the DEP:
         realtime KPIs    15s
         operational      60s   (default)
         daily rollups    900s
         static dims      3600s
```

This is where the money is saved. A warm dashboard serves entirely from B and
never touches Cube. **INVARIANT:** `tenant_id` and `sc_hash` are mandatory key
segments (security invariants #1, #4). A query with no security context still
gets a stable sentinel `sc_hash = blake3("{}")[..16]`, but the tenant segment is
never optional.

Invalidation: TTL is primary. Add (a) explicit purge by tenant prefix
`q:{tenant}:*` on warehouse-refresh webhooks, and (b) a `result_version` bump
channel for a specific cube. With Redis, fold a **per-tenant epoch** into the key
(`q:{tenant}:{epoch}:…`) so a single epoch increment invalidates a whole tenant
cheaply (§15).

### 10.3 (C) HTML / ETag cache — optional

```
key:   html:{dashboard_id}:{content_hash}:{tenant_id}:{sc_hash}
value: { etag, html_gz, rendered_at }
etag:  blake3(content_hash ‖ sc_hash ‖ max(result_version over queries))
TTL:   min(query TTLs of that DEP), default 60s. Off by default for MVP.
```

Invalidation is derived: the ETag changes whenever `content_hash` or any
underlying `result_version` changes, so staleness is bounded by layer B's TTL.
Responses carry `Cache-Control: private, max-age=<ttl>` + `ETag` — **`private`
because the content is tenant/user-specific (RLS); never `public`.** An
`If-None-Match` hit returns 304 (no body) and still meters a light render.

### 10.4 The moka → Redis path

Caches A/B/C are all in-process `moka` (async, TTL + weight bounds) for the MVP.
They sit **behind traits**, so moving B (and optionally C) to Redis for
multi-instance deployments is a config change, not a rewrite. A is naturally
per-node (it's just deserialized `Arc<Dep>`s) and stays in-process even at scale.

---

## 11. Pricing model

Lumen is **usage-priced on compute credits, never seat-priced.** You are billed
per emitted lumen (per render), and the price tracks how much work the render
actually did. The deliberate incentive: a warm dashboard is cheap; a cold, heavy
analytical query costs more. (`crates/shared/src/meter.rs` is the source of
truth for the constants below.)

### 11.1 The formula

```
credits(request) =
    Σ over widgets       widget_base[w.kind]            // layout cost
  + QUERY_BASE × N                                       // +1 per executed (non-cached) query
  + Σ over executed queries  ceil(exec_ms / 100)         // +1 per 100 ms of compute
  + ceil(bytes_transferred / BYTES_PER_CREDIT)           // egress component

constants (as built):
  QUERY_BASE        = 1
  BYTES_PER_CREDIT  = 1_048_576        // 1 credit per MiB out
  RENDER_FLOOR      = 1                // flat floor for a 304 / fully-warm render

widget_base:  kpi = 1   table = 2   line_chart = 2   bar_chart = 2
              (designed extensions: pivot = 3, map = 4, heavy/custom = 5)
```

Equivalently, the per-query contribution is the helper
`query_credits(exec_ms) = QUERY_BASE + ceil(exec_ms / 100)` (i.e. `1 + ceil(ms/100)`),
summed over executed queries.

**A cache hit costs only its `widget_base`** — no `QUERY_BASE`, no compute term.
That is the pricing incentive made mechanical: warm dashboards bill the layout
floor, cold/expensive queries bill their true compute. A 304 (ETag hit) bills a
flat `RENDER_FLOOR = 1`.

### 11.2 Worked example (the research example, cold vs warm)

A dashboard with **3 KPIs + 1 line chart**, **2 queries executed** taking
**180 ms** and **240 ms**, **400 KB** transferred, **all cold**:

```
  widgets:   3 × widget_base[kpi]=1  +  1 × widget_base[line_chart]=2   = 5
  QUERY_BASE × N:                       1 × 2                            = 2
  compute:   ceil(180/100)=2  +  ceil(240/100)=3                        = 5
  egress:    ceil(400 KB / 1 MiB) = ceil(409600 / 1048576) = 1          = 1
  ─────────────────────────────────────────────────────────────────────────
  TOTAL (cold)                                                          = 13 credits
```

**Fully warm** (both queries served from cache B): only the widget bases →
**5 credits**. A **304** (ETag hit): **1 credit** (`RENDER_FLOOR`). Same
dashboard, 13 → 5 → 1 depending on coldness. That spread is the product's
economic argument: caching is not just a latency win, it is the buyer's bill
going down.

### 11.3 Metered events

Four `MeterKind`s; `bytes_transferred` is a **field** on the render event, not a
separate kind:

| `MeterKind` | When | Key fields |
|---|---|---|
| `dashboard_render` | once per 200/304 | `dashboard_id`, `bytes` (= `bytes_transferred`), `credits` |
| `semantic_compute` | per query executed (cache miss) | `query_hash`, `exec_time_ms`, `bytes` (Cube payload) |
| `cache_hit` | per query served from B | `query_hash` |
| `cache_miss` | per query that hit Cube | `query_hash` |

```rust
pub struct MeterEvent {
    pub event_id: String,          // uuid, idempotency key
    pub occurred_at: DateTime<Utc>,
    pub tenant_id: String,
    pub user_sub: String,
    pub dashboard_id: String,
    pub request_id: String,        // correlates all events of one request
    pub kind: MeterKind,           // dashboard_render|semantic_compute|cache_hit|cache_miss
    pub query_hash: Option<String>,
    pub exec_time_ms: Option<i64>,
    pub bytes: Option<i64>,
    pub credits: i64,              // contribution of THIS event
    pub sc_hash: String,          // audit: which security context produced it
}
```

### 11.4 Emission path

`Meter::emit()` pushes into a bounded `tokio::mpsc` channel and **never blocks or
fails the request** — backpressure is drop-newest with a dropped-counter metric,
not a stall. A background task batches (size 500 / 1 s flush) into Postgres via
`COPY`/multi-row insert. Credits are computed once at request end from the
in-request event vector; the `dashboard_render` row plus the per-query rows are
written in one batch. `event_id` is the PK → the batched writer is safely
retryable with `ON CONFLICT DO NOTHING`.

### 11.5 Postgres schema

```sql
CREATE TABLE meter_events (
    event_id      UUID        PRIMARY KEY,           -- idempotent ingest
    occurred_at   TIMESTAMPTZ NOT NULL,
    tenant_id     TEXT        NOT NULL,
    user_sub      TEXT        NOT NULL,
    dashboard_id  TEXT        NOT NULL,
    request_id    UUID        NOT NULL,
    kind          TEXT        NOT NULL,              -- enum mirror
    query_hash    TEXT,
    exec_time_ms  BIGINT,
    bytes         BIGINT,
    credits       BIGINT      NOT NULL DEFAULT 0,
    sc_hash       TEXT        NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
) PARTITION BY RANGE (occurred_at);                  -- monthly partitions

CREATE INDEX ON meter_events (tenant_id, occurred_at);
CREATE INDEX ON meter_events (request_id);
CREATE INDEX ON meter_events (tenant_id, kind, occurred_at);

-- Billing rollup (refresh hourly; or a continuous aggregate under Timescale).
CREATE MATERIALIZED VIEW meter_tenant_daily AS
SELECT tenant_id,
       date_trunc('day', occurred_at) AS day,
       count(*) FILTER (WHERE kind='dashboard_render') AS renders,
       count(*) FILTER (WHERE kind='semantic_compute') AS computes,
       count(*) FILTER (WHERE kind='cache_hit')        AS cache_hits,
       count(*) FILTER (WHERE kind='cache_miss')       AS cache_misses,
       sum(exec_time_ms)                               AS total_exec_ms,
       sum(bytes)                                      AS total_bytes,
       sum(credits)                                    AS total_credits
FROM meter_events
GROUP BY 1, 2;
```

Partition by month; drop old partitions per retention policy. Billing reads
`meter_tenant_daily.total_credits` per tenant per period. The
`cache_hits / (cache_hits + cache_misses)` ratio is both an ops health signal and
the lever the customer pulls to lower their own bill.

---

## 12. Local development

Everything is driven by `mise` (toolchain pins + tasks) and Docker Compose
(Postgres + Cube). Pins live in `mise.toml`: `rust = "1.95"`, `node = "25"`.

### 12.1 Environment

`mise.toml` sets local dev defaults (override secrets in `.env`):

```
DATABASE_URL     = postgres://lumen:lumen@localhost:5544/lumen
CUBE_URL         = http://localhost:4000
LUMEN_JWT_SECRET = dev-only-insecure-secret-change-me
RUST_LOG         = lumen=debug,tower_http=debug,info
```

### 12.2 Infra (Docker Compose)

`infra/docker-compose.yml` brings up two services:

- **Postgres** on host port **5544** (db/user/pass `lumen`), the TPC-H warehouse.
- **Cube Core** pinned to **`cubejs/cube:v1.6.64`** on port **4000** (REST +
  Playground) and **15432** (SQL API), in dev mode, pointed at the Postgres above,
  with `model/` (the cube YAML) and `cube.js` mounted at `/cube/conf`:

```yaml
services:
  postgres:
    image: postgres:17
    environment: { POSTGRES_USER: lumen, POSTGRES_PASSWORD: lumen, POSTGRES_DB: lumen }
    ports: [ "5544:5432" ]
  cube:
    image: cubejs/cube:v1.6.64
    depends_on: [ postgres ]
    ports: [ "4000:4000", "15432:15432" ]
    environment:
      CUBEJS_DEV_MODE: "true"
      CUBEJS_API_SECRET: "dev-cube-secret"
      CUBEJS_DB_TYPE: postgres
      CUBEJS_DB_HOST: postgres
      CUBEJS_DB_PORT: "5432"
      CUBEJS_DB_NAME: lumen
      CUBEJS_DB_USER: lumen
      CUBEJS_DB_PASS: lumen
    volumes: [ "./cube:/cube/conf" ]   # cube.js + model/cubes/*.yml
```

### 12.3 The dev loop

`mise` tasks wrap the `lumen` CLI:

| Task | Command | Does |
|---|---|---|
| `mise run up` | `docker compose -f infra/docker-compose.yml up -d` | Postgres + Cube |
| `mise run seed` | `cargo run -p lumen-cli -- seed` | load TPC-H sample data into Postgres |
| `mise run compile` | `cargo run -p lumen-cli -- compile dashboards/sales.json -o .lumen-build/sales.lumen` | dashboard → DEP |
| `mise run serve` | `cargo run -p lumen-cli -- serve` | start the embedding runtime |
| `mise run down` | `docker compose … down` | stop infra |
| `mise run dev` | `up` → `sleep 3` → `seed` → `compile` → `serve` | the whole loop in one command |

The full local story:

```bash
mise install     # pin Rust 1.95 + Node 25
mise run up      # Postgres + Cube
mise run seed    # TPC-H into Postgres
mise run compile # dashboards/sales.json → .lumen-build/sales.lumen
mise run serve   # runtime on :PORT
# open examples/embed.html → iframe → /embed/dashboard/sales with a dev JWT
```

`examples/embed.html` is a one-page host that mints a dev JWT (HS256 over
`LUMEN_JWT_SECRET`) and drops an `<iframe src="…/embed/dashboard/sales?token=…">`.

---

## 13. Cube.dev setup with TPC-H sample data

The MVP uses the **TPC-H** schema as a realistic, well-understood warehouse:
real revenue semantics, real join fan-out hazards, and a tiny footprint at low
scale factors.

### 13.1 The schema (5 of 8 TPC-H tables for the MVP)

We model `region → nation → customer → orders → lineitem` (the path that powers
revenue-by-time, revenue-by-status, revenue-by-region). Real TPC-H column names:

```
region   (r_regionkey PK, r_name, r_comment)
nation   (n_nationkey PK, n_name, n_regionkey → region, n_comment)
customer (c_custkey PK, c_name, c_address, c_nationkey → nation, c_phone,
          c_acctbal, c_mktsegment, c_comment)
orders   (o_orderkey PK, o_custkey → customer, o_orderstatus, o_totalprice,
          o_orderdate, o_orderpriority, o_clerk, o_shippriority, o_comment)
lineitem (l_orderkey → orders, l_partkey, l_suppkey, l_linenumber, l_quantity,
          l_extendedprice, l_discount, l_tax, l_returnflag, l_linestatus,
          l_shipdate, l_commitdate, l_receiptdate, l_shipinstruct, l_shipmode,
          l_comment, PK (l_orderkey, l_linenumber))
```

Load-bearing categorical values (used by dashboard filters/dimensions):
`o_orderstatus ∈ {O,F,P}`; `c_mktsegment ∈ {BUILDING, AUTOMOBILE, MACHINERY,
HOUSEHOLD, FURNITURE}`; `l_shipmode ∈ {TRUCK, MAIL, SHIP, RAIL, AIR, REG AIR,
FOB}`; dates span **1992-01-01 … 1998-12-31** (so time dimensions look real).
At **SF = 0.01** you get ~15k orders / ~60k lineitems / 1.5k customers — "real
numbers, tiny footprint."

### 13.2 The canonical revenue definition

The standard TPC-H revenue (used in Q1, Q3, Q5, Q6, Q10) is **line-level**:

```
revenue = SUM( l_extendedprice * (1 - l_discount) )
```

Variants: **charge** (with tax) `SUM(l_extendedprice*(1-l_discount)*(1+l_tax))`;
**order-grain total** `SUM(o_totalprice)`. **Rule:** revenue/charge/quantity live
on `lineitem`; `o_totalprice` lives on `orders`. **Never sum `o_totalprice` after
joining to lineitem** — the fan-out duplicates it. Keep `total_price` an
orders-only measure.

### 13.3 The Cube model (`model/cubes/*.yml`)

Joins are declared on the **many** side (`many_to_one`) and Cube traverses them
transitively (`lineitem → orders → customers → nation → region`).

```yaml
# model/cubes/lineitem.yml
cubes:
  - name: lineitem
    sql_table: public.lineitem
    joins:
      - name: orders
        sql: "{CUBE}.l_orderkey = {orders.o_orderkey}"
        relationship: many_to_one
    measures:
      - name: count
        type: count
      - name: revenue                       # canonical TPC-H revenue
        sql: "{CUBE}.l_extendedprice * (1 - {CUBE}.l_discount)"
        type: sum
        format: currency
      - name: charge                        # revenue including tax
        sql: "{CUBE}.l_extendedprice * (1 - {CUBE}.l_discount) * (1 + {CUBE}.l_tax)"
        type: sum
        format: currency
      - name: total_quantity
        sql: l_quantity
        type: sum
      - name: avg_discount
        sql: l_discount
        type: avg
    dimensions:
      - name: id
        sql: "{CUBE}.l_orderkey || '-' || {CUBE}.l_linenumber"
        type: string
        primary_key: true
      - name: return_flag
        sql: l_returnflag
        type: string
      - name: ship_mode
        sql: l_shipmode
        type: string
      - name: ship_date
        sql: l_shipdate
        type: time
```

```yaml
# model/cubes/orders.yml
cubes:
  - name: orders
    sql_table: public.orders
    joins:
      - name: customers
        sql: "{CUBE}.o_custkey = {customers.c_custkey}"
        relationship: many_to_one
    measures:
      - name: count
        type: count
      - name: total_price                   # order-grain; never mix with lineitem revenue
        sql: o_totalprice
        type: sum
        format: currency
      - name: avg_order_value
        sql: o_totalprice
        type: avg
        format: currency
    dimensions:
      - name: o_orderkey
        sql: o_orderkey
        type: number
        primary_key: true
      - name: status                        # 'O' open / 'F' finished / 'P' partial
        sql: o_orderstatus
        type: string
      - name: priority
        sql: o_orderpriority
        type: string
      - name: order_date
        sql: o_orderdate
        type: time
```

```yaml
# model/cubes/customers.yml
cubes:
  - name: customers
    sql_table: public.customer
    joins:
      - name: nation
        sql: "{CUBE}.c_nationkey = {nation.n_nationkey}"
        relationship: many_to_one
    measures:
      - name: count
        type: count
      - name: avg_acctbal
        sql: c_acctbal
        type: avg
        format: currency
    dimensions:
      - name: c_custkey
        sql: c_custkey
        type: number
        primary_key: true
      - name: market_segment
        sql: c_mktsegment
        type: string

# model/cubes/nation.yml
cubes:
  - name: nation
    sql_table: public.nation
    joins:
      - name: region
        sql: "{CUBE}.n_regionkey = {region.r_regionkey}"
        relationship: many_to_one
    dimensions:
      - name: n_nationkey
        sql: n_nationkey
        type: number
        primary_key: true
      - name: name
        sql: n_name
        type: string

# model/cubes/region.yml
cubes:
  - name: region
    sql_table: public.region
    dimensions:
      - name: r_regionkey
        sql: r_regionkey
        type: number
        primary_key: true
      - name: name
        sql: r_name
        type: string
```

How the dashboard's queries resolve against this model:

- **Revenue over time** → `measures:[lineitem.revenue]`, `timeDimensions:[{orders.order_date, month}]` (lineitem→orders join).
- **Revenue by status** → `measures:[lineitem.revenue]`, `dimensions:[orders.status]`.
- **Revenue by region** → `measures:[lineitem.revenue]`, `dimensions:[region.name]` (full transitive traverse).
- **Orders / AOV** → `measures:[orders.count, orders.avg_order_value]` — orders-only, no lineitem measure, so no fan-out.

An optional pre-aggregation rolls up the main query (`measures:[lineitem.revenue,
lineitem.count]`, `dimensions:[orders.status]`, `time_dimension: orders.order_date`,
`granularity: month`) so the hot dashboard query is a memory hit in Cube.

### 13.4 Seeding the sample data

Three options, in order of fidelity vs. friction; `lumen seed` implements one
behind the CLI:

1. **DuckDB's bundled `tpch` extension → CSV → Postgres `\copy`** (cleanest, exact
   spec data, no dbgen build):
   ```sql
   -- duckdb
   INSTALL tpch; LOAD tpch; CALL dbgen(sf = 0.01);
   COPY region TO 'region.csv' (HEADER); -- … one COPY per table …
   ```
   then `\copy <table> FROM '<table>.csv' WITH (FORMAT csv, HEADER true)` in FK
   order (region, nation, customer, orders, lineitem).
2. **Pure-SQL synthesis in Postgres** (zero external deps; what `lumen seed` ships
   for the MVP): insert the 5 real regions + 25 real nations verbatim, then
   `generate_series` the fact tables — 1,500 customers, 5,000 orders over
   1992–1998, and 1–7 lineitems per order (~20k rows) with
   `l_extendedprice`/`l_discount`/`l_tax` in spec-plausible ranges. Real
   distributions, ~20k rows, no downloads.
3. **`tpchgen-rs`** (fastest pure-Rust generator; emits CSV/Parquet/`.tbl`
   directly) for higher scale factors without a dbgen toolchain.

> Sources for §13: TPC-H spec rev 2.17.1; `dimitri/tpch-citus` DDL; DuckDB TPC-H
> extension; `tvondra/pg_tpch`; `tpchgen-rs`.

---

## 14. MVP implementation plan

The MVP proves the whole thesis end to end on a laptop: a real semantic layer, a
real compile step, a real runtime, real metering, one embeddable dashboard.

### 14.1 In scope

| Area | MVP scope |
|---|---|
| Warehouse | Local **Postgres** with **TPC-H** seed data (`lumen seed`, pure-SQL synthesis). |
| Semantic layer | Local **Cube Core v1.6.64** with the §13 model; the `lumen-semantic` Cube adapter (translate IR, mint Cube JWT, `Continue wait` poll). |
| Compiler | `lumen-compiler`: validate → lower → **dedup by `QueryId`** → renderer policy (kpi→html, line→svg, bar→echarts) → precompile specs → credit estimate → pack. |
| Artifact | **`lumen-artifact`** (done): pack/read `.lumen`, content hash, per-section integrity. |
| Runtime | `lumen-runtime`: `GET /embed/dashboard/{id}`, **caches A + B** (moka), the request flow of §6, async metering to Postgres. |
| Renderer | `lumen-renderer`: Askama shell, **server SVG for KPI/line**, **ECharts data-island stub for bar**, theme inlining. |
| Auth | `lumen-auth`: **HS256** embed JWT verify, `Principal`, the one `cache_key`, invariants #1–#5. |
| Embedding | **iframe** + `examples/embed.html` host with a dev JWT. |
| Pricing | The full credit formula + `meter_events` table + `meter_tenant_daily` rollup. |
| Infra | **Docker Compose** (Postgres + Cube) + `mise` tasks; `lumen compile|serve|seed`. |
| Dashboard | One dashboard: `dashboards/sales.json` (2 KPIs + line + bar). |

### 14.2 Explicitly out of scope for the MVP

Cache C (HTML/ETag) on by default; Redis; RS256/JWKS; Web Component / JS SDK
embedding; multi-dashboard authoring UI; MessagePack section codec; WASM/edge
runtime; live/streaming data; pre-aggregation management. Each has a designed
home (§7, §10, §15) so adding it is additive, not a rewrite.

### 14.3 Suggested build order (tracer-bullet first)

1. **Seed + Cube model** — prove `POST /cubejs-api/v1/load` returns TPC-H revenue.
2. **Compiler** — `sales.json` → `sales.lumen`; assert 4 widgets → 4 queries,
   `renderers = [html, svg, echarts]`.
3. **Cube adapter** — execute the DEP's queries with a forwarded security context;
   handle `Continue wait`.
4. **Renderer** — bind results: server SVG KPI/line + ECharts bar island.
5. **Auth + runtime** — wire the §6 flow with caches A/B; serve the iframe.
6. **Metering** — emit events, compute credits, write `meter_events`; verify the
   13/5/1 worked example numbers fall out.

The contract crates (`lumen-shared`, `lumen-artifact`) are already implemented and
tested, so every step above codes against fixed types.

---

## 15. Future evolution toward an edge-native analytics runtime

The architecture's split (§2.2) was chosen so the request-time half can be lifted
to the edge with no compiler in it. The destination: **compile the execution
plan, then run it at a CDN POP.** The DEP is already the portable unit.

### 15.1 Compile the DEP to run at the edge (WASM)

The runtime's request path is pure binding logic: load a DEP, execute queries
through a `SemanticLayer`, bind into precompiled specs, emit HTML. None of it
needs a compiler, a layout engine, or a chart library. That makes it a candidate
for a **WASM module at the edge**:

- Ship the request-time binder (auth verify + DEP read + bind + assemble) as a
  WASM component deployed to every POP.
- The DEP, being a single content-addressed packed file, is the ideal WASM input:
  fetch once, **mmap** the bytes (the artifact format is already mmap-friendly;
  §5.2), read the TOC, bind. No deserialization of the layout engine because there
  is no layout engine.
- The `SemanticLayer` call becomes a `fetch` to a regional Cube/warehouse from the
  POP, or a hit on an edge-local query cache (next point).

### 15.2 Edge KV for DEPs and query results

- **DEP store at the edge.** Key by full `content_hash`; immutable, so
  `Cache-Control: public, max-age=31536000, immutable` and global edge-KV
  replication are trivially correct (cache A becomes a KV namespace).
- **Query cache at the edge.** Cache B moves to edge KV/Redis with the **same key
  shape** `q:{tenant}:{sc_hash}:{query_hash}` — the keys were always
  tenant-isolated, so they're already safe to share across a POP. A warm
  dashboard never leaves the edge.

### 15.3 Per-tenant epoch cache invalidation

Replace TTL-only invalidation with a **per-tenant epoch counter** folded into the
key: `q:{tenant}:{epoch}:{sc_hash}:{query_hash}`. A warehouse refresh webhook
increments `epoch[tenant]` once; every key for that tenant is instantly stale at
O(1) cost, no scan, no prefix delete. Cheap global invalidation is what makes a
shared edge query cache viable.

### 15.4 Partial re-render and prefetching

- **Partial re-render.** Because each widget binds an independent query result,
  the runtime can re-render **only the widgets whose `result_version` changed**,
  not the whole page — a server-driven analogue of a virtual DOM diff, keyed on
  the DEP's per-widget query ids.
- **Prefetching.** The DEP's manifest already lists every distinct query. The edge
  can **warm cache B for a tenant before the first open** (e.g. on session start),
  turning the first render warm. A future `prefetch` section in the DEP (forward-
  compatible: unknown sections are ignored, §5.5) could carry a warming hint.

### 15.5 CDN / ETag discipline already in place

The ETag derivation (`content_hash ‖ sc_hash ‖ result_version`) and the
`Cache-Control: private` discipline (§9.4 invariant #5) are exactly what an edge
needs: immutable-public for the DEP object, private + content-derived ETag for
the rendered, RLS-scoped HTML. Nothing about going to the edge changes the
caching contract; it only changes where the caches live.

### 15.6 Streaming / live data via `setOption` diffing

The ECharts data-island design (§8.3) already separates the frozen spec from the
fresh data. Live data is then a push of new rows into the existing chart:
`chart.setOption({ dataset: { source: newRows } })` — ECharts diffs and animates,
no spec rebuild. A WebSocket/SSE channel from the edge can stream
`result_version` bumps and row deltas, and the client binds them late into the
already-compiled option. The "compile once, bind later" contract extends
cleanly from "bind once per request" to "bind continuously."

### 15.7 Richer embedding surfaces

The iframe MVP is the floor, not the ceiling. The same runtime endpoint backs:

- a **Web Component** `<analytics-dashboard dashboard="sales" token="…">` that
  fetches the embed HTML and attaches it in a shadow root;
- a **JS SDK** `Analytics.render(el, { dashboard, token })` for programmatic
  mounting, theming, and event hooks.

Both consume the identical `/embed/dashboard/{id}` response; the DEP, the auth
model, the caching, and the metering are unchanged. The embedding surface is a
thin client over a stable runtime — which is the whole point of having compiled
the dashboard in the first place.

---

*End of design document. The contract that makes all of this composable —
`DashboardDef`, the neutral `Query` IR, the `ChartSpec` renderer tag, the
content-addressed ids, the `.lumen` container, and the credit constants — lives
in `crates/shared` and `crates/artifact`, implemented and tested. Everything
else binds to it.*

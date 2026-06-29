# Lumen

**An embedding runtime for analytics — not another BI tool.**

Lumen sits on top of a semantic layer (initially [Cube.dev](https://cube.dev)) and turns
dashboard definitions into **compiled, embeddable artifacts**. It does not own metrics or
modeling — your semantic layer does. Lumen owns *compilation, rendering, embedding, and
metering*.

> Think **Vercel for dashboards** — a build pipeline that compiles dashboards and an edge
> runtime that serves them — rather than Looker or Tableau.

A *lumen* is the SI unit of luminous flux: the measure of rendered light output. The product
is billed per **render** — usage-priced, not seat-priced — so the unit of value is literally
a lumen emitted.

## The core idea: compile, don't interpret

Traditional BI re-parses a dashboard JSON, re-runs the layout engine, and rebuilds the
visualization pipeline on **every open**. Lumen does the expensive work once, when a
dashboard *changes*:

```
Dashboard Definition (JSON)
        │
        ▼
   ┌──────────┐
   │ Compiler │   ← runs when the dashboard changes
   └──────────┘
        │
        ▼
  Dashboard Execution Plan  (.lumen bundle: manifest + layout + queries + chart specs)
        │
        ▼
   ┌──────────┐
   │ Runtime  │   ← runs per request: auth → load DEP → execute queries → bind → serve
   └──────────┘
        │
   inject fresh data
        │
        ▼
      HTML
```

The browser only ever receives a **compiled layout** + **fresh metric JSON** + a tiny JS
runtime. No layout compilation happens during a request.

### The Dashboard Execution Plan (DEP)

The compiler's output is not HTML — it's an execution plan, the way a query planner emits a
plan or a bundler emits a bundle. A `.lumen` bundle contains:

```
sales.lumen
├── manifest.json      content hash, version, widget + query ids, credit estimate
├── layout.json        flat list of positioned widgets → (query id, chart-spec id)
├── queries.json       deduplicated, content-addressed semantic-layer queries
├── charts/*.chart     precompiled, renderer-agnostic chart specs
├── theme.css          compiled theme
└── runtime.js         tiny client runtime (binds fresh JSON → compiled specs)
```

The runtime's only job: **authenticate → load DEP → execute queries → bind results → return HTML.**

## Workspace

| crate            | responsibility |
|------------------|----------------|
| `lumen-shared`   | core domain types: dashboard def, DEP, `Query`, `Data`, ids, errors |
| `lumen-artifact` | read/write the `.lumen` DEP bundle; content hashing |
| `lumen-semantic` | `SemanticLayer` trait + Cube.dev adapter |
| `lumen-compiler` | dashboard JSON → DEP (query dedup, chart-spec compilation) |
| `lumen-renderer` | DEP + data → HTML (Askama templates, server SVG, client chart specs) |
| `lumen-auth`     | JWT verification, tenant + security-context model |
| `lumen-runtime`  | Axum embedding server: `/embed/dashboard/:id`, caching, metering |
| `lumen-cli`      | `lumen compile | serve | seed` |

This is a **polyglot monorepo** — the Rust workspace above plus a pnpm JS workspace:

```
lumen/
├─ crates/                 # Rust: runtime, compiler, semantic, renderer, ...
│  └─ runtime/assets/runtime.js   # the ~1KB client hydration runtime (server-embedded)
├─ apps/
│  └─ web/                 # TanStack Start control-plane app (@lumen/web)
├─ packages/
│  ├─ embed-sdk/           # @lumen/embed-sdk — customer iframe loader
│  └─ contracts/           # @lumen/contracts — TS types mirroring lumen-shared
├─ infra/                  # Docker Compose: Postgres 18 + Cube.dev (TPC-H data)
├─ dashboards/  docs/  examples/
├─ Cargo.toml              # Rust workspace
└─ pnpm-workspace.yaml     # JS workspace
```

The **embed runtime output** (what end-users see) stays server-SVG + the tiny vanilla
runtime — no React. The **control-plane app** (`apps/web`, where you author dashboards,
manage embed tokens, and watch usage) is React/TanStack Start. Different audiences,
different stacks; they share types via `@lumen/contracts`.

```bash
mise run install      # pnpm install (JS workspace)
mise run up           # (if not already) Postgres — the control plane needs it for auth
mise run web          # TanStack Start app on http://localhost:3000
```

The web app is a real SaaS shell:

| Route | Access | What |
|-------|--------|------|
| `/` | public | **landing page** (marketing) |
| `/login`, `/signup` | public | **Better Auth** email/password (sessions in Postgres) |
| `/dashboards` (+ `/$id`, `/embedding`, `/usage`) | **protected** | the control plane (server-side session guard → redirects to `/login`) |
| `/api/auth/*` | — | Better Auth handler |

Auth tables (`user`/`session`/`account`/`verification`) are created by
`npx @better-auth/cli migrate` against `DATABASE_URL`. Set `BETTER_AUTH_SECRET`
in `apps/web/.env`.

## Quickstart

```bash
mise install          # pin Rust + Node
mise run up           # Postgres + Cube.dev via Docker Compose
mise run seed         # load TPC-H sample data
mise run compile      # compile dashboards/sales.json → .lumen-build/sales.lumen
mise run serve        # start the embedding runtime on http://localhost:8088
# open http://localhost:8088/             → built-in demo (iframe + minted dev JWT)
# or  examples/embed.html                 → external iframe-embedding example
# or  examples/embed-webcomponent.html    → non-iframe <analytics-dashboard> (Shadow DOM)
```

### Embedding modes

| Mode | How | Best for |
|------|-----|----------|
| **iframe** | `<iframe src="/embed/dashboard/:id?token=…">` | untrusted/third-party hosts needing a hard same-origin wall |
| **Web Component** | `<analytics-dashboard base dashboard token>` → fetches the `?format=fragment` HTML into a **Shadow DOM** | trusted host apps wanting native sizing/theming, no iframe |

Both use the **same scoped token** minted server-side at `POST /tokens` (carrying the
`security_context` forwarded to Cube for row-level security). The token — not the iframe —
is what isolates data, so dropping the iframe doesn't weaken data security.

See [`docs/DESIGN.md`](docs/DESIGN.md) for the full architecture.

## Status

MVP scaffold. See `docs/DESIGN.md` → "MVP implementation plan" and "Future evolution toward
an edge-native runtime."

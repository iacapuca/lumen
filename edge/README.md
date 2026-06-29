# lumen-edge — Cloudflare Workers runtime (skeleton)

The **edge-native** hydrate/compile runtime. It reuses Lumen's I/O-free compute
core verbatim and swaps only the I/O adapters for Cloudflare bindings. This is the
"edge rendering" end state from the design: compiled `.lumen` bundles live in R2,
a Worker at the edge loads one, calls the semantic layer, binds fresh data, and
returns HTML — globally distributed, scale-to-zero, billed per invocation.

## Why this is a small change, not a rewrite

The pure-compute crates (`lumen-shared`, `lumen-artifact`, `lumen-compiler`,
`lumen-renderer`) are **verified to compile to `wasm32-unknown-unknown` unchanged**.
Only the adapters differ — and they sit behind seams the native runtime already
uses:

| Concern | Native runtime | Edge (this crate) |
|---|---|---|
| DEP store | filesystem | **R2** (`LUMEN_DEP`) |
| query cache | `moka` | **Workers KV** (`LUMEN_CACHE`) |
| Cube client | `reqwest` | **Fetch** binding |
| metering | `sqlx` → Postgres | **Durable Object** (`TenantMeter`) |
| JWT HS256 | `jsonwebtoken` (`aws_lc_rs`) | pure-Rust `hmac`+`sha2` (aws-lc doesn't build for wasm) |
| HTTP | Axum | `worker::Router` |
| `chrono` clock | system | `js-sys` (auto on wasm) / `worker::Date` |

`render_dashboard` / `render_fragment` / `compile` / `Dep` are called **identically**
to the native runtime.

## Build & deploy

This is a wasm Worker — build it with wrangler (which drives `worker-build`), not
bare `cargo build`. It is its own Cargo workspace, excluded from the native build.

```bash
cd edge
npm i -g wrangler                              # or npx wrangler ...
wrangler kv namespace create LUMEN_CACHE       # paste the id into wrangler.jsonc
wrangler r2 bucket create lumen-deps
wrangler secret put LUMEN_JWT_SECRET
wrangler secret put CUBE_API_SECRET
wrangler dev                                   # local; or: wrangler deploy
```

Upload a compiled DEP to R2 so the Worker can serve it:

```bash
# from the repo root, after `mise run compile`
wrangler r2 object put lumen-deps/sales.lumen --file ../.lumen-build/sales.lumen
```

## Status / TODO

- **Skeleton** pinned to `worker` 0.5 — some binding calls may need version tweaks.
- The Cube `Query → REST` translation is duplicated here; factor it (and the
  response parser) into a wasm-safe `lumen-semantic-core` shared with the native
  `CubeClient`.
- Compile-on-edge (`POST /compile` → write `.lumen` to R2) is a natural next route
  reusing `lumen_compiler::compile`.
- Cloudflare meters requests + CPU-ms natively; the `TenantMeter` DO adds the
  per-tenant **credit** ledger on top for product-level usage pricing.

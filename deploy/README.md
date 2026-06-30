# Deploying Lumen to Cloudflare

```bash
cp deploy/.env.deploy.example deploy/.env.deploy   # fill in Neon + Cube + domains
wrangler login                                     # one-time
bash deploy/deploy-cloudflare.sh                   # generates keys + deploys everything
```

The script deploys two Workers and wires them together:

| Worker | What | Bindings |
|--------|------|----------|
| **lumen-edge** (`edge/`) | embed/hydrate runtime | R2 `lumen-deps`, KV `LUMEN_CACHE`, DO `TENANT_METER` |
| **@lumen/web** (`apps/web`) | control plane (SSR) | Hyperdrive → Postgres |

External services you supply: **Postgres** (Neon/Supabase) and **Cube.dev** (Cube Cloud or self-hosted). Cloudflare doesn't host these.

### Key facts the script handles
- **Generates `LUMEN_JWT_SECRET` once and shares it** across both Workers — the control plane mints embed tokens, the edge verifies them. They MUST match.
- **Postgres on Workers goes through Hyperdrive** (`pg`'s TCP sockets aren't allowed directly); the script creates the Hyperdrive config from your `NEON_DATABASE_URL`. The Better Auth migration runs directly against Postgres (not via Hyperdrive).
- Secrets go in via `wrangler secret put` (never in `wrangler.jsonc`). `deploy/.env.deploy` is gitignored.

### Manual paste-backs (Cloudflare returns ids you wire into config)
- KV id → `edge/wrangler.jsonc` → `kv_namespaces[].id`
- Hyperdrive id → `apps/web/wrangler.jsonc` → `hyperdrive` binding
- After the edge deploys, set `EDGE_WORKER_URL` in `.env.deploy` and re-run step 4 so the control plane points at it.

### Prerequisites still being finalized
- `apps/web` Cloudflare adapter (CF build target + `nodejs_compat` + lazy Better-Auth init for per-request bindings).
- `edge/` first `wrangler dev` shakeout (the `worker` 0.5 bindings).

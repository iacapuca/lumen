#!/usr/bin/env bash
#
# Lumen → Cloudflare, end to end. Generates the shared secrets and runs every
# wrangler command for the edge runtime + the control plane.
#
#   1. cp deploy/.env.deploy.example deploy/.env.deploy   # then fill it in
#   2. wrangler login                                     # one-time, interactive
#   3. bash deploy/deploy-cloudflare.sh
#
# Prereqs you provide: a Cloudflare account, a managed Postgres (Neon/Supabase),
# and a reachable Cube.dev instance. Re-runnable (creates are idempotent).

set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
ENVF="deploy/.env.deploy"
[ -f "$ENVF" ] || { echo "✗ Create $ENVF from deploy/.env.deploy.example first."; exit 1; }
set -a; source "$ENVF"; set +a
command -v wrangler >/dev/null || { echo "✗ Need wrangler:  npm i -g wrangler && wrangler login"; exit 1; }

# ── 1. shared secrets (generated once, persisted back into .env.deploy) ───────
gen() { openssl rand -base64 48 | tr -d '\n'; }
if [ -z "${LUMEN_JWT_SECRET:-}" ]; then
  LUMEN_JWT_SECRET="$(gen)"; printf '\nLUMEN_JWT_SECRET="%s"\n' "$LUMEN_JWT_SECRET" >> "$ENVF"
fi
if [ -z "${BETTER_AUTH_SECRET:-}" ]; then
  BETTER_AUTH_SECRET="$(gen)"; printf 'BETTER_AUTH_SECRET="%s"\n' "$BETTER_AUTH_SECRET" >> "$ENVF"
fi
echo "→ Secrets ready (saved in $ENVF). LUMEN_JWT_SECRET is shared by edge + control plane."

# ── 2. edge runtime: KV + R2 + secrets + deploy ──────────────────────────────
echo "════ Edge runtime (edge/) ════"
# worker-build compiles Rust→wasm with the cargo on PATH — make sure it has the
# wasm target (Arch's system rust doesn't; the isolated rustup install does).
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
pushd edge >/dev/null
  wrangler r2 bucket create lumen-deps || true
  if grep -q '<run:' wrangler.jsonc; then
    KV_OUT="$(wrangler kv namespace create LUMEN_CACHE 2>&1 || true)"; echo "$KV_OUT"
    KV_ID="$(printf '%s' "$KV_OUT" | grep -oE '[0-9a-f]{32}' | head -1 || true)"
    [ -n "$KV_ID" ] && echo "   ⓘ KV id $KV_ID — paste it into edge/wrangler.jsonc → kv_namespaces[].id, then re-run."
  else
    echo "   ⓘ KV already configured in edge/wrangler.jsonc — skipping create."
  fi
  printf '%s' "$LUMEN_JWT_SECRET" | wrangler secret put LUMEN_JWT_SECRET
  printf '%s' "$CUBE_API_SECRET"  | wrangler secret put CUBE_API_SECRET
  [ -n "${LUMEN_API_KEY:-}" ] && printf '%s' "$LUMEN_API_KEY" | wrangler secret put LUMEN_API_KEY || true
  wrangler deploy --var CUBE_URL:"$CUBE_URL"
popd >/dev/null

# ── 3. compile a dashboard and publish it to R2 ──────────────────────────────
echo "════ Publish a compiled dashboard to R2 ════"
( command -v mise >/dev/null && mise run compile ) \
  || cargo run -p lumen-cli -- compile dashboards/sales.json -o .lumen-build/sales.lumen
( cd edge && wrangler r2 object put lumen-deps/sales.lumen --file ../.lumen-build/sales.lumen )

# ── 4. control plane: Hyperdrive + auth migration + build + deploy ───────────
echo "════ Control plane (apps/web) ════"
# Postgres over TCP isn't allowed on Workers — route pg through Hyperdrive.
HD_OUT="$(cd apps/web && wrangler hyperdrive create lumen-pg --connection-string="$NEON_DATABASE_URL" 2>&1 || true)"
echo "$HD_OUT"
HD_ID="$(printf '%s' "$HD_OUT" | grep -oE '[0-9a-f]{32}' | head -1 || true)"
[ -n "$HD_ID" ] && echo "   ⓘ Hyperdrive id $HD_ID — ensure it's in apps/web/wrangler.jsonc → hyperdrive binding"

# Better Auth schema on your managed Postgres (run directly, not via Hyperdrive).
( cd apps/web && DATABASE_URL="$NEON_DATABASE_URL" npx @better-auth/cli@latest migrate -y )

pnpm install
pnpm --filter @lumen/web build
pushd apps/web >/dev/null
  printf '%s' "$BETTER_AUTH_SECRET" | wrangler secret put BETTER_AUTH_SECRET
  printf '%s' "$LUMEN_JWT_SECRET"   | wrangler secret put LUMEN_JWT_SECRET
  wrangler deploy \
    --var BETTER_AUTH_URL:"$CONTROL_PLANE_DOMAIN" \
    --var VITE_LUMEN_RUNTIME_URL:"$EDGE_WORKER_URL"
popd >/dev/null

echo
echo "✅ Deployed."
echo "   Control plane → $CONTROL_PLANE_DOMAIN"
echo "   Edge runtime  → $EDGE_WORKER_URL"
echo "   Both share LUMEN_JWT_SECRET; the control plane mints embed tokens, the edge verifies them."

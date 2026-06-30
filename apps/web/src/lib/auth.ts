// SERVER-ONLY. This module imports `pg` (a Node-only package). It must never be
// bundled into the client. Only import it from server routes
// (`src/routes/api/auth/$.ts`) or from the server-side of a `createServerFn`
// handler (`src/lib/auth-session.ts`).
//
// On Cloudflare Workers there is no process-wide `process.env` for secrets and
// bindings — they arrive as a per-request `env` object. So instead of building
// one `auth` instance at module load, we expose a `getAuth(env)` factory:
//   * On Workers, callers pass the Cloudflare `env` (see `cf-env.ts`); the
//     Postgres connection string comes from the Hyperdrive binding
//     (`env.HYPERDRIVE.connectionString`).
//   * In local Node dev, callers pass `undefined` and we fall back to
//     `process.env` (populated by the `--env-file` dev script).
import { betterAuth } from 'better-auth'
import { tanstackStartCookies } from 'better-auth/tanstack-start'
import { Pool } from 'pg'

import type { CloudflareEnv } from './cf-env'

type AuthInstance = ReturnType<typeof betterAuth>

// Memoise the built instance (and its `pg` Pool) per connection string so we
// don't open a new pool on every request inside a warm Worker isolate / Node
// process. The key folds in the values that change the instance's behaviour.
const cache = new Map<string, AuthInstance>()

function buildAuth(connectionString: string | undefined, baseURL: string, secret: string | undefined): AuthInstance {
  return betterAuth({
    // Postgres via the built-in `pg` Pool adapter — Better Auth's Kysely layer
    // talks to it directly (no ORM). On Workers this pool runs over Hyperdrive
    // and requires the `nodejs_compat` compatibility flag. The migration CLI
    // uses the same DATABASE_URL.
    database: new Pool({ connectionString }),
    emailAndPassword: {
      enabled: true,
      // No email provider wired up yet — don't gate sign-in on verification.
      requireEmailVerification: false,
    },
    baseURL,
    secret,
    trustedOrigins: [baseURL, 'http://localhost:3000'],
    // Forwards Set-Cookie headers through TanStack Start's server runtime so
    // sessions set inside server functions land on the response.
    plugins: [tanstackStartCookies()],
  })
}

/**
 * Build (or reuse) a Better Auth instance for the given runtime environment.
 *
 * @param env Cloudflare Workers bindings/secrets for the current request, or
 *   `undefined` in local Node dev (we then read `process.env`).
 */
export function getAuth(env?: CloudflareEnv): AuthInstance {
  const connectionString = env?.HYPERDRIVE?.connectionString ?? process.env.DATABASE_URL
  const baseURL = env?.BETTER_AUTH_URL ?? process.env.BETTER_AUTH_URL ?? 'http://localhost:3000'
  const secret = env?.BETTER_AUTH_SECRET ?? process.env.BETTER_AUTH_SECRET

  const key = `${connectionString ?? ''}|${baseURL}`
  let instance = cache.get(key)
  if (!instance) {
    instance = buildAuth(connectionString, baseURL, secret)
    cache.set(key, instance)
  }
  return instance
}

export type Session = AuthInstance['$Infer']['Session']

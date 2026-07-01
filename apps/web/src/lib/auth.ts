// SERVER-ONLY. This module imports `pg` (a Node-only package). It must never be
// bundled into the client. Only import it from server routes
// (`src/routes/api/auth/$.ts`) or from the server-side of a `createServerFn`
// handler (`src/lib/auth-session.ts`).
//
// Bindings/secrets come from `getBindings()` (see bindings.ts) — real in every
// mode (dev included), since `@cloudflare/vite-plugin` runs the app under
// workerd/Miniflare even in `pnpm dev`.
import { betterAuth } from 'better-auth'
import { organization } from 'better-auth/plugins'
import { tanstackStartCookies } from 'better-auth/tanstack-start'
import { drizzleAdapter } from '@better-auth/drizzle-adapter'

import { getBindings } from './bindings'
import { getDb } from './db/client'
import * as schema from './db/schema'

// NB: `AuthInstance` is derived below from `ReturnType<typeof createAuth>`,
// NOT `ReturnType<typeof betterAuth>` directly — the latter resolves against
// `betterAuth`'s generic, argument-less call signature and silently erases
// every plugin-provided type augmentation (session fields, `.api.*`
// endpoints). `createAuth` has no explicit return-type annotation for the
// same reason: annotating it would re-widen the return type back down to the
// generic shape it's trying to avoid.
function createAuth() {
  const bindings = getBindings()

  // Fail fast rather than let Better Auth silently run with no/blank secret —
  // that would sign sessions with an empty key. `.dev.vars.example`
  // documents generating one with `openssl rand -base64 48`; this only ever
  // fires if that step was skipped, not on the already-configured local dev
  // secret.
  if (!bindings.BETTER_AUTH_SECRET) {
    throw new Error(
      'BETTER_AUTH_SECRET is not set. Generate one with `openssl rand -base64 48` and set it in apps/web/.dev.vars (or `wrangler secret put BETTER_AUTH_SECRET` in production).',
    )
  }
  const baseURL = bindings.BETTER_AUTH_URL || 'http://localhost:3000'

  return betterAuth({
    // Drizzle adapter over the same shared `getDb()` client every
    // Lumen-owned table (dashboards, data_sources) uses — see db/client.ts.
    // `camelCase: true` matches the columns Better Auth's own CLI already
    // created (runtimeApiKeyHash, activeOrganizationId, ...) — confirmed live
    // against Postgres before wiring this in, not assumed.
    database: drizzleAdapter(getDb(), { provider: 'pg', schema, camelCase: true }),
    emailAndPassword: {
      enabled: true,
      // No email provider wired up yet — don't gate sign-in on verification.
      requireEmailVerification: false,
    },
    baseURL,
    secret: bindings.BETTER_AUTH_SECRET,
    trustedOrigins: [baseURL, 'http://localhost:3000'],
    // Cache the session in a short-lived signed cookie so most requests
    // (every page load, every server fn) skip the round-trip to Postgres
    // that `getSession` would otherwise make every time. Trade-off: a
    // revoked session or an org change (e.g. `setActiveOrganization`) can
    // take up to `maxAge` to be observed — acceptable here since Lumen has
    // no forced-signout/admin-revocation feature yet and `activeOrganizationId`
    // is set once at signup and never changed afterward.
    session: {
      cookieCache: { enabled: true, maxAge: 5 * 60 },
    },
    // Forwards Set-Cookie headers through TanStack Start's server runtime so
    // sessions set inside server functions land on the response.
    plugins: [
      tanstackStartCookies(),
      // Every signed-up user gets exactly one organization (their "workspace")
      // — see lib/organizations.ts::ensureOrganization. `organization.id` is
      // the account boundary the Rust runtime enforces: dashboards belong to
      // one org, and POST /tokens (on the runtime) resolves the caller's
      // org via the runtimeApiKeyHash below, never a client-supplied value.
      //
      // `input: false` on both fields blocks them from the public
      // create/update API bodies — they're only ever written by our own
      // server code via a direct SQL UPDATE (lib/organizations.ts), never by
      // a client-supplied field, which is the whole point: nobody can set
      // their own org's key hash from the client. `returned: false` on the
      // hash keeps it out of API responses entirely; the prefix is safe to
      // return (display only, e.g. "lumen_sk_ab12…").
      organization({
        schema: {
          organization: {
            additionalFields: {
              runtimeApiKeyHash: { type: 'string', required: false, input: false, returned: false },
              runtimeApiKeyPrefix: { type: 'string', required: false, input: false, returned: true },
            },
          },
        },
      }),
    ],
  })
}

type AuthInstance = ReturnType<typeof createAuth>

// Lazy singleton. Bindings stay stable for the isolate's lifetime, so
// caching the configured instance (and its `pg` Pool, via getDb()) is
// correct — do NOT call this at module top-level, only from within a
// request/handler, same constraint `getBindings()` itself has.
let _auth: AuthInstance | undefined

export function getAuth(): AuthInstance {
  if (!_auth) _auth = createAuth()
  return _auth
}

export type Session = AuthInstance['$Infer']['Session']

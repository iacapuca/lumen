// SERVER-ONLY. This module imports `pg` (a Node-only package) and reads
// secrets from `process.env`. It must never be bundled into the client.
// Only import it from server routes (`src/routes/api/auth/$.ts`) or from the
// server-side of a `createServerFn` handler (`src/lib/auth-session.ts`).
import { betterAuth } from 'better-auth'
import { tanstackStartCookies } from 'better-auth/tanstack-start'
import { Pool } from 'pg'

const baseURL = process.env.BETTER_AUTH_URL ?? 'http://localhost:3000'

export const auth = betterAuth({
  // Postgres via the built-in `pg` Pool adapter — Better Auth's Kysely layer
  // talks to it directly (no ORM). The migration CLI uses the same DATABASE_URL.
  database: new Pool({ connectionString: process.env.DATABASE_URL }),
  emailAndPassword: {
    enabled: true,
    // No email provider wired up yet — don't gate sign-in on verification.
    requireEmailVerification: false,
  },
  baseURL,
  secret: process.env.BETTER_AUTH_SECRET,
  trustedOrigins: [baseURL, 'http://localhost:3000'],
  // Forwards Set-Cookie headers through TanStack Start's server runtime so
  // sessions set inside server functions land on the response.
  plugins: [tanstackStartCookies()],
})

export type Session = typeof auth.$Infer.Session

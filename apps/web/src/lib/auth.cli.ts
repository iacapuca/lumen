// Entry point for the Better Auth CLI ONLY (schema generation / migrations).
//
// The CLI auto-discovers an exported `auth` instance, but the runtime uses the
// `getAuth(env)` factory (so it can read per-request Cloudflare bindings). This
// file bridges the two without forcing an eager, never-used pool into the Worker
// bundle: the CLI runs in Node and reads `process.env.DATABASE_URL`.
//
//   npx @better-auth/cli@latest migrate --config src/lib/auth.cli.ts -y
import { getAuth } from './auth'

export const auth = getAuth()

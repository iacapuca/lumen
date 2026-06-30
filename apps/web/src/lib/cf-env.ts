// SERVER-ONLY. Access to Cloudflare Workers bindings + secrets.
//
// On Cloudflare Workers, secrets and bindings (Hyperdrive, vars, ...) are NOT
// on `process.env`. They live on a per-isolate `env` object exposed by the
// `cloudflare:workers` runtime module. That module exists ONLY in the Workers
// runtime — importing it from local Node dev throws — so we resolve it through
// a guarded dynamic import and cache the result. In Node the import fails and
// we return `undefined`, which makes callers (`getAuth`) fall back to
// `process.env`.

export type CloudflareEnv = {
  /** Hyperdrive binding for Postgres. `connectionString` is the pooled DSN. */
  HYPERDRIVE?: { connectionString: string }
  BETTER_AUTH_URL?: string
  BETTER_AUTH_SECRET?: string
  DATABASE_URL?: string
  VITE_LUMEN_RUNTIME_URL?: string
  [key: string]: unknown
}

// `undefined` = not resolved yet, `null` = resolved but not on Workers.
let cached: CloudflareEnv | null | undefined

/**
 * Returns the Cloudflare Workers `env` for the current isolate, or `undefined`
 * when not running on Workers (e.g. local Node dev).
 */
export async function getCloudflareEnv(): Promise<CloudflareEnv | undefined> {
  if (cached !== undefined) return cached ?? undefined
  try {
    // `@vite-ignore` keeps Vite from trying to resolve/bundle this at build
    // time for the Node target, where `cloudflare:workers` does not exist.
    const mod = await import(/* @vite-ignore */ 'cloudflare:workers')
    cached = (mod.env ?? null) as CloudflareEnv | null
  } catch {
    cached = null
  }
  return cached ?? undefined
}

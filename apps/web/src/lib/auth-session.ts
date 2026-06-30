import { createServerFn } from '@tanstack/react-start'
import { getRequestHeaders } from '@tanstack/react-start/server'

import { getAuth } from './auth'
import { getCloudflareEnv } from './cf-env'

/**
 * Resolve the current Better Auth session on the server from the incoming
 * request cookies. Runs during SSR (no flash) and on client navigations (the
 * server-fn round-trips to the server). Returns `{ user, session } | null`.
 *
 * `getAuth` (and its `pg` import) only appears inside the `.handler()` body,
 * which the TanStack Start compiler strips from the client bundle. On Workers
 * the Cloudflare `env` (Hyperdrive + secrets) is resolved per request; in Node
 * dev `getCloudflareEnv()` returns `undefined` and `getAuth` reads process.env.
 */
export const fetchSession = createServerFn({ method: 'GET' }).handler(async () => {
  const env = await getCloudflareEnv()
  const headers = getRequestHeaders()
  const session = await getAuth(env).api.getSession({ headers })
  return session
})

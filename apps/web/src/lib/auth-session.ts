import { createServerFn } from '@tanstack/react-start'
import { getRequestHeaders } from '@tanstack/react-start/server'

import { getAuth } from './auth'

/**
 * Resolve the current Better Auth session on the server from the incoming
 * request cookies. Runs during SSR (no flash) and on client navigations (the
 * server-fn round-trips to the server). Returns `{ user, session } | null`.
 *
 * `getAuth` (and its `pg` import) only appears inside the `.handler()` body,
 * which the TanStack Start compiler strips from the client bundle.
 */
export const fetchSession = createServerFn({ method: 'GET' }).handler(async () => {
  const headers = getRequestHeaders()
  const session = await getAuth().api.getSession({ headers })
  return session
})

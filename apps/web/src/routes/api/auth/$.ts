import { createFileRoute } from '@tanstack/react-router'

import { getAuth } from '../../../lib/auth'
import { getCloudflareEnv } from '../../../lib/cf-env'

// Catch-all server route: forwards every method on /api/auth/* to Better Auth's
// Web-standard handler (Request -> Response). `server.handlers` runs server-side
// only, so importing `getAuth` (which pulls in `pg`) here never reaches the
// client. On Cloudflare Workers the per-request `env` (Hyperdrive + secrets) is
// resolved via `getCloudflareEnv()`; in local Node dev it returns `undefined`
// and `getAuth` falls back to `process.env`.
export const Route = createFileRoute('/api/auth/$')({
  server: {
    handlers: {
      GET: async ({ request }) => getAuth(await getCloudflareEnv()).handler(request),
      POST: async ({ request }) => getAuth(await getCloudflareEnv()).handler(request),
    },
  },
})

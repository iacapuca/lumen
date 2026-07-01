import { createFileRoute } from '@tanstack/react-router'

import { getAuth } from '../../../lib/auth'

// Catch-all server route: forwards every method on /api/auth/* to Better Auth's
// Web-standard handler (Request -> Response). `server.handlers` runs server-side
// only, so importing `getAuth` (which pulls in `pg`) here never reaches the
// client.
export const Route = createFileRoute('/api/auth/$')({
  server: {
    handlers: {
      GET: async ({ request }) => getAuth().handler(request),
      POST: async ({ request }) => getAuth().handler(request),
    },
  },
})

import { Outlet, createFileRoute, redirect } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { getRequestHeaders } from '@tanstack/react-start/server'

import { AppShell } from '../components/AppShell'
import { getAuth } from '../lib/auth'
import { ensureOrganization } from '../lib/organizations'

// Resolves the session AND guarantees it has an organization — every
// dashboard/data-source in the control plane is scoped to one, and the Rust
// runtime enforces that scoping at embed time (see docs/BUSINESS-PLAN.md
// §5.1). Fresh users get one auto-created here on their first protected-route
// visit; existing sessions are a no-op (ensureOrganization is idempotent).
const requireSessionFn = createServerFn({ method: 'GET' }).handler(async () => {
  const headers = getRequestHeaders()
  const session = await getAuth().api.getSession({ headers })
  if (!session) return null
  const { organizationId } = await ensureOrganization(session, headers)
  return { user: session.user, organizationId }
})

// Pathless layout (`_app`) that wraps every authenticated control-plane route.
// The session is resolved on the server in `beforeLoad`, so unauthenticated
// users are redirected to /login during SSR — no protected content ever flashes.
export const Route = createFileRoute('/_app')({
  beforeLoad: async () => {
    const result = await requireSessionFn()
    if (!result) {
      throw redirect({ to: '/login' })
    }
    return result
  },
  component: AppLayout,
})

function AppLayout() {
  return (
    <AppShell>
      <Outlet />
    </AppShell>
  )
}

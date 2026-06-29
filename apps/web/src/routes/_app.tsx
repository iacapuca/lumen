import { Outlet, createFileRoute, redirect } from '@tanstack/react-router'

import { AppShell } from '../components/AppShell'
import { fetchSession } from '../lib/auth-session'

// Pathless layout (`_app`) that wraps every authenticated control-plane route.
// The session is resolved on the server in `beforeLoad`, so unauthenticated
// users are redirected to /login during SSR — no protected content ever flashes.
export const Route = createFileRoute('/_app')({
  beforeLoad: async () => {
    const session = await fetchSession()
    if (!session) {
      throw redirect({ to: '/login' })
    }
    return { user: session.user }
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

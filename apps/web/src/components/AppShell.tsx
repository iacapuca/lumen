import type { ReactNode } from 'react'
import { useState } from 'react'
import { Link, useRouter } from '@tanstack/react-router'
import { RUNTIME_URL } from '../lib/config'
import { signOut, useSession } from '../lib/auth-client'

const NAV = [
  { to: '/dashboards', label: 'Dashboards' },
  { to: '/embedding', label: 'Embedding' },
  { to: '/data-sources', label: 'Data Sources' },
  { to: '/usage', label: 'Usage' },
] as const

export function AppShell({ children }: { children: ReactNode }) {
  const router = useRouter()
  const { data: session } = useSession()
  const [signingOut, setSigningOut] = useState(false)

  const email = session?.user?.email

  async function handleSignOut() {
    setSigningOut(true)
    try {
      await signOut()
      await router.invalidate()
      await router.navigate({ to: '/login' })
    } finally {
      setSigningOut(false)
    }
  }

  return (
    <div className="app">
      <div className="brand">
        <span className="brand__mark" />
        <span className="brand__name">Lumen</span>
      </div>

      <header className="header">
        <span className="header__title">Control plane</span>
        <div className="header__right">
          <span className="header__env" title="Lumen runtime base URL">
            {RUNTIME_URL}
          </span>
          {email ? <span className="header__user" title="Signed in">{email}</span> : null}
          <button
            type="button"
            className="header__signout"
            onClick={handleSignOut}
            disabled={signingOut}
          >
            {signingOut ? 'Signing out…' : 'Sign out'}
          </button>
        </div>
      </header>

      <nav className="sidebar">
        <div className="sidebar__section">Platform</div>
        {NAV.map((item) => (
          <Link
            key={item.to}
            to={item.to}
            className="nav-link"
            activeProps={{ className: 'nav-link active' }}
          >
            <span className="nav-link__dot" />
            {item.label}
          </Link>
        ))}
      </nav>

      <main className="main">{children}</main>
    </div>
  )
}

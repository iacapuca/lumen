import { useState } from 'react'
import type { FormEvent } from 'react'
import { Link, createFileRoute, useRouter } from '@tanstack/react-router'

import { signIn } from '../lib/auth-client'

export const Route = createFileRoute('/login')({
  component: LoginPage,
})

function LoginPage() {
  const router = useRouter()
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)

  async function onSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    setError(null)
    setPending(true)
    const { error: err } = await signIn.email({ email, password })
    if (err) {
      setError(err.message ?? 'Could not sign in. Check your email and password.')
      setPending(false)
      return
    }
    await router.invalidate()
    await router.navigate({ to: '/dashboards' })
  }

  return (
    <div className="auth">
      <Link to="/" className="auth__brand">
        <span className="brand__mark" />
        <span className="brand__name">Lumen</span>
      </Link>

      <form className="auth__card" onSubmit={onSubmit}>
        <h1 className="auth__title">Sign in</h1>
        <p className="auth__sub">Welcome back to the control plane.</p>

        {error ? <div className="auth__error">{error}</div> : null}

        <label className="auth__field">
          <span>Email</span>
          <input
            type="email"
            autoComplete="email"
            required
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder="you@company.com"
          />
        </label>

        <label className="auth__field">
          <span>Password</span>
          <input
            type="password"
            autoComplete="current-password"
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="••••••••"
          />
        </label>

        <button type="submit" className="auth__submit" disabled={pending}>
          {pending ? 'Signing in…' : 'Sign in'}
        </button>

        <p className="auth__alt">
          No account yet? <Link to="/signup">Create one</Link>
        </p>
      </form>
    </div>
  )
}

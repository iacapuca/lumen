import { useState } from 'react'
import type { FormEvent } from 'react'
import { Link, createFileRoute, useRouter } from '@tanstack/react-router'

import { signUp } from '../lib/auth-client'

export const Route = createFileRoute('/signup')({
  component: SignupPage,
})

function SignupPage() {
  const router = useRouter()
  const [name, setName] = useState('')
  const [email, setEmail] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)

  async function onSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault()
    setError(null)
    setPending(true)
    const { error: err } = await signUp.email({ name, email, password })
    if (err) {
      setError(err.message ?? 'Could not create your account.')
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
        <h1 className="auth__title">Create your account</h1>
        <p className="auth__sub">Start compiling dashboards for the edge.</p>

        {error ? <div className="auth__error">{error}</div> : null}

        <label className="auth__field">
          <span>Name</span>
          <input
            type="text"
            autoComplete="name"
            required
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="Ada Lovelace"
          />
        </label>

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
            autoComplete="new-password"
            required
            minLength={8}
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            placeholder="At least 8 characters"
          />
        </label>

        <button type="submit" className="auth__submit" disabled={pending}>
          {pending ? 'Creating account…' : 'Create account'}
        </button>

        <p className="auth__alt">
          Already have an account? <Link to="/login">Sign in</Link>
        </p>
      </form>
    </div>
  )
}

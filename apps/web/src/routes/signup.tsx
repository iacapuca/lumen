import { useState } from 'react'
import type { FormEvent } from 'react'
import { Link, createFileRoute, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { getRequestHeaders } from '@tanstack/react-start/server'

import { signUp } from '../lib/auth-client'
import { getAuth } from '../lib/auth'
import { ensureOrganization } from '../lib/organizations'

// Runs right after signUp.email() succeeds — creates the new user's one
// workspace/organization and its runtime API key (see lib/organizations.ts).
// Returns the plaintext key ONLY the first time (org just created); it is
// never retrievable again after this response.
const createWorkspaceFn = createServerFn({ method: 'GET' }).handler(async () => {
  const headers = getRequestHeaders()
  const session = await getAuth().api.getSession({ headers })
  if (!session) throw new Error('not signed in')
  return ensureOrganization(session, headers)
})

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
  const [apiKey, setApiKey] = useState<string | null>(null)

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
    const { apiKey: newKey } = await createWorkspaceFn()
    if (newKey) {
      // Show the "save this key" screen instead of navigating away — it's
      // never retrievable again once this response is gone.
      setApiKey(newKey)
      setPending(false)
      return
    }
    await router.invalidate()
    await router.navigate({ to: '/dashboards' })
  }

  async function continueToDashboards() {
    await router.invalidate()
    await router.navigate({ to: '/dashboards' })
  }

  if (apiKey) {
    return (
      <div className="auth">
        <Link to="/" className="auth__brand">
          <span className="brand__mark" />
          <span className="brand__name">Lumen</span>
        </Link>
        <div className="auth__card">
          <h1 className="auth__title">Save your runtime API key</h1>
          <p className="auth__sub">
            Your backend uses this to mint scoped embed tokens (<code>POST /tokens</code>).
            It's shown once — copy it now.
          </p>
          <pre className="code mono" style={{ wordBreak: 'break-all', whiteSpace: 'pre-wrap' }}>
            {apiKey}
          </pre>
          <button type="button" className="auth__submit" onClick={continueToDashboards}>
            I've saved it — continue
          </button>
        </div>
      </div>
    )
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

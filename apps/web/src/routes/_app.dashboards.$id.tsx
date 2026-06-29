import { useEffect, useRef, useState } from 'react'
import { Link, createFileRoute } from '@tanstack/react-router'
import { embedUrl } from '@lumen/contracts'

import { RUNTIME_URL } from '../lib/config'
import { getDashboard } from '../lib/dashboards'

export const Route = createFileRoute('/_app/dashboards/$id')({
  component: DashboardPreview,
})

type LoadState =
  | { status: 'loading' }
  | { status: 'ready'; token: string }
  | { status: 'error'; message: string }

type Mode = 'webcomponent' | 'iframe'

function DashboardPreview() {
  const { id } = Route.useParams()
  const dashboard = getDashboard(id)
  const [state, setState] = useState<LoadState>({ status: 'loading' })
  const [mode, setMode] = useState<Mode>('webcomponent')
  const [reload, setReload] = useState(0)
  const wcRef = useRef<HTMLDivElement>(null)

  // Mint a scoped token from the runtime's POST /tokens (the Embeddable model:
  // the token, not the iframe, is what scopes data via row-level security).
  // Fetched on the client so a down runtime degrades gracefully instead of
  // breaking SSR.
  useEffect(() => {
    let cancelled = false
    setState({ status: 'loading' })

    fetch(`${RUNTIME_URL}/tokens`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        dashboard: id,
        tenant_id: 'acme',
        user: 'control-plane-preview',
        security_context: { tenant_id: 'acme' },
      }),
    })
      .then(async (res) => {
        if (!res.ok) throw new Error(`runtime responded ${res.status} ${res.statusText}`)
        const body = (await res.json()) as { token: string }
        return body.token
      })
      .then((token) => {
        if (!cancelled) setState({ status: 'ready', token })
      })
      .catch((err: unknown) => {
        if (!cancelled) {
          const message = err instanceof Error ? err.message : String(err)
          setState({ status: 'error', message })
        }
      })

    return () => {
      cancelled = true
    }
  }, [id, reload])

  // Web Component mode: dynamically register <analytics-dashboard> (client-only)
  // and mount it imperatively — avoids custom-element JSX typing.
  useEffect(() => {
    if (mode !== 'webcomponent' || state.status !== 'ready') return
    const container = wcRef.current
    if (!container) return

    let el: HTMLElement | null = null
    let active = true
    import('@lumen/embed-sdk/element').then(() => {
      if (!active || !container) return
      el = document.createElement('analytics-dashboard')
      el.setAttribute('base', RUNTIME_URL)
      el.setAttribute('dashboard', id)
      el.setAttribute('token', state.token)
      el.style.display = 'block'
      el.style.minHeight = '720px'
      container.appendChild(el)
    })

    return () => {
      active = false
      if (el) el.remove()
    }
  }, [mode, state, id])

  return (
    <div>
      <div className="page-head">
        <h1>{dashboard?.title ?? id}</h1>
        <p>
          <Link to="/dashboards" className="dash-item__cta">
            ← Dashboards
          </Link>
          <span style={{ color: 'var(--text-faint)', marginLeft: 12 }}>
            <span className="badge">id: {id}</span>
          </span>
        </p>
      </div>

      <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 12 }}>
        <button
          className="btn"
          onClick={() => setMode('webcomponent')}
          style={mode === 'webcomponent' ? { outline: '2px solid var(--accent, #2563eb)' } : undefined}
        >
          Web Component
        </button>
        <button
          className="btn"
          onClick={() => setMode('iframe')}
          style={mode === 'iframe' ? { outline: '2px solid var(--accent, #2563eb)' } : undefined}
        >
          iframe
        </button>
        <span style={{ color: 'var(--text-faint)', fontSize: 12 }}>
          {mode === 'webcomponent'
            ? 'Shadow DOM — no iframe, native sizing'
            : 'sandboxed iframe — hard isolation'}
        </span>
      </div>

      <div className="preview">
        <div className="preview__frame-wrap">
          {state.status === 'loading' && (
            <div className="state">
              <div className="spinner" />
              <div className="state__title">Minting scoped token…</div>
              <div>Connecting to the runtime at {RUNTIME_URL}</div>
            </div>
          )}

          {state.status === 'error' && (
            <div className="state state--error">
              <div className="state__title">Runtime unreachable</div>
              <div>
                Could not reach the Lumen runtime at <code>{RUNTIME_URL}</code>.
                <br />
                {state.message}
              </div>
              <button
                className="btn"
                onClick={() => setReload((n) => n + 1)}
                style={{ marginTop: 8 }}
              >
                Retry
              </button>
            </div>
          )}

          {state.status === 'ready' && mode === 'iframe' && (
            <iframe
              className="preview__frame"
              title={dashboard?.title ?? id}
              src={embedUrl(RUNTIME_URL, id, state.token)}
            />
          )}

          {state.status === 'ready' && mode === 'webcomponent' && (
            <div ref={wcRef} />
          )}
        </div>
      </div>
    </div>
  )
}

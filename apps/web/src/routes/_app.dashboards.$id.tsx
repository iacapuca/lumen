import { useEffect, useRef, useState } from 'react'
import { Link, createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { queryOptions, useQuery, useSuspenseQuery } from '@tanstack/react-query'
import { embedUrl } from '@lumen/contracts'

import { RUNTIME_URL } from '../lib/config'
import { getBindings } from '../lib/bindings'
import { getDashboard } from '../lib/dashboards'

const getDashboardFn = createServerFn({ method: 'GET' })
  .validator((d: { id: string; organizationId: string }) => d)
  .handler(async ({ data }) => (await getDashboard(data.id, data.organizationId)) ?? null)

// Query key ['dashboards', id] is shared with _app.dashboards.builder.$id.tsx
// (same key, independently-defined queryFn) — invalidating it from the
// builder after a save also refreshes this preview page. Keep the key shape
// in sync between the two files if either changes.
const dashboardOptions = (id: string, organizationId: string) =>
  queryOptions({
    queryKey: ['dashboards', id],
    queryFn: () => getDashboardFn({ data: { id, organizationId } }),
  })

export const Route = createFileRoute('/_app/dashboards/$id')({
  loader: ({ params, context }) =>
    context.queryClient.ensureQueryData(dashboardOptions(params.id, context.organizationId)),
  component: DashboardPreview,
})

// Mints the preview token SERVER-SIDE (never in the browser): the control
// plane previews dashboards on behalf of whichever organization owns them,
// which requires LUMEN_INTERNAL_API_KEY — a first-party secret distinct from
// a design partner's own per-organization runtime API key (see
// crates/runtime/src/lib.rs `resolve_account`). That secret must never reach
// client JS, so this can't be a plain client-side fetch like it used to be.
const mintTokenFn = createServerFn({ method: 'POST' })
  .validator((d: { id: string; organizationId: string }) => d)
  .handler(async ({ data }) => {
    const internalKey = getBindings().LUMEN_INTERNAL_API_KEY
    const res = await fetch(`${RUNTIME_URL}/tokens`, {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        ...(internalKey ? { 'x-api-key': internalKey } : {}),
      },
      body: JSON.stringify({
        dashboard: data.id,
        account_id: data.organizationId,
        tenant_id: 'acme',
        user: 'control-plane-preview',
        security_context: { tenant_id: 'acme' },
      }),
    })
    if (!res.ok) throw new Error(`runtime responded ${res.status} ${res.statusText}`)
    return (await res.json()) as { token: string }
  })

type Mode = 'webcomponent' | 'iframe'

function DashboardPreview() {
  const { id } = Route.useParams()
  const { organizationId } = Route.useRouteContext()
  const { data: dashboard } = useSuspenseQuery(dashboardOptions(id, organizationId))
  const [mode, setMode] = useState<Mode>('webcomponent')
  const wcRef = useRef<HTMLDivElement>(null)

  // Client-triggered (via the server fn above), NOT prefetched in the loader —
  // a down runtime should degrade gracefully instead of breaking SSR of the
  // page itself. `retry: false` preserves the original
  // single-attempt-then-manual-"Retry" behavior.
  const tokenQuery = useQuery({
    queryKey: ['embed-token', id],
    queryFn: async () => (await mintTokenFn({ data: { id, organizationId } })).token,
    retry: false,
  })

  // Web Component mode: dynamically register <analytics-dashboard> (client-only)
  // and mount it imperatively — avoids custom-element JSX typing.
  useEffect(() => {
    if (mode !== 'webcomponent' || !tokenQuery.data) return
    const container = wcRef.current
    if (!container) return

    let el: HTMLElement | null = null
    let active = true
    import('@lumen/embed-sdk/element').then(() => {
      if (!active || !container) return
      el = document.createElement('analytics-dashboard')
      el.setAttribute('base', RUNTIME_URL)
      el.setAttribute('dashboard', id)
      el.setAttribute('token', tokenQuery.data)
      el.style.display = 'block'
      el.style.minHeight = '720px'
      container.appendChild(el)
    })

    return () => {
      active = false
      if (el) el.remove()
    }
  }, [mode, tokenQuery.data, id])

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
          {tokenQuery.isPending && (
            <div className="state">
              <div className="spinner" />
              <div className="state__title">Minting scoped token…</div>
              <div>Connecting to the runtime at {RUNTIME_URL}</div>
            </div>
          )}

          {tokenQuery.isError && (
            <div className="state state--error">
              <div className="state__title">Runtime unreachable</div>
              <div>
                Could not reach the Lumen runtime at <code>{RUNTIME_URL}</code>.
                <br />
                {tokenQuery.error instanceof Error ? tokenQuery.error.message : String(tokenQuery.error)}
              </div>
              <button className="btn" onClick={() => tokenQuery.refetch()} style={{ marginTop: 8 }}>
                Retry
              </button>
            </div>
          )}

          {tokenQuery.data && mode === 'iframe' && (
            <iframe
              className="preview__frame"
              title={dashboard?.title ?? id}
              src={embedUrl(RUNTIME_URL, id, tokenQuery.data)}
            />
          )}

          {tokenQuery.data && mode === 'webcomponent' && <div ref={wcRef} />}
        </div>
      </div>
    </div>
  )
}

import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/_app/usage')({
  component: Usage,
})

function Usage() {
  return (
    <div>
      <div className="page-head">
        <h1>Usage</h1>
        <p>Consumption and billing for your Lumen workspace.</p>
      </div>

      <div className="card card--muted">
        <div style={{ fontWeight: 600, color: 'var(--text)', marginBottom: 6 }}>
          Usage &amp; billing — compute credits
        </div>
        <div>
          Per-dashboard compute credits, query counts, and invoices will appear
          here. <span className="badge">coming soon</span>
        </div>
      </div>
    </div>
  )
}

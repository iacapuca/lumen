import { Link, createFileRoute } from '@tanstack/react-router'
import { dashboards } from '../lib/dashboards'

export const Route = createFileRoute('/_app/dashboards/')({
  component: DashboardsList,
})

function DashboardsList() {
  return (
    <div>
      <div className="page-head">
        <h1>Dashboards</h1>
        <p>Compiled dashboards available on this Lumen runtime.</p>
      </div>

      <div className="list">
        {dashboards.map((d) => {
          const id = d.id ?? ''
          return (
            <Link
              key={id}
              to="/dashboards/$id"
              params={{ id }}
              className="dash-item"
            >
              <div>
                <div className="dash-item__title">{d.title}</div>
                <div className="dash-item__meta">
                  id: {id} · {d.layout.length} widget{d.layout.length === 1 ? '' : 's'}
                  {d.theme ? ` · ${d.theme}` : ''}
                </div>
              </div>
              <span className="dash-item__cta">Preview →</span>
            </Link>
          )
        })}
      </div>
    </div>
  )
}

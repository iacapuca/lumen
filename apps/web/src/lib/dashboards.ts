import type { DashboardDef } from '@lumen/contracts'

/**
 * There is no list API on the runtime yet, so the control plane ships a
 * hardcoded catalog. Shape it as the real contract (`DashboardDef`) so swapping
 * in `GET /dashboards` later is a drop-in change.
 *
 * The runtime currently serves exactly one dashboard, id `sales`.
 */
export const dashboards: DashboardDef[] = [
  {
    id: 'sales',
    title: 'Sales Dashboard',
    theme: 'dark',
    layout: [
      { type: 'kpi', title: 'Revenue', measure: 'orders.revenue', format: 'currency' },
      { type: 'line_chart', title: 'Revenue over time', x: 'orders.created_at', y: 'orders.revenue', granularity: 'day' },
      { type: 'table', title: 'Orders' },
    ],
  },
]

export function getDashboard(id: string): DashboardDef | undefined {
  return dashboards.find((d) => d.id === id)
}

import { useState } from 'react'
import { Link, createFileRoute, useNavigate } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { queryOptions, useMutation, useQueryClient, useSuspenseQuery } from '@tanstack/react-query'
import { isValidDashboardId } from '@lumen/contracts'

import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'

import { createDashboard, listDashboards, type DashboardRecord } from '../lib/dashboards'

const listFn = createServerFn({ method: 'GET' })
  .validator((d: { organizationId: string }) => d)
  .handler(async ({ data }) => listDashboards(data.organizationId))

const createFn = createServerFn({ method: 'POST' })
  .validator((d: { id: string; title: string; organizationId: string }) => d)
  .handler(async ({ data }) => createDashboard(data))

// 3-element key ([kind, 'list', organizationId]) so it can never collide with
// the single-dashboard key ['dashboards', id] used by the preview/builder
// routes — those are 2-element and this is 3, structurally distinct regardless
// of what an organizationId or dashboard id string happens to contain.
const dashboardsOptions = (organizationId: string) =>
  queryOptions({
    queryKey: ['dashboards', 'list', organizationId],
    queryFn: () => listFn({ data: { organizationId } }),
  })

export const Route = createFileRoute('/_app/dashboards/')({
  loader: ({ context }) => context.queryClient.ensureQueryData(dashboardsOptions(context.organizationId)),
  component: DashboardsList,
})

function DashboardsList() {
  const { organizationId } = Route.useRouteContext()
  const { data: dashboards } = useSuspenseQuery(dashboardsOptions(organizationId))
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [dialogOpen, setDialogOpen] = useState(false)

  const createMutation = useMutation({
    mutationFn: (input: { id: string; title: string }) =>
      createFn({ data: { ...input, organizationId } }),
    onSuccess: async (dashboard) => {
      await queryClient.invalidateQueries({ queryKey: ['dashboards', 'list', organizationId] })
      setDialogOpen(false)
      await navigate({ to: '/dashboards/builder/$id', params: { id: dashboard.id } })
    },
  })

  return (
    <div>
      <div className="page-head">
        <h1>Dashboards</h1>
        <p>Compiled dashboards available on this Lumen runtime.</p>
      </div>

      <div style={{ marginBottom: 16 }}>
        <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
          <DialogTrigger asChild>
            <Button>New dashboard</Button>
          </DialogTrigger>
          <NewDashboardDialog mutation={createMutation} />
        </Dialog>
      </div>

      {dashboards.length === 0 ? (
        <p className="text-muted-foreground text-sm">
          No dashboards yet — click "New dashboard" to build one.
        </p>
      ) : (
        <div className="list">
          {dashboards.map((d: DashboardRecord) => {
            const widgetCount = d.definition.layout.length
            return (
              <div key={d.id} className="dash-item">
                <Link to="/dashboards/$id" params={{ id: d.id }} style={{ flex: 1 }}>
                  <div className="dash-item__title">{d.title}</div>
                  <div className="dash-item__meta">
                    id: {d.id} · {widgetCount} widget{widgetCount === 1 ? '' : 's'} · {d.theme}
                    {!d.compiledAt && ' · not compiled yet'}
                  </div>
                </Link>
                <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
                  <Link to="/dashboards/builder/$id" params={{ id: d.id }} className="dash-item__cta">
                    Edit
                  </Link>
                  <Link to="/dashboards/$id" params={{ id: d.id }} className="dash-item__cta">
                    Preview →
                  </Link>
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}

function NewDashboardDialog({
  mutation,
}: {
  mutation: ReturnType<typeof useMutation<DashboardRecord, Error, { id: string; title: string }>>
}) {
  const [id, setId] = useState('')
  const [title, setTitle] = useState('')

  const idError = id.length > 0 && !isValidDashboardId(id) ? 'Use letters, numbers, _, - only' : null

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!isValidDashboardId(id)) return
    mutation.mutate({ id, title: title || id })
  }

  return (
    <DialogContent>
      <form onSubmit={handleSubmit}>
        <DialogHeader>
          <DialogTitle>New dashboard</DialogTitle>
          <DialogDescription>
            The id becomes the dashboard's URL slug and compiled DEP filename —
            it can't be changed later.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          <div className="grid gap-2">
            <Label htmlFor="new-dash-id">Id</Label>
            <Input
              id="new-dash-id"
              value={id}
              onChange={(e) => setId(e.target.value)}
              placeholder="sales"
              required
            />
            {idError && <p className="text-destructive text-sm">{idError}</p>}
          </div>

          <div className="grid gap-2">
            <Label htmlFor="new-dash-title">Title</Label>
            <Input
              id="new-dash-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Sales Dashboard"
            />
          </div>

          {mutation.isError && (
            <p className="text-destructive text-sm">
              {mutation.error instanceof Error ? mutation.error.message : String(mutation.error)}
            </p>
          )}
        </div>

        <DialogFooter>
          <Button type="submit" disabled={mutation.isPending || !id}>
            {mutation.isPending ? 'Creating…' : 'Create & open builder'}
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  )
}

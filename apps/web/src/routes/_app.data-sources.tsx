import { useState } from 'react'
import { createFileRoute } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { queryOptions, useMutation, useQueryClient, useSuspenseQuery } from '@tanstack/react-query'
import type { ColumnDef } from '@tanstack/react-table'

import { DataTable, SortableHeader } from '@/components/data-table'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
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
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

import {
  createDataSource,
  deleteDataSource,
  listDataSources,
  setActiveDataSource,
  updateDataSource,
  type DataSource,
  type Provider,
} from '../lib/data-sources'

// ---------------------------------------------------------------------------
// Server functions — thin wrappers around lib/data-sources.ts. The `pg` import
// in that module only reaches the server bundle because it's only ever called
// from inside these .handler() bodies (same pattern as lib/auth-session.ts).
// ---------------------------------------------------------------------------

const listFn = createServerFn({ method: 'GET' }).handler(async () => listDataSources())

type UpsertInput = { name: string; provider: Provider; config: Record<string, string> }

const createFn = createServerFn({ method: 'POST' })
  .validator((d: UpsertInput) => d)
  .handler(async ({ data }) => createDataSource(data))

const updateFn = createServerFn({ method: 'POST' })
  .validator((d: UpsertInput & { id: string }) => d)
  .handler(async ({ data }) => {
    const { id, ...rest } = data
    return updateDataSource(id, rest)
  })

const deleteFn = createServerFn({ method: 'POST' })
  .validator((d: { id: string }) => d)
  .handler(async ({ data }) => {
    await deleteDataSource(data.id)
  })

const setActiveFn = createServerFn({ method: 'POST' })
  .validator((d: { id: string }) => d)
  .handler(async ({ data }) => {
    await setActiveDataSource(data.id)
  })

const dataSourcesOptions = () =>
  queryOptions({
    queryKey: ['data-sources'],
    queryFn: () => listFn(),
  })

export const Route = createFileRoute('/_app/data-sources')({
  loader: ({ context }) => context.queryClient.ensureQueryData(dataSourcesOptions()),
  component: DataSourcesPage,
})

// ---------------------------------------------------------------------------
// Form field definitions per provider — Lumen never talks to a warehouse
// directly, only to a semantic layer; each provider's config IS that
// semantic layer's connection (which already carries its own warehouse).
// ---------------------------------------------------------------------------

const PROVIDER_FIELDS: Record<Provider, { key: string; label: string; secret?: boolean; placeholder?: string }[]> = {
  cube: [
    { key: 'cube_url', label: 'Cube REST URL', placeholder: 'http://localhost:4000' },
    { key: 'cube_secret', label: 'Cube API secret', secret: true },
  ],
  dbt: [
    {
      key: 'dbt_graphql_url',
      label: 'dbt Semantic Layer GraphQL URL',
      placeholder: 'https://semantic-layer.cloud.getdbt.com/api/graphql',
    },
    { key: 'dbt_service_token', label: 'Service token', secret: true },
    { key: 'dbt_environment_id', label: 'Environment ID' },
  ],
}

const PROVIDER_LABEL: Record<Provider, string> = { cube: 'Cube', dbt: 'dbt Semantic Layer' }

function DataSourcesPage() {
  const { data: dataSources } = useSuspenseQuery(dataSourcesOptions())
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<DataSource | null>(null)
  const [dialogOpen, setDialogOpen] = useState(false)

  function invalidate() {
    return queryClient.invalidateQueries({ queryKey: ['data-sources'] })
  }

  const setActiveMutation = useMutation({
    mutationFn: (id: string) => setActiveFn({ data: { id } }),
    onSuccess: invalidate,
  })
  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteFn({ data: { id } }),
    onSuccess: invalidate,
  })

  function openCreate() {
    setEditing(null)
    setDialogOpen(true)
  }

  function openEdit(ds: DataSource) {
    setEditing(ds)
    setDialogOpen(true)
  }

  const columns: ColumnDef<DataSource>[] = [
    {
      accessorKey: 'name',
      header: ({ column }) => <SortableHeader column={column} label="Name" />,
      cell: ({ row }) => <span className="font-medium">{row.original.name}</span>,
    },
    {
      accessorKey: 'provider',
      header: ({ column }) => <SortableHeader column={column} label="Provider" />,
      cell: ({ row }) => PROVIDER_LABEL[row.original.provider],
    },
    {
      id: 'status',
      header: 'Status',
      cell: ({ row }) =>
        row.original.isActive ? <Badge>active</Badge> : <Badge variant="outline">inactive</Badge>,
    },
    {
      id: 'actions',
      header: () => <div className="text-right">Actions</div>,
      cell: ({ row }) => {
        const ds = row.original
        const settingActive = setActiveMutation.isPending && setActiveMutation.variables === ds.id
        const deleting = deleteMutation.isPending && deleteMutation.variables === ds.id
        return (
          <div className="flex justify-end gap-2">
            {!ds.isActive && (
              <Button
                variant="secondary"
                size="sm"
                disabled={settingActive}
                onClick={() => setActiveMutation.mutate(ds.id)}
              >
                Set active
              </Button>
            )}
            <Button variant="outline" size="sm" onClick={() => openEdit(ds)}>
              Edit
            </Button>
            <Button
              variant="destructive"
              size="sm"
              disabled={deleting}
              onClick={() => deleteMutation.mutate(ds.id)}
            >
              Delete
            </Button>
          </div>
        )
      },
    },
  ]

  return (
    <div>
      <div className="page-head">
        <h1>Data Sources</h1>
        <p>
          Connect Lumen to a semantic layer — Cube or dbt Semantic Layer. Lumen never
          queries a warehouse directly; the active connection here determines which
          one the runtime uses (no restart needed, applies within ~5s).
        </p>
      </div>

      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <CardTitle>Connections</CardTitle>
          <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
            <DialogTrigger asChild>
              <Button onClick={openCreate}>Add connection</Button>
            </DialogTrigger>
            <ConnectionDialog
              key={editing?.id ?? 'new'}
              editing={editing}
              onSaved={async () => {
                setDialogOpen(false)
                await invalidate()
              }}
            />
          </Dialog>
        </CardHeader>
        <CardContent>
          {dataSources.length === 0 ? (
            <p className="text-muted-foreground text-sm">
              No data sources configured yet. Lumen is falling back to the
              environment-derived default (SEMANTIC_PROVIDER / CUBE_URL).
            </p>
          ) : (
            <DataTable columns={columns} data={dataSources} />
          )}
        </CardContent>
      </Card>
    </div>
  )
}

function ConnectionDialog({
  editing,
  onSaved,
}: {
  editing: DataSource | null
  onSaved: () => void | Promise<void>
}) {
  const [name, setName] = useState(editing?.name ?? '')
  const [provider, setProvider] = useState<Provider>(editing?.provider ?? 'cube')
  const [config, setConfig] = useState<Record<string, string>>(
    (editing?.config as Record<string, string> | undefined) ?? {},
  )

  const saveMutation = useMutation({
    mutationFn: () =>
      editing
        ? updateFn({ data: { id: editing.id, name, provider, config } })
        : createFn({ data: { name, provider, config } }),
    onSuccess: onSaved,
  })

  function setField(key: string, value: string) {
    setConfig((c) => ({ ...c, [key]: value }))
  }

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    saveMutation.mutate()
  }

  return (
    <DialogContent>
      <form onSubmit={handleSubmit}>
        <DialogHeader>
          <DialogTitle>{editing ? 'Edit connection' : 'Add connection'}</DialogTitle>
          <DialogDescription>
            Provider config is stored as-is in Postgres (`data_sources` table) — treat
            secrets here the same as any other dev-grade credential.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-4 py-4">
          <div className="grid gap-2">
            <Label htmlFor="ds-name">Name</Label>
            <Input
              id="ds-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="Production Cube"
              required
            />
          </div>

          <div className="grid gap-2">
            <Label htmlFor="ds-provider">Semantic layer</Label>
            <Select
              value={provider}
              onValueChange={(v) => {
                setProvider(v as Provider)
                setConfig({})
              }}
            >
              <SelectTrigger id="ds-provider">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="cube">Cube</SelectItem>
                <SelectItem value="dbt">dbt Semantic Layer</SelectItem>
              </SelectContent>
            </Select>
          </div>

          {PROVIDER_FIELDS[provider].map((f) => (
            <div className="grid gap-2" key={f.key}>
              <Label htmlFor={`ds-${f.key}`}>{f.label}</Label>
              <Input
                id={`ds-${f.key}`}
                type={f.secret ? 'password' : 'text'}
                value={config[f.key] ?? ''}
                onChange={(e) => setField(f.key, e.target.value)}
                placeholder={f.placeholder}
                required
              />
            </div>
          ))}

          {saveMutation.isError && (
            <p className="text-destructive text-sm">
              {saveMutation.error instanceof Error ? saveMutation.error.message : String(saveMutation.error)}
            </p>
          )}
        </div>

        <DialogFooter>
          <Button type="submit" disabled={saveMutation.isPending}>
            {saveMutation.isPending ? 'Saving…' : 'Save'}
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  )
}

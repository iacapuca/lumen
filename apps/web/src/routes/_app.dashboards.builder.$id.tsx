import { useMemo, useState } from 'react'
import { createFileRoute, Link } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { queryOptions, useMutation, useQuery, useQueryClient, useSuspenseQuery } from '@tanstack/react-query'
import GridLayout, { useContainerWidth, type Layout } from 'react-grid-layout'
import 'react-grid-layout/css/styles.css'
import 'react-resizable/css/styles.css'
import type {
  CompileSummary,
  DashboardDef,
  Granularity,
  SemanticMeta,
  ValueFormat,
  WidgetDef,
  WidgetKind,
} from '@lumen/contracts'

import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'

import { RUNTIME_URL } from '../lib/config'
import { getDashboard, markDashboardCompiled, updateDashboardDefinition } from '../lib/dashboards'

// ---------------------------------------------------------------------------
// Server functions — same thin-wrapper pattern as the other dashboard routes.
// ---------------------------------------------------------------------------

const getDashboardFn = createServerFn({ method: 'GET' })
  .validator((d: { id: string; organizationId: string }) => d)
  .handler(async ({ data }) => (await getDashboard(data.id, data.organizationId)) ?? null)

const saveDashboardFn = createServerFn({ method: 'POST' })
  .validator((d: { id: string; organizationId: string; definition: DashboardDef }) => d)
  .handler(async ({ data }) => updateDashboardDefinition(data.id, data.organizationId, data.definition))

const markCompiledFn = createServerFn({ method: 'POST' })
  .validator((d: { id: string; organizationId: string; contentHash: string }) => d)
  .handler(async ({ data }) => {
    await markDashboardCompiled(data.id, data.organizationId, data.contentHash)
  })

// Same key shape as _app.dashboards.$id.tsx's dashboardOptions — invalidating
// ['dashboards', id] from here (after Save & Compile) also refreshes the
// preview page's cached copy. Keep the key shape in sync between the two files.
const dashboardOptions = (id: string, organizationId: string) =>
  queryOptions({
    queryKey: ['dashboards', id],
    queryFn: () => getDashboardFn({ data: { id, organizationId } }),
  })

export const Route = createFileRoute('/_app/dashboards/builder/$id')({
  loader: ({ params, context }) =>
    context.queryClient.ensureQueryData(dashboardOptions(params.id, context.organizationId)),
  component: DashboardBuilder,
})

// ---------------------------------------------------------------------------
// Widget kind metadata — mirrors crates/compiler/src/lib.rs exactly
// (widget_size + resolve_widget's per-kind field requirements).
// ---------------------------------------------------------------------------

type BuilderWidget = { key: string; def: WidgetDef }

const WIDGET_KINDS: WidgetKind[] = ['kpi', 'line_chart', 'bar_chart', 'table']

const WIDGET_LABEL: Record<WidgetKind, string> = {
  kpi: 'KPI',
  line_chart: 'Line chart',
  bar_chart: 'Bar chart',
  table: 'Table',
}

const DEFAULT_SIZE: Record<WidgetKind, { w: number; h: number }> = {
  kpi: { w: 3, h: 2 },
  line_chart: { w: 12, h: 4 },
  bar_chart: { w: 12, h: 4 },
  table: { w: 12, h: 4 },
}

type FieldRules = {
  measure?: 'required'
  x?: 'required' | 'optional'
  y?: 'required' | 'optional'
  granularity?: boolean
  format?: boolean
}

const FIELD_RULES: Record<WidgetKind, FieldRules> = {
  kpi: { measure: 'required', format: true },
  line_chart: { x: 'required', y: 'required', granularity: true, format: true },
  bar_chart: { x: 'required', y: 'required', format: true },
  table: { x: 'optional', y: 'optional', format: true },
}

const GRANULARITIES: Granularity[] = [
  'second', 'minute', 'hour', 'day', 'week', 'month', 'quarter', 'year',
]
const FORMATS: ValueFormat[] = ['currency', 'number', 'percent']

async function fetchMeta(): Promise<SemanticMeta> {
  const res = await fetch(`${RUNTIME_URL}/meta`)
  if (!res.ok) throw new Error(`runtime responded ${res.status}`)
  return (await res.json()) as SemanticMeta
}

function flattenMeta(meta: SemanticMeta | undefined) {
  const measures: { value: string; label: string }[] = []
  const dimensions: { value: string; label: string; type?: string }[] = []
  for (const cube of meta?.cubes ?? []) {
    for (const m of cube.measures) measures.push({ value: m.name, label: m.title ?? m.name })
    for (const d of cube.dimensions) {
      dimensions.push({ value: d.name, label: d.title ?? d.name, type: d.type })
    }
  }
  return { measures, dimensions }
}

function toGridLayout(widgets: BuilderWidget[]): Layout {
  return widgets.map(({ key, def }) => {
    const size = DEFAULT_SIZE[def.type]
    const pos = def.pos ?? { x: 0, y: 0, w: size.w, h: size.h }
    return { i: key, x: pos.x, y: pos.y, w: pos.w, h: pos.h }
  })
}

function nextFreeRow(widgets: BuilderWidget[]): number {
  return widgets.reduce((m, w) => Math.max(m, (w.def.pos?.y ?? 0) + (w.def.pos?.h ?? 0)), 0)
}

async function saveAndCompile(input: {
  dashboardId: string
  organizationId: string
  definition: DashboardDef
}): Promise<CompileSummary> {
  await saveDashboardFn({
    data: { id: input.dashboardId, organizationId: input.organizationId, definition: input.definition },
  })

  const res = await fetch(`${RUNTIME_URL}/compile`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(input.definition),
  })
  if (!res.ok) {
    // Compile errors (e.g. CompileError::MissingField) come back as plain
    // text, not JSON — only the 200 path is Json(...).
    throw new Error(await res.text())
  }
  const result = (await res.json()) as CompileSummary
  await markCompiledFn({
    data: { id: input.dashboardId, organizationId: input.organizationId, contentHash: result.content_hash },
  })
  return result
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

function DashboardBuilder() {
  const { id } = Route.useParams()
  const { organizationId } = Route.useRouteContext()
  const { data: dashboard } = useSuspenseQuery(dashboardOptions(id, organizationId))

  if (!dashboard) {
    return (
      <div>
        <div className="page-head">
          <h1>Dashboard not found</h1>
          <p>
            <Link to="/dashboards" className="dash-item__cta">← Dashboards</Link>
          </p>
        </div>
      </div>
    )
  }

  return (
    <BuilderInner
      key={id}
      dashboardId={id}
      organizationId={organizationId}
      initialTitle={dashboard.title}
      initialTheme={dashboard.theme}
      initialLayout={dashboard.definition.layout}
      initiallyCompiled={!!dashboard.compiledAt}
    />
  )
}

function BuilderInner({
  dashboardId,
  organizationId,
  initialTitle,
  initialTheme,
  initialLayout,
  initiallyCompiled,
}: {
  dashboardId: string
  organizationId: string
  initialTitle: string
  initialTheme: string
  initialLayout: WidgetDef[]
  initiallyCompiled: boolean
}) {
  const queryClient = useQueryClient()
  const [title, setTitle] = useState(initialTitle)
  const [theme, setTheme] = useState(initialTheme)
  const [widgets, setWidgets] = useState<BuilderWidget[]>(() =>
    initialLayout.map((def) => ({ key: crypto.randomUUID(), def })),
  )
  const [selectedKey, setSelectedKey] = useState<string | null>(null)
  // "Just compiled" indicator — local, set immediately on mutation success.
  // Distinct from the ['dashboards', id] query (also invalidated below), which
  // is what keeps the dashboards list / preview page in sync.
  const [compiled, setCompiled] = useState(initiallyCompiled)

  const { width, containerRef, mounted } = useContainerWidth()
  const layout = useMemo(() => toGridLayout(widgets), [widgets])
  const selected = widgets.find((w) => w.key === selectedKey) ?? null

  const metaQuery = useQuery({ queryKey: ['semantic-meta'], queryFn: fetchMeta })

  const compileMutation = useMutation({
    mutationFn: saveAndCompile,
    onSuccess: () => {
      setCompiled(true)
      queryClient.invalidateQueries({ queryKey: ['dashboards', dashboardId] })
      queryClient.invalidateQueries({ queryKey: ['dashboards', 'list', organizationId] })
    },
  })

  function addWidget(kind: WidgetKind) {
    const key = crypto.randomUUID()
    const size = DEFAULT_SIZE[kind]
    const y = nextFreeRow(widgets)
    setWidgets((ws) => [...ws, { key, def: { type: kind, pos: { x: 0, y, w: size.w, h: size.h } } }])
    setSelectedKey(key)
  }

  function removeWidget(key: string) {
    setWidgets((ws) => ws.filter((w) => w.key !== key))
    if (selectedKey === key) setSelectedKey(null)
  }

  function updateSelected(patch: Partial<WidgetDef>) {
    if (!selectedKey) return
    setWidgets((ws) =>
      ws.map((w) => (w.key === selectedKey ? { ...w, def: { ...w.def, ...patch } } : w)),
    )
  }

  function handleLayoutChange(newLayout: Layout) {
    setWidgets((ws) =>
      ws.map((w) => {
        const item = newLayout.find((l) => l.i === w.key)
        if (!item) return w
        return { ...w, def: { ...w.def, pos: { x: item.x, y: item.y, w: item.w, h: item.h } } }
      }),
    )
  }

  function handleSaveAndCompile() {
    const definition: DashboardDef = {
      id: dashboardId,
      title,
      theme,
      layout: widgets.map((w) => w.def),
    }
    compileMutation.mutate({ dashboardId, organizationId, definition })
  }

  return (
    <div>
      <div className="page-head" style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', gap: 16 }}>
        <div style={{ flex: 1 }}>
          <p style={{ marginBottom: 6 }}>
            <Link to="/dashboards" className="dash-item__cta">← Dashboards</Link>
          </p>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
            <Input
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              style={{ maxWidth: 320, fontWeight: 650, fontSize: 18 }}
            />
            <Select value={theme} onValueChange={setTheme}>
              <SelectTrigger style={{ width: 110 }}>
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="light">light</SelectItem>
                <SelectItem value="dark">dark</SelectItem>
              </SelectContent>
            </Select>
            <Badge variant="outline">id: {dashboardId}</Badge>
          </div>
        </div>
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          {compiled && (
            <Link to="/dashboards/$id" params={{ id: dashboardId }} className="dash-item__cta">
              Preview →
            </Link>
          )}
          <Button onClick={handleSaveAndCompile} disabled={compileMutation.isPending}>
            {compileMutation.isPending ? 'Saving…' : 'Save & Compile'}
          </Button>
        </div>
      </div>

      {compileMutation.isSuccess && (
        <div className="card card--muted" style={{ marginBottom: 16 }}>
          ✓ Compiled — {compileMutation.data.widgets} widgets · {compileMutation.data.queries} queries · ~
          {compileMutation.data.credits} credits · {compileMutation.data.content_hash.slice(0, 19)}…
        </div>
      )}
      {compileMutation.isError && (
        <div className="card card--muted" style={{ marginBottom: 16, color: 'var(--danger)' }}>
          {compileMutation.error instanceof Error ? compileMutation.error.message : String(compileMutation.error)}
        </div>
      )}

      <div style={{ display: 'grid', gridTemplateColumns: '160px 1fr 300px', gap: 16, alignItems: 'start' }}>
        <Card style={{ padding: 12, display: 'flex', flexDirection: 'column', gap: 8 }}>
          <Label style={{ marginBottom: 4 }}>Add widget</Label>
          {WIDGET_KINDS.map((kind) => (
            <Button key={kind} variant="outline" size="sm" onClick={() => addWidget(kind)}>
              {WIDGET_LABEL[kind]}
            </Button>
          ))}
        </Card>

        <div ref={containerRef}>
          {widgets.length === 0 && (
            <p className="text-muted-foreground text-sm">
              Click a widget type on the left to add one to the canvas.
            </p>
          )}
          {mounted && widgets.length > 0 && (
            <GridLayout
              width={width}
              layout={layout}
              gridConfig={{ cols: 12, rowHeight: 80, margin: [16, 16] }}
              onLayoutChange={handleLayoutChange}
            >
              {widgets.map((w) => (
                <div key={w.key} onClick={() => setSelectedKey(w.key)}>
                  <Card
                    className={`h-full w-full p-3 flex flex-col gap-1 cursor-pointer ${
                      w.key === selectedKey ? 'ring-2 ring-primary' : ''
                    }`}
                  >
                    <Badge className="self-start">{WIDGET_LABEL[w.def.type]}</Badge>
                    <div style={{ fontWeight: 600 }}>{w.def.title || '(untitled)'}</div>
                    <div className="text-muted-foreground" style={{ fontSize: 12 }}>
                      {w.def.measure && `measure: ${w.def.measure}`}
                      {(w.def.x || w.def.y) && `${w.def.x ?? '—'} / ${w.def.y ?? '—'}`}
                      {!w.def.measure && !w.def.x && !w.def.y && 'no fields bound yet'}
                    </div>
                  </Card>
                </div>
              ))}
            </GridLayout>
          )}
        </div>

        {selected ? (
          <PropertyPanel
            widget={selected}
            meta={metaQuery.data}
            metaError={metaQuery.isError ? String(metaQuery.error) : null}
            onChange={updateSelected}
            onDelete={() => removeWidget(selected.key)}
          />
        ) : (
          <Card style={{ padding: 16 }}>
            <p className="text-muted-foreground text-sm">Select a widget to edit its fields.</p>
          </Card>
        )}
      </div>
    </div>
  )
}

function PropertyPanel({
  widget,
  meta,
  metaError,
  onChange,
  onDelete,
}: {
  widget: BuilderWidget
  meta: SemanticMeta | undefined
  metaError: string | null
  onChange: (patch: Partial<WidgetDef>) => void
  onDelete: () => void
}) {
  const rules = FIELD_RULES[widget.def.type]
  const { measures, dimensions } = useMemo(() => flattenMeta(meta), [meta])
  const xOptions = widget.def.type === 'line_chart' ? dimensions.filter((d) => d.type === 'time') : dimensions

  return (
    <Card style={{ padding: 16, display: 'flex', flexDirection: 'column', gap: 14 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <Badge>{WIDGET_LABEL[widget.def.type]}</Badge>
        <Button variant="destructive" size="sm" onClick={onDelete}>
          Delete
        </Button>
      </div>

      {metaError && (
        <p className="text-destructive" style={{ fontSize: 12 }}>
          Could not load fields from {RUNTIME_URL}/meta: {metaError}
        </p>
      )}

      <div className="grid gap-2">
        <Label>Title</Label>
        <Input value={widget.def.title ?? ''} onChange={(e) => onChange({ title: e.target.value })} />
      </div>

      {rules.measure && (
        <FieldSelect
          label="Measure"
          value={widget.def.measure}
          options={measures}
          onChange={(v) => onChange({ measure: v })}
        />
      )}
      {rules.x && (
        <FieldSelect
          label={`X dimension${rules.x === 'optional' ? ' (optional)' : ''}`}
          value={widget.def.x}
          options={xOptions}
          onChange={(v) => onChange({ x: v })}
          clearable={rules.x === 'optional'}
        />
      )}
      {rules.y && (
        <FieldSelect
          label={`Y measure${rules.y === 'optional' ? ' (optional)' : ''}`}
          value={widget.def.y}
          options={measures}
          onChange={(v) => onChange({ y: v })}
          clearable={rules.y === 'optional'}
        />
      )}
      {rules.granularity && (
        <div className="grid gap-2">
          <Label>Granularity</Label>
          <Select
            value={widget.def.granularity ?? 'month'}
            onValueChange={(v) => onChange({ granularity: v as Granularity })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {GRANULARITIES.map((g) => (
                <SelectItem key={g} value={g}>
                  {g}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
      {rules.format && (
        <div className="grid gap-2">
          <Label>Format</Label>
          <Select
            value={widget.def.format ?? '__none'}
            onValueChange={(v) => onChange({ format: v === '__none' ? undefined : (v as ValueFormat) })}
          >
            <SelectTrigger>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="__none">—</SelectItem>
              {FORMATS.map((f) => (
                <SelectItem key={f} value={f}>
                  {f}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
    </Card>
  )
}

// Radix's <Select.Item> rejects an empty-string value (reserved internally),
// so "unset"/"clear" use non-empty sentinels instead of "".
const UNSET = '__unset'
const CLEAR = '__clear'

function FieldSelect({
  label,
  value,
  options,
  onChange,
  clearable,
}: {
  label: string
  value?: string
  options: { value: string; label: string }[]
  onChange: (v: string | undefined) => void
  clearable?: boolean
}) {
  return (
    <div className="grid gap-2">
      <Label>{label}</Label>
      <Select
        value={value ?? UNSET}
        onValueChange={(v) => onChange(v === CLEAR ? undefined : v)}
      >
        <SelectTrigger>
          <SelectValue placeholder="Select a field…" />
        </SelectTrigger>
        <SelectContent>
          {clearable && <SelectItem value={CLEAR}>—</SelectItem>}
          {options.map((o) => (
            <SelectItem key={o.value} value={o.value}>
              {o.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

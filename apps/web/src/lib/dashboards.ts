// SERVER-ONLY. This module imports `pg` (transitively, via the Drizzle
// client) — only import it from server functions / route handlers, never
// from client components.
//
// Postgres-backed dashboard catalog, scoped to an organization (see
// lib/organizations.ts). `id` is the primary key directly (not a UUID) because
// it round-trips as-is into the runtime's dashboard id and `{id}.lumen`
// filename — see crates/runtime/src/lib.rs's `is_valid_dashboard_id` /
// `compile_dashboard`. Schema lives in lib/db/schema.ts.
import { and, asc, eq } from 'drizzle-orm'

import type { DashboardDef } from '@lumen/contracts'
import { isValidDashboardId } from '@lumen/contracts'

import { getDb } from './db/client'
import { dashboards } from './db/schema'

export type DashboardRecord = {
  id: string
  organizationId: string
  title: string
  theme: string
  definition: DashboardDef
  compiledAt: string | null
  contentHash: string | null
  createdAt: string
  updatedAt: string
}

type Row = typeof dashboards.$inferSelect

function fromRow(r: Row): DashboardRecord {
  return {
    id: r.id,
    organizationId: r.organizationId,
    title: r.title,
    theme: r.theme,
    definition: r.definition as DashboardDef,
    compiledAt: r.compiledAt?.toISOString() ?? null,
    contentHash: r.contentHash,
    createdAt: r.createdAt.toISOString(),
    updatedAt: r.updatedAt.toISOString(),
  }
}

/** Lists only the calling organization's dashboards — never a global list. */
export async function listDashboards(organizationId: string): Promise<DashboardRecord[]> {
  const db = getDb()
  const rows = await db
    .select()
    .from(dashboards)
    .where(eq(dashboards.organizationId, organizationId))
    .orderBy(asc(dashboards.createdAt))
  return rows.map(fromRow)
}

/**
 * Fetch a dashboard IF it belongs to `organizationId` — returns `undefined`
 * both when the id doesn't exist and when it belongs to a different org
 * (same response either way, so this can't be used to probe which ids
 * exist). This is the control-plane's own access check; the Rust runtime
 * enforces the identical rule independently at embed time.
 */
export async function getDashboard(
  id: string,
  organizationId: string,
): Promise<DashboardRecord | undefined> {
  const db = getDb()
  const [row] = await db
    .select()
    .from(dashboards)
    .where(and(eq(dashboards.id, id), eq(dashboards.organizationId, organizationId)))
    .limit(1)
  return row ? fromRow(row) : undefined
}

export async function createDashboard(
  input: { id: string; title: string; organizationId: string; theme?: string },
): Promise<DashboardRecord> {
  if (!isValidDashboardId(input.id)) {
    throw new Error(`invalid dashboard id "${input.id}" (use letters, numbers, _, - only)`)
  }
  const db = getDb()
  const theme = input.theme ?? 'light'
  const definition: DashboardDef = { id: input.id, title: input.title, theme, layout: [] }
  const [row] = await db
    .insert(dashboards)
    .values({ id: input.id, organizationId: input.organizationId, title: input.title, theme, definition })
    .returning()
  return fromRow(row)
}

export async function updateDashboardDefinition(
  id: string,
  organizationId: string,
  definition: DashboardDef,
): Promise<DashboardRecord> {
  const db = getDb()
  const [row] = await db
    .update(dashboards)
    .set({ title: definition.title, theme: definition.theme ?? 'light', definition, updatedAt: new Date() })
    .where(and(eq(dashboards.id, id), eq(dashboards.organizationId, organizationId)))
    .returning()
  if (!row) throw new Error(`dashboard ${id} not found in this organization`)
  return fromRow(row)
}

export async function markDashboardCompiled(
  id: string,
  organizationId: string,
  contentHash: string,
): Promise<void> {
  const db = getDb()
  await db
    .update(dashboards)
    .set({ compiledAt: new Date(), contentHash })
    .where(and(eq(dashboards.id, id), eq(dashboards.organizationId, organizationId)))
}

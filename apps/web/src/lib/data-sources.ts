// SERVER-ONLY. This module imports `pg` (transitively, via the Drizzle
// client) — only import it from server functions / route handlers, never
// from client components.
//
// Reads and writes the same `data_sources` table the Lumen runtime reads from
// (crates/runtime/src/lib.rs `resolve_active_semantic`) — both sides share
// the same Postgres. The runtime caches its read for ~5s, so a change made
// here takes effect on the runtime within that window, no restart needed.
//
// Schema lives in lib/db/schema.ts; the Rust runtime still creates this same
// table via raw SQL (`ensure_data_sources_schema`) since it has no ORM — keep
// the two in sync if either changes.
import { asc, eq } from 'drizzle-orm'

import { getDb } from './db/client'
import { dataSources } from './db/schema'

export type Provider = 'cube' | 'dbt'

export type CubeConfig = {
  cube_url: string
  cube_secret: string
}

export type DbtConfig = {
  dbt_graphql_url: string
  dbt_service_token: string
  dbt_environment_id: string
}

export type DataSource = {
  id: string
  name: string
  provider: Provider
  config: CubeConfig | DbtConfig | Record<string, string>
  isActive: boolean
  createdAt: string
  updatedAt: string
}

type Row = typeof dataSources.$inferSelect

function fromRow(r: Row): DataSource {
  return {
    id: r.id,
    name: r.name,
    provider: r.provider as Provider,
    config: r.config as Record<string, string>,
    isActive: r.isActive,
    createdAt: r.createdAt.toISOString(),
    updatedAt: r.updatedAt.toISOString(),
  }
}

export async function listDataSources(): Promise<DataSource[]> {
  const db = getDb()
  const rows = await db.select().from(dataSources).orderBy(asc(dataSources.createdAt))
  return rows.map(fromRow)
}

export async function createDataSource(
  input: { name: string; provider: Provider; config: Record<string, string> },
): Promise<DataSource> {
  const db = getDb()
  const [row] = await db
    .insert(dataSources)
    .values({ name: input.name, provider: input.provider, config: input.config })
    .returning()
  return fromRow(row)
}

export async function updateDataSource(
  id: string,
  input: { name: string; provider: Provider; config: Record<string, string> },
): Promise<DataSource> {
  const db = getDb()
  const [row] = await db
    .update(dataSources)
    .set({ name: input.name, provider: input.provider, config: input.config, updatedAt: new Date() })
    .where(eq(dataSources.id, id))
    .returning()
  if (!row) throw new Error(`data source ${id} not found`)
  return fromRow(row)
}

export async function deleteDataSource(id: string): Promise<void> {
  const db = getDb()
  await db.delete(dataSources).where(eq(dataSources.id, id))
}

/**
 * Mark exactly one data source active. Two statements in one transaction:
 * the unique partial index (`data_sources_one_active`) only allows one
 * `is_active = true` row at a time, so the old row must be cleared before the
 * new one is set, or the second statement would violate the index.
 */
export async function setActiveDataSource(id: string): Promise<void> {
  const db = getDb()
  await db.transaction(async (tx) => {
    await tx.update(dataSources).set({ isActive: false }).where(eq(dataSources.isActive, true))
    const result = await tx
      .update(dataSources)
      .set({ isActive: true, updatedAt: new Date() })
      .where(eq(dataSources.id, id))
      .returning({ id: dataSources.id })
    if (result.length === 0) throw new Error(`data source ${id} not found`)
  })
}

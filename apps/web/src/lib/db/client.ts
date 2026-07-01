// SERVER-ONLY. Shared Drizzle client factory — the single place that builds a
// `pg.Pool` for this app. Better Auth (via drizzleAdapter, see ../auth.ts) and
// every Lumen-owned table (lib/data-sources.ts, lib/dashboards.ts,
// lib/organizations.ts) all go through this same client.
//
// Memoised once per isolate: bindings (and thus the Hyperdrive connection
// string) are stable for the isolate's lifetime, so building one Pool and
// reusing it is correct — matches Better Auth's own singleton in ../auth.ts.
import { drizzle } from 'drizzle-orm/node-postgres'
import { Pool } from 'pg'

import { getBindings } from '../bindings'
import * as schema from './schema'

export type Db = ReturnType<typeof drizzle<typeof schema>>

let _db: Db | undefined

export function getDb(): Db {
  if (!_db) {
    const connectionString = getBindings().HYPERDRIVE.connectionString
    _db = drizzle(new Pool({ connectionString }), { schema })
  }
  return _db
}

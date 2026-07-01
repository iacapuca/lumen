// drizzle-kit config — used by `pnpm drizzle:generate`/`pnpm drizzle:migrate`
// (plain Node CLI commands, run OUTSIDE the Workers runtime, so this reads
// DATABASE_URL from a plain env var, not getBindings()). Local dev default
// matches infra/docker-compose.yml's Postgres; override DATABASE_URL for
// any other target.
import { defineConfig } from 'drizzle-kit'

export default defineConfig({
  schema: './src/lib/db/schema.ts',
  out: './drizzle',
  dialect: 'postgresql',
  dbCredentials: {
    url: process.env.DATABASE_URL ?? 'postgres://lumen:lumen@localhost:5544/lumen',
  },
})

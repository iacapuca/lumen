// SERVER-ONLY. Every signed-up user gets exactly one organization (their
// "workspace") — this is the account boundary the Rust runtime enforces
// (see crates/runtime/src/lib.rs: dashboards.organization_id, and
// organization.runtimeApiKeyHash gating POST /tokens). No multi-org-per-user,
// no invites/teams yet — that's deliberately out of scope until a design
// partner actually needs it (see docs/BUSINESS-PLAN.md §6).
import { randomBytes, createHash } from 'node:crypto'
import { asc, eq } from 'drizzle-orm'

import { getAuth, type Session } from './auth'
import { getDb } from './db/client'
import { member, organization } from './db/schema'

function generateApiKey(): { key: string; prefix: string; hash: string } {
  const key = `lumen_sk_${randomBytes(24).toString('hex')}`
  const hash = createHash('sha256').update(key).digest('hex')
  return { key, prefix: key.slice(0, 16), hash }
}

/**
 * Ensure the current session's user has an active organization, creating one
 * (with a freshly generated runtime API key) if this is their first time.
 * Idempotent — a no-op (and `apiKey: null`) once an org already exists.
 *
 * The runtime API key is generated and hashed here, then written directly
 * via SQL — never through Better Auth's public create/update API bodies,
 * because `runtimeApiKeyHash`/`runtimeApiKeyPrefix` are declared `input:
 * false` in auth.ts specifically so a client can never set their own hash.
 */
export async function ensureOrganization(
  session: Session,
  headers: Headers,
): Promise<{ organizationId: string; apiKey: string | null }> {
  const existing = session.session.activeOrganizationId
  if (existing) return { organizationId: existing, apiKey: null }

  const db = getDb()
  const auth = getAuth()

  // `session.session.activeOrganizationId` is per-SESSION, not per-user — a
  // brand-new session (new device/browser/tab, or after clearing cookies)
  // always starts with it null, which is NOT the same as "this user has no
  // organization yet". Check the user's actual membership first, or every
  // fresh sign-in mints a duplicate workspace (this is exactly the bug that
  // gave one user four organizations).
  const [existingMembership] = await db
    .select({ organizationId: member.organizationId })
    .from(member)
    .where(eq(member.userId, session.user.id))
    .orderBy(asc(member.createdAt))
    .limit(1)

  if (existingMembership) {
    await auth.api.setActiveOrganization({
      headers,
      body: { organizationId: existingMembership.organizationId },
    })
    return { organizationId: existingMembership.organizationId, apiKey: null }
  }

  const slug = `org-${randomBytes(6).toString('hex')}`
  const org = await auth.api.createOrganization({
    headers,
    body: { name: `${session.user.name}'s workspace`, slug },
  })
  if (!org) throw new Error('failed to create organization')

  const { key, prefix, hash } = generateApiKey()
  await db
    .update(organization)
    .set({ runtimeApiKeyHash: hash, runtimeApiKeyPrefix: prefix })
    .where(eq(organization.id, org.id))
  await auth.api.setActiveOrganization({ headers, body: { organizationId: org.id } })

  return { organizationId: org.id, apiKey: key }
}

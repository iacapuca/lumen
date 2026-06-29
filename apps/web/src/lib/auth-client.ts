import { createAuthClient } from 'better-auth/react'

// Client-safe. Talks to the catch-all route at `/api/auth/*`. The base URL is
// the app's own origin; we read it from a Vite-public env var when present and
// fall back to the current origin (client) or localhost (SSR).
const baseURL =
  (import.meta.env.VITE_BETTER_AUTH_URL as string | undefined) ??
  (typeof window !== 'undefined' ? window.location.origin : 'http://localhost:3000')

export const authClient = createAuthClient({ baseURL })

export const { signIn, signUp, signOut, useSession, getSession } = authClient

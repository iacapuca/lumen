/**
 * Base URL of the Lumen Rust runtime (the backend that serves compiled
 * dashboards). Single source of truth — read from an env var, with the local
 * dev default. Set `VITE_LUMEN_RUNTIME_URL` to point at another runtime.
 *
 * Runtime surface (see @lumen/contracts):
 *   GET /healthz
 *   GET /dev/token                      → text/plain dev JWT
 *   GET /embed/dashboard/:id?token=...  → text/html (compiled dashboard)
 *   GET /_lumen/runtime.js
 */
export const RUNTIME_URL: string =
  (import.meta.env.VITE_LUMEN_RUNTIME_URL as string | undefined) ??
  'http://localhost:8088'

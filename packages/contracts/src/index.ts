/**
 * Shared contracts between the Lumen Rust backend and the TypeScript frontend.
 *
 * These MIRROR the Rust types in `crates/shared` (the authored dashboard model)
 * and the runtime's HTTP surface. They are hand-written for now; the intended
 * path is to emit an OpenAPI spec from the Axum runtime (e.g. `utoipa`) and
 * generate this file with `openapi-typescript`, so FE/BE never drift.
 *
 * Keep in sync with: crates/shared/src/lib.rs
 */

// ---- Authored dashboard model (mirrors DashboardDef / WidgetDef) ----

export type WidgetKind = "kpi" | "line_chart" | "bar_chart" | "table";
export type ValueFormat = "currency" | "number" | "percent";
export type Granularity =
  | "second"
  | "minute"
  | "hour"
  | "day"
  | "week"
  | "month"
  | "quarter"
  | "year";

export interface WidgetDef {
  type: WidgetKind;
  title?: string;
  /** KPI: the measure to show, e.g. "orders.revenue". */
  measure?: string;
  /** Charts: x dimension (often a time dimension). */
  x?: string;
  /** Charts: y measure. */
  y?: string;
  granularity?: Granularity;
  format?: ValueFormat;
}

export interface DashboardDef {
  id?: string;
  title: string;
  /** "light" | "dark" */
  theme?: string;
  layout: WidgetDef[];
}

// ---- Runtime HTTP surface ----

/** GET /healthz → "ok" */
export const HEALTHZ = "/healthz";

/** GET /dev/token → text/plain JWT (dev only). */
export const DEV_TOKEN = "/dev/token";

/** GET /_lumen/runtime.js → the client hydration runtime. */
export const RUNTIME_JS = "/_lumen/runtime.js";

/** GET /embed/dashboard/:id?token=... → text/html (the compiled dashboard). */
export function embedUrl(baseUrl: string, dashboardId: string, token: string): string {
  const base = baseUrl.replace(/\/$/, "");
  return `${base}/embed/dashboard/${encodeURIComponent(dashboardId)}?token=${encodeURIComponent(token)}`;
}

/** A claim payload mintable into an embed JWT (mirrors lumen-auth Claims). */
export interface EmbedClaims {
  sub: string;
  tenant_id: string;
  roles?: string[];
  permissions?: string[];
  /** Forwarded verbatim to the semantic layer for row-level security. */
  security_context?: Record<string, unknown>;
}

// ---- Compiled-artifact metadata (mirrors the DEP Manifest, read-only) ----

export type Renderer = "svg" | "echarts" | "html";

export interface DepManifestView {
  dep_id: string;
  title: string;
  theme: string;
  content_hash: string;
  created_at: string;
  widgets: string[];
  queries: string[];
  renderers: Renderer[];
  required_credits: { model: string; estimate: number; queries: number };
}

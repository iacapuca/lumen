# Adversarial review — findings & resolutions

A multi-agent adversarial review (3 dimension reviewers → independent verifiers)
ran over the Rust crates. 23 raised, **21 confirmed**. Resolutions below.

## Fixed (verified)

| # | Sev | Issue | Fix |
|---|-----|-------|-----|
| 1 | High | Stored XSS — Cube values serialized unescaped into `<script type="application/json">` islands (`</script>` breakout) | `json_island()` escapes `<>&` + U+2028/9 to `\u00xx`; added a strict `Content-Security-Policy` + `X-Content-Type-Options: nosniff` |
| 2 | High | Per-query compute credits double-counted (carried on both `semantic_compute` and the `dashboard_render` total) | Each event carries only its own contribution; render event = Σ widget_base + egress, compute on the query events. `sum(credits)` now correct |
| 3 | High | 304/ETag path billed 0 and emitted no render event | 304 now emits a `dashboard_render` event billed `RENDER_FLOOR`; ETag check moved before query execution so a 304 skips querying + rendering |
| 4 | High | DEP cache (A) keyed only by id, no TTL → recompiled dashboard stale forever | 10s TTL on the DEP cache |
| 5 | Med | Path traversal via unsanitized `{id}` in `build_dir.join` | `is_valid_dashboard_id` — slug charset only (`[A-Za-z0-9_-]`, ≤128) |
| 6 | Med | ETag omitted tenant_id, truncated sc_hash; no freshness component | ETag = content-hash + hash(tenant+sc) + TTL window |
| 7 | Med | Unbounded metering channel | Bounded (10k) `try_send`, drop-newest |
| 8 | Med | `short_label` byte-slice panics on multibyte UTF-8 | char-boundary checked |
| 9 | Med | `.lumen` TOC bounds check could overflow usize on crafted input | `checked_add` + EOF bound; section slice uses `checked_add` |
| 10 | Low | `group_thousands` panics on `i64::MIN` | `unsigned_abs()` |
| 11 | Low | NaN/Inf from numeric-string coercion reached SVG coords | `value_as_f64` rejects non-finite |

## Deferred (deliberate, tracked)

- **Single-flight on the query cache (Med):** concurrent cold requests for the same
  `(tenant, sc, query)` can stampede Cube. Acceptable for MVP; the fix is
  `moka`'s `get_with` coalescing (or a per-key lock), landing with the
  control-plane work.
- **DEPs not tenant-scoped / authz opt-in (Med):** any valid token can load any
  compiled dashboard's layout. Closing this needs a dashboard→tenant ownership
  model — a control-plane feature (see the monorepo `apps/web` + a `dashboards`
  registry). The `dashboard:{id}:read` permission hook already exists.
- **Insecure dev-default secrets + token in iframe URL (Low):** the defaults are
  dev-only; production must inject real secrets. Short-lived embed tokens in the
  iframe URL are inherent to iframe embedding (mitigated by short TTL + `private`
  caching). The Web Component / SDK roadmap moves to `postMessage`.
- **Failed semantic load → empty widget, no audit event (Low):** intentional
  graceful degradation; a warn is logged. An `error` meter kind can be added later.

## Dismissed by verifiers (2)

Ambiguous cache-key delimiter (keys are structurally minted, collision-safe in
practice) and "header content_hash non-deterministic" (it is deterministic given
canonical section bytes; the manifest also carries a source-stable hash).

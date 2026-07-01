//! `lumen-runtime` — the Axum embedding server.
//!
//! `GET /embed/dashboard/{id}` does NO compilation: authenticate → resolve
//! tenant principal → load the compiled DEP (cache A) → execute each distinct
//! query (cache B, tenant+sc scoped) → bind results into the compiled layout →
//! return HTML (ETag, `Cache-Control: private`) → emit meter events async.
//!
//! Security invariant: every query/HTML cache key includes `tenant_id` AND
//! `sc_hash`, and the security context is always forwarded to the semantic layer.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use lumen_artifact::Dep;
use lumen_auth::{mint_token, verify, AuthConfig, Principal, TokenInput};
use lumen_renderer::{render_dashboard, render_fragment, RenderContext};
use lumen_semantic::{Provider, SemanticConfig, SemanticLayer};
use lumen_shared::{
    meter::{self, MeterEvent, MeterKind},
    Data, DashboardDef, QueryId,
};
use moka::future::Cache;
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tokio::sync::mpsc::{Receiver, Sender};
use uuid::Uuid;

const RUNTIME_JS: &str = include_str!("../assets/runtime.js");
const ELEMENT_JS: &str =
    include_str!("../../../packages/embed-sdk/src/analytics-dashboard.js");

/// Server configuration, populated from the environment.
pub struct Config {
    pub bind_addr: String,
    pub build_dir: PathBuf,
    /// Neutral semantic-layer config (Cube or dbt SL), selected by
    /// `SEMANTIC_PROVIDER`.
    pub semantic: SemanticConfig,
    pub jwt_secret: String,
    pub database_url: Option<String>,
    pub query_ttl_secs: u64,
    pub echarts_cdn: String,
    /// Shared secret for the Lumen control plane's own server (never a design
    /// partner's own key) — lets it mint preview tokens on behalf of whichever
    /// organization owns the dashboard being previewed. See `resolve_account`.
    pub internal_api_key: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        Config {
            bind_addr: env("LUMEN_BIND", "0.0.0.0:8080"),
            build_dir: PathBuf::from(env("LUMEN_BUILD_DIR", ".lumen-build")),
            semantic: SemanticConfig {
                // Defaults to Cube when SEMANTIC_PROVIDER is unset (back-compat).
                provider: Some(Provider::parse(&env("SEMANTIC_PROVIDER", "cube"))),
                cube_url: env("CUBE_URL", "http://localhost:4000"),
                cube_secret: env("CUBEJS_API_SECRET", "lumen-dev-cube-secret"),
                dbt_graphql_url: env(
                    "DBT_SL_GRAPHQL_URL",
                    "https://semantic-layer.cloud.getdbt.com/api/graphql",
                ),
                dbt_service_token: env("DBT_SL_SERVICE_TOKEN", ""),
                dbt_environment_id: env("DBT_SL_ENVIRONMENT_ID", ""),
            },
            jwt_secret: env("LUMEN_JWT_SECRET", "dev-only-insecure-secret-change-me"),
            database_url: std::env::var("DATABASE_URL").ok(),
            query_ttl_secs: env("LUMEN_QUERY_TTL", "60").parse().unwrap_or(60),
            echarts_cdn: env(
                "LUMEN_ECHARTS_CDN",
                "https://cdn.jsdelivr.net/npm/echarts@6.1.0/dist/echarts.min.js",
            ),
            internal_api_key: std::env::var("LUMEN_INTERNAL_API_KEY").ok(),
        }
    }
}

const DEV_JWT_SECRET: &str = "dev-only-insecure-secret-change-me";
const DEV_CUBE_SECRET: &str = "lumen-dev-cube-secret";
const DEV_INTERNAL_API_KEY: &str = "dev-only-internal-key-change-me";

/// Warn (always) or hard-fail (`LUMEN_ENV=production`) when a secret is still
/// at its known-insecure dev default. Unset secrets already warn at their own
/// call sites (`LUMEN_INTERNAL_API_KEY` in `serve`) — this closes the
/// different gap where a secret that IS set, but left at the placeholder
/// value from mise.toml/docs, was silently accepted.
fn check_secrets(cfg: &Config) -> anyhow::Result<()> {
    let mut insecure = Vec::new();
    if cfg.jwt_secret == DEV_JWT_SECRET {
        insecure.push("LUMEN_JWT_SECRET");
    }
    if cfg.semantic.cube_secret == DEV_CUBE_SECRET {
        insecure.push("CUBEJS_API_SECRET");
    }
    if cfg.internal_api_key.as_deref() == Some(DEV_INTERNAL_API_KEY) {
        insecure.push("LUMEN_INTERNAL_API_KEY");
    }
    if insecure.is_empty() {
        return Ok(());
    }
    let is_production = std::env::var("LUMEN_ENV").as_deref() == Ok("production");
    if is_production {
        anyhow::bail!(
            "refusing to start with insecure default secret(s) in production (LUMEN_ENV=production): {}",
            insecure.join(", ")
        );
    }
    tracing::warn!(
        secrets = insecure.join(", ").as_str(),
        "using known-insecure default secret(s) — fine for local dev, must be overridden before any real deployment"
    );
    Ok(())
}

#[derive(Clone)]
struct AppState {
    /// Env-derived fallback, built once at startup. Used when no `data_sources`
    /// row is active (or the control-plane DB is unreachable) — keeps the
    /// runtime working unconfigured, same as before this table existed.
    default_semantic: Arc<dyn SemanticLayer>,
    /// The active `data_sources` row's adapter, short-TTL cached so a change
    /// from the control plane's Data Sources settings page takes effect
    /// without restarting the runtime. `None` once expired forces a re-read.
    semantic_cache: Cache<u8, Arc<dyn SemanticLayer>>,
    /// Shared with metering — also backs the dynamic data-source lookup.
    pool: Option<PgPool>,
    auth: AuthConfig,
    dep_cache: Cache<String, Arc<Dep>>,
    query_cache: Cache<String, Data>,
    meter_tx: Sender<MeterEvent>,
    cfg: Arc<Config>,
}

/// Build the app, connect optional metering storage, and serve until shutdown.
pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    check_secrets(&cfg)?;
    let cfg = Arc::new(cfg);

    let default_semantic: Arc<dyn SemanticLayer> = cfg.semantic.build();
    let semantic_cache: Cache<u8, Arc<dyn SemanticLayer>> = Cache::builder()
        .max_capacity(1)
        .time_to_live(Duration::from_secs(5))
        .build();
    let auth = AuthConfig::new(cfg.jwt_secret.clone().into_bytes());

    // Short TTL so a recompiled dashboard (same id, new content) is picked up
    // within the window rather than served from the old Arc forever.
    let dep_cache: Cache<String, Arc<Dep>> = Cache::builder()
        .max_capacity(256)
        .time_to_live(Duration::from_secs(10))
        .build();
    let query_cache: Cache<String, Data> = Cache::builder()
        .time_to_live(Duration::from_secs(cfg.query_ttl_secs))
        .max_capacity(10_000)
        .build();

    // Metering: bounded non-blocking channel → background writer. Bounded so a
    // stalled writer/Postgres can't grow memory without limit (drop-newest).
    let (meter_tx, meter_rx) = tokio::sync::mpsc::channel::<MeterEvent>(10_000);
    let pool = connect_metering(&cfg).await;
    tokio::spawn(meter_writer(meter_rx, pool.clone()));

    if cfg.internal_api_key.is_none() {
        tracing::warn!(
            "LUMEN_INTERNAL_API_KEY not set — the control plane cannot mint preview tokens \
             (fine for a runtime-only deployment; required if apps/web talks to this runtime)"
        );
    }

    let state = AppState {
        default_semantic,
        semantic_cache,
        pool,
        auth,
        dep_cache,
        query_cache,
        meter_tx,
        cfg: cfg.clone(),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/healthz", get(|| async { "ok" }))
        .route("/dev/token", get(dev_token))
        .route("/tokens", post(tokens))
        .route("/_lumen/runtime.js", get(runtime_js))
        .route("/_lumen/analytics-dashboard.js", get(element_js))
        .route("/embed/dashboard/{id}", get(embed))
        .route("/meta", get(meta))
        .route("/compile", post(compile_dashboard))
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .layer(tower_http::cors::CorsLayer::very_permissive())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await?;
    tracing::info!(addr = %cfg.bind_addr, build_dir = %cfg.build_dir.display(), "lumen runtime listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn connect_metering(cfg: &Config) -> Option<PgPool> {
    let url = cfg.database_url.as_ref()?;
    match PgPoolOptions::new()
        .max_connections(5)
        .acquire_timeout(Duration::from_secs(5))
        .connect(url)
        .await
    {
        Ok(pool) => {
            if let Err(e) = ensure_schema(&pool).await {
                tracing::warn!(error = %e, "metering schema init failed");
            }
            if let Err(e) = ensure_data_sources_schema(&pool).await {
                tracing::warn!(error = %e, "data_sources schema init failed");
            }
            if let Err(e) = ensure_dashboards_schema(&pool).await {
                tracing::warn!(error = %e, "dashboards schema init failed");
            }
            tracing::info!("metering: connected to Postgres");
            Some(pool)
        }
        Err(e) => {
            tracing::warn!(error = %e, "metering disabled: cannot reach Postgres");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct EmbedQuery {
    token: Option<String>,
    /// `fragment` → Shadow-DOM-mountable HTML (for `<analytics-dashboard>`);
    /// anything else → a full standalone page (for iframe / direct nav).
    format: Option<String>,
}

async fn embed(
    State(st): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<EmbedQuery>,
) -> Result<Response, AppError> {
    let started = Instant::now();

    // 1-2. Authenticate (fail-closed).
    let token = extract_token(&headers, q.token.as_deref())
        .ok_or_else(|| AppError::Unauthorized("missing token".into()))?;
    let principal =
        verify(&token, &st.auth).map_err(|e| AppError::Unauthorized(format!("invalid token: {e}")))?;

    // Reject ids that could escape the build directory (path traversal).
    if !is_valid_dashboard_id(&id) {
        return Err(AppError::NotFound("invalid dashboard id".into()));
    }

    // 2.5. Tenant-scoping: a dashboard is only loadable by a token minted for
    // the account that owns it. Fails closed — no registered owner (or an
    // unreachable DB) denies, it never falls through to "allow". This is the
    // fix for the gap flagged in docs/REVIEW-FINDINGS.md ("any valid token
    // can load any compiled dashboard's layout").
    match lookup_dashboard_account(&st, &id).await {
        Some(owner) if owner == principal.account_id => {}
        _ => {
            // Same response whether the dashboard doesn't exist or belongs to
            // a different account — don't let this endpoint be used to probe
            // which dashboard ids exist in other accounts.
            return Err(AppError::NotFound(format!("no compiled dashboard '{id}'")));
        }
    }

    // 3. Load the compiled DEP (cache A).
    let dep = load_dep(&st, &id).await?;
    let manifest = dep.manifest().map_err(internal)?;
    let layout = dep.layout().map_err(internal)?;
    let queries = dep.queries().map_err(internal)?;

    // 4. Authorize (lenient for MVP: enforce only when the token carries scopes).
    let needed = format!("dashboard:{id}:read");
    if !principal.permissions.is_empty() && !principal.has_permission(&needed) {
        return Err(AppError::Forbidden(format!("missing permission `{needed}`")));
    }

    let request_id = Uuid::new_v4().to_string();
    let fragment = q.format.as_deref() == Some("fragment");

    // ETag = DEP content hash + tenant/sc scope + freshness window (bounded by the
    // query-cache TTL so revalidating clients see refreshed data each window).
    // Computed BEFORE query execution: a 304 skips both querying and rendering.
    let etag = make_etag(&dep, &principal, st.cfg.query_ttl_secs, fragment);
    if header_matches(&headers, header::IF_NONE_MATCH, &etag) {
        emit_all(
            &st,
            vec![meter_event(
                &principal, &id, &request_id, MeterKind::DashboardRender,
                None, None, None, meter::RENDER_FLOOR as i64,
            )],
        );
        let mut resp = (StatusCode::NOT_MODIFIED, "").into_response();
        if let Ok(v) = HeaderValue::from_str(&etag) {
            resp.headers_mut().insert(header::ETAG, v);
        }
        return Ok(resp);
    }

    // 5. Execute each DISTINCT query once, tenant+sc scoped (cache B). Per-query
    // compute credits are billed ONLY on the semantic_compute event (no double-count).
    let mut results: HashMap<QueryId, Data> = HashMap::with_capacity(queries.queries.len());
    let mut events: Vec<MeterEvent> = Vec::new();
    let mut compute_credits: u32 = 0;
    let semantic = active_semantic(&st).await;

    for (qid, query) in &queries.queries {
        // INVARIANT: tenant_id + sc_hash are mandatory key segments.
        let key = format!("q:{}:{}:{}", principal.tenant_id, principal.sc_hash, qid.as_str());

        if let Some(data) = st.query_cache.get(&key).await {
            results.insert(qid.clone(), data);
            events.push(meter_event(
                &principal, &id, &request_id, MeterKind::CacheHit,
                Some(qid.as_str()), None, None, 0,
            ));
        } else {
            let t = Instant::now();
            match semantic.load(query, &principal.security_context).await {
                Ok(data) => {
                    let exec_ms = t.elapsed().as_millis() as i64;
                    let bytes = serde_json::to_vec(&data.rows).map(|v| v.len()).unwrap_or(0) as i64;
                    st.query_cache.insert(key, data.clone()).await;
                    results.insert(qid.clone(), data);

                    let qc = meter::query_credits(exec_ms as u64);
                    compute_credits += qc;
                    events.push(meter_event(
                        &principal, &id, &request_id, MeterKind::SemanticCompute,
                        Some(qid.as_str()), Some(exec_ms), Some(bytes), qc as i64,
                    ));
                    events.push(meter_event(
                        &principal, &id, &request_id, MeterKind::CacheMiss,
                        Some(qid.as_str()), None, None, 0,
                    ));
                }
                Err(e) => {
                    tracing::warn!(error = %e, query = %qid, "semantic load failed; empty widget");
                    results.insert(qid.clone(), Data::default());
                }
            }
        }
    }

    // 6. Render.
    let ctx = RenderContext {
        runtime_url: "/_lumen/runtime.js".into(),
        echarts_cdn: st.cfg.echarts_cdn.clone(),
        footer: format!(
            "Lumen · DEP {} · compiled {}",
            short_hash(manifest.content_hash.as_str()),
            manifest.created_at.format("%Y-%m-%d %H:%M UTC")
        ),
    };
    let html = if fragment {
        render_fragment(&dep, &results, &ctx).map_err(internal)?
    } else {
        render_dashboard(&dep, &results, &ctx).map_err(internal)?
    };
    let bytes_out = html.len() as i64;

    // Render-attributed credits = Σ widget_base + egress. Summing the credits of
    // ALL events (these + the per-query compute events) yields the correct total.
    let mut render_credits: u32 = layout.widgets.iter().map(|w| meter::widget_base(w.kind)).sum();
    render_credits += (bytes_out as u64).div_ceil(meter::BYTES_PER_CREDIT) as u32;

    events.push(meter_event(
        &principal, &id, &request_id, MeterKind::DashboardRender,
        None, None, Some(bytes_out), render_credits as i64,
    ));
    emit_all(&st, events);

    tracing::info!(
        dashboard = %id, tenant = %principal.tenant_id,
        credits = render_credits + compute_credits,
        total_ms = started.elapsed().as_millis() as u64, "rendered"
    );

    Ok(html_response(
        StatusCode::OK,
        &etag,
        &format!("private, max-age={}", st.cfg.query_ttl_secs),
        html,
    ))
}

async fn load_dep(st: &AppState, id: &str) -> Result<Arc<Dep>, AppError> {
    if let Some(d) = st.dep_cache.get(id).await {
        return Ok(d);
    }
    let path = st.cfg.build_dir.join(format!("{id}.lumen"));
    let bytes = tokio::fs::read(&path).await.map_err(|_| {
        AppError::NotFound(format!(
            "no compiled dashboard '{id}' (expected {})",
            path.display()
        ))
    })?;
    let dep = Dep::from_bytes(bytes).map_err(|e| AppError::Internal(format!("corrupt DEP: {e}")))?;
    let arc = Arc::new(dep);
    st.dep_cache.insert(id.to_string(), arc.clone()).await;
    Ok(arc)
}

/// Raw data-model metadata (Cube cubes+measures+dimensions, or dbt's metrics
/// listing) from the active semantic layer — feeds the builder's field picker.
/// Unauthenticated for now (schema shape, not data; matches `/dev/token`'s
/// open dev posture). Gating behind the same api_key check as `/tokens` is a
/// fast-follow.
async fn meta(State(st): State<AppState>) -> Result<Json<serde_json::Value>, AppError> {
    let semantic = active_semantic(&st).await;
    let val = semantic.meta().await.map_err(internal)?;
    Ok(Json(val))
}

/// Compile a `DashboardDef` and write it to `build_dir`, same as `lumen
/// compile` from the CLI — but over HTTP, for the control-plane builder's
/// "Save & Compile" action. The id is REQUIRED (the caller — the web app —
/// always has a stable dashboard id; unlike the CLI there's no source
/// filename to derive one from).
async fn compile_dashboard(
    State(st): State<AppState>,
    Json(def): Json<DashboardDef>,
) -> Result<Json<serde_json::Value>, AppError> {
    let id = def
        .id
        .clone()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("dashboard `id` is required".into()))?;
    if !is_valid_dashboard_id(&id) {
        return Err(AppError::BadRequest(format!("invalid dashboard id `{id}`")));
    }

    let bytes = lumen_compiler::compile(&def).map_err(|e| match &e {
        lumen_compiler::CompileError::MissingField(..) => AppError::BadRequest(e.to_string()),
        _ => internal(e),
    })?;

    let out = st.cfg.build_dir.join(format!("{id}.lumen"));
    if let Some(parent) = out.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(internal)?;
    }
    tokio::fs::write(&out, &bytes).await.map_err(internal)?;
    // Without this, a save can serve the previous compiled DEP for up to the
    // dep_cache's 10s TTL.
    st.dep_cache.invalidate(&id).await;

    let dep = Dep::from_bytes(bytes).map_err(internal)?;
    let m = dep.manifest().map_err(internal)?;
    Ok(Json(json!({
        "id": id,
        "title": m.title,
        "widgets": m.widgets.len(),
        "queries": m.queries.len(),
        "credits": m.required_credits.estimate,
        "content_hash": m.content_hash.0,
        "renderers": m.renderers,
    })))
}

async fn dev_token(State(st): State<AppState>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        demo_token(&st),
    )
}

#[derive(serde::Deserialize)]
struct TokenRequest {
    #[serde(default)]
    dashboard: Option<String>,
    /// Which account this token is for — resolved server-side by
    /// `resolve_account` for a real API key; only trusted verbatim as a
    /// client-supplied value in the internal-key / no-accounts-configured
    /// paths. See [`resolve_account`].
    #[serde(default)]
    account_id: Option<String>,
    tenant_id: String,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    permissions: Vec<String>,
    /// Forwarded verbatim to Cube for row-level security.
    #[serde(default)]
    security_context: serde_json::Value,
    #[serde(default)]
    ttl_secs: Option<i64>,
}

/// Server-side scoped-token minting (the Embeddable model): your backend calls
/// this with an API key and gets a short-lived JWT scoped to a tenant/user/RLS
/// context. The browser never sees the signing secret.
async fn tokens(
    State(st): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<TokenRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let account_id = resolve_account(&st, &headers, req.account_id.clone()).await?;

    let ttl = req.ttl_secs.unwrap_or(3600).clamp(60, 86_400);
    let sc = if req.security_context.is_null() {
        json!({ "tenant_id": req.tenant_id })
    } else {
        req.security_context
    };
    let token = mint_token(
        &st.auth,
        &TokenInput {
            account_id,
            tenant_id: req.tenant_id,
            sub: req.user.unwrap_or_else(|| "embed".into()),
            roles: req.roles,
            permissions: req.permissions,
            security_context: sc,
            ttl_secs: ttl,
        },
    )
    .map_err(|e| AppError::Internal(format!("token mint failed: {e}")))?;

    Ok(Json(json!({
        "token": token,
        "expires_in": ttl,
        "dashboard": req.dashboard,
    })))
}

async fn runtime_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        RUNTIME_JS,
    )
}

async fn element_js() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        ELEMENT_JS,
    )
}

async fn index(State(st): State<AppState>) -> Html<String> {
    let token = demo_token(&st);
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Lumen — embedded analytics runtime</title>
<style>
  body{{margin:0;font:14px/1.5 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Arial;background:#0b0e14;color:#e6e6e6}}
  .wrap{{max-width:1200px;margin:0 auto;padding:24px}}
  h1{{font-size:20px;margin:0 0 4px}} .muted{{color:#9aa4b2}}
  code{{background:#141925;padding:2px 6px;border-radius:6px}}
  iframe{{width:100%;height:760px;border:1px solid #222a39;border-radius:12px;background:#fff;margin-top:16px}}
</style></head><body><div class="wrap">
  <h1>Lumen</h1>
  <div class="muted">Compiled, embeddable dashboards on top of your semantic layer. Below is a live
  <code>&lt;iframe&gt;</code> embed of <code>/embed/dashboard/sales</code>, authenticated with a freshly
  minted dev JWT.</div>
  <iframe src="/embed/dashboard/sales?token={token}" title="Sales dashboard"></iframe>
  <p class="muted">Token endpoint: <code>GET /dev/token</code> · Runtime: <code>GET /_lumen/runtime.js</code></p>
</div></body></html>"#
    ))
}

fn demo_token(st: &AppState) -> String {
    mint_token(
        &st.auth,
        &TokenInput {
            // Matches the "default" account the demo dashboards (sales) are
            // registered under — see docs/BUSINESS-PLAN.md §5.1 backfill.
            account_id: "default".into(),
            tenant_id: "demo".into(),
            sub: "dev-user".into(),
            roles: vec!["viewer".into()],
            permissions: vec![], // empty → authz lenient for the demo
            security_context: json!({ "tenant_id": "demo" }),
            ttl_secs: 3600,
        },
    )
    .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Metering
// ---------------------------------------------------------------------------

async fn ensure_schema(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS meter_events (
            event_id     TEXT        PRIMARY KEY,
            occurred_at  TIMESTAMPTZ NOT NULL,
            tenant_id    TEXT        NOT NULL,
            user_sub     TEXT        NOT NULL,
            dashboard_id TEXT        NOT NULL,
            request_id   TEXT        NOT NULL,
            kind         TEXT        NOT NULL,
            query_hash   TEXT,
            exec_time_ms BIGINT,
            bytes        BIGINT,
            credits      BIGINT      NOT NULL DEFAULT 0,
            sc_hash      TEXT        NOT NULL,
            created_at   TIMESTAMPTZ NOT NULL DEFAULT now()
        )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS meter_events_tenant_time ON meter_events (tenant_id, occurred_at)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Data sources — the control plane's "Data Sources" settings page reads and
// writes this same table directly (it shares DATABASE_URL); the runtime only
// reads it. Schema is intentionally duplicated (CREATE TABLE IF NOT EXISTS) in
// apps/web/src/lib/data-sources.ts — keep the two in sync if either changes.
// ---------------------------------------------------------------------------

async fn ensure_data_sources_schema(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS data_sources (
            id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
            name       TEXT        NOT NULL,
            provider   TEXT        NOT NULL CHECK (provider IN ('cube','dbt')),
            config     JSONB       NOT NULL,
            is_active  BOOLEAN     NOT NULL DEFAULT false,
            created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
        )"#,
    )
    .execute(pool)
    .await?;
    // At most one active row, enforced at the DB level (not just app logic).
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS data_sources_one_active \
         ON data_sources ((is_active)) WHERE is_active",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Resolve the `SemanticLayer` to use for this request: the active
/// `data_sources` row if one is configured and the control-plane DB is
/// reachable, else the env-derived default (`AppState::default_semantic`).
/// Cached for `semantic_cache`'s TTL so a typical request doesn't round-trip
/// to Postgres just to find out nothing changed.
async fn active_semantic(st: &AppState) -> Arc<dyn SemanticLayer> {
    if let Some(layer) = st.semantic_cache.get(&0u8).await {
        return layer;
    }
    let layer = resolve_active_semantic(st)
        .await
        .unwrap_or_else(|| st.default_semantic.clone());
    st.semantic_cache.insert(0u8, layer.clone()).await;
    layer
}

async fn resolve_active_semantic(st: &AppState) -> Option<Arc<dyn SemanticLayer>> {
    let pool = st.pool.as_ref()?;
    let row: (String, serde_json::Value) = sqlx::query_as(
        "SELECT provider, config FROM data_sources WHERE is_active LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| tracing::warn!(error = %e, "data_sources lookup failed"))
    .ok()??;
    let (provider, config) = row;
    Some(semantic_config_from_row(&provider, &config).build())
}

fn semantic_config_from_row(provider: &str, config: &serde_json::Value) -> SemanticConfig {
    let get = |k: &str| {
        config
            .get(k)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    SemanticConfig {
        provider: Some(Provider::parse(provider)),
        cube_url: get("cube_url"),
        cube_secret: get("cube_secret"),
        dbt_graphql_url: get("dbt_graphql_url"),
        dbt_service_token: get("dbt_service_token"),
        dbt_environment_id: get("dbt_environment_id"),
    }
}

// ---------------------------------------------------------------------------
// Dashboards / accounts — the control plane (apps/web) owns and writes these
// tables (Drizzle-managed there; see apps/web/src/lib/db/schema.ts and
// lib/organizations.ts). The runtime only READS them, over the same shared
// Postgres — it has no ORM, so this is plain sqlx. `organization` is a
// Better-Auth-managed table (camelCase, double-quoted identifiers); keep this
// query in sync if the control plane's org schema changes.
//
// Tenant-scoping invariant: a dashboard is only ever loadable by a token whose
// `account_id` matches the dashboard's `organization_id` — see `embed`'s
// ownership check. Fails closed: no row, no pool, or a mismatch are all
// treated identically (reject), never treated as "allow".
// ---------------------------------------------------------------------------

async fn ensure_dashboards_schema(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS dashboards (
            id              TEXT        PRIMARY KEY,
            organization_id TEXT        NOT NULL DEFAULT '',
            title           TEXT        NOT NULL,
            theme           TEXT        NOT NULL DEFAULT 'light',
            definition      JSONB       NOT NULL,
            compiled_at     TIMESTAMPTZ,
            content_hash    TEXT,
            created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
            updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
        )"#,
    )
    .execute(pool)
    .await?;
    sqlx::query("ALTER TABLE dashboards ADD COLUMN IF NOT EXISTS organization_id TEXT NOT NULL DEFAULT ''")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS dashboards_organization_id ON dashboards (organization_id)")
        .execute(pool)
        .await?;
    Ok(())
}

/// Which account (organization) owns dashboard `id` — `None` if the dashboard
/// has no row at all (never compiled through the control plane, e.g. a raw
/// CLI-authored fixture) or the DB is unreachable. Both are treated as "not
/// accessible" by the caller, not "allow".
async fn lookup_dashboard_account(st: &AppState, id: &str) -> Option<String> {
    let pool = st.pool.as_ref()?;
    sqlx::query_scalar::<_, String>("SELECT organization_id FROM dashboards WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| tracing::warn!(error = %e, "dashboard ownership lookup failed"))
        .ok()?
}

/// Resolve which account is calling `POST /tokens`.
///
/// Two callers, two trust levels:
/// - A design partner's OWN backend, presenting their org's runtime API key
///   (`X-Api-Key` / `Bearer`) — resolved by hashing the presented key and
///   matching it against `organization.runtimeApiKeyHash`. The account is
///   ALWAYS the one that owns the matched key; any client-supplied
///   `account_id` in the request body is ignored (can't be spoofed).
/// - The Lumen control plane itself, previewing a dashboard on behalf of
///   whichever organization owns it — presents `LUMEN_INTERNAL_API_KEY`
///   instead, and its client-supplied `account_id` IS trusted (the control
///   plane already enforced dashboard ownership at its own DB layer before
///   ever calling here — see apps/web/src/lib/dashboards.ts).
///
/// Dev-mode fallback: if no organization has a key configured yet AND no key
/// was presented, trust the client-supplied `account_id` (default
/// `"default"`) — matches this codebase's existing "open until configured"
/// posture for other dev-only gates.
async fn resolve_account(
    st: &AppState,
    headers: &HeaderMap,
    requested: Option<String>,
) -> Result<String, AppError> {
    let provided_key = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .or_else(|| {
            headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .map(|v| v.strip_prefix("Bearer ").unwrap_or(v))
        });

    if let Some(key) = provided_key {
        if let Some(internal) = &st.cfg.internal_api_key {
            if key == internal {
                return Ok(requested.unwrap_or_else(|| "default".into()));
            }
        }
        let Some(pool) = &st.pool else {
            return Err(AppError::Internal("account resolution unavailable".into()));
        };
        // SHA-256, matching apps/web/src/lib/organizations.ts::generateApiKey
        // exactly (Node's `crypto.createHash('sha256')`) — the two sides must
        // agree on the algorithm since the web app is where the hash is
        // written and this is where it's compared.
        use sha2::{Digest, Sha256};
        let hash = hex::encode(Sha256::digest(key.as_bytes()));
        let account_id: Option<String> =
            sqlx::query_scalar(r#"SELECT id FROM "organization" WHERE "runtimeApiKeyHash" = $1"#)
                .bind(&hash)
                .fetch_optional(pool)
                .await
                .map_err(internal)?;
        return account_id.ok_or_else(|| AppError::Unauthorized("invalid API key".into()));
    }

    // No key presented at all — only acceptable while no account has a key
    // configured yet (fresh/dev install).
    let Some(pool) = &st.pool else {
        return Ok(requested.unwrap_or_else(|| "default".into()));
    };
    let any_keys_configured: Option<i32> =
        sqlx::query_scalar(r#"SELECT 1 FROM "organization" WHERE "runtimeApiKeyHash" IS NOT NULL LIMIT 1"#)
            .fetch_optional(pool)
            .await
            .map_err(internal)?;
    if any_keys_configured.is_some() {
        return Err(AppError::Unauthorized("API key required".into()));
    }
    tracing::warn!("no organizations have a runtime API key yet — trusting client-supplied account_id (dev only)");
    Ok(requested.unwrap_or_else(|| "default".into()))
}

async fn meter_writer(mut rx: Receiver<MeterEvent>, pool: Option<PgPool>) {
    while let Some(e) = rx.recv().await {
        match &pool {
            Some(p) => {
                let res = sqlx::query(
                    "INSERT INTO meter_events \
                     (event_id,occurred_at,tenant_id,user_sub,dashboard_id,request_id,kind,query_hash,exec_time_ms,bytes,credits,sc_hash) \
                     VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) ON CONFLICT (event_id) DO NOTHING",
                )
                .bind(e.event_id.as_str())
                .bind(e.occurred_at)
                .bind(e.tenant_id.as_str())
                .bind(e.user_sub.as_str())
                .bind(e.dashboard_id.as_str())
                .bind(e.request_id.as_str())
                .bind(e.kind.as_str())
                .bind(e.query_hash.as_deref())
                .bind(e.exec_time_ms)
                .bind(e.bytes)
                .bind(e.credits)
                .bind(e.sc_hash.as_str())
                .execute(p)
                .await;
                if let Err(err) = res {
                    tracing::warn!(error = %err, "meter insert failed");
                }
            }
            None => {
                tracing::debug!(kind = e.kind.as_str(), credits = e.credits, "meter (no db)");
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn meter_event(
    p: &Principal,
    dashboard: &str,
    request_id: &str,
    kind: MeterKind,
    query_hash: Option<&str>,
    exec_ms: Option<i64>,
    bytes: Option<i64>,
    credits: i64,
) -> MeterEvent {
    MeterEvent {
        event_id: Uuid::new_v4().to_string(),
        occurred_at: chrono::Utc::now(),
        tenant_id: p.tenant_id.clone(),
        user_sub: p.sub.clone(),
        dashboard_id: dashboard.to_string(),
        request_id: request_id.to_string(),
        kind,
        query_hash: query_hash.map(str::to_string),
        exec_time_ms: exec_ms,
        bytes,
        credits,
        sc_hash: p.sc_hash.clone(),
    }
}

fn emit_all(st: &AppState, events: Vec<MeterEvent>) {
    for e in events {
        // try_send is non-blocking; drop-newest if the bounded channel is full/closed.
        let _ = st.meter_tx.try_send(e);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn extract_token(headers: &HeaderMap, query_token: Option<&str>) -> Option<String> {
    if let Some(auth) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        return Some(
            auth.strip_prefix("Bearer ")
                .unwrap_or(auth)
                .trim()
                .to_string(),
        );
    }
    query_token.map(str::to_string)
}

fn header_matches(headers: &HeaderMap, name: header::HeaderName, value: &str) -> bool {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == value)
        .unwrap_or(false)
}

/// Dashboard ids are slugs. Rejecting everything else closes the path-traversal
/// vector in `build_dir.join(format!("{id}.lumen"))`.
fn is_valid_dashboard_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// ETag = DEP content hash (version) + a hash of tenant_id+sc_hash (RLS scope) +
/// a freshness window keyed to the query-cache TTL. The scope segment prevents
/// cross-tenant ETag collisions; the window makes revalidating clients refetch as
/// underlying data rolls over (bounded by one TTL).
fn make_etag(dep: &Dep, p: &Principal, ttl_secs: u64, fragment: bool) -> String {
    use std::hash::{Hash, Hasher};
    let ch = dep.content_hash_hex();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    p.tenant_id.hash(&mut h);
    p.sc_hash.hash(&mut h);
    let scope = h.finish();
    let window = chrono::Utc::now().timestamp() / (ttl_secs.max(1) as i64);
    let mode = if fragment { "f" } else { "p" }; // fragment vs full page differ
    format!("\"lumen:{mode}:{}-{:x}-{}\"", &ch[..16], scope, window)
}

fn short_hash(h: &str) -> String {
    h.strip_prefix("blake3:")
        .unwrap_or(h)
        .chars()
        .take(12)
        .collect()
}

fn html_response(status: StatusCode, etag: &str, cache: &str, body: String) -> Response {
    let mut resp = Response::new(axum::body::Body::from(body));
    *resp.status_mut() = status;
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    if let Ok(v) = HeaderValue::from_str(etag) {
        h.insert(header::ETAG, v);
    }
    if let Ok(v) = HeaderValue::from_str(cache) {
        h.insert(header::CACHE_CONTROL, v);
    }
    // Defense-in-depth against injected markup: no inline/eval scripts; only
    // self + the ECharts CDN. The JSON data islands are non-executable.
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp
}

const CSP: &str = "default-src 'self'; \
script-src 'self' https://cdn.jsdelivr.net; \
style-src 'self' 'unsafe-inline'; \
img-src 'self' data:; \
connect-src 'self'; \
frame-ancestors *; \
base-uri 'none'; \
object-src 'none'";

enum AppError {
    Unauthorized(String),
    Forbidden(String),
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

fn internal<E: std::fmt::Display>(e: E) -> AppError {
    AppError::Internal(e.to_string())
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            AppError::Unauthorized(m) => (StatusCode::UNAUTHORIZED, m),
            AppError::Forbidden(m) => (StatusCode::FORBIDDEN, m),
            AppError::NotFound(m) => (StatusCode::NOT_FOUND, m),
            AppError::BadRequest(m) => (StatusCode::BAD_REQUEST, m),
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, msg).into_response()
    }
}

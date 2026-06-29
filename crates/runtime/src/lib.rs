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
use lumen_semantic::{CubeClient, SemanticLayer};
use lumen_shared::{
    meter::{self, MeterEvent, MeterKind},
    Data, QueryId,
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
    pub cube_url: String,
    pub cube_secret: String,
    pub jwt_secret: String,
    pub database_url: Option<String>,
    pub query_ttl_secs: u64,
    pub echarts_cdn: String,
    /// If set, `POST /tokens` requires this key (X-Api-Key / Bearer). Unset = open (dev).
    pub api_key: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        Config {
            bind_addr: env("LUMEN_BIND", "0.0.0.0:8080"),
            build_dir: PathBuf::from(env("LUMEN_BUILD_DIR", ".lumen-build")),
            cube_url: env("CUBE_URL", "http://localhost:4000"),
            cube_secret: env("CUBEJS_API_SECRET", "lumen-dev-cube-secret"),
            jwt_secret: env("LUMEN_JWT_SECRET", "dev-only-insecure-secret-change-me"),
            database_url: std::env::var("DATABASE_URL").ok(),
            query_ttl_secs: env("LUMEN_QUERY_TTL", "60").parse().unwrap_or(60),
            echarts_cdn: env(
                "LUMEN_ECHARTS_CDN",
                "https://cdn.jsdelivr.net/npm/echarts@6.1.0/dist/echarts.min.js",
            ),
            api_key: std::env::var("LUMEN_API_KEY").ok(),
        }
    }
}

#[derive(Clone)]
struct AppState {
    semantic: Arc<dyn SemanticLayer>,
    auth: AuthConfig,
    dep_cache: Cache<String, Arc<Dep>>,
    query_cache: Cache<String, Data>,
    meter_tx: Sender<MeterEvent>,
    cfg: Arc<Config>,
}

/// Build the app, connect optional metering storage, and serve until shutdown.
pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    let cfg = Arc::new(cfg);

    let semantic: Arc<dyn SemanticLayer> =
        Arc::new(CubeClient::new(cfg.cube_url.clone(), cfg.cube_secret.clone()));
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
    tokio::spawn(meter_writer(meter_rx, pool));

    if cfg.api_key.is_none() {
        tracing::warn!("LUMEN_API_KEY not set — POST /tokens is unauthenticated (dev only)");
    }

    let state = AppState {
        semantic,
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
            match st.semantic.load(query, &principal.security_context).await {
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
    if let Some(expected) = &st.cfg.api_key {
        let provided = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .or_else(|| {
                headers
                    .get(header::AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.strip_prefix("Bearer ").unwrap_or(v))
            });
        if provided != Some(expected.as_str()) {
            return Err(AppError::Unauthorized("invalid or missing API key".into()));
        }
    }

    let ttl = req.ttl_secs.unwrap_or(3600).clamp(60, 86_400);
    let sc = if req.security_context.is_null() {
        json!({ "tenant_id": req.tenant_id })
    } else {
        req.security_context
    };
    let token = mint_token(
        &st.auth,
        &TokenInput {
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
            AppError::Internal(m) => (StatusCode::INTERNAL_SERVER_ERROR, m),
        };
        (status, msg).into_response()
    }
}

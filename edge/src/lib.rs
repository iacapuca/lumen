//! lumen-edge — the Cloudflare Workers (wasm32) entrypoint.
//!
//! It REUSES the proven, I/O-free compute core (compiler/renderer/artifact/shared
//! — all verified to compile to `wasm32-unknown-unknown`) and swaps only the I/O
//! adapters for Cloudflare bindings:
//!
//!   * DEP store ........ filesystem → **R2** (content-addressed `.lumen`)
//!   * query cache ...... moka       → **Workers KV** (tenant+sc scoped, TTL)
//!   * Cube client ...... reqwest    → **Fetch** binding
//!   * metering ......... sqlx/PG    → **Durable Object** per-tenant credit counter
//!   * JWT (HS256) ...... jsonwebtoken(aws_lc) → pure-Rust **hmac+sha2** (wasm-safe)
//!
//! This is a deploy-time skeleton: build/run it with `npx wrangler dev|deploy`
//! (which drives worker-build), not bare `cargo build`. Some `worker` API calls
//! are version-sensitive (pinned to worker 0.5 here).

use std::collections::HashMap;

use base64::Engine;
use lumen_artifact::Dep;
use lumen_renderer::{render_dashboard, render_fragment, RenderContext};
use lumen_shared::{Data, Granularity, Query, QueryId, SecurityContext, SortDir};
use serde_json::{json, Map, Value};
use worker::*;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

#[event(fetch)]
async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();
    Router::new()
        .get_async("/healthz", |_, _| async { Response::ok("ok") })
        .post_async("/tokens", tokens)
        .get_async("/embed/dashboard/:id", embed)
        .run(req, env)
        .await
}

// ---------------------------------------------------------------------------
// POST /tokens — server-side scoped token minting (the Embeddable model)
// ---------------------------------------------------------------------------

async fn tokens(mut req: Request, ctx: RouteContext<()>) -> Result<Response> {
    if let Ok(expected) = ctx.secret("LUMEN_API_KEY").map(|s| s.to_string()) {
        let provided = req
            .headers()
            .get("x-api-key")
            .ok()
            .flatten()
            .or_else(|| bearer(&req));
        if provided.as_deref() != Some(expected.as_str()) {
            return Response::error("invalid or missing API key", 401);
        }
    }

    let body: Value = req.json().await?;
    let tenant = body.get("tenant_id").and_then(Value::as_str).unwrap_or("");
    if tenant.is_empty() {
        return Response::error("tenant_id required", 400);
    }
    let ttl = body
        .get("ttl_secs")
        .and_then(Value::as_i64)
        .unwrap_or(3600)
        .clamp(60, 86_400);
    let sc = body
        .get("security_context")
        .cloned()
        .unwrap_or_else(|| json!({ "tenant_id": tenant }));
    let now = now_secs();
    let claims = json!({
        "sub": body.get("user").and_then(Value::as_str).unwrap_or("embed"),
        "tenant_id": tenant,
        "iat": now,
        "exp": now + ttl,
        "security_context": sc,
    });
    let secret = ctx.secret("LUMEN_JWT_SECRET")?.to_string();
    let token = mint_jwt(&claims, secret.as_bytes());
    Response::from_json(&json!({ "token": token, "expires_in": ttl }))
}

// ---------------------------------------------------------------------------
// GET /embed/dashboard/:id — the edge hydrate path
// ---------------------------------------------------------------------------

async fn embed(req: Request, ctx: RouteContext<()>) -> Result<Response> {
    let id = ctx.param("id").cloned().unwrap_or_default();
    if !valid_id(&id) {
        return Response::error("invalid dashboard id", 404);
    }

    let url = req.url()?;
    let q: HashMap<String, String> = url.query_pairs().into_owned().collect();
    let token = match q.get("token").cloned().or_else(|| bearer(&req)) {
        Some(t) => t,
        None => return Response::error("missing token", 401),
    };
    let fragment = q.get("format").map(|f| f == "fragment").unwrap_or(false);

    // 1. Auth (pure-Rust HS256 verify; fail-closed).
    let jwt_secret = ctx.secret("LUMEN_JWT_SECRET")?.to_string();
    let principal = match verify_jwt(&token, jwt_secret.as_bytes()) {
        Some(p) => p,
        None => return Response::error("invalid or expired token", 401),
    };

    // 2. Load the compiled DEP from R2 (content-addressed, immutable).
    let bucket = ctx.bucket("LUMEN_DEP")?;
    // NB: `ObjectBody<'_>` borrows the `Object`, so the object must outlive the
    // body stream — bind it first, then take its body (cannot `and_then` a moved
    // local away under it).
    let object = match bucket.get(format!("{id}.lumen")).execute().await? {
        Some(o) => o,
        None => return Response::error("no compiled dashboard", 404),
    };
    let bytes = match object.body() {
        Some(body) => body.bytes().await?,
        None => return Response::error("no compiled dashboard", 404),
    };
    let dep = Dep::from_bytes(bytes).map_err(|e| Error::RustError(e.to_string()))?;
    let queries = dep.queries().map_err(|e| Error::RustError(e.to_string()))?;

    // 3. Execute each distinct query — KV cache B (INVARIANT: tenant + sc scoped).
    let cube_url = ctx.var("CUBE_URL")?.to_string();
    let cube_secret = ctx.secret("CUBE_API_SECRET")?.to_string();
    let kv = ctx.kv("LUMEN_CACHE")?;
    let ttl = ctx
        .var("QUERY_TTL_SECS")
        .ok()
        .and_then(|v| v.to_string().parse::<u64>().ok())
        .unwrap_or(60);

    let mut results: HashMap<QueryId, Data> = HashMap::with_capacity(queries.queries.len());
    let mut compute_credits: u32 = 0;
    for (qid, query) in &queries.queries {
        let key = format!("q:{}:{}:{}", principal.tenant_id, principal.sc_hash, qid.as_str());
        if let Some(text) = kv.get(&key).text().await? {
            if let Ok(data) = serde_json::from_str::<Data>(&text) {
                results.insert(qid.clone(), data);
                continue;
            }
        }
        match cube_load(&cube_url, &cube_secret, query, &principal.security_context).await {
            Ok(data) => {
                if let Ok(serialized) = serde_json::to_string(&data) {
                    let _ = kv.put(&key, serialized)?.expiration_ttl(ttl).execute().await;
                }
                compute_credits += 2; // QUERY_BASE + ~compute (CPU-ms also metered by CF)
                results.insert(qid.clone(), data);
            }
            Err(e) => {
                console_log!("cube load failed for {qid}: {e}");
                results.insert(qid.clone(), Data::default());
            }
        }
    }

    // 4. Render — the REUSED core (identical to the native runtime).
    let rctx = RenderContext {
        runtime_url: "/_lumen/runtime.js".into(),
        echarts_cdn: "https://cdn.jsdelivr.net/npm/echarts@6.1.0/dist/echarts.min.js".into(),
        footer: format!("Lumen edge · {}", dep.content_hash_hex()[..12].to_string()),
    };
    let html = if fragment {
        render_fragment(&dep, &results, &rctx)
    } else {
        render_dashboard(&dep, &results, &rctx)
    }
    .map_err(|e| Error::RustError(e.to_string()))?;

    // 5. Meter into the per-tenant Durable Object (credits = widgets + compute).
    let layout_credits = dep.layout().map(|l| l.widgets.len() as u32 * 2).unwrap_or(0);
    let _ = meter_increment(&ctx, &principal.tenant_id, compute_credits + layout_credits).await;

    let mut headers = Headers::new();
    headers.set("content-type", "text/html; charset=utf-8")?;
    headers.set("cache-control", "private, max-age=60")?;
    Ok(Response::ok(html)?.with_headers(headers))
}

// ---------------------------------------------------------------------------
// Durable Object: per-tenant credit accumulator (transactional usage ledger)
// ---------------------------------------------------------------------------

#[durable_object]
pub struct TenantMeter {
    state: State,
}

#[durable_object]
impl DurableObject for TenantMeter {
    fn new(state: State, _env: Env) -> Self {
        Self { state }
    }

    async fn fetch(&mut self, mut req: Request) -> Result<Response> {
        let add: i64 = req.text().await?.trim().parse().unwrap_or(0);
        let current: i64 = self.state.storage().get("credits").await.unwrap_or(0);
        let total = current + add;
        self.state.storage().put("credits", total).await?;
        Response::ok(total.to_string())
    }
}

async fn meter_increment(ctx: &RouteContext<()>, tenant: &str, credits: u32) -> Result<()> {
    let ns = ctx.durable_object("TENANT_METER")?;
    let stub = ns.id_from_name(tenant)?.get_stub()?;
    let mut init = RequestInit::new();
    init.with_method(Method::Post)
        .with_body(Some(credits.to_string().into()));
    let req = Request::new_with_init("https://meter.internal/increment", &init)?;
    stub.fetch_with_request(req).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Cube client over the Fetch binding
// ---------------------------------------------------------------------------

async fn cube_load(
    cube_url: &str,
    cube_secret: &str,
    query: &Query,
    sc: &SecurityContext,
) -> Result<Data> {
    // Mint the Cube JWT (payload = security context).
    let now = now_secs();
    let mut claims: Map<String, Value> = match &sc.0 {
        Value::Object(m) => m.clone(),
        _ => Map::new(),
    };
    claims.insert("iat".into(), json!(now));
    claims.insert("exp".into(), json!(now + 300));
    let token = mint_jwt(&Value::Object(claims), cube_secret.as_bytes());

    let body = json!({ "query": to_cube_query(query) });
    let endpoint = format!("{}/cubejs-api/v1/load", cube_url.trim_end_matches('/'));

    for _ in 0..40 {
        let mut headers = Headers::new();
        headers.set("content-type", "application/json")?;
        headers.set("authorization", &token)?;
        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(serde_json::to_string(&body)?.into()));
        let request = Request::new_with_init(&endpoint, &init)?;

        let mut resp = Fetch::Request(request).send().await?;
        let val: Value = resp.json().await?;

        if val.get("error").and_then(Value::as_str) == Some("Continue wait") {
            continue; // re-send the idempotent request
        }
        let rows = val
            .get("data")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|r| r.as_object().cloned()).collect())
            .unwrap_or_default();
        return Ok(Data {
            rows,
            annotation: val.get("annotation").cloned().unwrap_or(Value::Null),
        });
    }
    Err(Error::RustError("cube: Continue wait timeout".into()))
}

/// Neutral Query IR → Cube REST query. (TODO: factor this + the response parser
/// into a wasm-safe `lumen-semantic-core` shared with the native CubeClient.)
fn to_cube_query(q: &Query) -> Value {
    let mut m = Map::new();
    if !q.measures.is_empty() {
        m.insert("measures".into(), json!(q.measures));
    }
    if !q.dimensions.is_empty() {
        m.insert("dimensions".into(), json!(q.dimensions));
    }
    if !q.time_dimensions.is_empty() {
        let tds: Vec<Value> = q
            .time_dimensions
            .iter()
            .map(|td| {
                json!({
                    "dimension": td.dimension,
                    "granularity": granularity_str(td.granularity),
                })
            })
            .collect();
        m.insert("timeDimensions".into(), json!(tds));
    }
    if !q.filters.is_empty() {
        let fs: Vec<Value> = q
            .filters
            .iter()
            .map(|f| json!({ "member": f.member, "operator": f.op, "values": f.values }))
            .collect();
        m.insert("filters".into(), json!(fs));
    }
    if !q.order.is_empty() {
        let ord: Vec<Value> = q
            .order
            .iter()
            .map(|o| {
                let dir = match o.dir {
                    SortDir::Asc => "asc",
                    SortDir::Desc => "desc",
                };
                json!([o.field, dir])
            })
            .collect();
        m.insert("order".into(), json!(ord));
    }
    if let Some(l) = q.limit {
        m.insert("limit".into(), json!(l));
    }
    Value::Object(m)
}

fn granularity_str(g: Granularity) -> &'static str {
    g.as_str()
}

// ---------------------------------------------------------------------------
// Auth — pure-Rust HS256 (mint + verify). No jsonwebtoken/aws_lc on wasm.
// ---------------------------------------------------------------------------

struct EdgePrincipal {
    tenant_id: String,
    #[allow(dead_code)]
    sub: String,
    security_context: SecurityContext,
    sc_hash: String,
}

fn mint_jwt(claims: &Value, secret: &[u8]) -> String {
    let header = b64url(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload = b64url(&serde_json::to_vec(claims).unwrap_or_default());
    let signing_input = format!("{header}.{payload}");
    let sig = b64url(&hs256(signing_input.as_bytes(), secret));
    format!("{signing_input}.{sig}")
}

fn verify_jwt(token: &str, secret: &[u8]) -> Option<EdgePrincipal> {
    let mut parts = token.split('.');
    let (h, p, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let expected = b64url(&hs256(format!("{h}.{p}").as_bytes(), secret));
    if !ct_eq(expected.as_bytes(), s.as_bytes()) {
        return None;
    }
    let claims: Value = serde_json::from_slice(&b64url_decode(p)?).ok()?;
    // exp check (fail-closed).
    let exp = claims.get("exp").and_then(Value::as_i64)?;
    if exp < now_secs() {
        return None;
    }
    let tenant_id = claims.get("tenant_id")?.as_str()?.to_string();
    let sub = claims
        .get("sub")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let sc = SecurityContext(claims.get("security_context").cloned().unwrap_or(json!({})));
    let sc_hash = sc.sc_hash();
    Some(EdgePrincipal {
        tenant_id,
        sub,
        security_context: sc,
        sc_hash,
    })
}

fn hs256(msg: &[u8], secret: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(msg);
    mac.finalize().into_bytes().to_vec()
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
fn b64url_decode(s: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).ok()
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

fn bearer(req: &Request) -> Option<String> {
    req.headers()
        .get("authorization")
        .ok()
        .flatten()
        .map(|a| a.strip_prefix("Bearer ").unwrap_or(&a).to_string())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn now_secs() -> i64 {
    (Date::now().as_millis() / 1000) as i64
}

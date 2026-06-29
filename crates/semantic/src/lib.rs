//! `lumen-semantic` — the [`SemanticLayer`] seam and the Cube.dev adapter.
//!
//! Lumen never owns metrics or modeling; it delegates to a semantic layer. The
//! neutral [`Query`] IR is translated at execution time, so nothing
//! Cube-specific is ever frozen into a compiled DEP. Swapping in dbt Semantic
//! Layer / MetricFlow / a custom REST API is a new `impl SemanticLayer`.

use std::time::Duration;

use async_trait::async_trait;
use lumen_shared::{Data, Query, SecurityContext};
use serde_json::{json, Map, Value};

#[derive(Debug, thiserror::Error)]
pub enum SemanticError {
    #[error("transport error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("token signing error: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    #[error("semantic layer error: {0}")]
    Backend(String),
    #[error("query timed out after {0} retries (Continue wait)")]
    Timeout(u32),
}

/// The swappable semantic-layer seam. `load` *requires* a [`SecurityContext`] so
/// "forgot to forward RLS" is a compile error, not a runtime bug.
#[async_trait]
pub trait SemanticLayer: Send + Sync {
    async fn load(&self, query: &Query, sc: &SecurityContext) -> Result<Data, SemanticError>;
    /// Raw `/meta` data-model metadata (cubes + views).
    async fn meta(&self) -> Result<Value, SemanticError>;
}

/// Cube.dev (Cube Core OSS) adapter. Talks the REST `/load` + `/meta` API.
pub struct CubeClient {
    http: reqwest::Client,
    base_url: String,
    /// `CUBEJS_API_SECRET` — used to sign the per-request Cube JWT whose payload
    /// *is* the security context.
    api_secret: String,
    max_continue_wait_retries: u32,
}

impl CubeClient {
    pub fn new(base_url: impl Into<String>, api_secret: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest client builds");
        CubeClient {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_secret: api_secret.into(),
            max_continue_wait_retries: 40,
        }
    }

    /// Mint a short-lived HS256 JWT whose payload carries the security context.
    /// Cube interprets the entire JWT payload as `securityContext` and uses it
    /// for row-level security via `queryRewrite` / `COMPILE_CONTEXT`.
    fn mint_token(&self, sc: &SecurityContext) -> Result<String, jsonwebtoken::errors::Error> {
        use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};

        let mut claims: Map<String, Value> = match &sc.0 {
            Value::Object(m) => m.clone(),
            Value::Null => Map::new(),
            other => {
                let mut m = Map::new();
                m.insert("securityContext".into(), other.clone());
                m
            }
        };
        let now = chrono::Utc::now().timestamp();
        claims.insert("iat".into(), json!(now));
        claims.insert("exp".into(), json!(now + 300));

        encode(
            &Header::new(Algorithm::HS256),
            &Value::Object(claims),
            &EncodingKey::from_secret(self.api_secret.as_bytes()),
        )
    }

    async fn post_load(&self, token: &str, body: &Value) -> Result<Value, SemanticError> {
        let mut retries = 0u32;
        loop {
            let resp = self
                .http
                .post(format!("{}/cubejs-api/v1/load", self.base_url))
                .header("Authorization", token)
                .json(body)
                .send()
                .await?;
            let status = resp.status();
            let val: Value = resp.json().await?;

            // Cube returns HTTP 200 with {"error":"Continue wait"} while the
            // query is still computing. Re-send the identical (idempotent) request.
            if let Some(err) = val.get("error").and_then(Value::as_str) {
                if err == "Continue wait" {
                    retries += 1;
                    if retries > self.max_continue_wait_retries {
                        return Err(SemanticError::Timeout(retries));
                    }
                    tracing::debug!(retries, "cube: continue wait, retrying");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    continue;
                }
                return Err(SemanticError::Backend(err.to_string()));
            }
            if !status.is_success() {
                return Err(SemanticError::Backend(format!("cube HTTP {status}")));
            }
            return Ok(val);
        }
    }
}

#[async_trait]
impl SemanticLayer for CubeClient {
    async fn load(&self, query: &Query, sc: &SecurityContext) -> Result<Data, SemanticError> {
        let token = self.mint_token(sc)?;
        let body = json!({ "query": to_cube_query(query) });
        let val = self.post_load(&token, &body).await?;

        let rows = val
            .get("data")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|r| r.as_object().cloned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let annotation = val.get("annotation").cloned().unwrap_or(Value::Null);
        Ok(Data { rows, annotation })
    }

    async fn meta(&self) -> Result<Value, SemanticError> {
        let token = self.mint_token(&SecurityContext::empty())?;
        let val: Value = self
            .http
            .get(format!("{}/cubejs-api/v1/meta", self.base_url))
            .header("Authorization", token)
            .send()
            .await?
            .json()
            .await?;
        Ok(val)
    }
}

/// Translate the neutral [`Query`] IR into a Cube REST query object.
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
                let mut o = Map::new();
                o.insert("dimension".into(), json!(td.dimension));
                o.insert("granularity".into(), json!(td.granularity.as_str()));
                if let Some(dr) = &td.date_range {
                    let v = match dr {
                        lumen_shared::DateRange::Relative(s) => json!(s),
                        lumen_shared::DateRange::Absolute(a) => json!(a),
                    };
                    o.insert("dateRange".into(), v);
                }
                Value::Object(o)
            })
            .collect();
        m.insert("timeDimensions".into(), json!(tds));
    }
    if !q.filters.is_empty() {
        let fs: Vec<Value> = q
            .filters
            .iter()
            // FilterOp serializes (camelCase) to Cube's operator strings directly.
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
                    lumen_shared::SortDir::Asc => "asc",
                    lumen_shared::SortDir::Desc => "desc",
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

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_shared::{Granularity, TimeDimension};

    #[test]
    fn translates_time_dimension_to_cube_shape() {
        let q = Query {
            measures: vec!["orders.revenue".into()],
            time_dimensions: vec![TimeDimension {
                dimension: "orders.created_at".into(),
                granularity: Granularity::Month,
                date_range: None,
            }],
            ..Default::default()
        };
        let v = to_cube_query(&q);
        assert_eq!(v["measures"][0], "orders.revenue");
        assert_eq!(v["timeDimensions"][0]["dimension"], "orders.created_at");
        assert_eq!(v["timeDimensions"][0]["granularity"], "month");
    }

    #[test]
    fn translates_filters_with_cube_operators() {
        use lumen_shared::{Filter, FilterOp};
        let q = Query {
            filters: vec![Filter {
                member: "orders.status".into(),
                op: FilterOp::NotEquals,
                values: vec!["F".into()],
            }],
            ..Default::default()
        };
        let v = to_cube_query(&q);
        assert_eq!(v["filters"][0]["operator"], "notEquals");
        assert_eq!(v["filters"][0]["member"], "orders.status");
    }
}

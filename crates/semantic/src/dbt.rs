//! dbt Semantic Layer (MetricFlow) adapter — talks the GraphQL API.
//!
//! SCAFFOLD: the pure request-building + RLS translation below is complete and
//! unit-tested. The live GraphQL round-trip in [`DbtClient::load`] / `meta` is
//! wired but UNVERIFIED against a real dbt Cloud instance — the result-envelope
//! parsing (marked `TODO(dbt-live)`) may need adjusting once tested end-to-end.
//!
//! ## Why this looks different from the Cube adapter
//!
//! Cube enforces row-level security server-side: the signed-JWT payload *is* the
//! security context. dbt SL has **no equivalent** — auth is a static service
//! token, so RLS is *our* responsibility. We translate the [`SecurityContext`]
//! into explicit MetricFlow `where` filters ([`sc_to_where`]) appended to every
//! query. Get this wrong and tenants see each other's rows, so [`sc_to_where`]
//! **fails closed**: a non-empty context it cannot map is an error, never a
//! silently-dropped filter.

use std::time::Duration;

use async_trait::async_trait;
use lumen_shared::{Data, DateRange, FilterOp, Query, SecurityContext, SortDir};
use serde_json::{json, Map, Value};

use crate::{SemanticError, SemanticLayer};

/// dbt Semantic Layer adapter. Talks the GraphQL `createQuery` → poll → result
/// API at `https://{host}/api/graphql`.
pub struct DbtClient {
    http: reqwest::Client,
    graphql_url: String,
    /// dbt Cloud service token (sent as `Authorization: Bearer …`).
    service_token: String,
    /// dbt Cloud environment id the metrics are defined in.
    environment_id: String,
    max_poll_retries: u32,
}

impl DbtClient {
    pub fn new(
        graphql_url: impl Into<String>,
        service_token: impl Into<String>,
        environment_id: impl Into<String>,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest client builds");
        DbtClient {
            http,
            graphql_url: graphql_url.into().trim_end_matches('/').to_string(),
            service_token: service_token.into(),
            environment_id: environment_id.into(),
            max_poll_retries: 40,
        }
    }

    async fn graphql(&self, query: &str, variables: Value) -> Result<Value, SemanticError> {
        let resp = self
            .http
            .post(&self.graphql_url)
            .bearer_auth(&self.service_token)
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(SemanticError::Backend(format!("dbt HTTP {}", resp.status())));
        }
        let val: Value = resp.json().await?;
        if let Some(errs) = val.get("errors").and_then(Value::as_array) {
            if !errs.is_empty() {
                return Err(SemanticError::Backend(format!("dbt GraphQL: {errs:?}")));
            }
        }
        Ok(val)
    }
}

const CREATE_QUERY: &str = r#"
mutation CreateQuery($environmentId: BigInt!, $metrics: [MetricInput!]!, $groupBy: [GroupByInput!], $where: [WhereInput!], $orderBy: [OrderByInput!], $limit: Int) {
  createQuery(environmentId: $environmentId, metrics: $metrics, groupBy: $groupBy, where: $where, orderBy: $orderBy, limit: $limit) {
    queryId
  }
}"#;

const POLL_QUERY: &str = r#"
query GetResults($environmentId: BigInt!, $queryId: String!) {
  query(environmentId: $environmentId, queryId: $queryId) {
    status
    error
    jsonResult(encoded: false)
  }
}"#;

#[async_trait]
impl SemanticLayer for DbtClient {
    async fn load(&self, query: &Query, sc: &SecurityContext) -> Result<Data, SemanticError> {
        let mut vars = to_dbt_variables(query, sc)?;
        vars.insert("environmentId".into(), json!(self.environment_id));

        let created = self.graphql(CREATE_QUERY, Value::Object(vars)).await?;
        let query_id = created
            .pointer("/data/createQuery/queryId")
            .and_then(Value::as_str)
            .ok_or_else(|| SemanticError::Backend("dbt: no queryId returned".into()))?
            .to_string();

        // Poll until the async query completes — same shape as Cube's "Continue wait".
        let mut retries = 0u32;
        let result = loop {
            let polled = self
                .graphql(
                    POLL_QUERY,
                    json!({ "environmentId": self.environment_id, "queryId": query_id }),
                )
                .await?;
            let node = polled.pointer("/data/query").cloned().unwrap_or(Value::Null);
            match node.get("status").and_then(Value::as_str) {
                Some("SUCCESSFUL") => break node,
                Some("FAILED") => {
                    let err = node.get("error").and_then(Value::as_str).unwrap_or("unknown");
                    return Err(SemanticError::Backend(format!("dbt query failed: {err}")));
                }
                _ => {
                    retries += 1;
                    if retries > self.max_poll_retries {
                        return Err(SemanticError::Timeout(retries));
                    }
                    tracing::debug!(retries, "dbt: query pending, polling");
                    tokio::time::sleep(Duration::from_millis(250)).await;
                }
            }
        };

        // TODO(dbt-live): verify the result envelope against a live instance.
        // `jsonResult(encoded: false)` is expected to be a JSON string of an
        // array of row objects; adjust here once confirmed end-to-end.
        let rows = parse_json_result(result.get("jsonResult"));
        Ok(Data {
            rows,
            // dbt has no Cube-style `annotation` block; the renderer treats this
            // as optional. TODO(dbt-live): synthesize column titles/types from
            // the dbt result schema so the renderer reaches parity with Cube.
            annotation: Value::Null,
        })
    }

    async fn meta(&self) -> Result<Value, SemanticError> {
        // dbt exposes the data model via GraphQL introspection of `metrics` /
        // `dimensions`, not a single `/meta` document. Nothing in a compiled DEP
        // depends on this shape (the compiler never calls `meta()`), so a thin
        // metrics listing is enough for now.
        let q = r#"
query Meta($environmentId: BigInt!) {
  metrics(environmentId: $environmentId) { name description type }
}"#;
        let val = self
            .graphql(q, json!({ "environmentId": self.environment_id }))
            .await?;
        Ok(val.pointer("/data").cloned().unwrap_or(Value::Null))
    }
}

/// Build the GraphQL `createQuery` variables (minus `environmentId`) from the
/// neutral [`Query`] IR, injecting RLS `where` filters from `sc`.
fn to_dbt_variables(q: &Query, sc: &SecurityContext) -> Result<Map<String, Value>, SemanticError> {
    let mut vars = Map::new();

    // metrics: Lumen `measures` are dbt metric names.
    vars.insert(
        "metrics".into(),
        json!(q.measures.iter().map(|m| json!({ "name": m })).collect::<Vec<_>>()),
    );

    // groupBy: plain dimensions + time dimensions (the latter carry a grain).
    let mut group_by: Vec<Value> = q.dimensions.iter().map(|d| json!({ "name": d })).collect();
    for td in &q.time_dimensions {
        group_by.push(json!({
            "name": td.dimension,
            // dbt grains are uppercase enum values (DAY, MONTH, …).
            "grain": td.granularity.as_str().to_uppercase(),
        }));
    }
    if !group_by.is_empty() {
        vars.insert("groupBy".into(), json!(group_by));
    }

    // where: user filters + date-range filters + injected RLS filters.
    let mut wheres: Vec<String> = Vec::new();
    for f in &q.filters {
        wheres.push(filter_to_sql(&f.member, f.op, &f.values));
    }
    for td in &q.time_dimensions {
        if let Some(DateRange::Absolute([from, to])) = &td.date_range {
            wheres.push(format!(
                "{} BETWEEN {} AND {}",
                dimension_ref(&td.dimension),
                quote(from),
                quote(to),
            ));
        }
        // NB: Relative ranges ("last 12 months") have no direct MetricFlow `where`
        // equivalent — TODO(dbt-live): map via a grain-aware relative window.
    }
    wheres.extend(sc_to_where(sc)?);
    if !wheres.is_empty() {
        vars.insert(
            "where".into(),
            json!(wheres.into_iter().map(|sql| json!({ "sql": sql })).collect::<Vec<_>>()),
        );
    }

    if !q.order.is_empty() {
        let order: Vec<Value> = q
            .order
            .iter()
            .map(|o| {
                let descending = matches!(o.dir, SortDir::Desc);
                // A field that is a requested metric sorts as a metric; otherwise
                // as a groupBy. We approximate by membership in `measures`.
                if q.measures.iter().any(|m| m == &o.field) {
                    json!({ "metric": { "name": o.field }, "descending": descending })
                } else {
                    json!({ "groupBy": { "name": o.field }, "descending": descending })
                }
            })
            .collect();
        vars.insert("orderBy".into(), json!(order));
    }

    if let Some(l) = q.limit {
        vars.insert("limit".into(), json!(l));
    }

    Ok(vars)
}

/// Translate a [`SecurityContext`] into MetricFlow `where` clause strings.
///
/// Convention (TODO(dbt-live): make this mapping configurable per deployment):
/// each top-level key `k: v` in the context object becomes
/// `{{ Dimension('k') }} = 'v'`. Fails closed — a non-empty context that is not
/// a flat object of scalars is rejected rather than silently ignored, because a
/// dropped RLS filter is a cross-tenant data leak.
pub fn sc_to_where(sc: &SecurityContext) -> Result<Vec<String>, SemanticError> {
    match &sc.0 {
        Value::Null => Ok(Vec::new()),
        Value::Object(m) if m.is_empty() => Ok(Vec::new()),
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| match v {
                Value::String(s) => Ok(format!("{} = {}", dimension_ref(k), quote(s))),
                Value::Number(n) => Ok(format!("{} = {}", dimension_ref(k), n)),
                Value::Bool(b) => Ok(format!("{} = {}", dimension_ref(k), b)),
                _ => Err(SemanticError::Backend(format!(
                    "security context key '{k}' is not a scalar; cannot build a safe RLS filter \
                     (refusing to drop it)"
                ))),
            })
            .collect(),
        other => Err(SemanticError::Backend(format!(
            "security context must be an object for dbt RLS, got: {other}"
        ))),
    }
}

/// Render one neutral [`Filter`](lumen_shared::Filter) as a MetricFlow `where`
/// SQL fragment.
fn filter_to_sql(member: &str, op: FilterOp, values: &[String]) -> String {
    let d = dimension_ref(member);
    let first = values.first().map(|s| s.as_str()).unwrap_or("");
    match op {
        FilterOp::Equals if values.len() > 1 => format!("{d} IN ({})", quote_list(values)),
        FilterOp::Equals => format!("{d} = {}", quote(first)),
        FilterOp::NotEquals if values.len() > 1 => format!("{d} NOT IN ({})", quote_list(values)),
        FilterOp::NotEquals => format!("{d} != {}", quote(first)),
        FilterOp::Gt | FilterOp::AfterDate => format!("{d} > {}", quote(first)),
        FilterOp::Gte => format!("{d} >= {}", quote(first)),
        FilterOp::Lt | FilterOp::BeforeDate => format!("{d} < {}", quote(first)),
        FilterOp::Lte => format!("{d} <= {}", quote(first)),
        FilterOp::Contains => format!("{d} LIKE {}", quote(&format!("%{first}%"))),
        FilterOp::NotContains => format!("{d} NOT LIKE {}", quote(&format!("%{first}%"))),
        FilterOp::Set => format!("{d} IS NOT NULL"),
        FilterOp::NotSet => format!("{d} IS NULL"),
        FilterOp::InDateRange if values.len() >= 2 => {
            format!("{d} BETWEEN {} AND {}", quote(&values[0]), quote(&values[1]))
        }
        FilterOp::InDateRange => format!("{d} = {}", quote(first)),
    }
}

/// A MetricFlow dimension reference: `{{ Dimension('member') }}`.
fn dimension_ref(member: &str) -> String {
    format!("{{{{ Dimension('{}') }}}}", member.replace('\'', ""))
}

/// Single-quote a SQL string literal, escaping embedded quotes by doubling.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

fn quote_list(values: &[String]) -> String {
    values.iter().map(|v| quote(v)).collect::<Vec<_>>().join(", ")
}

/// Parse dbt's `jsonResult` (a JSON string of row objects) into Lumen rows.
fn parse_json_result(v: Option<&Value>) -> Vec<lumen_shared::Row> {
    let parsed: Value = match v {
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
        Some(other) => other.clone(),
        None => Value::Null,
    };
    parsed
        .as_array()
        .map(|arr| arr.iter().filter_map(|r| r.as_object().cloned()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_shared::{Granularity, Order, TimeDimension};

    #[test]
    fn metrics_and_groupby_grain_uppercased() {
        let q = Query {
            measures: vec!["revenue".into()],
            time_dimensions: vec![TimeDimension {
                dimension: "metric_time".into(),
                granularity: Granularity::Month,
                date_range: None,
            }],
            ..Default::default()
        };
        let v = to_dbt_variables(&q, &SecurityContext::empty()).unwrap();
        assert_eq!(v["metrics"][0]["name"], "revenue");
        assert_eq!(v["groupBy"][0]["name"], "metric_time");
        assert_eq!(v["groupBy"][0]["grain"], "MONTH");
    }

    #[test]
    fn filter_operators_map_to_sql() {
        assert_eq!(
            filter_to_sql("orders.status", FilterOp::NotEquals, &["F".into()]),
            "{{ Dimension('orders.status') }} != 'F'"
        );
        assert_eq!(
            filter_to_sql("c.region", FilterOp::Equals, &["EMEA".into(), "APAC".into()]),
            "{{ Dimension('c.region') }} IN ('EMEA', 'APAC')"
        );
        assert_eq!(
            filter_to_sql("c.name", FilterOp::Contains, &["ax".into()]),
            "{{ Dimension('c.name') }} LIKE '%ax%'"
        );
        assert_eq!(
            filter_to_sql("c.deleted", FilterOp::NotSet, &[]),
            "{{ Dimension('c.deleted') }} IS NULL"
        );
    }

    #[test]
    fn sc_injects_rls_where_filters() {
        let sc = SecurityContext(json!({ "tenant_id": "acme", "tier": 3 }));
        let mut clauses = sc_to_where(&sc).unwrap();
        clauses.sort();
        assert_eq!(
            clauses,
            vec![
                "{{ Dimension('tenant_id') }} = 'acme'".to_string(),
                "{{ Dimension('tier') }} = 3".to_string(),
            ]
        );
    }

    #[test]
    fn sc_fails_closed_on_unmappable_context() {
        // A nested object can't be turned into a single equality — must error,
        // never silently drop (that would leak across tenants).
        let sc = SecurityContext(json!({ "scope": { "nested": true } }));
        assert!(sc_to_where(&sc).is_err());
    }

    #[test]
    fn empty_context_yields_no_filters() {
        assert!(sc_to_where(&SecurityContext::empty()).unwrap().is_empty());
        assert!(sc_to_where(&SecurityContext(Value::Null)).unwrap().is_empty());
    }

    #[test]
    fn quotes_escape_embedded_single_quotes() {
        assert_eq!(quote("O'Brien"), "'O''Brien'");
    }

    #[test]
    fn order_routes_metric_vs_groupby() {
        let q = Query {
            measures: vec!["revenue".into()],
            dimensions: vec!["region".into()],
            order: vec![
                Order { field: "revenue".into(), dir: SortDir::Desc },
                Order { field: "region".into(), dir: SortDir::Asc },
            ],
            ..Default::default()
        };
        let v = to_dbt_variables(&q, &SecurityContext::empty()).unwrap();
        assert_eq!(v["orderBy"][0]["metric"]["name"], "revenue");
        assert_eq!(v["orderBy"][0]["descending"], true);
        assert_eq!(v["orderBy"][1]["groupBy"]["name"], "region");
        assert_eq!(v["orderBy"][1]["descending"], false);
    }
}

//! `lumen-shared` — the pinned contract every other Lumen crate compiles against.
//!
//! It holds:
//!   * the **input** model: [`DashboardDef`] (the dashboard JSON authors write);
//!   * the **neutral query IR**: [`Query`] (semantic-layer agnostic) and [`Data`] (results);
//!   * the **compiled** model: the Dashboard Execution Plan sections
//!     ([`Manifest`], [`Layout`], [`Queries`], [`Charts`]);
//!   * cross-cutting types: ids, [`SecurityContext`], metering ([`MeterEvent`]), [`LumenError`].
//!
//! Nothing here is Cube-specific — the Cube adapter translates [`Query`] at execution time.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod ids;
pub mod meter;

pub use ids::{ChartId, DashboardId, Hash, QueryId, WidgetId};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum LumenError {
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("invalid dashboard definition: {0}")]
    InvalidDashboard(String),
    #[error("unknown widget type: {0}")]
    UnknownWidget(String),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, LumenError>;

// ---------------------------------------------------------------------------
// 1. Input model — the dashboard JSON authors write
// ---------------------------------------------------------------------------

/// A dashboard definition, as authored (e.g. `dashboards/sales.json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardDef {
    /// Stable id; defaults to the file stem if omitted.
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    pub layout: Vec<WidgetDef>,
}

fn default_theme() -> String {
    "light".into()
}

/// One widget in the authored layout. Flat & permissive — the compiler validates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetDef {
    #[serde(rename = "type")]
    pub kind: WidgetKind,
    #[serde(default)]
    pub title: Option<String>,
    /// KPI: the single measure to show.
    #[serde(default)]
    pub measure: Option<String>,
    /// Charts: x dimension (often a time dimension).
    #[serde(default)]
    pub x: Option<String>,
    /// Charts: y measure.
    #[serde(default)]
    pub y: Option<String>,
    /// Time grouping for the x axis when x is a time dimension.
    #[serde(default)]
    pub granularity: Option<Granularity>,
    /// Value formatting hint (`currency`, `number`, `percent`).
    #[serde(default)]
    pub format: Option<ValueFormat>,
    /// Explicit grid placement (builder-authored). Absent ⇒ compiler auto-flow.
    #[serde(default)]
    pub pos: Option<GridPos>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    Kpi,
    LineChart,
    BarChart,
    Table,
}

impl WidgetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WidgetKind::Kpi => "kpi",
            WidgetKind::LineChart => "line_chart",
            WidgetKind::BarChart => "bar_chart",
            WidgetKind::Table => "table",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ValueFormat {
    Currency,
    Number,
    Percent,
}

// ---------------------------------------------------------------------------
// 2. Neutral query IR + results
// ---------------------------------------------------------------------------

/// Semantic-layer-agnostic query. Its canonical JSON encoding is the dedup key
/// (see [`QueryId::of`]). `skip_serializing_if` keeps empty fields out so that
/// "absent" and "empty" hash identically.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Query {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub measures: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dimensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub time_dimensions: Vec<TimeDimension>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<Filter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<Order>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeDimension {
    pub dimension: String,
    pub granularity: Granularity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_range: Option<DateRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DateRange {
    /// Relative, e.g. `"last 12 months"`.
    Relative(String),
    /// Inclusive `[from, to]` ISO range.
    Absolute([String; 2]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Granularity {
    Second,
    Minute,
    Hour,
    Day,
    Week,
    Month,
    Quarter,
    Year,
}

impl Granularity {
    pub fn as_str(self) -> &'static str {
        match self {
            Granularity::Second => "second",
            Granularity::Minute => "minute",
            Granularity::Hour => "hour",
            Granularity::Day => "day",
            Granularity::Week => "week",
            Granularity::Month => "month",
            Granularity::Quarter => "quarter",
            Granularity::Year => "year",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Filter {
    pub member: String,
    pub op: FilterOp,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterOp {
    Equals,
    NotEquals,
    Gt,
    Gte,
    Lt,
    Lte,
    Contains,
    NotContains,
    Set,
    NotSet,
    InDateRange,
    BeforeDate,
    AfterDate,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Order {
    pub field: String,
    pub dir: SortDir,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDir {
    Asc,
    Desc,
}

/// A row keyed by full member name (`"orders.revenue"` → value). Mirrors a Cube
/// `/load` data row; numeric values may arrive as JSON strings.
pub type Row = serde_json::Map<String, serde_json::Value>;

/// Result of executing a [`Query`] against a semantic layer.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Data {
    pub rows: Vec<Row>,
    /// Raw annotation map from the semantic layer (titles, types). Optional.
    #[serde(default)]
    pub annotation: serde_json::Value,
}

impl Data {
    /// Read a member from the first row as f64 (parsing string-encoded numbers).
    pub fn scalar(&self, member: &str) -> Option<f64> {
        self.rows.first().and_then(|r| value_as_f64(r.get(member)?))
    }
}

/// Coerce a Cube value (number or numeric string) into a *finite* f64. Non-finite
/// values (NaN/±Inf) are rejected so they can't corrupt SVG coordinates or credits.
pub fn value_as_f64(v: &serde_json::Value) -> Option<f64> {
    let n = match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    };
    n.filter(|x| x.is_finite())
}

// ---------------------------------------------------------------------------
// 3. Compiled model — the Dashboard Execution Plan (DEP) sections
// ---------------------------------------------------------------------------

/// Section names used inside the `.lumen` container.
pub mod section {
    pub const MANIFEST: &str = "manifest";
    pub const LAYOUT: &str = "layout";
    pub const QUERIES: &str = "queries";
    pub const CHARTS: &str = "charts";
    pub const THEME: &str = "theme";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: String, // "lumen.dep.manifest/1"
    pub dep_id: DashboardId,
    pub content_hash: Hash,
    pub source_hash: Hash,
    pub title: String,
    pub theme: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub compiler: CompilerInfo,
    pub format: FormatInfo,
    pub widgets: Vec<WidgetId>,
    pub queries: Vec<QueryId>,
    pub renderers: Vec<Renderer>,
    pub required_credits: CreditEstimate,
    pub runtime_asset: RuntimeAsset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompilerInfo {
    pub name: String,
    pub version: String,
    pub semantic_adapter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatInfo {
    pub container_ver: u16,
    pub min_runtime: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditEstimate {
    pub model: String,
    pub estimate: u32,
    pub queries: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAsset {
    pub url: String,
    pub integrity: Hash,
    pub embedded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Renderer {
    Svg,
    Echarts,
    Html,
}

// --- layout section ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Layout {
    pub schema: String, // "lumen.dep.layout/1"
    pub grid: Grid,
    pub widgets: Vec<WidgetPlacement>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Grid {
    pub cols: u16,
    pub row_height: u16,
    pub gap: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WidgetPlacement {
    pub id: WidgetId,
    pub kind: WidgetKind,
    pub title: String,
    pub pos: GridPos,
    pub query: QueryId,
    pub chart: ChartId,
    pub binding: Binding,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GridPos {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

/// How a query's result columns map onto a chart's channels. Deliberately
/// separate from the chart spec so one query can feed many widgets.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Binding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<ValueFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<String>,
}

// --- queries section ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Queries {
    pub schema: String, // "lumen.dep.queries/1"
    pub queries: BTreeMap<QueryId, Query>,
}

// --- charts section ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Charts {
    pub schema: String, // "lumen.dep.charts/1"
    pub specs: BTreeMap<ChartId, ChartSpec>,
}

/// Precompiled, renderer-tagged render instructions. The `renderer` tag is the
/// whole renderer-agnosticism story: the runtime `match`es and dispatches.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "renderer", rename_all = "lowercase")]
pub enum ChartSpec {
    /// Server-rendered SVG. Zero client JS for this widget.
    Svg {
        kind: ChartKind,
        encoding: Encoding,
        geometry: Geometry,
        palette: Vec<String>,
        theme_ref: String,
    },
    /// Client hydrates via runtime.js from a precompiled ECharts `option` skeleton.
    Echarts {
        kind: ChartKind,
        option_template: serde_json::Value,
        theme_ref: String,
    },
    /// Pure template binding (KPIs, tables).
    Html {
        kind: ChartKind,
        template: String,
        #[serde(default)]
        number_format: Option<ValueFormat>,
        theme_ref: String,
    },
}

impl ChartSpec {
    pub fn renderer(&self) -> Renderer {
        match self {
            ChartSpec::Svg { .. } => Renderer::Svg,
            ChartSpec::Echarts { .. } => Renderer::Echarts,
            ChartSpec::Html { .. } => Renderer::Html,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChartKind {
    Kpi,
    Line,
    Bar,
    Table,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Encoding {
    pub x: AxisSpec,
    pub y: AxisSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AxisSpec {
    pub scale: String, // "time" | "linear" | "band"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default)]
    pub grid: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub margin: [u32; 4], // top, right, bottom, left
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curve: Option<String>,
}

// ---------------------------------------------------------------------------
// 4. Security context (tenant / RLS) — forwarded verbatim to the semantic layer
// ---------------------------------------------------------------------------

/// Opaque-to-Lumen security context. Meaningful to the semantic layer (Cube),
/// which uses it for row-level security. Lumen only forwards and hashes it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(transparent)]
pub struct SecurityContext(pub serde_json::Value);

impl SecurityContext {
    pub fn empty() -> Self {
        SecurityContext(serde_json::json!({}))
    }

    /// Stable blake3 over canonical JSON. Two logically-equal contexts coalesce;
    /// differing ones never collide. Used as a mandatory cache-key segment.
    pub fn sc_hash(&self) -> String {
        // serde_json::Map is a BTreeMap by default → sorted keys → canonical.
        let bytes = serde_json::to_vec(&self.0).unwrap_or_default();
        hex::encode(&blake3::hash(&bytes).as_bytes()[..16])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_id_is_deterministic_and_dedupes() {
        let a = Query {
            measures: vec!["orders.revenue".into()],
            ..Default::default()
        };
        let b = Query {
            measures: vec!["orders.revenue".into()],
            ..Default::default()
        };
        let c = Query {
            measures: vec!["orders.count".into()],
            ..Default::default()
        };
        assert_eq!(QueryId::of(&a), QueryId::of(&b), "identical queries dedupe");
        assert_ne!(QueryId::of(&a), QueryId::of(&c), "distinct queries differ");
    }

    #[test]
    fn sc_hash_isolates_tenants() {
        let a = SecurityContext(serde_json::json!({"tenant_id":"acme"}));
        let b = SecurityContext(serde_json::json!({"tenant_id":"globex"}));
        assert_ne!(a.sc_hash(), b.sc_hash());
    }

    #[test]
    fn value_coercion_handles_cube_strings() {
        assert_eq!(value_as_f64(&serde_json::json!("700")), Some(700.0));
        assert_eq!(value_as_f64(&serde_json::json!(42)), Some(42.0));
    }
}

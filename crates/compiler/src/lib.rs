//! `lumen-compiler` — transforms an authored [`DashboardDef`] into a compiled
//! Dashboard Execution Plan (`.lumen`). All the expensive work happens here, once
//! per dashboard change:
//!
//!   * each widget → a neutral [`Query`], content-hashed and **deduplicated**;
//!   * the auto-flow grid → **pre-resolved absolute cell positions**;
//!   * each widget → a **renderer-tagged** [`ChartSpec`] (kpi→html, line→svg, bar→echarts);
//!   * a credit estimate keyed to *distinct queries*.
//!
//! The runtime never re-runs any of this — it just executes the listed queries
//! and binds results.

use std::collections::BTreeMap;

use lumen_artifact::DepWriter;
use lumen_shared::{
    meter, section, AxisSpec, Binding, ChartId, ChartKind, ChartSpec, Charts, CompilerInfo,
    CreditEstimate, DashboardDef, DashboardId, Encoding, FormatInfo, Geometry, Grid, Granularity,
    Hash, Layout, Manifest, Order, Query, QueryId, Renderer, SortDir, TimeDimension, ValueFormat,
    WidgetDef, WidgetId, WidgetKind, WidgetPlacement,
};

pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");
const GRID_COLS: u16 = 12;

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("widget {0}: missing required field `{1}`")]
    MissingField(usize, &'static str),
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("artifact error: {0}")]
    Artifact(#[from] lumen_artifact::ArtifactError),
}

/// A fully-resolved widget, pre-binding.
struct Compiled {
    id: WidgetId,
    kind: WidgetKind,
    title: String,
    query: Query,
    query_id: QueryId,
    chart_id: ChartId,
    chart: ChartSpec,
    binding: Binding,
    /// Builder-authored explicit grid placement. `None` ⇒ auto-flow.
    pos: Option<lumen_shared::GridPos>,
}

/// Compile a dashboard definition into `.lumen` bytes.
pub fn compile(def: &DashboardDef) -> Result<Vec<u8>, CompileError> {
    let dep_id = DashboardId(def.id.clone().unwrap_or_else(|| "dashboard".into()));
    let theme_ref = def.theme.clone();

    // 1. Resolve every widget.
    let mut compiled = Vec::with_capacity(def.layout.len());
    for (i, w) in def.layout.iter().enumerate() {
        compiled.push(resolve_widget(i, w, &theme_ref)?);
    }

    // 2. Dedup queries (structural, by content hash).
    let mut queries: BTreeMap<QueryId, Query> = BTreeMap::new();
    for c in &compiled {
        queries.entry(c.query_id.clone()).or_insert_with(|| c.query.clone());
    }

    // 3. Collect chart specs.
    let mut specs: BTreeMap<ChartId, ChartSpec> = BTreeMap::new();
    for c in &compiled {
        specs.insert(c.chart_id.clone(), c.chart.clone());
    }

    // 4. Resolve the auto-flow grid into absolute positions.
    let placements = place_widgets(&compiled);

    // 5. Assemble sections.
    let theme_css = compiled_theme(&theme_ref);

    let layout = Layout {
        schema: "lumen.dep.layout/1".into(),
        grid: Grid {
            cols: GRID_COLS,
            row_height: 80,
            gap: 16,
        },
        widgets: placements,
    };
    let queries_section = lumen_shared::Queries {
        schema: "lumen.dep.queries/1".into(),
        queries,
    };
    let charts_section = Charts {
        schema: "lumen.dep.charts/1".into(),
        specs,
    };

    // content_hash over the non-manifest sections (source-derived, deterministic).
    let layout_bytes = serde_json::to_vec(&layout)?;
    let queries_bytes = serde_json::to_vec(&queries_section)?;
    let charts_bytes = serde_json::to_vec(&charts_section)?;
    let mut hash_input =
        Vec::with_capacity(layout_bytes.len() + queries_bytes.len() + charts_bytes.len());
    hash_input.extend_from_slice(&layout_bytes);
    hash_input.extend_from_slice(&queries_bytes);
    hash_input.extend_from_slice(&charts_bytes);
    hash_input.extend_from_slice(theme_css.as_bytes());
    let content_hash = Hash::of_bytes(&hash_input);
    let source_hash = Hash::of_bytes(&serde_json::to_vec(def)?);

    // Credit estimate: layout cost + 1 per distinct (executed) query.
    let distinct_queries = queries_section.queries.len() as u32;
    let layout_credits: u32 = compiled.iter().map(|c| meter::chart_base(chart_kind(c.kind))).sum();
    let estimate = layout_credits + meter::QUERY_BASE * distinct_queries;

    let manifest = Manifest {
        schema: "lumen.dep.manifest/1".into(),
        dep_id: dep_id.clone(),
        content_hash,
        source_hash,
        title: def.title.clone(),
        theme: theme_ref,
        created_at: chrono::Utc::now(),
        compiler: CompilerInfo {
            name: "lumen-compiler".into(),
            version: COMPILER_VERSION.into(),
            semantic_adapter: "cube@v1".into(),
        },
        format: FormatInfo {
            container_ver: lumen_artifact::CONTAINER_VER,
            min_runtime: 1,
        },
        widgets: compiled.iter().map(|c| c.id.clone()).collect(),
        queries: queries_section.queries.keys().cloned().collect(),
        renderers: distinct_renderers(&charts_section),
        required_credits: CreditEstimate {
            model: "distinct-query/v1".into(),
            estimate,
            queries: distinct_queries,
        },
        runtime_asset: lumen_shared::RuntimeAsset {
            url: "/_lumen/runtime.js".into(),
            integrity: Hash("blake3:runtime".into()),
            embedded: false,
        },
    };

    // 6. Pack the container.
    let mut writer = DepWriter::new();
    writer.add_json(section::MANIFEST, &manifest)?;
    writer.add_json(section::LAYOUT, &layout)?;
    writer.add_json(section::QUERIES, &queries_section)?;
    writer.add_json(section::CHARTS, &charts_section)?;
    writer.add_raw(section::THEME, theme_css.into_bytes());
    Ok(writer.finish())
}

/// Compile and write a `.lumen` file. Returns the byte length written.
pub fn compile_to_file(def: &DashboardDef, path: &std::path::Path) -> anyhow::Result<usize> {
    let bytes = compile(def)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let len = bytes.len();
    std::fs::write(path, bytes)?;
    Ok(len)
}

fn chart_kind(k: WidgetKind) -> ChartKind {
    match k {
        WidgetKind::Kpi => ChartKind::Kpi,
        WidgetKind::LineChart => ChartKind::Line,
        WidgetKind::BarChart => ChartKind::Bar,
        WidgetKind::Table => ChartKind::Table,
    }
}

fn resolve_widget(i: usize, w: &WidgetDef, theme_ref: &str) -> Result<Compiled, CompileError> {
    let id = WidgetId::positional(i);
    let chart_id = ChartId(format!("c_{}", id.as_str()));
    let default_title = || w.title.clone().unwrap_or_default();

    match w.kind {
        WidgetKind::Kpi => {
            let measure = w
                .measure
                .clone()
                .ok_or(CompileError::MissingField(i, "measure"))?;
            let query = Query {
                measures: vec![measure.clone()],
                ..Default::default()
            };
            let title = w.title.clone().unwrap_or_else(|| measure.clone());
            let chart = ChartSpec::Html {
                kind: ChartKind::Kpi,
                template: "kpi".into(),
                number_format: w.format.or(Some(ValueFormat::Number)),
                theme_ref: theme_ref.to_owned(),
            };
            let binding = Binding {
                value: Some(measure),
                format: w.format,
                ..Default::default()
            };
            Ok(Compiled {
                query_id: QueryId::of(&query),
                id,
                kind: w.kind,
                title,
                query,
                chart_id,
                chart,
                binding,
                pos: w.pos,
            })
        }
        WidgetKind::LineChart => {
            let x = w.x.clone().ok_or(CompileError::MissingField(i, "x"))?;
            let y = w.y.clone().ok_or(CompileError::MissingField(i, "y"))?;
            let granularity = w.granularity.unwrap_or(Granularity::Month);
            let query = Query {
                measures: vec![y.clone()],
                time_dimensions: vec![TimeDimension {
                    dimension: x.clone(),
                    granularity,
                    date_range: None,
                }],
                order: vec![Order {
                    field: x.clone(),
                    dir: SortDir::Asc,
                }],
                ..Default::default()
            };
            let chart = ChartSpec::Svg {
                kind: ChartKind::Line,
                encoding: Encoding {
                    x: AxisSpec {
                        scale: "time".into(),
                        format: Some("%b %Y".into()),
                        grid: false,
                    },
                    y: AxisSpec {
                        scale: "linear".into(),
                        format: axis_format(w.format),
                        grid: true,
                    },
                },
                geometry: Geometry {
                    width: 720,
                    height: 300,
                    margin: [16, 24, 32, 64],
                    curve: Some("monotone".into()),
                },
                palette: vec!["#2563eb".into()],
                theme_ref: theme_ref.to_owned(),
            };
            let binding = Binding {
                x: Some(x),
                y: Some(y),
                format: w.format,
                ..Default::default()
            };
            Ok(Compiled {
                query_id: QueryId::of(&query),
                id,
                kind: w.kind,
                title: default_title(),
                query,
                chart_id,
                chart,
                binding,
                pos: w.pos,
            })
        }
        WidgetKind::BarChart => {
            let x = w.x.clone().ok_or(CompileError::MissingField(i, "x"))?;
            let y = w.y.clone().ok_or(CompileError::MissingField(i, "y"))?;
            let query = Query {
                measures: vec![y.clone()],
                dimensions: vec![x.clone()],
                order: vec![Order {
                    field: y.clone(),
                    dir: SortDir::Desc,
                }],
                ..Default::default()
            };
            // Precompiled ECharts option skeleton; data injected at request time.
            let option_template = serde_json::json!({
                "grid": { "left": 48, "right": 16, "top": 16, "bottom": 28, "containLabel": true },
                "tooltip": { "trigger": "axis" },
                "xAxis": { "type": "category" },
                "yAxis": { "type": "value" },
                "dataset": { "dimensions": ["category", "value"], "source": [] },
                "series": [
                    { "type": "bar", "encode": { "x": "category", "y": "value" },
                      "itemStyle": { "color": "#2563eb" } }
                ]
            });
            let chart = ChartSpec::Echarts {
                kind: ChartKind::Bar,
                option_template,
                theme_ref: theme_ref.to_owned(),
            };
            let binding = Binding {
                x: Some(x),
                y: Some(y),
                format: w.format,
                ..Default::default()
            };
            Ok(Compiled {
                query_id: QueryId::of(&query),
                id,
                kind: w.kind,
                title: default_title(),
                query,
                chart_id,
                chart,
                binding,
                pos: w.pos,
            })
        }
        WidgetKind::Table => {
            // Minimal: one dimension + one measure column.
            let x = w.x.clone();
            let y = w.y.clone();
            let mut query = Query::default();
            if let Some(x) = &x {
                query.dimensions.push(x.clone());
            }
            if let Some(y) = &y {
                query.measures.push(y.clone());
            }
            let chart = ChartSpec::Html {
                kind: ChartKind::Table,
                template: "table".into(),
                number_format: w.format,
                theme_ref: theme_ref.to_owned(),
            };
            let binding = Binding {
                x,
                y,
                format: w.format,
                ..Default::default()
            };
            Ok(Compiled {
                query_id: QueryId::of(&query),
                id,
                kind: w.kind,
                title: default_title(),
                query,
                chart_id,
                chart,
                binding,
                pos: w.pos,
            })
        }
    }
}

/// Axis format hint for the SVG renderer (`$` / `%` / none).
fn axis_format(f: Option<ValueFormat>) -> Option<String> {
    match f {
        Some(ValueFormat::Currency) => Some("$".into()),
        Some(ValueFormat::Percent) => Some("%".into()),
        _ => None,
    }
}

/// Default cell size (in grid units) per widget kind.
fn widget_size(kind: WidgetKind) -> (u16, u16) {
    match kind {
        WidgetKind::Kpi => (3, 2),
        WidgetKind::LineChart => (12, 4),
        WidgetKind::BarChart => (12, 4),
        WidgetKind::Table => (12, 4),
    }
}

/// Shelf-pack widgets left-to-right, wrapping rows — a deterministic auto-flow
/// layout resolved entirely at compile time. A widget with an explicit
/// builder-authored `pos` is placed verbatim and does NOT advance the
/// auto-flow cursor — so dashboards that mix explicit and auto-flow widgets
/// (or that set no `pos` at all, e.g. every hand-authored dashboard today)
/// compile identically to before this field existed.
fn place_widgets(compiled: &[Compiled]) -> Vec<WidgetPlacement> {
    let mut out = Vec::with_capacity(compiled.len());
    let (mut x, mut y, mut row_h) = (0u16, 0u16, 0u16);
    for c in compiled {
        let pos = match c.pos {
            Some(p) => p,
            None => {
                let (w, h) = widget_size(c.kind);
                if x + w > GRID_COLS {
                    x = 0;
                    y += row_h;
                    row_h = 0;
                }
                let p = lumen_shared::GridPos { x, y, w, h };
                x += w;
                row_h = row_h.max(h);
                p
            }
        };
        out.push(WidgetPlacement {
            id: c.id.clone(),
            kind: c.kind,
            title: c.title.clone(),
            pos,
            query: c.query_id.clone(),
            chart: c.chart_id.clone(),
            binding: c.binding.clone(),
        });
    }
    out
}

fn distinct_renderers(charts: &Charts) -> Vec<Renderer> {
    let mut seen = Vec::new();
    for spec in charts.specs.values() {
        let r = spec.renderer();
        if !seen.contains(&r) {
            seen.push(r);
        }
    }
    seen
}

/// The compiled theme stylesheet shipped inside the DEP.
fn compiled_theme(theme: &str) -> String {
    let (bg, fg, muted, card, border, accent) = if theme == "dark" {
        ("#0b0e14", "#e6e6e6", "#9aa4b2", "#141925", "#222a39", "#3b82f6")
    } else {
        ("#f6f7f9", "#0f172a", "#64748b", "#ffffff", "#e7eaee", "#2563eb")
    };
    format!(
        // Vars on both :root (full page) and .lumen-root (works inside a Shadow
        // DOM, where :root/body don't match). Visual base lives on .lumen-root.
        r#":root,.lumen-root{{--bg:{bg};--fg:{fg};--muted:{muted};--card:{card};--border:{border};--accent:{accent}}}
*{{box-sizing:border-box}}
body{{margin:0;background:var(--bg);color:var(--fg);font:14px/1.45 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Helvetica,Arial}}
.lumen-root{{background:var(--bg);color:var(--fg);font:14px/1.45 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto,Helvetica,Arial;padding:20px;max-width:1200px;margin:0 auto}}
.lumen-head{{display:flex;align-items:baseline;gap:12px;margin:0 0 16px}}
.lumen-head h1{{font-size:18px;font-weight:650;margin:0}}
.lumen-head .sub{{color:var(--muted);font-size:12px}}
.lumen-grid{{display:grid;gap:16px}}
.lumen-card{{background:var(--card);border:1px solid var(--border);border-radius:12px;padding:16px;overflow:hidden}}
.lumen-card h3{{margin:0 0 8px;font-size:12px;font-weight:600;color:var(--muted);text-transform:uppercase;letter-spacing:.04em}}
.kpi .value{{font-size:30px;font-weight:680;letter-spacing:-.02em}}
.lumen-card svg{{display:block;width:100%;height:auto}}
.lumen-chart{{width:100%}}
.axis{{stroke:var(--border)}}
.gridline{{stroke:var(--border);stroke-dasharray:2 3}}
.tick{{fill:var(--muted);font-size:10px}}
.series{{fill:none;stroke:var(--accent);stroke-width:2}}
.area{{fill:var(--accent);opacity:.08}}
.lumen-foot{{margin-top:16px;color:var(--muted);font-size:11px;text-align:right}}
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_artifact::Dep;

    fn sample() -> DashboardDef {
        serde_json::from_str(
            r#"{
              "id":"sales","title":"Sales","theme":"light",
              "layout":[
                {"type":"kpi","title":"Revenue","measure":"orders.revenue","format":"currency"},
                {"type":"kpi","title":"Orders","measure":"orders.count"},
                {"type":"line_chart","title":"Revenue/mo","x":"orders.created_at","y":"orders.revenue","granularity":"month"},
                {"type":"bar_chart","title":"By status","x":"orders.status","y":"orders.revenue"}
              ]
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn compiles_into_a_loadable_dep() {
        let bytes = compile(&sample()).unwrap();
        let dep = Dep::from_bytes(bytes).unwrap();

        let manifest = dep.manifest().unwrap();
        assert_eq!(manifest.title, "Sales");
        assert_eq!(manifest.widgets.len(), 4);

        let layout = dep.layout().unwrap();
        assert_eq!(layout.widgets.len(), 4);

        let charts = dep.charts().unwrap();
        // kpi+kpi+line+bar → html, svg, echarts all present.
        assert!(charts.specs.values().any(|s| s.renderer() == Renderer::Svg));
        assert!(charts
            .specs
            .values()
            .any(|s| s.renderer() == Renderer::Echarts));
        assert!(charts.specs.values().any(|s| s.renderer() == Renderer::Html));
    }

    #[test]
    fn dedupes_identical_queries() {
        // Two KPIs on the same measure must collapse to one query.
        let def: DashboardDef = serde_json::from_str(
            r#"{"id":"d","title":"t","layout":[
                {"type":"kpi","measure":"orders.revenue"},
                {"type":"kpi","measure":"orders.revenue"}
            ]}"#,
        )
        .unwrap();
        let dep = Dep::from_bytes(compile(&def).unwrap()).unwrap();
        assert_eq!(dep.manifest().unwrap().widgets.len(), 2);
        assert_eq!(dep.queries().unwrap().queries.len(), 1, "queries dedupe");
    }

    #[test]
    fn respects_explicit_pos_and_skips_autoflow_cursor_for_it() {
        // First widget has an explicit pos; the second has none and should
        // still auto-flow from {x:0,y:0} as if the first widget weren't
        // there at all — explicit-pos widgets must not consume shelf space.
        let def: DashboardDef = serde_json::from_str(
            r#"{"id":"d","title":"t","layout":[
                {"type":"kpi","measure":"orders.revenue","pos":{"x":6,"y":0,"w":6,"h":3}},
                {"type":"kpi","measure":"orders.count"}
            ]}"#,
        )
        .unwrap();
        let dep = Dep::from_bytes(compile(&def).unwrap()).unwrap();
        let layout = dep.layout().unwrap();

        let explicit = layout.widgets.iter().find(|w| w.id.as_str() == "w_0").unwrap();
        assert_eq!((explicit.pos.x, explicit.pos.y, explicit.pos.w, explicit.pos.h), (6, 0, 6, 3));

        let auto = layout.widgets.iter().find(|w| w.id.as_str() == "w_1").unwrap();
        assert_eq!((auto.pos.x, auto.pos.y), (0, 0), "auto-flow widget unaffected by sibling's explicit pos");
    }
}

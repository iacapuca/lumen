//! `lumen-renderer` — binds fresh results into a compiled DEP and emits HTML.
//!
//! The browser only ever receives: a **compiled layout**, **fresh metric JSON**,
//! and a tiny runtime. Per renderer tag:
//!   * `html`    → KPI cards / tables, pure server HTML (0 KB client JS);
//!   * `svg`     → line charts, server-rendered inline SVG (0 KB client JS);
//!   * `echarts` → a precompiled `option` skeleton + a *separate* fresh-data
//!     island, hydrated late by `runtime.js`.

use std::collections::HashMap;

use askama::Template;
use lumen_artifact::Dep;
use lumen_shared::{value_as_f64, ChartSpec, Data, Encoding, Geometry, QueryId, ValueFormat, WidgetPlacement};
use serde_json::{json, Value};

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("artifact error: {0}")]
    Artifact(#[from] lumen_artifact::ArtifactError),
    #[error("template error: {0}")]
    Template(#[from] askama::Error),
}

/// Per-request render configuration (asset URLs the page should reference).
pub struct RenderContext {
    pub runtime_url: String,
    pub echarts_cdn: String,
    /// Short footer note (e.g. content hash + timestamp).
    pub footer: String,
}

impl Default for RenderContext {
    fn default() -> Self {
        RenderContext {
            runtime_url: "/_lumen/runtime.js".into(),
            echarts_cdn: "https://cdn.jsdelivr.net/npm/echarts@6.1.0/dist/echarts.min.js".into(),
            footer: "Lumen".into(),
        }
    }
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    title: String,
    subtitle: String,
    theme: String,
    theme_css: String,
    grid_cols: u16,
    gap: u16,
    row_height: u16,
    cards: Vec<Card>,
    islands: String,
    needs_echarts: bool,
    echarts_cdn: String,
    runtime_url: String,
    footer: String,
}

struct Card {
    class: String,
    has_title: bool,
    title: String,
    style: String,
    body: String,
}

#[derive(Template)]
#[template(path = "fragment.html")]
struct FragmentTemplate {
    title: String,
    subtitle: String,
    theme: String,
    theme_css: String,
    grid_cols: u16,
    gap: u16,
    row_height: u16,
    cards: Vec<Card>,
    islands: String,
    footer: String,
}

/// Data-derived view parts shared by the full-page and fragment renderers.
struct Assembled {
    title: String,
    subtitle: String,
    theme: String,
    theme_css: String,
    grid_cols: u16,
    gap: u16,
    row_height: u16,
    cards: Vec<Card>,
    islands: String,
    needs_echarts: bool,
}

fn assemble(dep: &Dep, results: &HashMap<QueryId, Data>) -> Result<Assembled, RenderError> {
    let manifest = dep.manifest()?;
    let layout = dep.layout()?;
    let charts = dep.charts()?;
    let theme_css = String::from_utf8_lossy(dep.theme_css()).into_owned();

    let mut cards = Vec::with_capacity(layout.widgets.len());
    let mut islands = String::new();
    let mut needs_echarts = false;

    for w in &layout.widgets {
        let data = results.get(&w.query).cloned().unwrap_or_default();
        let spec = charts.specs.get(&w.chart);
        let rendered = render_widget(w, spec, &data);
        if rendered.needs_echarts {
            needs_echarts = true;
        }
        islands.push_str(&rendered.islands);

        let style = format!(
            "grid-column:{} / span {};grid-row:{} / span {}",
            w.pos.x + 1,
            w.pos.w,
            w.pos.y + 1,
            w.pos.h
        );
        cards.push(Card {
            class: w.kind.as_str().to_string(),
            has_title: !w.title.is_empty(),
            title: w.title.clone(),
            style,
            body: rendered.body,
        });
    }

    let subtitle = format!(
        "compiled · {} widgets · {} queries · ~{} credits",
        manifest.widgets.len(),
        manifest.queries.len(),
        manifest.required_credits.estimate,
    );

    Ok(Assembled {
        title: manifest.title.clone(),
        subtitle,
        theme: manifest.theme.clone(),
        theme_css,
        grid_cols: layout.grid.cols,
        gap: layout.grid.gap,
        row_height: layout.grid.row_height,
        cards,
        islands,
        needs_echarts,
    })
}

/// Render a compiled DEP into a full standalone HTML page (iframe / direct nav).
pub fn render_dashboard(
    dep: &Dep,
    results: &HashMap<QueryId, Data>,
    ctx: &RenderContext,
) -> Result<String, RenderError> {
    let a = assemble(dep, results)?;
    let tmpl = DashboardTemplate {
        title: a.title,
        subtitle: a.subtitle,
        theme: a.theme,
        theme_css: a.theme_css,
        grid_cols: a.grid_cols,
        gap: a.gap,
        row_height: a.row_height,
        cards: a.cards,
        islands: a.islands,
        needs_echarts: a.needs_echarts,
        echarts_cdn: ctx.echarts_cdn.clone(),
        runtime_url: ctx.runtime_url.clone(),
        footer: ctx.footer.clone(),
    };
    Ok(tmpl.render()?)
}

/// Render a DEP into an HTML *fragment* — style + grid + data islands, no document
/// wrapper and no scripts — for mounting inside a Web Component's Shadow DOM. The
/// `<analytics-dashboard>` element loads ECharts and hydrates within the shadow root.
pub fn render_fragment(
    dep: &Dep,
    results: &HashMap<QueryId, Data>,
    ctx: &RenderContext,
) -> Result<String, RenderError> {
    let a = assemble(dep, results)?;
    let tmpl = FragmentTemplate {
        title: a.title,
        subtitle: a.subtitle,
        theme: a.theme,
        theme_css: a.theme_css,
        grid_cols: a.grid_cols,
        gap: a.gap,
        row_height: a.row_height,
        cards: a.cards,
        islands: a.islands,
        footer: ctx.footer.clone(),
    };
    Ok(tmpl.render()?)
}

struct Rendered {
    body: String,
    islands: String,
    needs_echarts: bool,
}

impl Rendered {
    fn body(body: String) -> Self {
        Rendered {
            body,
            islands: String::new(),
            needs_echarts: false,
        }
    }
}

fn render_widget(w: &WidgetPlacement, spec: Option<&ChartSpec>, data: &Data) -> Rendered {
    match spec {
        Some(ChartSpec::Html { template, .. }) if template == "table" => {
            Rendered::body(render_table(data))
        }
        Some(ChartSpec::Html { .. }) => {
            // KPI
            let v = w.binding.value.as_ref().and_then(|m| data.scalar(m));
            let text = match v {
                Some(x) => fmt_value(x, w.binding.format),
                None => "—".to_string(),
            };
            Rendered::body(format!("<div class=\"value\">{text}</div>"))
        }
        Some(ChartSpec::Svg {
            encoding, geometry, ..
        }) => Rendered::body(render_line_svg(w, encoding, geometry, data)),
        Some(ChartSpec::Echarts {
            option_template, ..
        }) => render_echarts(w, option_template, data),
        None => Rendered::body("<div class=\"empty\">unconfigured widget</div>".into()),
    }
}

// --- ECharts (client hydration) -------------------------------------------

fn render_echarts(w: &WidgetPlacement, option_template: &Value, data: &Data) -> Rendered {
    let wid = w.id.as_str();
    let x = w.binding.x.as_deref();
    let y = w.binding.y.as_deref();

    let rows: Vec<Value> = data
        .rows
        .iter()
        .map(|row| {
            let cat = x
                .and_then(|m| row_get(row, m))
                .map(val_to_label)
                .unwrap_or_default();
            let val = y
                .and_then(|m| row_get(row, m))
                .and_then(value_as_f64)
                .unwrap_or(0.0);
            json!([cat, val])
        })
        .collect();

    // Spec island = compiled option skeleton (no data). Data island = fresh rows.
    // json_island() escapes HTML-significant chars so a data value containing
    // "</script>" cannot break out of the island (stored-XSS defense).
    let spec_json = json_island(&json!({ "option": option_template }));
    let data_json = json_island(&json!({ "rows": rows }));
    let islands = format!(
        "<script type=\"application/json\" id=\"spec:{wid}\">{spec_json}</script>\n<script type=\"application/json\" id=\"data:{wid}\">{data_json}</script>\n",
    );
    let body = format!(
        "<div class=\"lumen-chart\" data-chart=\"{wid}\" style=\"width:100%;height:260px\"></div>"
    );
    Rendered {
        body,
        islands,
        needs_echarts: true,
    }
}

// --- Server-rendered SVG line chart ---------------------------------------

fn render_line_svg(w: &WidgetPlacement, enc: &Encoding, geom: &Geometry, data: &Data) -> String {
    let xm = w.binding.x.as_deref();
    let ym = w.binding.y.as_deref();

    let mut xs: Vec<String> = Vec::new();
    let mut ys: Vec<f64> = Vec::new();
    for row in &data.rows {
        let xv = xm
            .and_then(|m| row_get(row, m))
            .map(val_to_label)
            .unwrap_or_default();
        let yv = ym
            .and_then(|m| row_get(row, m))
            .and_then(value_as_f64)
            .unwrap_or(0.0);
        xs.push(xv);
        ys.push(yv);
    }

    let w_px = geom.width as f64;
    let h_px = geom.height as f64;
    let (mt, mr, mb, ml) = (
        geom.margin[0] as f64,
        geom.margin[1] as f64,
        geom.margin[2] as f64,
        geom.margin[3] as f64,
    );
    let pw = (w_px - ml - mr).max(1.0);
    let ph = (h_px - mt - mb).max(1.0);
    let n = ys.len();

    if n == 0 {
        return "<div class=\"empty\">no data</div>".into();
    }

    let ymax = ys.iter().cloned().fold(f64::MIN, f64::max).max(1.0);
    let xpos = |i: usize| -> f64 {
        if n == 1 {
            ml + pw / 2.0
        } else {
            ml + pw * (i as f64) / ((n - 1) as f64)
        }
    };
    let ypos = |v: f64| -> f64 { mt + ph * (1.0 - (v / ymax)) };

    let yfmt = enc.y.format.as_deref();
    let mut svg = String::new();

    // Y gridlines + ticks (zero-baseline linear scale).
    let ticks = 4;
    for t in 0..=ticks {
        let val = ymax * (t as f64) / (ticks as f64);
        let y = ypos(val);
        svg.push_str(&format!(
            "<line class=\"gridline\" x1=\"{ml:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/>",
            ml + pw
        ));
        svg.push_str(&format!(
            "<text class=\"tick\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{}</text>",
            ml - 6.0,
            y + 3.0,
            fmt_axis(val, yfmt)
        ));
    }

    // Series polyline.
    let pts: String = (0..n)
        .map(|i| format!("{:.1},{:.1}", xpos(i), ypos(ys[i])))
        .collect::<Vec<_>>()
        .join(" ");
    // Filled area under the curve.
    let baseline = ypos(0.0);
    svg.push_str(&format!(
        "<polygon class=\"area\" points=\"{:.1},{:.1} {} {:.1},{:.1}\"/>",
        xpos(0),
        baseline,
        pts,
        xpos(n - 1),
        baseline
    ));
    svg.push_str(&format!("<polyline class=\"series\" points=\"{pts}\"/>"));

    // X tick labels: first, middle, last (deduped for small n).
    let mut idxs = vec![0usize, n / 2, n - 1];
    idxs.sort_unstable();
    idxs.dedup();
    for i in idxs {
        let anchor = if i == 0 {
            "start"
        } else if i == n - 1 {
            "end"
        } else {
            "middle"
        };
        svg.push_str(&format!(
            "<text class=\"tick\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"{anchor}\">{}</text>",
            xpos(i),
            h_px - mb + 16.0,
            html_escape(&short_label(&xs[i]))
        ));
    }

    format!(
        "<svg viewBox=\"0 0 {w_px} {h_px}\" preserveAspectRatio=\"xMidYMid meet\" role=\"img\" aria-label=\"line chart\">{svg}</svg>"
    )
}

// --- Table -----------------------------------------------------------------

fn render_table(data: &Data) -> String {
    if data.rows.is_empty() {
        return "<div class=\"empty\">no data</div>".into();
    }
    let headers: Vec<String> = data.rows[0].keys().cloned().collect();
    let mut out = String::from("<table class=\"lumen-table\"><thead><tr>");
    for h in &headers {
        out.push_str(&format!("<th>{}</th>", html_escape(h)));
    }
    out.push_str("</tr></thead><tbody>");
    for row in data.rows.iter().take(50) {
        out.push_str("<tr>");
        for h in &headers {
            let cell = row.get(h).map(val_to_label).unwrap_or_default();
            out.push_str(&format!("<td>{}</td>", html_escape(&cell)));
        }
        out.push_str("</tr>");
    }
    out.push_str("</tbody></table>");
    out
}

// --- helpers ---------------------------------------------------------------

/// Look up a member, tolerating Cube's `dimension.granularity` suffixing for
/// time dimensions (e.g. `orders.created_at` vs `orders.created_at.month`).
fn row_get<'a>(row: &'a lumen_shared::Row, member: &str) -> Option<&'a Value> {
    if let Some(v) = row.get(member) {
        return Some(v);
    }
    let prefix = format!("{member}.");
    row.iter()
        .find(|(k, _)| k.starts_with(&prefix))
        .map(|(_, v)| v)
}

/// Serialize JSON for safe embedding inside an HTML `<script>` element. Escapes
/// the characters the HTML parser reacts to so a string value cannot terminate
/// the script tag. The result is still valid JSON.
fn json_island(v: &Value) -> String {
    serde_json::to_string(v)
        .unwrap_or_else(|_| "{}".into())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn val_to_label(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Shorten an ISO timestamp to `YYYY-MM` for axis labels. Char-boundary checked
/// so a non-ASCII label can never panic the slice.
fn short_label(s: &str) -> String {
    if s.len() >= 7 && s.is_char_boundary(7) && s.as_bytes()[4] == b'-' {
        s[..7].to_string()
    } else {
        s.to_string()
    }
}

fn fmt_value(v: f64, f: Option<ValueFormat>) -> String {
    match f {
        Some(ValueFormat::Currency) => format!("${}", group_thousands(v.round() as i64)),
        Some(ValueFormat::Percent) => format!("{v:.1}%"),
        Some(ValueFormat::Number) | None => group_thousands(v.round() as i64),
    }
}

/// Compact axis label, e.g. `$1.2M`, `340k`, `42`.
fn fmt_axis(v: f64, fmt: Option<&str>) -> String {
    let prefix = match fmt {
        Some("$") => "$",
        _ => "",
    };
    let suffix = match fmt {
        Some("%") => "%",
        _ => "",
    };
    let a = v.abs();
    let body = if a >= 1_000_000.0 {
        format!("{:.1}M", v / 1_000_000.0)
    } else if a >= 1_000.0 {
        format!("{:.0}k", v / 1_000.0)
    } else {
        format!("{v:.0}")
    };
    format!("{prefix}{body}{suffix}")
}

fn group_thousands(n: i64) -> String {
    let neg = n < 0;
    // unsigned_abs avoids the i64::MIN.abs() overflow panic.
    let digits = n.unsigned_abs().to_string();
    let bytes = digits.as_bytes();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 1);
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thousands_grouping() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1_234_567), "1,234,567");
        assert_eq!(group_thousands(-12_000), "-12,000");
    }

    #[test]
    fn currency_format() {
        assert_eq!(fmt_value(1234.0, Some(ValueFormat::Currency)), "$1,234");
    }

    #[test]
    fn axis_compacts() {
        assert_eq!(fmt_axis(1_500_000.0, Some("$")), "$1.5M");
        assert_eq!(fmt_axis(3400.0, None), "3k");
    }

    #[test]
    fn row_get_tolerates_granularity_suffix() {
        let mut row = lumen_shared::Row::new();
        row.insert("orders.created_at.month".into(), json!("2024-01-01T00:00:00.000"));
        assert!(row_get(&row, "orders.created_at").is_some());
    }
}

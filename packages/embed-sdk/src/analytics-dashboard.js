/**
 * <analytics-dashboard> — non-iframe embedding for Lumen.
 *
 * Fetches the compiled dashboard as an HTML *fragment* and mounts it into a
 * Shadow DOM (style-isolated, natural sizing, no postMessage resize hacks), then
 * loads ECharts and hydrates the chart widgets within the shadow root. KPI and
 * line-chart widgets are server-rendered SVG/HTML and need no JS at all.
 *
 *   <analytics-dashboard
 *     base="https://embed.lumen"
 *     dashboard="sales"
 *     token="<jwt your backend minted via POST /tokens>"></analytics-dashboard>
 *
 * Data security comes from the scoped token + server-side row-level security
 * (Cube) — NOT from an iframe boundary. Use this for trusted host apps (your
 * customer embedding into their own product); use the iframe path for untrusted
 * third-party hosts that need a hard same-origin wall.
 */

const DEFAULT_ECHARTS =
  "https://cdn.jsdelivr.net/npm/echarts@6.1.0/dist/echarts.min.js";

let echartsPromise = null;
function loadECharts(src) {
  if (typeof window !== "undefined" && window.echarts) {
    return Promise.resolve(window.echarts);
  }
  if (echartsPromise) return echartsPromise;
  echartsPromise = new Promise((resolve, reject) => {
    const s = document.createElement("script");
    s.src = src || DEFAULT_ECHARTS;
    s.async = true;
    s.onload = () => resolve(window.echarts);
    s.onerror = () => reject(new Error("failed to load ECharts"));
    document.head.appendChild(s);
  });
  return echartsPromise;
}

function islandJSON(root, id) {
  const el = root.getElementById(id);
  if (!el) return null;
  try {
    return JSON.parse(el.textContent);
  } catch (e) {
    return null;
  }
}

function notice(text, color) {
  return `<div style="font:13px/1.4 system-ui,sans-serif;color:${color};padding:16px">${text}</div>`;
}

// Import-safe in non-DOM (SSR/Node) environments: HTMLElement is undefined there,
// so fall back to a dummy base; the element is only ever defined/used in a browser.
const Base = typeof HTMLElement !== "undefined" ? HTMLElement : class {};

class AnalyticsDashboard extends Base {
  static get observedAttributes() {
    return ["dashboard", "token", "base", "echarts-src"];
  }

  connectedCallback() {
    this._render();
  }
  attributeChangedCallback() {
    if (this.isConnected) this._render();
  }
  disconnectedCallback() {
    this._teardown();
  }

  _teardown() {
    if (this._onResize) {
      window.removeEventListener("resize", this._onResize);
      this._onResize = null;
    }
    (this._charts || []).forEach((c) => c && c.dispose && c.dispose());
    this._charts = [];
  }

  async _render() {
    const dashboard = this.getAttribute("dashboard");
    const token = this.getAttribute("token");
    const base = (this.getAttribute("base") || "").replace(/\/$/, "");
    const echartsSrc = this.getAttribute("echarts-src") || DEFAULT_ECHARTS;

    const root = this.shadowRoot || this.attachShadow({ mode: "open" });
    this._teardown();

    if (!dashboard || !token) {
      root.innerHTML = notice("Missing required attribute: dashboard and token.", "#b91c1c");
      return;
    }
    root.innerHTML = notice("Loading dashboard…", "#888");

    const url = `${base}/embed/dashboard/${encodeURIComponent(
      dashboard
    )}?token=${encodeURIComponent(token)}&format=fragment`;

    let html;
    try {
      const res = await fetch(url, { credentials: "omit" });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      html = await res.text();
    } catch (e) {
      root.innerHTML = notice(`Could not load dashboard: ${e.message || e}`, "#b91c1c");
      this.dispatchEvent(new CustomEvent("error", { detail: e }));
      return;
    }

    // <style> applies inside the shadow root; JSON islands are inert <script> data.
    root.innerHTML = html;

    const nodes = root.querySelectorAll("[data-chart]");
    if (nodes.length) {
      let echarts;
      try {
        echarts = await loadECharts(echartsSrc);
      } catch (e) {
        // KPIs + line charts are already server-rendered SVG/HTML; only the
        // interactive ECharts widgets stay empty if the CDN is unreachable.
        this.dispatchEvent(new CustomEvent("load", { detail: { dashboard, charts: false } }));
        return;
      }
      this._charts = [];
      nodes.forEach((el) => {
        const key = el.getAttribute("data-chart");
        const spec = islandJSON(root, `spec:${key}`);
        const data = islandJSON(root, `data:${key}`);
        if (!spec || !data) return;
        const option = spec.option || {};
        option.dataset = option.dataset || {};
        option.dataset.source = data.rows || [];
        const chart = echarts.init(el, null, { renderer: "svg" });
        chart.setOption(option);
        this._charts.push(chart);
      });
      this._onResize = () => (this._charts || []).forEach((c) => c.resize());
      window.addEventListener("resize", this._onResize);
    }
    this.dispatchEvent(new CustomEvent("load", { detail: { dashboard, charts: true } }));
  }
}

if (
  typeof customElements !== "undefined" &&
  !customElements.get("analytics-dashboard")
) {
  customElements.define("analytics-dashboard", AnalyticsDashboard);
}

export { AnalyticsDashboard };

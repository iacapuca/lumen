/**
 * Lumen client runtime (~1 KB). The browser receives a COMPILED layout plus
 * FRESH metric JSON in separate <script type="application/json"> islands; this
 * binds the data into the precompiled ECharts `option` skeletons late.
 *
 * It does NOT compile layouts, build specs, or plan queries — all of that
 * happened at compile time inside the .lumen DEP. Server-rendered SVG (KPI/line)
 * needs no JS at all; this only hydrates `echarts` widgets.
 *
 * Production note: ship a tree-shaken ECharts build (line+bar+svg ≈ 50–70 KB gz)
 * instead of the CDN UMD bundle.
 */
(function () {
  "use strict";

  function island(id) {
    var el = document.getElementById(id);
    if (!el) return null;
    try {
      return JSON.parse(el.textContent);
    } catch (e) {
      return null;
    }
  }

  function hydrate() {
    if (!window.echarts) {
      console.warn("[lumen] echarts not loaded; skipping chart hydration");
      return;
    }
    var nodes = document.querySelectorAll("[data-chart]");
    for (var i = 0; i < nodes.length; i++) {
      (function (el) {
        var key = el.getAttribute("data-chart");
        var spec = island("spec:" + key); // compiled option skeleton (no data)
        var data = island("data:" + key); // fresh rows, injected at request time
        if (!spec || !data) return;

        var option = spec.option || {};
        option.dataset = option.dataset || {};
        option.dataset.source = data.rows || []; // <-- the late binding

        var chart = window.echarts.init(el, null, { renderer: "svg" });
        chart.setOption(option);
        window.addEventListener("resize", function () {
          chart.resize();
        });
      })(nodes[i]);
    }
  }

  if (document.readyState !== "loading") hydrate();
  else document.addEventListener("DOMContentLoaded", hydrate);
})();

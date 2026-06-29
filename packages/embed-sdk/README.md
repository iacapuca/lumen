# @lumen/embed-sdk

The customer-facing embed loader. Wraps the iframe embedding path today; the
roadmap swaps in a `<analytics-dashboard>` Web Component + a `postMessage`
resize/event channel without changing the call signature.

```html
<div id="report"></div>
<script type="module">
  import { Lumen } from "@lumen/embed-sdk";
  Lumen.render("#report", {
    baseUrl: "https://embed.your-host",
    dashboard: "sales",
    token: "<jwt your backend minted>",
  });
</script>
```

## Web Component (no iframe)

`@lumen/embed-sdk/element` registers `<analytics-dashboard>`, which fetches the
compiled dashboard as a fragment (`?format=fragment`) and mounts it into a **Shadow
DOM** — style-isolated, natural sizing, no `postMessage` resize hacks. KPI + line
charts are server-rendered SVG (0 KB JS); ECharts widgets hydrate within the shadow
root.

```html
<script type="module" src="https://embed.your-host/_lumen/analytics-dashboard.js"></script>
<analytics-dashboard base="https://embed.your-host" dashboard="sales" token="<jwt>">
</analytics-dashboard>
```

Mint the token server-side via `POST /tokens` (the embed host serves the element
script at `/_lumen/analytics-dashboard.js`, like a CDN).

> Note: the per-dashboard client `runtime.js` (which hydrates ECharts in the *iframe*
> path) is served at `/_lumen/runtime.js` and embedded into the full-page HTML — the
> Web Component does its own shadow-root hydration and doesn't use it.

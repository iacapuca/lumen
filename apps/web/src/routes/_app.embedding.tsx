import { createFileRoute } from '@tanstack/react-router'
import { RUNTIME_URL } from '../lib/config'

export const Route = createFileRoute('/_app/embedding')({
  component: Embedding,
})

const iframeSnippet = `<iframe
  src="${RUNTIME_URL}/embed/dashboard/sales?token=<JWT_YOUR_BACKEND_MINTED>"
  title="Sales Dashboard"
  style="width: 100%; height: 600px; border: 0"
  loading="lazy"
></iframe>`

const sdkSnippet = `<div id="report"></div>
<script type="module">
  import { Lumen } from "@lumen/embed-sdk";
  Lumen.render("#report", {
    baseUrl: "${RUNTIME_URL}",
    dashboard: "sales",
    token: "<jwt your backend minted>",
  });
</script>`

function Embedding() {
  return (
    <div className="prose">
      <div className="page-head">
        <h1>Embedding</h1>
        <p>
          Drop a compiled dashboard into any app. Your backend mints a short-lived
          JWT (see <code className="inline">EmbedClaims</code> in{' '}
          <code className="inline">@lumen/contracts</code>); the runtime serves the
          compiled HTML at{' '}
          <code className="inline">/embed/dashboard/:id?token=…</code>.
        </p>
      </div>

      <h2>1 · Raw iframe</h2>
      <p>The lowest-level integration — no script, just an iframe.</p>
      <pre className="code">{iframeSnippet}</pre>

      <h2>2 · Embed SDK (@lumen/embed-sdk)</h2>
      <p>
        The drop-in loader wraps the iframe path today and keeps the same call
        signature when it upgrades to a <code className="inline">&lt;analytics-dashboard&gt;</code>{' '}
        Web Component with a postMessage resize/event channel.
      </p>
      <pre className="code">{sdkSnippet}</pre>

      <h2>Notes</h2>
      <p>
        The per-dashboard client <code className="inline">runtime.js</code> (which
        hydrates ECharts widgets) is served by the runtime at{' '}
        <code className="inline">/_lumen/runtime.js</code> and is embedded into the
        compiled HTML — it is not part of the SDK package.
      </p>
    </div>
  )
}

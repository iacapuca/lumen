import { Link, createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/')({
  component: Landing,
})

const dashboardDef = `{
  "id": "sales",
  "title": "Sales Dashboard",
  "theme": "dark",
  "layout": [
    { "type": "kpi",        "title": "Revenue",
      "measure": "orders.revenue", "format": "currency" },
    { "type": "line_chart", "title": "Revenue over time",
      "x": "orders.created_at", "y": "orders.revenue",
      "granularity": "day" },
    { "type": "table",      "title": "Orders" }
  ]
}`

const FEATURES = [
  {
    n: '01',
    title: 'Drop-in embedding',
    body: 'One <analytics-dashboard> Web Component, or a sandboxed iframe — same compiled artifact, your choice of isolation. It mounts into your product, not a separate portal.',
  },
  {
    n: '02',
    title: 'Multi-tenant & secure',
    body: 'Scoped JWTs forward each user’s identity and security context to your semantic layer, so every embed is row-level-scoped. The token scopes the data, not the frame.',
  },
  {
    n: '03',
    title: 'Compile the plan, not the data',
    body: 'Compiling bakes in the layout, queries, and chart specs — never the results. Every open runs those queries live against your semantic layer, so embeds always show current data.',
  },
  {
    n: '04',
    title: 'Server-rendered, ~0 KB JS',
    body: 'KPIs and charts ship as server SVG/HTML behind a ~1 KB runtime. Your users get layout and numbers, never a megabyte of charting library.',
  },
  {
    n: '05',
    title: 'Usage-priced',
    body: 'Pay for compute credits, not seats — so embedding analytics for 10 users or 10,000 doesn’t change your pricing model.',
  },
  {
    n: '06',
    title: 'Edge-native',
    body: 'The compute core compiles to WebAssembly and runs on Cloudflare Workers. Render each embed next to your users.',
  },
]

const COMPARE = [
  { label: 'Category', lumen: 'Embedding runtime', looker: 'BI platform', sigma: 'BI platform' },
  { label: 'Semantic layer', lumen: 'Yours — Cube.dev', looker: 'Owns it (LookML)', sigma: 'Owns modeling' },
  { label: 'Embedding', lumen: 'Web Component + iframe', looker: 'iframe / embed SDK', sigma: 'iframe / embed' },
  { label: 'Delivery', lumen: 'Compiled · server-SVG · edge', looker: 'Interpreted at runtime', sigma: 'Interpreted at runtime' },
  { label: 'Pricing', lumen: 'Usage — compute credits', looker: 'Per-seat', sigma: 'Per-seat / consumption' },
  { label: 'Built for', lumen: 'Devs embedding in their app', looker: 'BI & analyst teams', sigma: 'Analysts' },
]

function Landing() {
  return (
    <div className="lp">
      <header className="pub">
        <div className="pub__brand">
          <span className="brand__mark" />
          <span className="brand__name">Lumen</span>
        </div>
        <nav className="pub__nav">
          <Link to="/login" className="pub__link">
            Sign in
          </Link>
          <Link to="/signup" className="pub__cta">
            Get started
          </Link>
        </nav>
      </header>

      <main>
        {/* ---- Hero ---- */}
        <section className="hero">
          <span className="eyebrow">Developer-first embedded analytics</span>
          <h1 className="hero__title">
            Analytics inside <span className="hero__accent">your product</span>
            <br />
            <span className="hero__muted">— compiled, not interpreted.</span>
          </h1>
          <p className="hero__sub">
            Lumen is an embedding runtime that drops live, multi-tenant dashboards
            into your app — as a <code className="mono">&lt;analytics-dashboard&gt;</code>{' '}
            Web Component or an iframe. It compiles each one into a portable{' '}
            <code className="mono">.lumen</code> artifact and serves it from the
            edge. You bring the semantic layer; Lumen handles embedding, rendering,
            and row-level security. Think Vercel, for embedded analytics.
          </p>
          <div className="hero__cta">
            <Link to="/signup" className="btn-primary">
              Get started
            </Link>
            <Link to="/login" className="btn-ghost">
              Sign in
            </Link>
          </div>
          <ul className="hero__meta">
            <li>Web Component or iframe</li>
            <li>Multi-tenant, row-level secure</li>
            <li>Usage-priced, not per-seat</li>
          </ul>
          <div className="hero__compat">
            <span className="hero__compat-label">Works with</span>
            <span className="chip-logo">Cube.dev</span>
            <span className="hero__compat-soon">
              dbt&nbsp;Semantic&nbsp;Layer · MetricFlow — soon
            </span>
          </div>
        </section>

        {/* ---- Compile pipeline ---- */}
        <section className="flow">
          <div className="flow__rail">
            <div className="flow__stage">
              <span className="flow__kind">source</span>
              <code className="flow__chip">sales.json</code>
              <span className="flow__note">a dashboard definition</span>
            </div>
            <div className="flow__arrow">
              <code className="flow__op">lumen&nbsp;compile</code>
              <span className="flow__line" />
            </div>
            <div className="flow__stage flow__stage--artifact">
              <span className="flow__kind">artifact</span>
              <code className="flow__chip flow__chip--hot">sales.lumen</code>
              <span className="flow__note">execution plan</span>
            </div>
            <div className="flow__arrow">
              <code className="flow__op">edge&nbsp;render</code>
              <span className="flow__line" />
            </div>
            <div className="flow__stage">
              <span className="flow__kind">output</span>
              <code className="flow__chip">{'<analytics-dashboard/>'}</code>
              <span className="flow__note">live in your app</span>
            </div>
          </div>
        </section>

        {/* ---- Feature grid ---- */}
        <section className="features">
          <div className="features__head">
            <h2>Analytics you embed — not a BI tool you send people to.</h2>
            <p>
              Lumen owns embedding, compilation, and delivery. It does{' '}
              <em>not</em> own your metrics — it sits on top of your semantic layer
              (Cube.dev first) and inherits your modeling untouched.
            </p>
          </div>
          <div className="features__grid">
            {FEATURES.map((f) => (
              <article key={f.n} className="feature">
                <span className="feature__n">{f.n}</span>
                <h3 className="feature__title">{f.title}</h3>
                <p className="feature__body">{f.body}</p>
              </article>
            ))}
          </div>
        </section>

        {/* ---- Comparison ---- */}
        <section className="compare">
          <div className="compare__head">
            <span className="eyebrow">Not another BI tool</span>
            <h2>An embedding runtime — not Looker or Sigma.</h2>
            <p>
              Looker and Sigma are BI platforms — they own the modeling layer and
              the place your users log in to. Lumen is the layer that puts analytics{' '}
              <em>inside your product</em>, on top of the semantic layer you already
              run.
            </p>
          </div>
          <div className="compare__table">
            <div className="compare__row compare__row--head">
              <span />
              <span className="compare__col--lumen">Lumen</span>
              <span>Looker</span>
              <span>Sigma</span>
            </div>
            {COMPARE.map((r) => (
              <div className="compare__row" key={r.label}>
                <span className="compare__label">{r.label}</span>
                <span className="compare__cell compare__col--lumen">{r.lumen}</span>
                <span className="compare__cell">{r.looker}</span>
                <span className="compare__cell">{r.sigma}</span>
              </div>
            ))}
          </div>
          <p className="compare__foot">
            A different category, not a feature-for-feature BI replacement — reach
            for Lumen when you need to <em>embed</em> analytics in a product you ship.
          </p>
        </section>

        {/* ---- Code snippet ---- */}
        <section className="snippet">
          <div className="snippet__copy">
            <span className="eyebrow">Define once</span>
            <h2>
              A dashboard is just data <br />until you compile it.
            </h2>
            <p>
              Write the definition. <code className="mono">lumen compile</code>{' '}
              folds your layout, queries, and theme into a single{' '}
              <code className="mono">.lumen</code> execution plan — ready to embed
              in your product and stream from any edge location, with row-level
              security and live data intact.
            </p>
          </div>
          <figure className="codeblock">
            <figcaption className="codeblock__bar">
              <span className="codeblock__dots" aria-hidden="true">
                <i />
                <i />
                <i />
              </span>
              <span className="codeblock__file">sales.json</span>
            </figcaption>
            <pre className="codeblock__pre">
              <code>{dashboardDef}</code>
            </pre>
          </figure>
        </section>

        {/* ---- Closing CTA ---- */}
        <section className="closing">
          <h2 className="closing__title">
            Give your customers analytics.
            <br />
            Not a BI tool to log into.
          </h2>
          <div className="hero__cta">
            <Link to="/signup" className="btn-primary">
              Get started
            </Link>
            <Link to="/login" className="btn-ghost">
              Sign in
            </Link>
          </div>
        </section>
      </main>

      <footer className="lp-foot">
        <div className="lp-foot__brand">
          <span className="brand__mark" />
          <span className="brand__name">Lumen</span>
        </div>
        <span className="lp-foot__tag">Embedded analytics, compiled for the edge.</span>
        <span className="lp-foot__copy">© {new Date().getFullYear()} Lumen</span>
      </footer>
    </div>
  )
}

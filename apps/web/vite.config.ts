import path from 'node:path'
import { defineConfig } from 'vite'
import { cloudflare } from '@cloudflare/vite-plugin'
import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import viteReact from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

// The Cloudflare Workers target is enabled by adding `@cloudflare/vite-plugin`'s
// `cloudflare()` plugin to the Vite plugin array (ordered BEFORE
// `tanstackStart()`, with `viteEnvironment: { name: 'ssr' }`). This is the
// official mechanism for `@tanstack/react-start` 1.168 + Vite 8 — NOT a
// `target: 'cloudflare-*'` option and NOT a Nitro preset (both belong to older
// Start versions). See:
//   https://developers.cloudflare.com/workers/framework-guides/web-apps/tanstack-start/
//   https://github.com/TanStack/router/tree/main/examples/react/start-basic-cloudflare
//
// Enabled in ALL modes, dev included (not just `vite build`) — `pnpm dev`
// runs the app inside workerd/Miniflare, so `cloudflare:workers`'s `env` is
// real there too (see src/lib/bindings.ts), sourced from wrangler.jsonc's
// `vars`/`hyperdrive.localConnectionString`, overridden by `.dev.vars` for
// local secrets. No separate Node-dev fallback path to keep in sync.
export default defineConfig({
  server: {
    port: 3000,
  },
  resolve: {
    alias: {
      '@': path.resolve(import.meta.dirname, './src'),
    },
  },
  plugins: [
    cloudflare({ viteEnvironment: { name: 'ssr' } }),
    tailwindcss(),
    // Generates routeTree.gen.ts, wires SSR + the file-based router.
    // (TanStack Start's `importProtection` option — which would additionally
    // guard `cloudflare:workers` out of the client bundle — isn't in this
    // pinned version yet. Not load-bearing here: every server-only import
    // (getAuth/getDb/getBindings) only ever appears inside a server
    // function's `.handler()` body, which the compiler already strips from
    // the client bundle. Worth adding once available.)
    tanstackStart(),
    viteReact(),
  ],
})

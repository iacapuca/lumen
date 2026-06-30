import { defineConfig } from 'vite'
import { tanstackStart } from '@tanstack/react-start/plugin/vite'
import viteReact from '@vitejs/plugin-react'

// The Cloudflare Workers target is enabled by adding `@cloudflare/vite-plugin`'s
// `cloudflare()` plugin to the Vite plugin array (ordered BEFORE
// `tanstackStart()`, with `viteEnvironment: { name: 'ssr' }`). This is the
// official mechanism for `@tanstack/react-start` 1.168 + Vite 8 — NOT a
// `target: 'cloudflare-*'` option and NOT a Nitro preset (both belong to older
// Start versions). See:
//   https://developers.cloudflare.com/workers/framework-guides/web-apps/tanstack-start/
//   https://github.com/TanStack/router/tree/main/examples/react/start-basic-cloudflare
//
// We only enable it for `vite build` (and explicit `CLOUDFLARE=1`). Plain
// `pnpm dev` stays a normal Node SSR dev server so the `--env-file` dev script
// and `process.env` keep working unchanged (the Cloudflare plugin would
// otherwise run the app under workerd, where `process.env` is not populated).
export default defineConfig(async ({ command }) => {
  const targetCloudflare = command === 'build' || process.env.CLOUDFLARE === '1'

  const plugins = [
    // Generates routeTree.gen.ts, wires SSR + the file-based router.
    tanstackStart(),
    viteReact(),
  ]

  if (targetCloudflare) {
    const { cloudflare } = await import('@cloudflare/vite-plugin')
    plugins.unshift(cloudflare({ viteEnvironment: { name: 'ssr' } }))
  }

  return {
    server: {
      port: 3000,
    },
    plugins,
  }
})

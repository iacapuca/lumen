import { createRouter as createTanStackRouter } from '@tanstack/react-router'
import { setupRouterSsrQueryIntegration } from '@tanstack/react-router-ssr-query'
import { QueryClient } from '@tanstack/react-query'
import { routeTree } from './routeTree.gen'

export function getRouter() {
  // One QueryClient per getRouter() call — this runs once per SSR request, so
  // a module-level singleton would leak cache entries across requests.
  const queryClient = new QueryClient({
    defaultOptions: {
      // Avoid an immediate client refetch of data SSR just fetched a moment ago.
      queries: { staleTime: 60_000 },
    },
  })

  const router = createTanStackRouter({
    routeTree,
    scrollRestoration: true,
    defaultPreload: 'intent',
    defaultPreloadStaleTime: 0,
    context: { queryClient },
  })

  // Wires automatic dehydrate/hydrate across navigations AND wraps the router
  // in <QueryClientProvider> (via router.options.Wrap) — no manual provider needed.
  setupRouterSsrQueryIntegration({ router, queryClient })

  return router
}

declare module '@tanstack/react-router' {
  interface Register {
    router: ReturnType<typeof getRouter>
  }
}

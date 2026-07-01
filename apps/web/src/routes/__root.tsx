import type { ReactNode } from 'react'
import { HeadContent, Scripts, createRootRouteWithContext } from '@tanstack/react-router'
import type { QueryClient } from '@tanstack/react-query'
import { ReactQueryDevtools } from '@tanstack/react-query-devtools'

import appCss from '../styles.css?url'

export const Route = createRootRouteWithContext<{ queryClient: QueryClient }>()({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1' },
      { title: 'Lumen — Compiled dashboards for the edge' },
    ],
    links: [{ rel: 'stylesheet', href: appCss }],
  }),
  shellComponent: RootDocument,
})

// With `shellComponent`, `children` is the matched route content (the Outlet).
// The app chrome (sidebar/header) now lives in the `_app` layout route so that
// public routes — landing, login, signup — render without it.
function RootDocument({ children }: { children: ReactNode }) {
  return (
    // Single-themed app (always dark) — `dark` is permanent, not a toggle.
    // Activates shadcn/ui's dark-mode CSS variables (see styles.css `.dark`).
    <html lang="en" className="dark">
      <head>
        <HeadContent />
      </head>
      <body>
        {children}
        {import.meta.env.DEV && <ReactQueryDevtools buttonPosition="bottom-left" />}
        <Scripts />
      </body>
    </html>
  )
}

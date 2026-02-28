import 'virtual:uno.css';
import './styles/global.scss';

import { Links, Meta, Outlet, Scripts, ScrollRestoration, useRouteError, isRouteErrorResponse } from 'react-router';
import { Toaster } from 'sonner';
import { useThemeValue } from '@tangle/blueprint-ui';
import { Web3Provider } from '~/providers/Web3Provider';
import { Header } from '~/components/layout/Header';
import { Footer } from '~/components/layout/Footer';

const inlineThemeCode = `
  (function() {
    var theme = localStorage.getItem('bp_theme') || localStorage.getItem('sandbox_cloud_theme');
    if (!theme) {
      theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
    }
    document.querySelector('html').setAttribute('data-theme', theme);
  })();
`;

export function Layout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" data-theme="dark">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <meta name="description" content="Tangle Sandbox Cloud - Provision and manage AI agent sandboxes" />
        <Meta />
        <Links />
        <link rel="icon" href="/favicon.svg" type="image/svg+xml" />
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="anonymous" />
        <link
          rel="stylesheet"
          href="https://fonts.googleapis.com/css2?family=DM+Sans:opsz,wght@9..40,400;500;600;700&family=IBM+Plex+Mono:wght@400;500;600;700&family=Outfit:wght@400;500;600;700;800;900&display=swap"
        />
        <script dangerouslySetInnerHTML={{ __html: inlineThemeCode }} />
      </head>
      <body>
        {children}
        <ScrollRestoration />
        <Scripts />
      </body>
    </html>
  );
}

export function ErrorBoundary() {
  const error = useRouteError();
  const is404 = isRouteErrorResponse(error) && error.status === 404;

  return (
    <div className="flex flex-col min-h-screen bg-cloud-elements-background-depth-1 text-cloud-elements-textPrimary bg-mesh bg-noise items-center justify-center">
      <div className="text-center max-w-md px-6">
        <h1 className="text-4xl font-display font-bold mb-2">
          {is404 ? '404' : 'Something went wrong'}
        </h1>
        <p className="text-cloud-elements-textSecondary mb-6">
          {is404
            ? "The page you're looking for doesn't exist."
            : error instanceof Error
              ? error.message
              : 'An unexpected error occurred.'}
        </p>
        <a
          href="/"
          className="inline-flex items-center gap-2 px-4 py-2 rounded-lg bg-violet-600 text-white text-sm font-medium hover:bg-violet-500 transition-colors"
        >
          Back to Dashboard
        </a>
      </div>
    </div>
  );
}

export default function App() {
  const theme = useThemeValue();

  return (
    <>
      <Toaster
        position="bottom-right"
        theme={theme as 'light' | 'dark' | 'system'}
        richColors
        closeButton
        duration={3000}
        toastOptions={{
          style: {
            background: 'var(--glass-bg-strong)',
            backdropFilter: 'blur(16px)',
            border: '1px solid var(--glass-border)',
            color: 'var(--cloud-elements-textPrimary)',
          },
        }}
      />
      <Web3Provider>
        <div className="flex flex-col min-h-screen bg-cloud-elements-background-depth-1 text-cloud-elements-textPrimary bg-mesh bg-noise">
          <Header />
          <main className="flex-1 pt-[var(--header-height)] relative z-1">
            <Outlet />
          </main>
          <Footer />
        </div>
      </Web3Provider>
    </>
  );
}

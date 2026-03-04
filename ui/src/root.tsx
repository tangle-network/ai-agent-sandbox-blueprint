// Must be the FIRST import — polyfills crypto.randomUUID before wagmi/ConnectKit
// initialize. ESM hoists imports above inline code, so the polyfill must live in
// its own module to run before wallet libraries evaluate.
import './polyfills';

import 'virtual:uno.css';
import './styles/global.scss';
import '~/lib/blueprints'; // side-effect: register all blueprints

import { Outlet, useRouteError, isRouteErrorResponse } from 'react-router';
import { AppDocument, AppFooter, AppToaster } from '@tangle-network/blueprint-ui/components';
import { Web3Provider } from '~/providers/Web3Provider';
import { Header } from '~/components/layout/Header';

export function Layout({ children }: { children: React.ReactNode }) {
  return (
    <AppDocument
      description="Tangle Sandbox Cloud - Provision and manage AI agent sandboxes"
      themeStorageKeys={['bp_theme', 'sandbox_cloud_theme']}
    >
      {children}
    </AppDocument>
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
  return (
    <>
      <AppToaster tone="cloud" />
      <Web3Provider>
        <div className="flex flex-col min-h-screen bg-cloud-elements-background-depth-1 text-cloud-elements-textPrimary bg-mesh bg-noise">
          <Header />
          <main className="flex-1 pt-[var(--header-height)] relative z-1">
            <Outlet />
          </main>
          <AppFooter tone="cloud" brandText="Sandbox Cloud · Tangle Network" />
        </div>
      </Web3Provider>
    </>
  );
}

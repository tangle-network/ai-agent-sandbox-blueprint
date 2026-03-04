import { reactRouter } from '@react-router/dev/vite';
import UnoCSS from 'unocss/vite';
import tsconfigPaths from 'vite-tsconfig-paths';
import { defineConfig, type Plugin } from 'vite';
import path from 'node:path';
import { createRequire } from 'node:module';

// Force linked packages (blueprint-ui) to share our exact wagmi/react copies.
// Without this, blueprint-ui resolves its own wagmi 3.5.0 from its node_modules,
// creating a separate React context that ConnectKit can't reach.
const cjsRequire = createRequire(import.meta.url);
function pkgDir(id: string) {
  // Try resolving package.json first (fastest), fall back to entry point traversal
  try {
    return path.dirname(cjsRequire.resolve(`${id}/package.json`));
  } catch {
    // Some packages don't export package.json — resolve entry and walk up
    const entry = cjsRequire.resolve(id);
    let dir = path.dirname(entry);
    const target = id.startsWith('@') ? id : id.split('/')[0];
    while (dir.length > 1 && !dir.endsWith(`/node_modules/${target}`)) {
      dir = path.dirname(dir);
    }
    return dir;
  }
}

// Provide a full browser DOM environment for SSR module evaluation.
// Some workspace packages (agent-ui) reference browser globals like
// `document`, `window`, `localStorage` at module scope.  React Router
// dev still evaluates the module graph server-side even with `ssr: false`
// (for route metadata extraction), so we install happy-dom before any
// app modules are loaded.
function ssrDomShim(): Plugin {
  return {
    name: 'ssr-dom-shim',
    enforce: 'pre',
    async configureServer() {
      // Wallet connector packages (e.g. family/connectkit) throw unhandled
      // rejections during SSR when they fail to connect.  Prevent these
      // from crashing the dev server.
      process.on('unhandledRejection', (reason: any) => {
        const msg = reason?.message ?? String(reason);
        if (msg.includes('Family Accounts') || msg.includes('EIP1193') || msg.includes('connection timeout')) {
          // Swallow known SSR-irrelevant wallet errors
          return;
        }
        // Re-throw unknown rejections
        console.error('[ssr-dom-shim] Unhandled rejection:', reason);
      });
      // Polyfill crypto.randomUUID for SSR (used by connectkit/wagmi deps)
      if (typeof globalThis.crypto === 'undefined') {
        const { webcrypto } = await import('node:crypto');
        (globalThis as any).crypto = webcrypto;
      } else if (typeof globalThis.crypto.randomUUID !== 'function') {
        const nodeCrypto = await import('node:crypto');
        (globalThis.crypto as any).randomUUID = () => nodeCrypto.randomUUID();
      }

      if (typeof globalThis.document === 'undefined') {
        const { Window } = await import('happy-dom');
        const win = new Window({ url: 'http://localhost:1338' });
        for (const key of ['document', 'window', 'navigator', 'location',
          'localStorage', 'sessionStorage', 'HTMLElement', 'CustomEvent',
          'Event', 'MutationObserver', 'IntersectionObserver', 'ResizeObserver',
          'requestAnimationFrame', 'cancelAnimationFrame', 'getComputedStyle',
          'matchMedia', 'URL', 'URLSearchParams',
        ] as const) {
          if (typeof (globalThis as any)[key] === 'undefined' && (win as any)[key] != null) {
            (globalThis as any)[key] = typeof (win as any)[key] === 'function' && !(win as any)[key].prototype
              ? ((win as any)[key] as Function).bind(win)
              : (win as any)[key];
          }
        }
      }
    },
  };
}

function clientChunks(): Plugin {
  return {
    name: 'client-chunks',
    config(_, { isSsrBuild }) {
      if (!isSsrBuild) {
        return {
          build: {
            rollupOptions: {
              output: {
                manualChunks: {
                  'react-vendor': ['react', 'react-dom', 'react-router'],
                  'web3-vendor': ['wagmi', 'viem', '@tanstack/react-query', 'connectkit', 'framer-motion'],
                  'terminal-vendor': ['@xterm/xterm', '@xterm/addon-fit', '@xterm/addon-web-links'],
                },
              },
            },
          },
        };
      }
    },
  };
}

export default defineConfig({
  plugins: [
    ssrDomShim(),
    UnoCSS(),
    reactRouter(),
    tsconfigPaths(),
    clientChunks(),
  ],
  define: {
    global: 'globalThis',
  },
  server: {
    proxy: {
      '/rpc-proxy': {
        target: process.env.VITE_RPC_URL || 'http://127.0.0.1:8645',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/rpc-proxy/, ''),
      },
    },
  },
  ssr: {
    // Force Vite to bundle workspace-linked packages during SSR module
    // evaluation instead of trying to resolve them via Node.
    noExternal: [
      '@tangle-network/agent-ui',
      '@tangle/blueprint-ui',
    ],
  },
  resolve: {
    alias: {
      events: 'events',
      // Pin wagmi to a single copy so linked blueprint-ui (which has wagmi
      // 3.5.0) doesn't create a separate WagmiContext that ConnectKit can't find.
      wagmi: pkgDir('wagmi'),
    },
    dedupe: [
      '@nanostores/react',
      '@radix-ui/react-dialog',
      '@radix-ui/react-select',
      '@radix-ui/react-separator',
      '@radix-ui/react-slot',
      '@radix-ui/react-tabs',
      '@tangle-network/agent-ui',
      'blo',
      'class-variance-authority',
      'clsx',
      'framer-motion',
      'nanostores',
      'react',
      'react-dom',
      'tailwind-merge',
      'viem',
      'wagmi',
    ],
  },
});

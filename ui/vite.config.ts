import { reactRouter } from '@react-router/dev/vite';
import UnoCSS from 'unocss/vite';
import tsconfigPaths from 'vite-tsconfig-paths';
import { defineConfig, type Plugin } from 'vite';

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
                  'web3-vendor': ['wagmi', 'viem', '@tanstack/react-query', 'connectkit'],
                  'motion-vendor': ['framer-motion'],
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
    UnoCSS(),
    reactRouter(),
    tsconfigPaths(),
    clientChunks(),
  ],
  define: {
    global: 'globalThis',
  },
  resolve: {
    alias: {
      events: 'events',
    },
  },
});

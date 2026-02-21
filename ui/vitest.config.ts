import { defineConfig } from 'vitest/config';
import tsconfigPaths from 'vite-tsconfig-paths';

export default defineConfig({
  plugins: [tsconfigPaths()],
  resolve: {
    alias: {
      // @tangle/agent-ui is an optional peer dep of blueprint-ui — stub it
      // so the optimizer doesn't fail when pre-bundling blueprint-ui.
      '@tangle/agent-ui': new URL('./src/test/stubs/agent-ui.ts', import.meta.url).pathname,
    },
    dedupe: [
      'class-variance-authority',
      'clsx',
      'nanostores',
      '@nanostores/react',
      'tailwind-merge',
      'viem',
      'wagmi',
      'react',
      'react-dom',
    ],
  },
  test: {
    environment: 'jsdom',
    globals: true,
    include: ['src/**/*.test.{ts,tsx}'],
    setupFiles: ['src/test/setup.ts'],
    deps: {
      optimizer: {
        web: {
          include: ['@tangle/blueprint-ui'],
          exclude: ['@tangle/agent-ui'],
        },
      },
    },
  },
});

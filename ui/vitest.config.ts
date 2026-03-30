import { defineConfig } from 'vitest/config';
import tsconfigPaths from 'vite-tsconfig-paths';

export default defineConfig({
  plugins: [tsconfigPaths()],
  resolve: {
    alias: [
      {
        find: '@tangle-network/agent-ui/primitives',
        replacement: new URL('./src/test/stubs/agent-ui-primitives.ts', import.meta.url).pathname,
      },
      {
        find: '@tangle-network/agent-ui/terminal',
        replacement: new URL('./src/test/stubs/agent-ui-terminal.tsx', import.meta.url).pathname,
      },
      {
        // @tangle-network/agent-ui is an optional peer dep of blueprint-ui — stub it
        // so the optimizer doesn't fail when pre-bundling blueprint-ui.
        find: '@tangle-network/agent-ui',
        replacement: new URL('./src/test/stubs/agent-ui.ts', import.meta.url).pathname,
      },
    ],
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
          include: ['@tangle-network/blueprint-ui'],
          exclude: ['@tangle-network/agent-ui'],
        },
      },
    },
  },
});

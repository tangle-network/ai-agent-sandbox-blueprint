import { defineConfig, devices } from '@playwright/test';

/**
 * Playwright E2E configuration for Sandbox Cloud UI.
 *
 * Tests run against the dev server (pnpm dev) or a static preview build.
 * Agent-driven tests require an LLM API key — set OPENAI_API_KEY or
 * LITELLM_BASE_URL + LITELLM_MASTER_KEY.
 *
 * Usage:
 *   pnpm test:e2e              # headless
 *   pnpm test:e2e --headed     # headed (visible browser)
 *   pnpm test:e2e --ui         # interactive Playwright UI
 */

const PORT = Number(process.env.E2E_PORT ?? 1338);
const BASE_URL = process.env.E2E_BASE_URL ?? `http://localhost:${PORT}`;

export default defineConfig({
  testDir: './e2e',
  testMatch: '**/*.spec.ts',
  fullyParallel: false,
  retries: 0,
  workers: 1,
  timeout: 5 * 60 * 1000, // 5 min default
  reporter: [['html', { open: 'never' }], ['list']],

  use: {
    baseURL: BASE_URL,
    screenshot: 'only-on-failure',
    trace: 'retain-on-failure',
    video: 'retain-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  webServer: process.env.E2E_BASE_URL
    ? undefined
    : {
        command: 'pnpm dev',
        port: PORT,
        reuseExistingServer: true,
        timeout: 30_000,
      },
});

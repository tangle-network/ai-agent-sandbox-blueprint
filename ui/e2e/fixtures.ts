/**
 * Playwright fixtures for Sandbox Cloud E2E tests.
 *
 * Provides:
 *   - page: standard Playwright page
 *   - operatorApi: helper for calling operator API endpoints
 *   - agentTools: lazy-loaded agent-browser-driver tools
 */

import { test as base, expect } from '@playwright/test';
import { E2E_CONFIG, brainConfig, hasApiKey } from './e2e.config.js';

export { E2E_CONFIG, hasApiKey, brainConfig };
export { expect };

/** Operator API helper — thin wrapper for fetch calls to the operator. */
interface OperatorApi {
  /** GET from operator API */
  get(path: string, token: string): Promise<Response>;
  /** POST to operator API */
  post(path: string, body: Record<string, unknown>, token: string): Promise<Response>;
}

function createOperatorApi(baseUrl: string): OperatorApi {
  return {
    get: (path, token) =>
      fetch(`${baseUrl}${path}`, {
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      }),
    post: (path, body, token) =>
      fetch(`${baseUrl}${path}`, {
        method: 'POST',
        headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      }),
  };
}

/** Extended test fixtures */
export const test = base.extend<{
  operatorApi: OperatorApi;
  instanceOperatorApi: OperatorApi;
}>({
  operatorApi: async ({}, use) => {
    await use(createOperatorApi(E2E_CONFIG.operatorApiUrl));
  },
  instanceOperatorApi: async ({}, use) => {
    await use(createOperatorApi(E2E_CONFIG.instanceOperatorApiUrl));
  },
});

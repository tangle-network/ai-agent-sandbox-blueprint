/**
 * E2E test configuration — resolved from environment variables.
 *
 * Required env vars:
 *   E2E_BASE_URL or E2E_PORT — the sandbox cloud UI URL
 *   OPERATOR_API_URL — operator API for lifecycle ops
 *
 * For agent-driven tests:
 *   OPENAI_API_KEY — or —
 *   LITELLM_BASE_URL + LITELLM_MASTER_KEY
 */

export const E2E_CONFIG = {
  /** UI base URL */
  baseUrl: process.env.E2E_BASE_URL ?? `http://localhost:${process.env.E2E_PORT ?? 1338}`,

  /** Operator API base URL */
  operatorApiUrl: process.env.OPERATOR_API_URL ?? 'http://localhost:9090',

  /** Instance operator API base URL (may differ from sandbox operator) */
  instanceOperatorApiUrl: process.env.INSTANCE_OPERATOR_API_URL ?? process.env.OPERATOR_API_URL ?? 'http://localhost:9091',

  /** Max timeout for a single agent turn */
  agentTurnTimeout: Number(process.env.E2E_AGENT_TIMEOUT ?? 120_000),

  /** LiteLLM config (alternative to direct OpenAI) */
  litellmBaseUrl: process.env.LITELLM_BASE_URL,
  litellmMasterKey: process.env.LITELLM_MASTER_KEY,
} as const;

/** Whether an LLM API key is available for agent-driven tests */
export const hasApiKey = !!(
  process.env.OPENAI_API_KEY ||
  (E2E_CONFIG.litellmBaseUrl && E2E_CONFIG.litellmMasterKey)
);

/** Resolve agent brain config */
export const brainConfig = {
  apiKey: process.env.OPENAI_API_KEY || E2E_CONFIG.litellmMasterKey || '',
  baseUrl: E2E_CONFIG.litellmBaseUrl || undefined,
  model: process.env.E2E_MODEL || (E2E_CONFIG.litellmBaseUrl ? 'openai/gpt-4o' : 'gpt-4o'),
};

import { useQuery, type UseQueryResult } from '@tanstack/react-query';

/**
 * Live readiness check against an operator's advertised runtime endpoint.
 *
 * Probes the three unauthenticated runtime endpoints exposed by
 * `sandbox-runtime` (see `sandbox-runtime/src/operator_api.rs`):
 *
 *   GET /health            → { status, checks: { runtime, store }, runtime_backend }
 *   GET /readyz            → 200 { status: "ready" } | 503 { status: "not_ready", ... }
 *   GET /api/capabilities  → { capabilities[], harnesses[] }
 *
 * The node is only reported "ready" when /readyz answers 200. /health and
 * /api/capabilities enrich the report so the operator can see *which*
 * backend and harnesses their node actually advertises.
 */

export interface OperatorHealthCheck {
  status: string;
}

export interface OperatorHealthResponse {
  status: string;
  checks?: {
    runtime?: OperatorHealthCheck;
    store?: OperatorHealthCheck;
  };
  runtime_backend?: string;
  runtime_error?: string | null;
}

export interface OperatorReadyResponse {
  status: string;
  runtime_backend?: string;
  runtime?: boolean;
  store?: boolean;
  runtime_error?: string | null;
}

export interface RuntimeCapability {
  id: string;
  label: string;
  description: string;
}

export interface HarnessCapability {
  id: string;
  label: string;
  mcp: boolean;
  skills: boolean;
  subagents: boolean;
}

export interface OperatorCapabilitiesResponse {
  capabilities: RuntimeCapability[];
  harnesses: HarnessCapability[];
}

export type ProbeState = 'idle' | 'pending' | 'ok' | 'error';

export interface ProbeResult<T> {
  state: ProbeState;
  status?: number;
  data?: T;
  error?: string;
}

export interface OperatorReadiness {
  reachable: boolean;
  ready: boolean;
  health: ProbeResult<OperatorHealthResponse>;
  readyz: ProbeResult<OperatorReadyResponse>;
  capabilities: ProbeResult<OperatorCapabilitiesResponse>;
}

const PROBE_TIMEOUT_MS = 6_000;

function normalizeBase(endpoint: string): string {
  return endpoint.trim().replace(/\/+$/, '');
}

/** Add scheme if the operator pasted a bare host:port. */
export function coerceEndpoint(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return '';
  if (/^https?:\/\//i.test(trimmed)) return trimmed;
  return `http://${trimmed}`;
}

async function probe<T>(url: string): Promise<ProbeResult<T>> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), PROBE_TIMEOUT_MS);
  try {
    const res = await fetch(url, {
      method: 'GET',
      headers: { Accept: 'application/json' },
      signal: controller.signal,
    });
    let data: T | undefined;
    try {
      data = (await res.json()) as T;
    } catch {
      data = undefined;
    }
    return {
      state: res.ok ? 'ok' : 'error',
      status: res.status,
      data,
      error: res.ok ? undefined : `HTTP ${res.status}`,
    };
  } catch (err) {
    const aborted = err instanceof DOMException && err.name === 'AbortError';
    return {
      state: 'error',
      error: aborted
        ? `No response within ${PROBE_TIMEOUT_MS / 1000}s`
        : err instanceof Error
          ? err.message
          : 'Request failed',
    };
  } finally {
    clearTimeout(timer);
  }
}

export function useOperatorReadiness(
  endpoint: string,
  enabled: boolean,
): UseQueryResult<OperatorReadiness, Error> {
  const base = normalizeBase(coerceEndpoint(endpoint));

  return useQuery<OperatorReadiness, Error>({
    queryKey: ['operator-readiness', base],
    enabled: enabled && base.length > 0,
    refetchInterval: enabled && base.length > 0 ? 8_000 : false,
    retry: false,
    queryFn: async () => {
      const [health, readyz, capabilities] = await Promise.all([
        probe<OperatorHealthResponse>(`${base}/health`),
        probe<OperatorReadyResponse>(`${base}/readyz`),
        probe<OperatorCapabilitiesResponse>(`${base}/api/capabilities`),
      ]);

      const reachable =
        health.state === 'ok' ||
        readyz.state === 'ok' ||
        capabilities.state === 'ok' ||
        // A 503 from /readyz still proves the node is answering.
        typeof readyz.status === 'number';

      const ready = readyz.state === 'ok' && readyz.data?.status === 'ready';

      return { reachable, ready, health, readyz, capabilities };
    },
  });
}

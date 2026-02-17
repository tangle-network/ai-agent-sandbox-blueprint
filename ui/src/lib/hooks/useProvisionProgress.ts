import { useCallback, useEffect, useRef, useState } from 'react';

// ---------------------------------------------------------------------------
// Types matching sandbox-runtime/src/provision_progress.rs
// ---------------------------------------------------------------------------

export type ProvisionPhase =
  | 'queued'
  | 'image_pull'
  | 'container_create'
  | 'container_start'
  | 'health_check'
  | 'ready'
  | 'failed';

export interface ProvisionStatus {
  call_id: number;
  sandbox_id: string | null;
  phase: ProvisionPhase;
  message: string | null;
  started_at: number;
  updated_at: number;
  progress_pct: number;
}

const PHASE_LABELS: Record<ProvisionPhase, string> = {
  queued: 'Queued',
  image_pull: 'Pulling image',
  container_create: 'Creating container',
  container_start: 'Starting container',
  health_check: 'Health check',
  ready: 'Ready',
  failed: 'Failed',
};

export function getPhaseLabel(phase: ProvisionPhase): string {
  return PHASE_LABELS[phase] ?? phase;
}

export function isTerminalPhase(phase: ProvisionPhase): boolean {
  return phase === 'ready' || phase === 'failed';
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

const POLL_INTERVAL = 2000;

interface UseProvisionProgressOptions {
  callId: number | null;
  apiUrl?: string;
  enabled?: boolean;
}

export function useProvisionProgress({
  callId,
  apiUrl,
  enabled = true,
}: UseProvisionProgressOptions) {
  const [status, setStatus] = useState<ProvisionStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isPolling, setIsPolling] = useState(false);
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const baseUrl = apiUrl ?? import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090';

  const fetchProgress = useCallback(async () => {
    if (callId == null) return;

    try {
      const res = await fetch(`${baseUrl}/api/provisions/${callId}`);
      if (res.status === 404) {
        // Not yet tracked — keep polling
        return;
      }
      if (!res.ok) {
        throw new Error(`HTTP ${res.status}`);
      }
      const data: ProvisionStatus = await res.json();
      setStatus(data);
      setError(null);

      // Stop polling on terminal phase
      if (isTerminalPhase(data.phase) && intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
        setIsPolling(false);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to fetch provision status');
    }
  }, [callId, baseUrl]);

  useEffect(() => {
    if (!enabled || callId == null) return;

    setIsPolling(true);
    fetchProgress(); // Initial fetch

    intervalRef.current = setInterval(fetchProgress, POLL_INTERVAL);

    return () => {
      if (intervalRef.current) {
        clearInterval(intervalRef.current);
        intervalRef.current = null;
      }
      setIsPolling(false);
    };
  }, [enabled, callId, fetchProgress]);

  return {
    status,
    error,
    isPolling,
    phase: status?.phase ?? null,
    progressPct: status?.progress_pct ?? 0,
    sandboxId: status?.sandbox_id ?? null,
    message: status?.message ?? null,
    isReady: status?.phase === 'ready',
    isFailed: status?.phase === 'failed',
  };
}

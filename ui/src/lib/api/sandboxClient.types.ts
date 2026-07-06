/**
 * Public type definitions for the unified sandbox API client.
 *
 * Consumers import these via `~/lib/api/sandboxClient`, which re-exports them.
 */

export type ClientMode = 'direct' | 'proxied';

export interface SandboxClientConfig {
  mode: ClientMode;
  /** Direct mode: sidecar URL (e.g. http://localhost:32768) */
  sidecarUrl?: string;
  /** Direct mode: sidecar auth token */
  sidecarToken?: string;
  /** Proxied mode: operator API URL (e.g. http://localhost:9090) */
  operatorApiUrl?: string;
  /** Proxied mode: session PASETO token */
  sessionToken?: string;
  /** Proxied mode: lazily resolve a fresh session token */
  sessionTokenProvider?: () => Promise<string | null>;
  /** Sandbox ID for proxied mode routing */
  sandboxId?: string;
  /** Proxied mode: explicit resource path prefix (e.g. `/api/sandbox`) */
  resourcePath?: string;
}

export interface ChatSessionSummary {
  session_id: string;
  title: string;
  active_run_id?: string | null;
}

export interface ChatRunSummary {
  id: string;
  session_id: string;
  kind: 'prompt' | 'task';
  status: 'queued' | 'running' | 'cancelling' | 'completed' | 'failed' | 'cancelled' | 'interrupted';
  request_text: string;
  created_at: number;
  started_at?: number | null;
  completed_at?: number | null;
  sidecar_session_id?: string | null;
  trace_id?: string | null;
  final_output?: string | null;
  error?: string | null;
}

export interface ChatSessionDetail {
  session_id: string;
  title: string;
  sidecar_session_id?: string | null;
  active_run_id?: string | null;
  messages: Array<{
    id: string;
    run_id?: string | null;
    role: string;
    content: string;
    created_at?: number;
    completed_at?: number | null;
    parts?: Array<Record<string, unknown>>;
    trace_id?: string | null;
    success?: boolean | null;
    error?: string | null;
  }>;
  run_progress?: Array<{
    seq?: number;
    run_id?: string | null;
    status?: ChatRunSummary['status'] | string;
    phase?: string;
    message?: string;
    timestamp_ms?: number;
  }>;
  runs: ChatRunSummary[];
}

export interface ExecResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export interface PromptResult {
  accepted?: boolean;
  response?: string;
  runId?: string;
  sessionId?: string;
  status?: string;
  acceptedAt?: number;
}

export interface TaskResult {
  accepted?: boolean;
  response?: string;
  runId?: string;
  sessionId?: string;
  status?: string;
  acceptedAt?: number;
  isComplete?: boolean;
}

export interface ChatStreamEvent {
  type: string;
  data: unknown;
}

export interface CancelChatRunResult {
  success: boolean;
  sessionId: string;
  runId: string;
  status: ChatRunSummary['status'] | string;
  cancelledAt: number;
}

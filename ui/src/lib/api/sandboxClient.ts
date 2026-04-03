/**
 * Unified sandbox API client.
 *
 * Supports two modes:
 * - `direct`: Talk directly to the sidecar URL with sidecar auth token
 * - `proxied`: Talk to the operator API with session auth (PASETO) token
 *
 * Browser-facing code should prefer proxied mode. Direct sidecar access is
 * retained for compatibility with non-browser callers and older integrations.
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

interface OperatorErrorBody {
  error?: string;
  code?: string;
  retry_after_ms?: number;
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

const CHAT_REQUEST_TIMEOUT_MS = 15_000;
const CHAT_STREAM_CONNECT_TIMEOUT_MS = 10_000;
const DEFAULT_PROMPT_RUN_TIMEOUT_MS = 10 * 60 * 1000;
const DEFAULT_TASK_RUN_TIMEOUT_MS = 30 * 60 * 1000;

export class SandboxClient {
  private config: SandboxClientConfig;

  constructor(config: SandboxClientConfig) {
    this.config = config;
  }

  private get baseUrl(): string {
    if (this.config.mode === 'direct') {
      return this.config.sidecarUrl ?? '';
    }
    return this.config.operatorApiUrl ?? '';
  }

  private get proxiedResourcePath(): string {
    if (this.config.resourcePath) return this.config.resourcePath;
    if (this.config.sandboxId) return `/api/sandboxes/${encodeURIComponent(this.config.sandboxId)}`;
    return '/api/sandbox';
  }

  private async resolveAuthHeaders(includeJsonContentType = true): Promise<Record<string, string>> {
    const headers: Record<string, string> = {};
    if (includeJsonContentType) headers['Content-Type'] = 'application/json';

    if (this.config.mode === 'direct') {
      if (this.config.sidecarToken) {
        headers.Authorization = `Bearer ${this.config.sidecarToken}`;
      }
      return headers;
    }

    const token =
      this.config.sessionToken
      ?? (this.config.sessionTokenProvider
        ? await this.config.sessionTokenProvider()
        : null);
    if (!token) {
      throw new Error('Operator session token unavailable');
    }
    headers.Authorization = `Bearer ${token}`;
    return headers;
  }

  private formatApiFailure(
    operation: 'Prompt' | 'Task',
    status: number,
    errorBody: string,
  ): Error {
    try {
      const parsed = JSON.parse(errorBody) as OperatorErrorBody;
      if (parsed.code === 'AGENT_WARMING_UP') {
        const retryMs = typeof parsed.retry_after_ms === 'number' ? parsed.retry_after_ms : null;
        const retryHint = retryMs && retryMs > 0
          ? ` Retry in about ${Math.ceil(retryMs / 1000)}s.`
          : '';
        return new Error(
          `${operation} failed (${status}): ${parsed.error ?? 'Sandbox agent is still starting up.'}${retryHint}`,
        );
      }
      if (parsed.code === 'CIRCUIT_BREAKER') {
        const retryMs = typeof parsed.retry_after_ms === 'number' ? parsed.retry_after_ms : null;
        const retryHint = retryMs && retryMs > 0
          ? ` Retrying in ~${Math.ceil(retryMs / 1000)}s.`
          : '';
        return new Error(
          `${operation} failed: Sidecar is temporarily unreachable (circuit breaker active).${retryHint}`,
        );
      }
    } catch {
      // Fall back to the raw response body for non-JSON errors.
    }

    return new Error(`${operation} failed (${status}): ${errorBody}`);
  }

  private createTimedSignal(timeoutMs: number, upstreamSignal?: AbortSignal) {
    const controller = new AbortController();
    let timedOut = false;
    const timeoutId = globalThis.setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, timeoutMs);

    const abortFromUpstream = () => controller.abort();
    if (upstreamSignal) {
      if (upstreamSignal.aborted) {
        controller.abort();
      } else {
        upstreamSignal.addEventListener('abort', abortFromUpstream, { once: true });
      }
    }

    return {
      signal: controller.signal,
      didTimeout: () => timedOut,
      cleanup: () => {
        globalThis.clearTimeout(timeoutId);
        if (upstreamSignal) {
          upstreamSignal.removeEventListener('abort', abortFromUpstream);
        }
      },
    };
  }

  private async fetchWithTimeout(
    url: string,
    init: RequestInit,
    timeoutMs: number,
    timeoutMessage: string,
  ): Promise<Response> {
    const timeout = this.createTimedSignal(timeoutMs, init.signal ?? undefined);
    try {
      const res = await fetch(url, { ...init, signal: timeout.signal });
      return res;
    } catch (error) {
      if (timeout.didTimeout()) {
        throw new Error(timeoutMessage);
      }
      throw error;
    } finally {
      timeout.cleanup();
    }
  }

  private parseSseFrame(frame: string): ChatStreamEvent | null {
    let eventType = 'message';
    const dataLines: string[] = [];

    for (const line of frame.split('\n')) {
      if (line.startsWith('event:')) {
        eventType = line.slice(6).trim();
        continue;
      }
      if (line.startsWith('data:')) {
        dataLines.push(line.slice(5).trimStart());
      }
    }

    if (dataLines.length === 0) {
      return null;
    }

    const rawData = dataLines.join('\n');
    try {
      return {
        type: eventType,
        data: JSON.parse(rawData),
      };
    } catch {
      return {
        type: eventType,
        data: rawData,
      };
    }
  }

  /** Execute a shell command in the sandbox. */
  async exec(command: string): Promise<ExecResult> {
    const url =
      this.config.mode === 'direct'
        ? `${this.baseUrl}/terminals/commands`
        : `${this.baseUrl}${this.proxiedResourcePath}/exec`;

    const res = await fetch(url, {
      method: 'POST',
      headers: await this.resolveAuthHeaders(true),
      body: JSON.stringify({ command }),
    });

    if (!res.ok) {
      const body = await res.text();
      throw new Error(`Exec failed (${res.status}): ${body}`);
    }

    const data = await res.json();
    return {
      stdout: data.stdout ?? data.output ?? '',
      stderr: data.stderr ?? '',
      exitCode: data.exitCode ?? data.exit_code ?? 0,
    };
  }

  /** Send a prompt to the sandbox agent. */
  async prompt(text: string, systemPrompt?: string, sessionId?: string): Promise<PromptResult> {
    const url =
      this.config.mode === 'direct'
        ? `${this.baseUrl}/agent/prompt`
        : `${this.baseUrl}${this.proxiedResourcePath}/prompt`;

    const body: Record<string, unknown> = this.config.mode === 'direct'
      ? { prompt: text }
      : { message: text };

    if (this.config.mode === 'direct') {
      if (systemPrompt) body.system_prompt = systemPrompt;
    } else if (systemPrompt) {
      body.context_json = JSON.stringify({ system_prompt: systemPrompt });
    }
    if (sessionId?.trim()) body.session_id = sessionId;
    if (this.config.mode === 'proxied') body.timeout_ms = DEFAULT_PROMPT_RUN_TIMEOUT_MS;

    const res = await this.fetchWithTimeout(url, {
      method: 'POST',
      headers: await this.resolveAuthHeaders(true),
      body: JSON.stringify(body),
    }, CHAT_REQUEST_TIMEOUT_MS, 'Prompt request timed out while waiting for the operator to accept the run.');

    if (!res.ok) {
      const errorBody = await res.text();
      throw this.formatApiFailure('Prompt', res.status, errorBody);
    }

    const data = await res.json();
    if (this.config.mode === 'proxied') {
      return {
        accepted: data.accepted ?? true,
        runId: data.run_id ?? data.runId,
        sessionId: data.session_id ?? data.sessionId ?? sessionId,
        status: data.status,
        acceptedAt: data.accepted_at ?? data.acceptedAt,
      };
    }
    return {
      response: data.response ?? data.text ?? '',
      sessionId: data.session_id ?? data.sessionId ?? sessionId,
    };
  }

  /** Submit an autonomous task to the sandbox agent. */
  async task(description: string, systemPrompt?: string, sessionId?: string): Promise<TaskResult> {
    const url =
      this.config.mode === 'direct'
        ? `${this.baseUrl}/agent/task`
        : `${this.baseUrl}${this.proxiedResourcePath}/task`;

    const body: Record<string, unknown> = this.config.mode === 'direct'
      ? { task: description }
      : { prompt: description };

    if (this.config.mode === 'direct') {
      if (systemPrompt) body.system_prompt = systemPrompt;
    } else if (systemPrompt) {
      body.context_json = JSON.stringify({ system_prompt: systemPrompt });
    }
    if (sessionId?.trim()) body.session_id = sessionId;
    if (this.config.mode === 'proxied') body.timeout_ms = DEFAULT_TASK_RUN_TIMEOUT_MS;

    const res = await this.fetchWithTimeout(url, {
      method: 'POST',
      headers: await this.resolveAuthHeaders(true),
      body: JSON.stringify(body),
    }, CHAT_REQUEST_TIMEOUT_MS, 'Task request timed out while waiting for the operator to accept the run.');

    if (!res.ok) {
      const errorBody = await res.text();
      throw this.formatApiFailure('Task', res.status, errorBody);
    }

    const data = await res.json();
    if (this.config.mode === 'proxied') {
      return {
        accepted: data.accepted ?? true,
        runId: data.run_id ?? data.runId,
        sessionId: data.session_id ?? data.sessionId ?? sessionId,
        status: data.status,
        acceptedAt: data.accepted_at ?? data.acceptedAt,
      };
    }
    return {
      response: data.result ?? data.response ?? data.text ?? '',
      sessionId: data.session_id ?? data.sessionId ?? sessionId,
      isComplete: data.is_complete ?? data.isComplete ?? true,
    };
  }

  /** Check sandbox health. */
  async health(): Promise<boolean> {
    try {
      const url =
        this.config.mode === 'direct'
          ? `${this.baseUrl}/health`
          : `${this.baseUrl}/health`;

      const res = await fetch(url, { headers: await this.resolveAuthHeaders(false) });
      return res.ok;
    } catch {
      return false;
    }
  }

  // ---------------------------------------------------------------------------
  // Chat session CRUD (proxied mode only)
  // ---------------------------------------------------------------------------

  private get chatSessionsBasePath(): string {
    if (this.config.mode === 'direct') {
      throw new Error('Chat session management is only available in proxied mode');
    }
    return `${this.baseUrl}${this.proxiedResourcePath}/live/chat/sessions`;
  }

  private getChatSessionPath(sessionId: string): string {
    return `${this.chatSessionsBasePath}/${encodeURIComponent(sessionId)}`;
  }

  private getChatRunCancelPath(sessionId: string, runId: string): string {
    return `${this.getChatSessionPath(sessionId)}/runs/${encodeURIComponent(runId)}/cancel`;
  }

  /** List all chat sessions for this resource. */
  async listChatSessions(): Promise<ChatSessionSummary[]> {
    const res = await fetch(this.chatSessionsBasePath, {
      headers: await this.resolveAuthHeaders(false),
    });
    if (!res.ok) {
      const body = await res.text();
      throw new Error(`List chat sessions failed (${res.status}): ${body}`);
    }
    const data = await res.json();
    return data.sessions ?? [];
  }

  /** Create a new chat session. */
  async createChatSession(title: string = 'New Chat'): Promise<ChatSessionSummary> {
    const res = await fetch(this.chatSessionsBasePath, {
      method: 'POST',
      headers: await this.resolveAuthHeaders(true),
      body: JSON.stringify({ title }),
    });
    if (!res.ok) {
      const body = await res.text();
      throw new Error(`Create chat session failed (${res.status}): ${body}`);
    }
    return await res.json();
  }

  /** Get a chat session with its message history. */
  async getChatSession(sessionId: string): Promise<ChatSessionDetail> {
    const url = this.getChatSessionPath(sessionId);
    const res = await fetch(url, {
      headers: await this.resolveAuthHeaders(false),
    });
    if (!res.ok) {
      const body = await res.text();
      throw new Error(`Get chat session failed (${res.status}): ${body}`);
    }
    return await res.json();
  }

  /** Delete a chat session. */
  async deleteChatSession(sessionId: string): Promise<void> {
    const url = this.getChatSessionPath(sessionId);
    const res = await fetch(url, {
      method: 'DELETE',
      headers: await this.resolveAuthHeaders(false),
    });
    if (!res.ok) {
      const body = await res.text();
      throw new Error(`Delete chat session failed (${res.status}): ${body}`);
    }
  }

  async cancelChatRun(sessionId: string, runId: string): Promise<CancelChatRunResult> {
    if (this.config.mode === 'direct') {
      throw new Error('Chat run cancellation is only available in proxied mode');
    }

    const res = await this.fetchWithTimeout(
      this.getChatRunCancelPath(sessionId, runId),
      {
        method: 'POST',
        headers: await this.resolveAuthHeaders(false),
      },
      CHAT_REQUEST_TIMEOUT_MS,
      'Cancel request timed out while waiting for the operator to acknowledge the run cancellation.',
    );

    if (!res.ok) {
      const body = await res.text();
      throw new Error(`Cancel chat run failed (${res.status}): ${body}`);
    }

    const data = await res.json();
    return {
      success: data.success ?? true,
      sessionId: data.session_id ?? data.sessionId ?? sessionId,
      runId: data.run_id ?? data.runId ?? runId,
      status: data.status ?? 'cancelled',
      cancelledAt: data.cancelled_at ?? data.cancelledAt ?? Date.now(),
    };
  }

  async streamChatSession(
    sessionId: string,
    options: {
      signal?: AbortSignal;
      onOpen?: () => void;
      onEvent: (event: ChatStreamEvent) => void;
    },
  ): Promise<void> {
    if (this.config.mode === 'direct') {
      throw new Error('Chat session streaming is only available in proxied mode');
    }

    const res = await this.fetchWithTimeout(
      `${this.getChatSessionPath(sessionId)}/stream`,
      {
        headers: await this.resolveAuthHeaders(false),
        signal: options.signal,
      },
      CHAT_STREAM_CONNECT_TIMEOUT_MS,
      'Chat stream connection timed out while opening the live session stream.',
    );

    if (!res.ok) {
      const body = await res.text();
      throw new Error(`Chat stream failed (${res.status}): ${body}`);
    }

    if (!res.body) {
      throw new Error('Chat stream is unavailable');
    }

    options.onOpen?.();

    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';

    try {
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buffer += decoder.decode(value, { stream: true });
        const frames = buffer.split('\n\n');
        buffer = frames.pop() ?? '';

        for (const frame of frames) {
          const parsed = this.parseSseFrame(frame);
          if (parsed) {
            options.onEvent(parsed);
          }
        }
      }
    } finally {
      reader.releaseLock();
    }
  }
}

/**
 * Create a direct-mode client from sidecar URL + token.
 *
 * @deprecated Browser features should use operator-proxied access instead of
 * direct sidecar access. This helper is retained for compatibility only.
 */
export function createDirectClient(sidecarUrl: string, sidecarToken: string): SandboxClient {
  return new SandboxClient({
    mode: 'direct',
    sidecarUrl,
    sidecarToken,
  });
}

/** Create a proxied-mode client via operator API. */
export function createProxiedClient(
  sandboxId: string,
  sessionTokenOrProvider: string | (() => Promise<string | null>),
  operatorApiUrl?: string,
): SandboxClient {
  return new SandboxClient({
    mode: 'proxied',
    sandboxId,
    sessionToken: typeof sessionTokenOrProvider === 'string' ? sessionTokenOrProvider : undefined,
    sessionTokenProvider:
      typeof sessionTokenOrProvider === 'function' ? sessionTokenOrProvider : undefined,
    operatorApiUrl: operatorApiUrl ?? import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090',
  });
}

/** Create a proxied-mode client for singleton instance endpoints (`/api/sandbox/*`). */
export function createProxiedInstanceClient(
  sessionTokenOrProvider: string | (() => Promise<string | null>),
  operatorApiUrl?: string,
): SandboxClient {
  return new SandboxClient({
    mode: 'proxied',
    resourcePath: '/api/sandbox',
    sessionToken: typeof sessionTokenOrProvider === 'string' ? sessionTokenOrProvider : undefined,
    sessionTokenProvider:
      typeof sessionTokenOrProvider === 'function' ? sessionTokenOrProvider : undefined,
    operatorApiUrl: operatorApiUrl ?? import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090',
  });
}

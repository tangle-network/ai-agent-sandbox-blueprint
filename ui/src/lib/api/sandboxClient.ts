/**
 * Unified sandbox API client.
 *
 * Supports two modes:
 * - `direct`: Talk directly to the sidecar URL with sidecar auth token
 * - `proxied`: Talk to the operator API with session auth (PASETO) token
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
  /** Sandbox ID for proxied mode routing */
  sandboxId?: string;
}

export interface ExecResult {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export interface PromptResult {
  response: string;
  sessionId?: string;
}

export interface TaskResult {
  response: string;
  sessionId?: string;
  isComplete: boolean;
}

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

  private get headers(): Record<string, string> {
    const h: Record<string, string> = {
      'Content-Type': 'application/json',
    };

    if (this.config.mode === 'direct' && this.config.sidecarToken) {
      h['Authorization'] = `Bearer ${this.config.sidecarToken}`;
    } else if (this.config.mode === 'proxied' && this.config.sessionToken) {
      h['Authorization'] = `Bearer ${this.config.sessionToken}`;
    }

    return h;
  }

  /** Execute a shell command in the sandbox. */
  async exec(command: string): Promise<ExecResult> {
    const url =
      this.config.mode === 'direct'
        ? `${this.baseUrl}/terminals/commands`
        : `${this.baseUrl}/api/sandboxes/${this.config.sandboxId}/exec`;

    const res = await fetch(url, {
      method: 'POST',
      headers: this.headers,
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
        : `${this.baseUrl}/api/sandboxes/${this.config.sandboxId}/prompt`;

    const body: Record<string, unknown> = { prompt: text };
    if (systemPrompt) body.system_prompt = systemPrompt;
    if (sessionId) body.session_id = sessionId;

    const res = await fetch(url, {
      method: 'POST',
      headers: this.headers,
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      const errorBody = await res.text();
      throw new Error(`Prompt failed (${res.status}): ${errorBody}`);
    }

    const data = await res.json();
    return {
      response: data.response ?? data.text ?? '',
      sessionId: data.session_id ?? data.sessionId,
    };
  }

  /** Submit an autonomous task to the sandbox agent. */
  async task(description: string, systemPrompt?: string, sessionId?: string): Promise<TaskResult> {
    const url =
      this.config.mode === 'direct'
        ? `${this.baseUrl}/agent/task`
        : `${this.baseUrl}/api/sandboxes/${this.config.sandboxId}/task`;

    const body: Record<string, unknown> = { task: description };
    if (systemPrompt) body.system_prompt = systemPrompt;
    if (sessionId) body.session_id = sessionId;

    const res = await fetch(url, {
      method: 'POST',
      headers: this.headers,
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      const errorBody = await res.text();
      throw new Error(`Task failed (${res.status}): ${errorBody}`);
    }

    const data = await res.json();
    return {
      response: data.response ?? data.text ?? '',
      sessionId: data.session_id ?? data.sessionId,
      isComplete: data.is_complete ?? data.isComplete ?? true,
    };
  }

  /** Check sandbox health. */
  async health(): Promise<boolean> {
    try {
      const url =
        this.config.mode === 'direct'
          ? `${this.baseUrl}/health`
          : `${this.baseUrl}/api/sandboxes/${this.config.sandboxId}/health`;

      const res = await fetch(url, { headers: this.headers });
      return res.ok;
    } catch {
      return false;
    }
  }
}

/** Create a direct-mode client from sidecar URL + token. */
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
  sessionToken: string,
  operatorApiUrl?: string,
): SandboxClient {
  return new SandboxClient({
    mode: 'proxied',
    sandboxId,
    sessionToken,
    operatorApiUrl: operatorApiUrl ?? import.meta.env.VITE_OPERATOR_API_URL ?? 'http://localhost:9090',
  });
}

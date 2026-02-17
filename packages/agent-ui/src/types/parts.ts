// ---------------------------------------------------------------------------
// Part primitives — generic equivalents of blueprint-agent's
// DevContainerSessionPart, decoupled from @opencode-ai/sdk.
// ---------------------------------------------------------------------------

export interface TextPart {
  type: 'text';
  text: string;
  /** If true this text was synthesised client-side (e.g. echo of user input). */
  synthetic?: boolean;
}

// -- Tool parts -------------------------------------------------------------

export type ToolStatus = 'pending' | 'running' | 'completed' | 'error';

export interface ToolTime {
  start?: number;
  end?: number;
}

export interface ToolState {
  status: ToolStatus;
  input?: unknown;
  output?: unknown;
  error?: string;
  metadata?: Record<string, unknown>;
  time?: ToolTime;
}

export interface ToolPart {
  type: 'tool';
  /** Unique ID for this tool invocation. */
  id: string;
  /** Tool name (e.g. "bash", "read", "write", "grep", "glob"). */
  tool: string;
  state: ToolState;
  callID?: string;
}

// -- Reasoning parts --------------------------------------------------------

export interface ReasoningPart {
  type: 'reasoning';
  text: string;
  time?: ToolTime;
}

// -- Union ------------------------------------------------------------------

export type SessionPart = TextPart | ToolPart | ReasoningPart;

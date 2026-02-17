import type { SessionMessage } from './message';

/** Broad category of a tool invocation, used for display grouping. */
export type ToolCategory =
  | 'command'
  | 'write'
  | 'read'
  | 'search'
  | 'edit'
  | 'task'
  | 'web'
  | 'todo'
  | 'other';

export interface RunStats {
  toolCount: number;
  messageCount: number;
  thinkingDurationMs: number;
  textPartCount: number;
  toolCategories: Set<ToolCategory>;
}

export interface FinalTextPart {
  messageId: string;
  partIndex: number;
  text: string;
}

/**
 * A Run is a consecutive group of assistant messages that form one
 * logical "turn" of the agent. Runs are collapsible in the UI and
 * show a summary header when collapsed.
 */
export interface Run {
  id: string;
  messages: SessionMessage[];
  isComplete: boolean;
  isStreaming: boolean;
  stats: RunStats;
  summaryText: string | null;
  finalTextPart: FinalTextPart | null;
}

// -- Grouped messages for rendering -----------------------------------------

export interface MessageRun {
  type: 'run';
  run: Run;
}

export interface MessageUser {
  type: 'user';
  message: SessionMessage;
}

export type GroupedMessage = MessageRun | MessageUser;

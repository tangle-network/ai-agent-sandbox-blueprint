/** A single message in a chat session. */
export interface SessionMessage {
  id: string;
  role: 'user' | 'assistant' | 'system';
  sessionID?: string;
  time?: {
    created?: number;
    updated?: number;
    completed?: number;
  };
  /** Monotonically increasing insertion index for stable ordering. */
  _insertionIndex?: number;
}

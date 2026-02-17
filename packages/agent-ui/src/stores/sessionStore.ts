import { atom } from 'nanostores';

export interface ChatSession {
  /** Sidecar HTTP base URL, e.g. "http://localhost:8080". */
  sidecarUrl: string;
  /** Auth token for the sidecar API. */
  token: string;
  /** Unique session identifier. */
  sessionId: string;
}

/** The currently active chat session (null when disconnected). */
export const sessionAtom = atom<ChatSession | null>(null);

export function connectSession(session: ChatSession) {
  sessionAtom.set(session);
}

export function disconnectSession() {
  sessionAtom.set(null);
}

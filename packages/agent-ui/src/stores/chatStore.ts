import { atom, map } from 'nanostores';
import type { SessionMessage } from '~/types/message';
import type { SessionPart } from '~/types/parts';

/** Ordered list of messages in the current chat session. */
export const messagesAtom = atom<SessionMessage[]>([]);

/** Map of message ID → parts for that message. */
export const partMapAtom = map<Record<string, SessionPart[]>>({});

/** Whether the assistant is currently streaming a response. */
export const isStreamingAtom = atom(false);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

let insertionCounter = 0;

export function addMessage(msg: SessionMessage) {
  const withIndex = { ...msg, _insertionIndex: insertionCounter++ };
  messagesAtom.set([...messagesAtom.get(), withIndex]);
}

export function addParts(messageId: string, parts: SessionPart[]) {
  const current = partMapAtom.get();
  const existing = current[messageId] ?? [];
  partMapAtom.setKey(messageId, [...existing, ...parts]);
}

export function updatePart(messageId: string, partIndex: number, part: SessionPart) {
  const current = partMapAtom.get();
  const existing = [...(current[messageId] ?? [])];
  existing[partIndex] = part;
  partMapAtom.setKey(messageId, existing);
}

export function clearChat() {
  messagesAtom.set([]);
  partMapAtom.set({});
  isStreamingAtom.set(false);
  insertionCounter = 0;
}

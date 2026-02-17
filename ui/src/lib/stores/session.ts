import { atom, computed } from 'nanostores';
import { persistedAtom } from './persistedAtom';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface SessionEntry {
  token: string;
  address: string;
  expiresAt: number;
  sandboxId: string;
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/**
 * Persisted session tokens keyed by sandbox ID.
 * Auto-cleaned on read if expired.
 */
export const sessionMapStore = persistedAtom<Record<string, SessionEntry>>({
  key: 'sandbox_cloud_sessions',
  initial: {},
});

/** Active session for a given sandbox. */
export function getSession(sandboxId: string): SessionEntry | null {
  const map = sessionMapStore.get();
  const entry = map[sandboxId];
  if (!entry) return null;

  // Check expiry with 60s buffer
  if (Date.now() / 1000 > entry.expiresAt - 60) {
    removeSession(sandboxId);
    return null;
  }

  return entry;
}

export function setSession(entry: SessionEntry) {
  const map = { ...sessionMapStore.get() };
  map[entry.sandboxId] = entry;
  sessionMapStore.set(map);
}

export function removeSession(sandboxId: string) {
  const map = { ...sessionMapStore.get() };
  delete map[sandboxId];
  sessionMapStore.set(map);
}

/** Clean up all expired sessions. */
export function gcSessions() {
  const now = Date.now() / 1000;
  const map = sessionMapStore.get();
  const cleaned: Record<string, SessionEntry> = {};
  let changed = false;

  for (const [key, entry] of Object.entries(map)) {
    if (entry.expiresAt > now) {
      cleaned[key] = entry;
    } else {
      changed = true;
    }
  }

  if (changed) {
    sessionMapStore.set(cleaned);
  }
}

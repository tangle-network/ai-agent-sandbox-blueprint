export type ActionTab =
  | 'overview'
  | 'terminal'
  | 'chat'
  | 'ssh'
  | 'secrets'
  | 'attestation'
  | 'automation'
  | 'storage';

export interface SshKey {
  username: string;
  publicKey: string;
}

export interface AgentDescriptor {
  identifier: string;
  displayName?: string;
  description?: string;
}

export function getInitialTabFromPath(pathname: string): ActionTab {
  if (pathname.endsWith('/runtime')) return 'terminal';
  if (pathname.endsWith('/sessions')) return 'chat';
  if (pathname.endsWith('/automation')) return 'automation';
  if (pathname.endsWith('/network')) return 'ssh';
  if (pathname.endsWith('/security')) return 'secrets';
  if (pathname.endsWith('/storage')) return 'storage';
  return 'overview';
}

export function getCurrentPathname() {
  return typeof window === 'undefined' ? '' : window.location.pathname;
}

/** Extract human-readable error from operator API Error messages. */
export function parseApiError(err: Error): string {
  const idx = err.message.indexOf('): ');
  if (idx === -1) return err.message;
  const body = err.message.slice(idx + 3);
  try {
    const parsed = JSON.parse(body) as { error?: string };
    if (typeof parsed.error === 'string') return parsed.error;
  } catch {
    // ignore non-JSON error bodies
  }
  return err.message;
}

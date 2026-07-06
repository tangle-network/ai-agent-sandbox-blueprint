import { getBlueprint } from '@tangle-network/blueprint-ui';

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
    const parsed = JSON.parse(body);
    if (typeof parsed.error === 'string') return parsed.error;
  } catch { /* not JSON */ }
  return err.message;
}

export function formatBlueprintLabel(blueprintId: string): string {
  const id = blueprintId.trim() || 'ai-agent-sandbox-blueprint';
  return getBlueprint(id)?.name ?? id;
}

export function formatServiceId(serviceId: string): string {
  const trimmed = serviceId.trim();
  if (!trimmed) return 'Not linked';
  if (trimmed.startsWith('#')) return trimmed;
  return /^\d+$/.test(trimmed) ? `#${trimmed}` : trimmed;
}

export function formatDuration(seconds: number): string {
  if (seconds <= 0) return 'Unlimited';
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (hours > 0 && minutes > 0) return `${hours}h ${minutes}m`;
  if (hours > 0) return `${hours} hour${hours !== 1 ? 's' : ''}`;
  if (minutes > 0) return `${minutes} minute${minutes !== 1 ? 's' : ''}`;
  return `${seconds}s`;
}

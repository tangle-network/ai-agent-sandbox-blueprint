import { decodeEventLog, type Address } from 'viem';
import {
  getAddresses,
  publicClient,
  tangleServicesAbi,
  type JobDefinition,
  type JobFieldDef,
} from '@tangle-network/blueprint-ui';
import type { ConsoleMetric } from '~/components/console/ConsolePrimitives';
import type { IdentityMeta } from '~/components/shared/VisualIdentity';
import { extractServiceRequestId } from '~/lib/contracts/serviceEvents';
import {
  INSTANCE_ONCHAIN_BLUEPRINT_ID,
  INSTANCE_ONCHAIN_SERVICE_ID,
  SANDBOX_ONCHAIN_BLUEPRINT_ID,
  SANDBOX_ONCHAIN_SERVICE_ID,
  TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
  TEE_INSTANCE_ONCHAIN_SERVICE_ID,
} from '~/lib/config';

export type ConsoleTone = NonNullable<ConsoleMetric['tone']>;

// ── Blueprint → on-chain ID mapping from env vars ──

export const BLUEPRINT_INFRA: Record<string, { blueprintId: string; serviceId: string }> = {
  'ai-agent-sandbox-blueprint': {
    blueprintId: SANDBOX_ONCHAIN_BLUEPRINT_ID,
    serviceId: SANDBOX_ONCHAIN_SERVICE_ID,
  },
  'ai-agent-instance-blueprint': {
    blueprintId: INSTANCE_ONCHAIN_BLUEPRINT_ID,
    serviceId: INSTANCE_ONCHAIN_SERVICE_ID,
  },
  'ai-agent-tee-instance-blueprint': {
    blueprintId: TEE_INSTANCE_ONCHAIN_BLUEPRINT_ID,
    serviceId: TEE_INSTANCE_ONCHAIN_SERVICE_ID,
  },
};

export const SERVICE_TTL_BLOCKS = 864000n;
export const ZERO_REQUESTER = '0x0000000000000000000000000000000000000000' as const;

export type ServiceReceiptLog = {
  data: `0x${string}`;
  topics: readonly `0x${string}`[];
};

export function getRequestIdFromServiceReceiptLogs(logs: ServiceReceiptLog[]): number | null {
  for (const log of logs) {
    const requestId = extractServiceRequestId(log);
    if (requestId != null) return requestId;
  }

  return null;
}

export async function resolveActivatedServiceId(requestId: number): Promise<string | null> {
  const addrs = getAddresses();
  const logs = await publicClient.getLogs({
    address: addrs.services,
    fromBlock: 0n,
    toBlock: 'latest',
  });

  for (const log of logs) {
    try {
      const decoded = decodeEventLog({
        abi: tangleServicesAbi,
        data: log.data,
        topics: [...log.topics] as [] | [`0x${string}`, ...`0x${string}`[]],
      });
      if (decoded.eventName !== 'ServiceActivated') continue;
      if (!('requestId' in decoded.args) || !('serviceId' in decoded.args)) continue;
      if (Number(decoded.args.requestId) !== requestId) continue;
      return String(decoded.args.serviceId);
    } catch {
      // Ignore unrelated service-manager logs.
    }
  }

  return null;
}

// ── Wizard Steps ──

export type WizardStep = 'blueprint' | 'configure' | 'deploy';
export type ServiceSetupMode = 'existing' | 'new';
export type LaunchSelectOption = { label: string; value: string; detail?: string; identity?: IdentityMeta };
export const CUSTOM_IMAGE_VALUE = '__custom_image__';

export function parsePortsInput(value: string): number[] {
  return value
    .split(',')
    .map((s) => parseInt(s.trim(), 10))
    .filter((n) => n > 0 && n <= 65535);
}

export function parsePositiveServiceId(value: string): bigint | null {
  const trimmed = value.trim();
  if (!/^[1-9]\d*$/.test(trimmed)) return null;

  try {
    return BigInt(trimmed);
  } catch {
    return null;
  }
}

export function isValidAddress(value: string): value is Address {
  return /^0x[a-fA-F0-9]{40}$/.test(value.trim());
}

export function parseCapabilitiesJson(value: unknown): Set<string> {
  if (Array.isArray(value)) {
    return new Set(value.filter((item): item is string => typeof item === 'string'));
  }
  try {
    const parsed = JSON.parse(String(value || '[]'));
    return Array.isArray(parsed)
      ? new Set(parsed.filter((item): item is string => typeof item === 'string'))
      : new Set();
  } catch {
    return new Set();
  }
}

export function setCapabilityJson(value: unknown, capability: string, enabled: boolean): string {
  const capabilities = parseCapabilitiesJson(value);
  if (enabled) {
    capabilities.add(capability);
  } else {
    capabilities.delete(capability);
  }
  return JSON.stringify(Array.from(capabilities).sort());
}

export function formatCapacityValue(value: number | bigint | undefined) {
  if (value == null) return '--';
  return typeof value === 'bigint' ? value.toString() : String(value);
}

export function runtimeLabel(value: string) {
  if (value === 'firecracker') return 'Firecracker';
  if (value === 'tee') return 'TEE';
  return 'Docker';
}

export function field(job: JobDefinition | null, name: string): JobFieldDef | undefined {
  return job?.fields.find((item) => item.name === name);
}

export function fieldOptions(job: JobDefinition | null, name: string): { label: string; value: string }[] {
  return field(job, name)?.options ?? [];
}

export function valueString(values: Record<string, unknown>, name: string, fallback = ''): string {
  const value = values[name];
  if (value === undefined || value === null) return fallback;
  return String(value);
}

export function valueNumber(values: Record<string, unknown>, name: string, fallback: number): number {
  const raw = Number(values[name]);
  return Number.isFinite(raw) ? raw : fallback;
}

export function clampNumber(value: number, min?: number, max?: number): number {
  if (typeof min === 'number' && value < min) return min;
  if (typeof max === 'number' && value > max) return max;
  return value;
}

export function formatImageOptionLabel(value: string, fallback: string) {
  const image = value.toLowerCase();
  if (image.includes('blueprint-sidecar')) {
    const tag = value.includes(':') ? value.split(':').pop() : '';
    return tag ? `Tangle sidecar: ${tag}` : 'Tangle sidecar';
  }
  if (image.startsWith('ghcr.io/tangle-network/')) {
    return value.replace(/^ghcr\.io\/tangle-network\//, 'Tangle image: ');
  }
  if (image.startsWith('ghcr.io/')) {
    return value.replace(/^ghcr\.io\//, 'GHCR: ');
  }
  return fallback;
}

export function hoursFromSeconds(value: unknown, fallbackSeconds: number): number {
  const seconds = Number(value);
  return Math.max(0, Math.round((Number.isFinite(seconds) ? seconds : fallbackSeconds) / 3600));
}

export function minutesFromSeconds(value: unknown, fallbackSeconds: number): number {
  const seconds = Number(value);
  return Math.max(0, Math.round((Number.isFinite(seconds) ? seconds : fallbackSeconds) / 60));
}

export function serviceTone({
  serviceValidating,
  serviceError,
  hasValidService,
  isNewService,
}: {
  serviceValidating: boolean;
  serviceError: string | null;
  hasValidService: boolean;
  isNewService: boolean;
}): ConsoleTone {
  if (serviceError) return 'danger';
  if (serviceValidating) return 'warn';
  if (hasValidService || isNewService) return 'ready';
  return 'muted';
}

export const executionMetricToneClass: Record<ConsoleTone, string> = {
  brand: 'text-[var(--sandbox-console-brand)]',
  ready: 'text-[var(--sandbox-console-success)]',
  warn: 'text-[var(--sandbox-console-warning)]',
  danger: 'text-[var(--sandbox-console-danger)]',
  muted: 'text-[var(--sandbox-console-text)]',
};

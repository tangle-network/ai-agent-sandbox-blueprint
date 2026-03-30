export interface AgentOption {
  label: string;
  value: string;
}

export const BUNDLED_NO_AGENT_VALUE = '__none__';

export const BUNDLED_SANDBOX_IMAGE_VALUES = [
  'agent-dev:latest',
  'agent-dev',
  'tangle-sidecar:local',
  'ghcr.io/tangle-network/sidecar:latest',
] as const;

export const BUNDLED_AGENT_OPTIONS: AgentOption[] = [
  { label: 'None (compute only)', value: BUNDLED_NO_AGENT_VALUE },
  { label: 'Default', value: 'default' },
  { label: 'Batch', value: 'batch' },
];

const BUNDLED_AGENT_VALUE_SET = new Set(BUNDLED_AGENT_OPTIONS.map((option) => option.value));

function normalizeString(value: unknown): string {
  if (typeof value === 'string') return value.trim();
  if (value == null) return '';
  return String(value).trim();
}

export function normalizeAgentIdentifier(value: unknown): string {
  return normalizeString(value);
}

export function isBundledSandboxImage(value: unknown): boolean {
  const image = normalizeString(value);
  return BUNDLED_SANDBOX_IMAGE_VALUES.includes(image as (typeof BUNDLED_SANDBOX_IMAGE_VALUES)[number]);
}

export function sanitizeBundledAgentIdentifier(value: unknown): string {
  const identifier = normalizeAgentIdentifier(value);
  if (identifier === BUNDLED_NO_AGENT_VALUE) return '';
  return BUNDLED_AGENT_VALUE_SET.has(identifier) ? identifier : '';
}

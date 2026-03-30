/**
 * Shared test fixtures and helpers for blueprint/encoder tests.
 *
 * Reusable primitives — every test file imports from here
 * instead of duplicating factory functions.
 */

import type { JobDefinition, JobFieldDef, AbiContextParam, JobCategory } from '@tangle-network/blueprint-ui';

// ── Job factory ──

const JOB_DEFAULTS: JobDefinition = {
  id: 99,
  name: 'test_job',
  label: 'Test Job',
  description: 'Test job for unit tests',
  category: 'execution',
  icon: 'i-ph:flask',
  pricingMultiplier: 1,
  requiresSandbox: false,
  fields: [],
};

/** Create a minimal JobDefinition with sensible defaults */
export function makeJob(overrides: Partial<JobDefinition> = {}): JobDefinition {
  return { ...JOB_DEFAULTS, ...overrides };
}

/** Create a JobFieldDef with sensible defaults */
export function makeField(overrides: Partial<JobFieldDef> & Pick<JobFieldDef, 'name' | 'type'>): JobFieldDef {
  return {
    label: overrides.name.charAt(0).toUpperCase() + overrides.name.slice(1),
    ...overrides,
  };
}

// ── ABI shape definitions (mirroring Rust sol! macros) ──
// Used by integration tests to verify TS encoding matches Rust decoding.

export type AbiParamDef = { name: string; type: string; components?: AbiParamDef[] };

/** SandboxCreateRequest — 16 fields */
export const SANDBOX_CREATE_ABI: AbiParamDef[] = [
  { name: 'name', type: 'string' },
  { name: 'image', type: 'string' },
  { name: 'stack', type: 'string' },
  { name: 'agent_identifier', type: 'string' },
  { name: 'env_json', type: 'string' },
  { name: 'metadata_json', type: 'string' },
  { name: 'ssh_enabled', type: 'bool' },
  { name: 'ssh_public_key', type: 'string' },
  { name: 'web_terminal_enabled', type: 'bool' },
  { name: 'max_lifetime_seconds', type: 'uint64' },
  { name: 'idle_timeout_seconds', type: 'uint64' },
  { name: 'cpu_cores', type: 'uint64' },
  { name: 'memory_mb', type: 'uint64' },
  { name: 'disk_gb', type: 'uint64' },
  { name: 'tee_required', type: 'bool' },
  { name: 'tee_type', type: 'uint8' },
];

/** SandboxIdRequest — sandbox_id context param */
export const SANDBOX_ID_ABI: AbiParamDef[] = [
  { name: 'sandbox_id', type: 'string' },
];

/** WorkflowCreateRequest */
export const WORKFLOW_CREATE_ABI: AbiParamDef[] = [
  { name: 'name', type: 'string' },
  { name: 'workflow_json', type: 'string' },
  { name: 'trigger_type', type: 'string' },
  { name: 'trigger_config', type: 'string' },
  { name: 'sandbox_config_json', type: 'string' },
  { name: 'target_kind', type: 'uint8' },
  { name: 'target_sandbox_id', type: 'string' },
  { name: 'target_service_id', type: 'uint64' },
];

/** WorkflowControlRequest */
export const WORKFLOW_CONTROL_ABI: AbiParamDef[] = [
  { name: 'workflow_id', type: 'uint64' },
];

/** ProvisionRequest — canonical instance shape without sidecar_token */
export const INSTANCE_PROVISION_ABI: AbiParamDef[] = [
  { name: 'name', type: 'string' },
  { name: 'image', type: 'string' },
  { name: 'stack', type: 'string' },
  { name: 'agent_identifier', type: 'string' },
  { name: 'env_json', type: 'string' },
  { name: 'metadata_json', type: 'string' },
  { name: 'ssh_enabled', type: 'bool' },
  { name: 'ssh_public_key', type: 'string' },
  { name: 'web_terminal_enabled', type: 'bool' },
  { name: 'max_lifetime_seconds', type: 'uint64' },
  { name: 'idle_timeout_seconds', type: 'uint64' },
  { name: 'cpu_cores', type: 'uint64' },
  { name: 'memory_mb', type: 'uint64' },
  { name: 'disk_gb', type: 'uint64' },
  { name: 'tee_required', type: 'bool' },
  { name: 'tee_type', type: 'uint8' },
];

/** JsonRequest (deprovision) */
export const JSON_REQUEST_ABI: AbiParamDef[] = [
  { name: 'json', type: 'string' },
];

// ── Typical form values for round-trip tests ──

/** Sandbox create form values matching all 16 fields */
export const SANDBOX_CREATE_VALUES: Record<string, unknown> = {
  name: 'test-sandbox',
  image: 'agent-dev:latest',
  stack: 'default',
  agentIdentifier: 'agent-1',
  envJson: '{"KEY":"val"}',
  metadataJson: '{}',
  sshEnabled: false,
  sshPublicKey: '',
  webTerminalEnabled: true,
  maxLifetimeSeconds: 86400,
  idleTimeoutSeconds: 3600,
  cpuCores: 2,
  memoryMb: 2048,
  diskGb: 10,
  teeRequired: false,
  teeType: '0',
};

/** Instance provision form values */
export const INSTANCE_PROVISION_VALUES: Record<string, unknown> = {
  ...SANDBOX_CREATE_VALUES,
  name: 'test-instance',
};

/** Workflow create form values */
export const WORKFLOW_CREATE_VALUES: Record<string, unknown> = {
  name: 'daily-backup',
  workflowJson: '{"steps":[]}',
  triggerType: 'cron',
  triggerConfig: '0 */6 * * *',
  sandboxConfigJson: '{"image":"agent-dev:latest"}',
  targetKind: '0',
  targetSandboxId: 'sb-test-001',
  targetServiceId: 1,
};

/** Context with sandbox_id */
export const SANDBOX_ID_CONTEXT: Record<string, unknown> = {
  sandbox_id: 'sb-test-001',
};

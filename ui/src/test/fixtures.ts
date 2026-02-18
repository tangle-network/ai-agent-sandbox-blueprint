/**
 * Shared test fixtures and helpers for blueprint/encoder tests.
 *
 * Reusable primitives — every test file imports from here
 * instead of duplicating factory functions.
 */

import type { JobDefinition, JobFieldDef, AbiContextParam, JobCategory } from '~/lib/blueprints/registry';

// ── Job factory ──

const JOB_DEFAULTS: JobDefinition = {
  id: 99,
  name: 'test_job',
  label: 'Test Job',
  description: 'Test job for unit tests',
  category: 'execution',
  icon: 'i-ph:test',
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
// These are the canonical ABI param arrays for each Rust request struct.
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

/** SandboxSnapshotRequest */
export const SANDBOX_SNAPSHOT_ABI: AbiParamDef[] = [
  { name: 'sidecar_url', type: 'string' },
  { name: 'destination', type: 'string' },
  { name: 'include_workspace', type: 'bool' },
  { name: 'include_state', type: 'bool' },
];

/** SandboxExecRequest */
export const SANDBOX_EXEC_ABI: AbiParamDef[] = [
  { name: 'sidecar_url', type: 'string' },
  { name: 'command', type: 'string' },
  { name: 'cwd', type: 'string' },
  { name: 'env_json', type: 'string' },
  { name: 'timeout_ms', type: 'uint64' },
];

/** SandboxPromptRequest */
export const SANDBOX_PROMPT_ABI: AbiParamDef[] = [
  { name: 'sidecar_url', type: 'string' },
  { name: 'message', type: 'string' },
  { name: 'session_id', type: 'string' },
  { name: 'model', type: 'string' },
  { name: 'context_json', type: 'string' },
  { name: 'timeout_ms', type: 'uint64' },
];

/** SandboxTaskRequest */
export const SANDBOX_TASK_ABI: AbiParamDef[] = [
  { name: 'sidecar_url', type: 'string' },
  { name: 'prompt', type: 'string' },
  { name: 'session_id', type: 'string' },
  { name: 'max_turns', type: 'uint64' },
  { name: 'model', type: 'string' },
  { name: 'context_json', type: 'string' },
  { name: 'timeout_ms', type: 'uint64' },
];

/** BatchTaskRequest */
export const BATCH_TASK_ABI: AbiParamDef[] = [
  { name: 'sidecar_urls', type: 'string[]' },
  { name: 'prompt', type: 'string' },
  { name: 'session_id', type: 'string' },
  { name: 'max_turns', type: 'uint64' },
  { name: 'model', type: 'string' },
  { name: 'context_json', type: 'string' },
  { name: 'timeout_ms', type: 'uint64' },
  { name: 'parallel', type: 'bool' },
  { name: 'aggregation', type: 'string' },
];

/** BatchExecRequest */
export const BATCH_EXEC_ABI: AbiParamDef[] = [
  { name: 'sidecar_urls', type: 'string[]' },
  { name: 'command', type: 'string' },
  { name: 'cwd', type: 'string' },
  { name: 'env_json', type: 'string' },
  { name: 'timeout_ms', type: 'uint64' },
  { name: 'parallel', type: 'bool' },
];

/** BatchCollectRequest */
export const BATCH_COLLECT_ABI: AbiParamDef[] = [
  { name: 'batch_id', type: 'string' },
];

/** BatchCreateRequest (with nested SandboxCreateRequest) */
export const BATCH_CREATE_ABI: AbiParamDef[] = [
  { name: 'count', type: 'uint32' },
  { name: 'template_request', type: 'tuple', components: SANDBOX_CREATE_ABI },
  { name: 'operators', type: 'address[]' },
  { name: 'distribution', type: 'string' },
];

/** WorkflowCreateRequest */
export const WORKFLOW_CREATE_ABI: AbiParamDef[] = [
  { name: 'name', type: 'string' },
  { name: 'workflow_json', type: 'string' },
  { name: 'trigger_type', type: 'string' },
  { name: 'trigger_config', type: 'string' },
  { name: 'sandbox_config_json', type: 'string' },
];

/** WorkflowControlRequest */
export const WORKFLOW_CONTROL_ABI: AbiParamDef[] = [
  { name: 'workflow_id', type: 'uint64' },
];

/** SshProvisionRequest / SshRevokeRequest */
export const SSH_REQUEST_ABI: AbiParamDef[] = [
  { name: 'sidecar_url', type: 'string' },
  { name: 'username', type: 'string' },
  { name: 'public_key', type: 'string' },
];

// ── Instance blueprint ABIs (no sidecar_url context) ──

/** ProvisionRequest — includes sidecar_token before TEE fields */
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
  { name: 'sidecar_token', type: 'string' },
  { name: 'tee_required', type: 'bool' },
  { name: 'tee_type', type: 'uint8' },
];

/** InstanceExecRequest */
export const INSTANCE_EXEC_ABI: AbiParamDef[] = [
  { name: 'command', type: 'string' },
  { name: 'cwd', type: 'string' },
  { name: 'env_json', type: 'string' },
  { name: 'timeout_ms', type: 'uint64' },
];

/** InstancePromptRequest */
export const INSTANCE_PROMPT_ABI: AbiParamDef[] = [
  { name: 'message', type: 'string' },
  { name: 'session_id', type: 'string' },
  { name: 'model', type: 'string' },
  { name: 'context_json', type: 'string' },
  { name: 'timeout_ms', type: 'uint64' },
];

/** InstanceTaskRequest */
export const INSTANCE_TASK_ABI: AbiParamDef[] = [
  { name: 'prompt', type: 'string' },
  { name: 'session_id', type: 'string' },
  { name: 'max_turns', type: 'uint64' },
  { name: 'model', type: 'string' },
  { name: 'context_json', type: 'string' },
  { name: 'timeout_ms', type: 'uint64' },
];

/** InstanceSshProvisionRequest / InstanceSshRevokeRequest */
export const INSTANCE_SSH_ABI: AbiParamDef[] = [
  { name: 'username', type: 'string' },
  { name: 'public_key', type: 'string' },
];

/** InstanceSnapshotRequest */
export const INSTANCE_SNAPSHOT_ABI: AbiParamDef[] = [
  { name: 'destination', type: 'string' },
  { name: 'include_workspace', type: 'bool' },
  { name: 'include_state', type: 'bool' },
];

/** JsonRequest (deprovision) */
export const JSON_REQUEST_ABI: AbiParamDef[] = [
  { name: 'json', type: 'string' },
];

// ── Typical form values for round-trip tests ──

/** Sandbox create form values matching all 16 fields */
export const SANDBOX_CREATE_VALUES: Record<string, unknown> = {
  name: 'test-sandbox',
  image: 'ubuntu:22.04',
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

/** Instance provision form values (includes sidecarToken as internal) */
export const INSTANCE_PROVISION_VALUES: Record<string, unknown> = {
  ...SANDBOX_CREATE_VALUES,
  name: 'test-instance',
  sidecarToken: '',
};

/** Exec form values */
export const EXEC_VALUES: Record<string, unknown> = {
  command: 'ls -la /workspace',
  cwd: '/workspace',
  envJson: '{}',
  timeoutMs: 30000,
};

/** Prompt form values */
export const PROMPT_VALUES: Record<string, unknown> = {
  message: 'What files are in the workspace?',
  sessionId: 'sess-123',
  model: 'claude-3',
  contextJson: '{}',
  timeoutMs: 60000,
};

/** Task form values */
export const TASK_VALUES: Record<string, unknown> = {
  prompt: 'Build a REST API',
  sessionId: 'sess-456',
  maxTurns: 10,
  model: 'claude-3',
  contextJson: '{}',
  timeoutMs: 300000,
};

/** SSH form values */
export const SSH_VALUES: Record<string, unknown> = {
  username: 'agent',
  publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest',
};

/** Snapshot form values */
export const SNAPSHOT_VALUES: Record<string, unknown> = {
  destination: 's3://bucket/snapshot-001',
  includeWorkspace: true,
  includeState: true,
};

/** Workflow create form values */
export const WORKFLOW_CREATE_VALUES: Record<string, unknown> = {
  name: 'daily-backup',
  workflowJson: '{"steps":[]}',
  triggerType: 'cron',
  triggerConfig: '0 */6 * * *',
  sandboxConfigJson: '{"image":"ubuntu:22.04"}',
};

/** Context with sidecar_url */
export const SIDECAR_CONTEXT: Record<string, unknown> = {
  sidecar_url: 'http://localhost:8080',
};

/** Context with sandbox_id */
export const SANDBOX_ID_CONTEXT: Record<string, unknown> = {
  sandbox_id: 'sb-test-001',
};

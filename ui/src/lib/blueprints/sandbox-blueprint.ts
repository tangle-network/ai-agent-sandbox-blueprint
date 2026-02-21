import { JOB_IDS } from '~/lib/types/sandbox';
import { type BlueprintDefinition, type JobDefinition, registerBlueprint } from './registry';
import type { Address } from 'viem';

/**
 * AI Agent Sandbox Blueprint — the default Tangle sandbox provisioning blueprint.
 *
 * On-chain jobs: sandbox lifecycle (create/delete) and workflows.
 * Read-only ops (exec, prompt, task, ssh, snapshot, stop, resume, batch)
 * are served by the operator API.
 *
 * ABI types verified against ai-agent-sandbox-blueprint-lib/src/lib.rs sol! macros.
 */

// ── Shared context params ──

const SANDBOX_ID_CTX = [{ abiName: 'sandbox_id', abiType: 'string' }] as const;

// ── TEE type options (shared between create and instance provision) ──

export const TEE_TYPE_OPTIONS = [
  { label: 'None', value: '0' },
  { label: 'TDX (Intel)', value: '1' },
  { label: 'Nitro (AWS)', value: '2' },
  { label: 'SEV (AMD)', value: '3' },
];

// ── Jobs ──

const SANDBOX_JOBS: JobDefinition[] = [
  // ── Lifecycle ──
  {
    // ABI: SandboxCreateRequest { name, image, stack, agent_identifier, env_json, metadata_json,
    //   ssh_enabled, ssh_public_key, web_terminal_enabled, max_lifetime_seconds, idle_timeout_seconds,
    //   cpu_cores, memory_mb, disk_gb, tee_required, tee_type }
    id: JOB_IDS.SANDBOX_CREATE,
    name: 'sandbox_create',
    label: 'Create Sandbox',
    description: 'Provision a new AI agent sandbox with Docker isolation, optional SSH, and sidecar.',
    category: 'lifecycle',
    icon: 'i-ph:plus-circle',
    pricingMultiplier: 50,
    requiresSandbox: false,
    fields: [
      { name: 'name', label: 'Sandbox Name', type: 'text', placeholder: 'my-agent-sandbox', required: true, abiType: 'string' },
      { name: 'image', label: 'Docker Image', type: 'combobox', placeholder: 'ubuntu:22.04', required: true, defaultValue: 'ubuntu:22.04', abiType: 'string',
        options: [
          { label: 'Ubuntu 22.04', value: 'ubuntu:22.04' },
          { label: 'Ubuntu 24.04', value: 'ubuntu:24.04' },
          { label: 'Debian Bookworm', value: 'debian:bookworm' },
          { label: 'Python 3.12', value: 'python:3.12' },
          { label: 'Node 22', value: 'node:22' },
          { label: 'Rust (latest)', value: 'rust:latest' },
          { label: 'Alpine 3.20', value: 'alpine:3.20' },
        ],
        helperText: 'Select a preset or enter any Docker Hub image' },
      { name: 'stack', label: 'Stack', type: 'select', defaultValue: 'default', abiType: 'string', options: [
        { label: 'Default', value: 'default' },
        { label: 'Python', value: 'python' },
        { label: 'Node.js', value: 'nodejs' },
        { label: 'Rust', value: 'rust' },
      ] },
      { name: 'agentIdentifier', label: 'Agent Identifier', type: 'text', placeholder: 'agent-1', helperText: 'Unique identifier for the agent in this sandbox', abiType: 'string', abiParam: 'agent_identifier' },
      { name: 'envJson', label: 'Environment Variables (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}', abiType: 'string', abiParam: 'env_json' },
      { name: 'metadataJson', label: 'Metadata (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}', abiType: 'string', abiParam: 'metadata_json' },
      { name: 'sshEnabled', label: 'Enable SSH', type: 'boolean', defaultValue: false, abiType: 'bool', abiParam: 'ssh_enabled' },
      { name: 'sshPublicKey', label: 'SSH Public Key', type: 'textarea', placeholder: 'ssh-ed25519 AAAA...', helperText: 'Required if SSH is enabled', abiType: 'string', abiParam: 'ssh_public_key' },
      { name: 'webTerminalEnabled', label: 'Web Terminal', type: 'boolean', defaultValue: true, abiType: 'bool', abiParam: 'web_terminal_enabled' },
      { name: 'maxLifetimeSeconds', label: 'Max Lifetime (s)', type: 'number', defaultValue: 86400, min: 0, helperText: '0 = unlimited, 3600 = 1h, 86400 = 24h', abiType: 'uint64', abiParam: 'max_lifetime_seconds' },
      { name: 'idleTimeoutSeconds', label: 'Idle Timeout (s)', type: 'number', defaultValue: 3600, min: 0, helperText: '0 = disabled, 300 = 5min, 3600 = 1h', abiType: 'uint64', abiParam: 'idle_timeout_seconds' },
      { name: 'cpuCores', label: 'CPU Cores', type: 'number', defaultValue: 2, min: 1, max: 16, helperText: 'Typical: 1\u20134 for dev, 8\u201316 for production', abiType: 'uint64', abiParam: 'cpu_cores' },
      { name: 'memoryMb', label: 'Memory (MB)', type: 'number', defaultValue: 2048, min: 512, max: 32768, step: 512, helperText: '512 = 0.5 GB, 2048 = 2 GB, 8192 = 8 GB', abiType: 'uint64', abiParam: 'memory_mb' },
      { name: 'diskGb', label: 'Disk (GB)', type: 'number', defaultValue: 10, min: 1, max: 100, helperText: '10 GB typical for dev, 50+ for large models', abiType: 'uint64', abiParam: 'disk_gb' },
      { name: 'teeRequired', label: 'TEE Required', type: 'boolean', defaultValue: false, abiType: 'bool', abiParam: 'tee_required' },
      { name: 'teeType', label: 'TEE Type', type: 'select', defaultValue: '0', abiType: 'uint8', abiParam: 'tee_type', options: TEE_TYPE_OPTIONS },
    ],
  },
  {
    // ABI: SandboxIdRequest { sandbox_id }
    id: JOB_IDS.SANDBOX_DELETE,
    name: 'sandbox_delete',
    label: 'Delete Sandbox',
    description: 'Permanently delete a sandbox and its data.',
    category: 'lifecycle',
    icon: 'i-ph:trash',
    pricingMultiplier: 1,
    requiresSandbox: true,
    contextParams: [...SANDBOX_ID_CTX],
    fields: [],
    warning: 'This action is irreversible. All sandbox data will be permanently deleted.',
  },

  // ── Workflows ──
  {
    // ABI: WorkflowCreateRequest { name, workflow_json, trigger_type, trigger_config, sandbox_config_json }
    id: JOB_IDS.WORKFLOW_CREATE,
    name: 'workflow_create',
    label: 'Create Workflow',
    description: 'Define a scheduled or event-driven workflow with sandbox automation.',
    category: 'workflow',
    icon: 'i-ph:flow-arrow',
    pricingMultiplier: 2,
    requiresSandbox: false,
    fields: [
      { name: 'name', label: 'Workflow Name', type: 'text', required: true, abiType: 'string' },
      { name: 'workflowJson', label: 'Workflow Definition (JSON)', type: 'json', required: true, abiType: 'string', abiParam: 'workflow_json' },
      { name: 'triggerType', label: 'Trigger Type', type: 'select', required: true, abiType: 'string', abiParam: 'trigger_type', options: [
        { label: 'Cron Schedule', value: 'cron' },
        { label: 'Webhook', value: 'webhook' },
        { label: 'Manual', value: 'manual' },
      ] },
      { name: 'triggerConfig', label: 'Trigger Config', type: 'text', placeholder: '0 */6 * * *', helperText: 'Cron expression or webhook URL', abiType: 'string', abiParam: 'trigger_config' },
      { name: 'sandboxConfigJson', label: 'Sandbox Config (JSON)', type: 'json', placeholder: '{}', abiType: 'string', abiParam: 'sandbox_config_json' },
    ],
  },
  {
    // ABI: WorkflowControlRequest { workflow_id }
    id: JOB_IDS.WORKFLOW_TRIGGER,
    name: 'workflow_trigger',
    label: 'Trigger Workflow',
    description: 'Manually trigger an existing workflow.',
    category: 'workflow',
    icon: 'i-ph:play',
    pricingMultiplier: 5,
    requiresSandbox: false,
    fields: [
      { name: 'workflowId', label: 'Workflow ID', type: 'number', required: true, min: 0, abiType: 'uint64', abiParam: 'workflow_id' },
    ],
  },
  {
    // ABI: WorkflowControlRequest { workflow_id }
    id: JOB_IDS.WORKFLOW_CANCEL,
    name: 'workflow_cancel',
    label: 'Cancel Workflow',
    description: 'Deactivate a workflow. Can be re-triggered later.',
    category: 'workflow',
    icon: 'i-ph:stop',
    pricingMultiplier: 1,
    requiresSandbox: false,
    fields: [
      { name: 'workflowId', label: 'Workflow ID', type: 'number', required: true, min: 0, abiType: 'uint64', abiParam: 'workflow_id' },
    ],
  },
];

// ── Blueprint Definition ──

export const SANDBOX_BLUEPRINT: BlueprintDefinition = {
  id: 'ai-agent-sandbox-blueprint',
  name: 'AI Agent Sandbox',
  version: '0.5.0',
  description: 'Provision isolated AI agent sandboxes with Docker, SSH, sidecar AI execution, and scheduled workflows.',
  icon: 'i-ph:cloud',
  color: 'teal',
  contracts: {},
  jobs: SANDBOX_JOBS,
  categories: [
    { key: 'lifecycle', label: 'Sandbox Lifecycle', icon: 'i-ph:hard-drives' },
    { key: 'workflow', label: 'Workflows', icon: 'i-ph:flow-arrow' },
  ],
};

/** Wire addresses from chain configs at init time */
export function initSandboxBlueprint(addressesByChain: Record<number, Address>) {
  SANDBOX_BLUEPRINT.contracts = addressesByChain;
  registerBlueprint(SANDBOX_BLUEPRINT);
}

// Auto-register with empty addresses (will be overridden when chain is known)
registerBlueprint(SANDBOX_BLUEPRINT);

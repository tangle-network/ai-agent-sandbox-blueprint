import { INSTANCE_JOB_IDS, INSTANCE_PRICING_TIERS } from '~/lib/types/instance';
import { type BlueprintDefinition, type JobDefinition, registerBlueprint } from '@tangle-network/blueprint-ui';
import type { Address } from 'viem';
import { RUNTIME_BACKEND_OPTIONS, TEE_TYPE_OPTIONS, SIDECAR_IMAGE_OPTIONS } from './sandbox-blueprint';

/**
 * Creates job definitions for the Instance blueprint family.
 * Shared between Instance and TEE Instance (which differs only in pricing and defaults).
 *
 * On-chain jobs: workflows only.
 * Instance lifecycle is operator-reported (`reportProvisioned`/`reportDeprovisioned`).
 * Read-only ops (exec, prompt, task, ssh, snapshot) are served by the operator API.
 *
 * ABI types verified against ai-agent-instance-blueprint-lib/src/lib.rs sol! macros.
 */
export function createInstanceJobs(opts?: {
  pricingOverrides?: Record<number, number>;
}): JobDefinition[] {
  const pricing = (id: number) => opts?.pricingOverrides?.[id] ?? INSTANCE_PRICING_TIERS[id]?.multiplier ?? 1;

  return [
    {
      // ABI: ProvisionRequest { name, image, stack, agent_identifier, env_json, metadata_json,
      //   ssh_enabled, ssh_public_key, web_terminal_enabled, max_lifetime_seconds,
      //   idle_timeout_seconds, cpu_cores, memory_mb, disk_gb, sidecar_token, tee_required, tee_type }
      // Not an on-chain submitJob target — the encoded fields are passed as requestInputs
      // to requestService (Path B) or used by the operator's auto-provision decoder.
      id: INSTANCE_JOB_IDS.PROVISION,
      name: 'instance_provision',
      label: 'Provision Instance',
      description: 'Configure and provision a new AI agent instance.',
      category: 'lifecycle',
      icon: 'i-ph:plus-circle',
      pricingMultiplier: 50,
      requiresSandbox: false,
      fields: [
        { name: 'name', label: 'Instance Name', type: 'text', placeholder: 'my-agent-instance', required: true, abiType: 'string' },
        { name: 'image', label: 'Docker Image', type: 'combobox', placeholder: 'agent-dev:latest', required: true, defaultValue: 'agent-dev:latest', abiType: 'string',
          options: SIDECAR_IMAGE_OPTIONS,
          helperText: 'Use a sidecar-compatible image that already runs the sandbox server on port 8080.' },
        { name: 'runtimeBackend', label: 'Runtime Backend', type: 'select', defaultValue: 'docker', options: RUNTIME_BACKEND_OPTIONS,
          helperText: 'Merged into metadata_json.runtime_backend for operator-side routing' },
        { name: 'stack', label: 'Stack', type: 'select', defaultValue: 'default', abiType: 'string', options: [
          { label: 'Default', value: 'default' },
          { label: 'Python', value: 'python' },
          { label: 'Node.js', value: 'nodejs' },
          { label: 'Rust', value: 'rust' },
        ] },
        { name: 'agentIdentifier', label: 'Agent Identifier', type: 'text', placeholder: 'default', helperText: 'Internal ABI field.', abiType: 'string', abiParam: 'agent_identifier', internal: true },
        { name: 'envJson', label: 'Environment Variables (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}', abiType: 'string', abiParam: 'env_json' },
        { name: 'metadataJson', label: 'Metadata (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}', abiType: 'string', abiParam: 'metadata_json' },
        { name: 'sshEnabled', label: 'Enable SSH', type: 'boolean', defaultValue: false, abiType: 'bool', abiParam: 'ssh_enabled' },
        { name: 'sshPublicKey', label: 'SSH Public Key', type: 'textarea', placeholder: 'ssh-ed25519 AAAA...', helperText: 'Required if SSH is enabled', abiType: 'string', abiParam: 'ssh_public_key' },
        { name: 'webTerminalEnabled', label: 'Web Terminal', type: 'boolean', defaultValue: true, abiType: 'bool', abiParam: 'web_terminal_enabled', internal: true },
        { name: 'maxLifetimeSeconds', label: 'Max Lifetime (hours)', type: 'number', defaultValue: 86400, min: 0, step: 3600, helperText: 'Value in seconds — 3600 = 1h, 86400 = 24h, 0 = unlimited', abiType: 'uint64', abiParam: 'max_lifetime_seconds' },
        { name: 'idleTimeoutSeconds', label: 'Idle Timeout (minutes)', type: 'number', defaultValue: 3600, min: 0, step: 300, helperText: 'Value in seconds — 300 = 5min, 3600 = 1h, 0 = disabled', abiType: 'uint64', abiParam: 'idle_timeout_seconds' },
        { name: 'cpuCores', label: 'CPU Cores', type: 'number', defaultValue: 2, min: 1, max: 16, helperText: '1–4 for dev, 8–16 for production', abiType: 'uint64', abiParam: 'cpu_cores' },
        { name: 'memoryMb', label: 'Memory (MB)', type: 'number', defaultValue: 2048, min: 512, max: 32768, step: 512, helperText: '512 = 0.5 GB, 2048 = 2 GB, 8192 = 8 GB', abiType: 'uint64', abiParam: 'memory_mb' },
        { name: 'diskGb', label: 'Disk (GB)', type: 'number', defaultValue: 10, min: 1, max: 100, helperText: '10 GB typical for dev, 50+ for large models', abiType: 'uint64', abiParam: 'disk_gb' },
        { name: 'sidecarToken', label: 'Sidecar Token', type: 'text', defaultValue: '', abiType: 'string', abiParam: 'sidecar_token', internal: true },
        { name: 'teeRequired', label: 'TEE Required', type: 'boolean', defaultValue: false, abiType: 'bool', abiParam: 'tee_required' },
        { name: 'teeType', label: 'TEE Type', type: 'select', defaultValue: '0', abiType: 'uint8', abiParam: 'tee_type', options: TEE_TYPE_OPTIONS },
      ],
    },
    {
      // ABI: WorkflowCreateRequest { name, workflow_json, trigger_type, trigger_config, sandbox_config_json }
      id: INSTANCE_JOB_IDS.WORKFLOW_CREATE,
      name: 'workflow_create',
      label: 'Create Workflow',
      description: 'Define a scheduled or event-driven workflow for this instance.',
      category: 'workflow',
      icon: 'i-ph:flow-arrow',
      pricingMultiplier: pricing(INSTANCE_JOB_IDS.WORKFLOW_CREATE),
      requiresSandbox: false,
      fields: [
        { name: 'name', label: 'Workflow Name', type: 'text', required: true, abiType: 'string' },
        { name: 'workflowJson', label: 'Workflow Definition (JSON)', type: 'json', required: true, abiType: 'string', abiParam: 'workflow_json' },
        { name: 'triggerType', label: 'Trigger Type', type: 'select', required: true, abiType: 'string', abiParam: 'trigger_type', options: [
          { label: 'Cron Schedule', value: 'cron' },
          { label: 'Webhook', value: 'webhook' },
          { label: 'Manual', value: 'manual' },
        ] },
        { name: 'triggerConfig', label: 'Trigger Config', type: 'text', placeholder: '0 */6 * * * *', helperText: 'Cron expression or webhook URL', abiType: 'string', abiParam: 'trigger_config' },
        { name: 'sandboxConfigJson', label: 'Sandbox Config (JSON)', type: 'json', placeholder: '{}', abiType: 'string', abiParam: 'sandbox_config_json' },
      ],
    },
    {
      // ABI: WorkflowControlRequest { workflow_id }
      id: INSTANCE_JOB_IDS.WORKFLOW_TRIGGER,
      name: 'workflow_trigger',
      label: 'Trigger Workflow',
      description: 'Manually trigger an existing workflow.',
      category: 'workflow',
      icon: 'i-ph:play',
      pricingMultiplier: pricing(INSTANCE_JOB_IDS.WORKFLOW_TRIGGER),
      requiresSandbox: false,
      fields: [
        { name: 'workflowId', label: 'Workflow ID', type: 'number', required: true, min: 0, abiType: 'uint64', abiParam: 'workflow_id' },
      ],
    },
    {
      // ABI: WorkflowControlRequest { workflow_id }
      id: INSTANCE_JOB_IDS.WORKFLOW_CANCEL,
      name: 'workflow_cancel',
      label: 'Cancel Workflow',
      description: 'Deactivate a workflow. Can be re-triggered later.',
      category: 'workflow',
      icon: 'i-ph:stop',
      pricingMultiplier: pricing(INSTANCE_JOB_IDS.WORKFLOW_CANCEL),
      requiresSandbox: false,
      fields: [
        { name: 'workflowId', label: 'Workflow ID', type: 'number', required: true, min: 0, abiType: 'uint64', abiParam: 'workflow_id' },
      ],
    },
  ];
}

// ── Instance Blueprint ──

export const INSTANCE_BLUEPRINT: BlueprintDefinition = {
  id: 'ai-agent-instance-blueprint',
  name: 'AI Agent Instance',
  version: '0.4.0',
  description: 'Subscription-based single-instance AI agent with on-chain workflows and operator-reported lifecycle.',
  icon: 'i-ph:cube',
  color: 'blue',
  contracts: {},
  jobs: createInstanceJobs(),
  categories: [
    { key: 'lifecycle', label: 'Instance Lifecycle', icon: 'i-ph:hard-drives' },
    { key: 'workflow', label: 'Workflows', icon: 'i-ph:flow-arrow' },
  ],
};

export function initInstanceBlueprint(addressesByChain: Record<number, Address>) {
  INSTANCE_BLUEPRINT.contracts = addressesByChain;
  registerBlueprint(INSTANCE_BLUEPRINT);
}

registerBlueprint(INSTANCE_BLUEPRINT);

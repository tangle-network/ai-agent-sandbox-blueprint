import { JOB_IDS } from '~/lib/types/sandbox';
import { type BlueprintDefinition, type JobDefinition, registerBlueprint } from './registry';
import type { Address } from 'viem';

/**
 * AI Agent Sandbox Blueprint — the default Tangle sandbox provisioning blueprint.
 * Registers 17 jobs across 5 categories.
 */

const SANDBOX_JOBS: JobDefinition[] = [
  // ── Lifecycle ──
  {
    id: JOB_IDS.SANDBOX_CREATE,
    name: 'sandbox_create',
    label: 'Create Sandbox',
    description: 'Provision a new AI agent sandbox with Docker isolation, optional SSH, and sidecar.',
    category: 'lifecycle',
    icon: 'i-ph:plus-circle',
    pricingMultiplier: 50,
    requiresSandbox: false,
    fields: [
      { name: 'name', label: 'Sandbox Name', type: 'text', placeholder: 'my-agent-sandbox', required: true },
      { name: 'image', label: 'Docker Image', type: 'text', placeholder: 'ubuntu:22.04', required: true, defaultValue: 'ubuntu:22.04' },
      { name: 'stack', label: 'Stack', type: 'select', options: [
        { label: 'Default', value: 'default' },
        { label: 'Python', value: 'python' },
        { label: 'Node.js', value: 'nodejs' },
        { label: 'Rust', value: 'rust' },
      ], defaultValue: 'default' },
      { name: 'agentIdentifier', label: 'Agent Identifier', type: 'text', placeholder: 'agent-1', helperText: 'Unique identifier for the agent in this sandbox' },
      { name: 'cpuCores', label: 'CPU Cores', type: 'number', defaultValue: 2 },
      { name: 'memoryMb', label: 'Memory (MB)', type: 'number', defaultValue: 2048 },
      { name: 'diskGb', label: 'Disk (GB)', type: 'number', defaultValue: 10 },
      { name: 'maxLifetimeSeconds', label: 'Max Lifetime (s)', type: 'number', defaultValue: 86400, helperText: '0 = unlimited' },
      { name: 'idleTimeoutSeconds', label: 'Idle Timeout (s)', type: 'number', defaultValue: 3600 },
      { name: 'sshEnabled', label: 'Enable SSH', type: 'boolean', defaultValue: false },
      { name: 'sshPublicKey', label: 'SSH Public Key', type: 'textarea', placeholder: 'ssh-ed25519 AAAA...', helperText: 'Required if SSH is enabled' },
      { name: 'webTerminalEnabled', label: 'Web Terminal', type: 'boolean', defaultValue: true },
      { name: 'envJson', label: 'Environment Variables (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}' },
      { name: 'metadataJson', label: 'Metadata (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}' },
    ],
  },
  {
    id: JOB_IDS.SANDBOX_STOP,
    name: 'sandbox_stop',
    label: 'Stop Sandbox',
    description: 'Stop a running sandbox. The container stays on disk for quick resume.',
    category: 'lifecycle',
    icon: 'i-ph:stop',
    pricingMultiplier: 1,
    requiresSandbox: true,
    fields: [],
  },
  {
    id: JOB_IDS.SANDBOX_RESUME,
    name: 'sandbox_resume',
    label: 'Resume Sandbox',
    description: 'Resume a stopped sandbox from its last state.',
    category: 'lifecycle',
    icon: 'i-ph:play',
    pricingMultiplier: 1,
    requiresSandbox: true,
    fields: [],
  },
  {
    id: JOB_IDS.SANDBOX_DELETE,
    name: 'sandbox_delete',
    label: 'Delete Sandbox',
    description: 'Permanently delete a sandbox and its data.',
    category: 'lifecycle',
    icon: 'i-ph:trash',
    pricingMultiplier: 1,
    requiresSandbox: true,
    fields: [],
    warning: 'This action is irreversible. All sandbox data will be permanently deleted.',
  },
  {
    id: JOB_IDS.SANDBOX_SNAPSHOT,
    name: 'sandbox_snapshot',
    label: 'Snapshot',
    description: 'Create a snapshot of the sandbox state at the specified storage tier.',
    category: 'lifecycle',
    icon: 'i-ph:camera',
    pricingMultiplier: 5,
    requiresSandbox: true,
    fields: [
      { name: 'tier', label: 'Storage Tier', type: 'select', required: true, options: [
        { label: 'Hot (Docker commit)', value: 'hot' },
        { label: 'Warm (Registry)', value: 'warm' },
        { label: 'Cold (S3/Archive)', value: 'cold' },
      ], defaultValue: 'hot' },
      { name: 'destination', label: 'Destination', type: 'text', placeholder: 'Optional registry/bucket path', helperText: 'Required for warm/cold tiers' },
    ],
  },

  // ── Execution ──
  {
    id: JOB_IDS.EXEC,
    name: 'exec',
    label: 'Execute Command',
    description: 'Run a shell command inside the sandbox.',
    category: 'execution',
    icon: 'i-ph:terminal',
    pricingMultiplier: 1,
    requiresSandbox: true,
    fields: [
      { name: 'command', label: 'Command', type: 'text', placeholder: 'ls -la', required: true },
      { name: 'args', label: 'Arguments', type: 'text', placeholder: '-la /workspace', helperText: 'Space-separated arguments' },
    ],
  },
  {
    id: JOB_IDS.PROMPT,
    name: 'prompt',
    label: 'AI Prompt',
    description: 'Send a prompt to the AI agent running in the sandbox.',
    category: 'execution',
    icon: 'i-ph:robot',
    pricingMultiplier: 20,
    requiresSandbox: true,
    fields: [
      { name: 'prompt', label: 'Prompt', type: 'textarea', placeholder: 'What files are in the workspace?', required: true },
      { name: 'systemPrompt', label: 'System Prompt', type: 'textarea', placeholder: 'You are a helpful coding assistant.' },
    ],
  },
  {
    id: JOB_IDS.TASK,
    name: 'task',
    label: 'Agent Task',
    description: 'Submit an autonomous task for the agent to complete.',
    category: 'execution',
    icon: 'i-ph:lightning',
    pricingMultiplier: 250,
    requiresSandbox: true,
    fields: [
      { name: 'task', label: 'Task Description', type: 'textarea', placeholder: 'Build a REST API with Express...', required: true },
      { name: 'systemPrompt', label: 'System Prompt', type: 'textarea', placeholder: 'You are an expert developer.' },
    ],
  },

  // ── Batch ──
  {
    id: JOB_IDS.BATCH_CREATE,
    name: 'batch_create',
    label: 'Batch Create',
    description: 'Create multiple sandboxes at once from a shared configuration.',
    category: 'batch',
    icon: 'i-ph:copy',
    pricingMultiplier: 100,
    requiresSandbox: false,
    fields: [
      { name: 'count', label: 'Count', type: 'number', required: true, defaultValue: 3, helperText: 'Number of sandboxes to create' },
      { name: 'configJson', label: 'Config (JSON)', type: 'json', required: true, placeholder: '{"image":"ubuntu:22.04","cpuCores":2}' },
    ],
  },
  {
    id: JOB_IDS.BATCH_TASK,
    name: 'batch_task',
    label: 'Batch Task',
    description: 'Run an autonomous task across multiple sandboxes in parallel.',
    category: 'batch',
    icon: 'i-ph:lightning',
    pricingMultiplier: 500,
    requiresSandbox: false,
    fields: [
      { name: 'sandboxIds', label: 'Sandbox IDs', type: 'textarea', required: true, placeholder: 'sandbox-1\nsandbox-2', helperText: 'One per line' },
      { name: 'task', label: 'Task', type: 'textarea', required: true },
      { name: 'systemPrompt', label: 'System Prompt', type: 'textarea' },
    ],
  },
  {
    id: JOB_IDS.BATCH_EXEC,
    name: 'batch_exec',
    label: 'Batch Exec',
    description: 'Execute a command across multiple sandboxes in parallel.',
    category: 'batch',
    icon: 'i-ph:terminal',
    pricingMultiplier: 50,
    requiresSandbox: false,
    fields: [
      { name: 'sandboxIds', label: 'Sandbox IDs', type: 'textarea', required: true, placeholder: 'sandbox-1\nsandbox-2', helperText: 'One per line' },
      { name: 'command', label: 'Command', type: 'text', required: true },
      { name: 'args', label: 'Arguments', type: 'text' },
    ],
  },
  {
    id: JOB_IDS.BATCH_COLLECT,
    name: 'batch_collect',
    label: 'Batch Collect',
    description: 'Collect results from a batch operation.',
    category: 'batch',
    icon: 'i-ph:receipt',
    pricingMultiplier: 1,
    requiresSandbox: false,
    fields: [
      { name: 'batchId', label: 'Batch ID', type: 'text', required: true },
    ],
  },

  // ── Workflows ──
  {
    id: JOB_IDS.WORKFLOW_CREATE,
    name: 'workflow_create',
    label: 'Create Workflow',
    description: 'Define a scheduled or event-driven workflow with sandbox automation.',
    category: 'workflow',
    icon: 'i-ph:flow-arrow',
    pricingMultiplier: 2,
    requiresSandbox: false,
    fields: [
      { name: 'name', label: 'Workflow Name', type: 'text', required: true },
      { name: 'workflowJson', label: 'Workflow Definition (JSON)', type: 'json', required: true },
      { name: 'triggerType', label: 'Trigger Type', type: 'select', required: true, options: [
        { label: 'Cron Schedule', value: 'cron' },
        { label: 'Webhook', value: 'webhook' },
        { label: 'Manual', value: 'manual' },
      ] },
      { name: 'triggerConfig', label: 'Trigger Config', type: 'text', placeholder: '0 */6 * * *', helperText: 'Cron expression or webhook URL' },
      { name: 'sandboxConfigJson', label: 'Sandbox Config (JSON)', type: 'json', placeholder: '{}' },
    ],
  },
  {
    id: JOB_IDS.WORKFLOW_TRIGGER,
    name: 'workflow_trigger',
    label: 'Trigger Workflow',
    description: 'Manually trigger an existing workflow.',
    category: 'workflow',
    icon: 'i-ph:play',
    pricingMultiplier: 5,
    requiresSandbox: false,
    fields: [
      { name: 'workflowId', label: 'Workflow ID', type: 'number', required: true },
    ],
  },
  {
    id: JOB_IDS.WORKFLOW_CANCEL,
    name: 'workflow_cancel',
    label: 'Cancel Workflow',
    description: 'Deactivate a workflow. Can be re-triggered later.',
    category: 'workflow',
    icon: 'i-ph:stop',
    pricingMultiplier: 1,
    requiresSandbox: false,
    fields: [
      { name: 'workflowId', label: 'Workflow ID', type: 'number', required: true },
    ],
  },

  // ── SSH ──
  {
    id: JOB_IDS.SSH_PROVISION,
    name: 'ssh_provision',
    label: 'Provision SSH',
    description: 'Add an SSH public key to a sandbox for remote access.',
    category: 'ssh',
    icon: 'i-ph:key',
    pricingMultiplier: 2,
    requiresSandbox: true,
    fields: [
      { name: 'publicKey', label: 'SSH Public Key', type: 'textarea', required: true, placeholder: 'ssh-ed25519 AAAA...' },
    ],
  },
  {
    id: JOB_IDS.SSH_REVOKE,
    name: 'ssh_revoke',
    label: 'Revoke SSH',
    description: 'Remove an SSH public key from a sandbox.',
    category: 'ssh',
    icon: 'i-ph:key',
    pricingMultiplier: 1,
    requiresSandbox: true,
    fields: [
      { name: 'publicKey', label: 'SSH Public Key', type: 'textarea', required: true },
    ],
  },
];

export const SANDBOX_BLUEPRINT: BlueprintDefinition = {
  id: 'ai-agent-sandbox-blueprint',
  name: 'AI Agent Sandbox',
  version: '0.3.0',
  description: 'Provision isolated AI agent sandboxes with Docker, SSH, sidecar AI execution, batch operations, and scheduled workflows.',
  icon: 'i-ph:cloud',
  color: 'teal',
  contracts: {},
  jobs: SANDBOX_JOBS,
  categories: [
    { key: 'lifecycle', label: 'Sandbox Lifecycle', icon: 'i-ph:hard-drives' },
    { key: 'execution', label: 'Execution', icon: 'i-ph:terminal' },
    { key: 'batch', label: 'Batch Operations', icon: 'i-ph:copy' },
    { key: 'workflow', label: 'Workflows', icon: 'i-ph:flow-arrow' },
    { key: 'ssh', label: 'SSH Management', icon: 'i-ph:key' },
  ],
};

/** Wire addresses from chain configs at init time */
export function initSandboxBlueprint(addressesByChain: Record<number, Address>) {
  SANDBOX_BLUEPRINT.contracts = addressesByChain;
  registerBlueprint(SANDBOX_BLUEPRINT);
}

// Auto-register with empty addresses (will be overridden when chain is known)
registerBlueprint(SANDBOX_BLUEPRINT);

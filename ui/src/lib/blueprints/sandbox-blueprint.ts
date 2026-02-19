import { encodeAbiParameters } from 'viem';
import { JOB_IDS } from '~/lib/types/sandbox';
import { type BlueprintDefinition, type JobDefinition, registerBlueprint } from './registry';
import type { Address } from 'viem';

/**
 * AI Agent Sandbox Blueprint — the default Tangle sandbox provisioning blueprint.
 * Registers 17 jobs across 5 categories.
 *
 * ABI types verified against ai-agent-sandbox-blueprint-lib/src/lib.rs sol! macros.
 */

// ── Shared context params ──

const SIDECAR_URL_CTX = [{ abiName: 'sidecar_url', abiType: 'string' }] as const;
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
    id: JOB_IDS.SANDBOX_STOP,
    name: 'sandbox_stop',
    label: 'Stop Sandbox',
    description: 'Stop a running sandbox. The container stays on disk for quick resume.',
    category: 'lifecycle',
    icon: 'i-ph:stop',
    pricingMultiplier: 1,
    requiresSandbox: true,
    contextParams: [...SANDBOX_ID_CTX],
    fields: [],
  },
  {
    // ABI: SandboxIdRequest { sandbox_id }
    id: JOB_IDS.SANDBOX_RESUME,
    name: 'sandbox_resume',
    label: 'Resume Sandbox',
    description: 'Resume a stopped sandbox from its last state.',
    category: 'lifecycle',
    icon: 'i-ph:play',
    pricingMultiplier: 1,
    requiresSandbox: true,
    contextParams: [...SANDBOX_ID_CTX],
    fields: [],
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
  {
    // ABI: SandboxSnapshotRequest { sidecar_url, destination, include_workspace, include_state }
    id: JOB_IDS.SANDBOX_SNAPSHOT,
    name: 'sandbox_snapshot',
    label: 'Snapshot',
    description: 'Create a snapshot of the current sandbox state.',
    category: 'lifecycle',
    icon: 'i-ph:camera',
    pricingMultiplier: 5,
    requiresSandbox: true,
    contextParams: [...SIDECAR_URL_CTX],
    fields: [
      { name: 'destination', label: 'Destination', type: 'text', placeholder: 'registry/path or s3://bucket/key', abiType: 'string' },
      { name: 'includeWorkspace', label: 'Include Workspace', type: 'boolean', defaultValue: true, abiType: 'bool', abiParam: 'include_workspace' },
      { name: 'includeState', label: 'Include State', type: 'boolean', defaultValue: true, abiType: 'bool', abiParam: 'include_state' },
    ],
  },

  // ── Execution ──
  {
    // ABI: SandboxExecRequest { sidecar_url, command, cwd, env_json, timeout_ms }
    id: JOB_IDS.EXEC,
    name: 'exec',
    label: 'Execute Command',
    description: 'Run a shell command inside the sandbox.',
    category: 'execution',
    icon: 'i-ph:terminal',
    pricingMultiplier: 1,
    requiresSandbox: true,
    contextParams: [...SIDECAR_URL_CTX],
    fields: [
      { name: 'command', label: 'Command', type: 'text', placeholder: 'ls -la', required: true, abiType: 'string' },
      { name: 'cwd', label: 'Working Directory', type: 'text', placeholder: '/workspace', abiType: 'string' },
      { name: 'envJson', label: 'Environment (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}', abiType: 'string', abiParam: 'env_json' },
      { name: 'timeoutMs', label: 'Timeout (ms)', type: 'number', defaultValue: 30000, min: 0, abiType: 'uint64', abiParam: 'timeout_ms' },
    ],
  },
  {
    // ABI: SandboxPromptRequest { sidecar_url, message, session_id, model, context_json, timeout_ms }
    id: JOB_IDS.PROMPT,
    name: 'prompt',
    label: 'AI Prompt',
    description: 'Send a prompt to the AI agent running in the sandbox.',
    category: 'execution',
    icon: 'i-ph:robot',
    pricingMultiplier: 20,
    requiresSandbox: true,
    contextParams: [...SIDECAR_URL_CTX],
    fields: [
      { name: 'message', label: 'Message', type: 'textarea', placeholder: 'What files are in the workspace?', required: true, abiType: 'string' },
      { name: 'sessionId', label: 'Session ID', type: 'text', placeholder: 'auto-generated if empty', abiType: 'string', abiParam: 'session_id' },
      { name: 'model', label: 'Model', type: 'text', placeholder: 'default', abiType: 'string' },
      { name: 'contextJson', label: 'Context (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}', abiType: 'string', abiParam: 'context_json' },
      { name: 'timeoutMs', label: 'Timeout (ms)', type: 'number', defaultValue: 60000, min: 0, abiType: 'uint64', abiParam: 'timeout_ms' },
    ],
  },
  {
    // ABI: SandboxTaskRequest { sidecar_url, prompt, session_id, max_turns, model, context_json, timeout_ms }
    id: JOB_IDS.TASK,
    name: 'task',
    label: 'Agent Task',
    description: 'Submit an autonomous task for the agent to complete.',
    category: 'execution',
    icon: 'i-ph:lightning',
    pricingMultiplier: 250,
    requiresSandbox: true,
    contextParams: [...SIDECAR_URL_CTX],
    fields: [
      { name: 'prompt', label: 'Task Prompt', type: 'textarea', placeholder: 'Build a REST API with Express...', required: true, abiType: 'string' },
      { name: 'sessionId', label: 'Session ID', type: 'text', placeholder: 'auto-generated if empty', abiType: 'string', abiParam: 'session_id' },
      { name: 'maxTurns', label: 'Max Turns', type: 'number', defaultValue: 10, min: 1, abiType: 'uint64', abiParam: 'max_turns' },
      { name: 'model', label: 'Model', type: 'text', placeholder: 'default', abiType: 'string' },
      { name: 'contextJson', label: 'Context (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}', abiType: 'string', abiParam: 'context_json' },
      { name: 'timeoutMs', label: 'Timeout (ms)', type: 'number', defaultValue: 300000, min: 0, abiType: 'uint64', abiParam: 'timeout_ms' },
    ],
  },

  // ── Batch ──
  {
    // ABI: BatchCreateRequest { count, template_request (nested struct), operators, distribution }
    // Uses customEncoder due to nested SandboxCreateRequest tuple
    id: JOB_IDS.BATCH_CREATE,
    name: 'batch_create',
    label: 'Batch Create',
    description: 'Create multiple sandboxes from a shared configuration template.',
    category: 'batch',
    icon: 'i-ph:copy',
    pricingMultiplier: 100,
    requiresSandbox: false,
    fields: [
      { name: 'count', label: 'Count', type: 'number', required: true, defaultValue: 3, min: 1, helperText: 'Number of sandboxes to create', abiType: 'uint32' },
      { name: 'configJson', label: 'Template Config (JSON)', type: 'json', required: true, placeholder: '{"name":"batch","image":"ubuntu:22.04","stack":"default","agent_identifier":"","env_json":"{}","metadata_json":"{}","ssh_enabled":false,"ssh_public_key":"","web_terminal_enabled":true,"max_lifetime_seconds":86400,"idle_timeout_seconds":3600,"cpu_cores":2,"memory_mb":2048,"disk_gb":10,"tee_required":false,"tee_type":0}', abiType: 'string', abiParam: 'config_json' },
      { name: 'operators', label: 'Operators', type: 'textarea', placeholder: '0xabc...\n0xdef...', helperText: 'One address per line', abiType: 'address[]' },
      { name: 'distribution', label: 'Distribution', type: 'select', defaultValue: 'round_robin', abiType: 'string', options: [
        { label: 'Round Robin', value: 'round_robin' },
        { label: 'Random', value: 'random' },
      ] },
    ],
    customEncoder: (values) => {
      // BatchCreateRequest has nested SandboxCreateRequest — encode the template from JSON
      const config = JSON.parse(String(values.configJson || '{}'));
      const operators = String(values.operators || '').split('\n').map((s) => s.trim()).filter((s) => /^0x[a-fA-F0-9]{40}$/.test(s)) as `0x${string}`[];
      return encodeAbiParameters(
        [
          { name: 'count', type: 'uint32' },
          {
            name: 'template_request', type: 'tuple',
            components: [
              { name: 'name', type: 'string' }, { name: 'image', type: 'string' },
              { name: 'stack', type: 'string' }, { name: 'agent_identifier', type: 'string' },
              { name: 'env_json', type: 'string' }, { name: 'metadata_json', type: 'string' },
              { name: 'ssh_enabled', type: 'bool' }, { name: 'ssh_public_key', type: 'string' },
              { name: 'web_terminal_enabled', type: 'bool' },
              { name: 'max_lifetime_seconds', type: 'uint64' }, { name: 'idle_timeout_seconds', type: 'uint64' },
              { name: 'cpu_cores', type: 'uint64' }, { name: 'memory_mb', type: 'uint64' }, { name: 'disk_gb', type: 'uint64' },
              { name: 'tee_required', type: 'bool' }, { name: 'tee_type', type: 'uint8' },
            ],
          },
          { name: 'operators', type: 'address[]' },
          { name: 'distribution', type: 'string' },
        ],
        [
          Number(values.count) || 3,
          {
            name: String(config.name || 'batch'),
            image: String(config.image || 'ubuntu:22.04'),
            stack: String(config.stack || 'default'),
            agent_identifier: String(config.agent_identifier || ''),
            env_json: String(config.env_json || '{}'),
            metadata_json: String(config.metadata_json || '{}'),
            ssh_enabled: Boolean(config.ssh_enabled),
            ssh_public_key: String(config.ssh_public_key || ''),
            web_terminal_enabled: config.web_terminal_enabled !== false,
            max_lifetime_seconds: BigInt(Number(config.max_lifetime_seconds) || 86400),
            idle_timeout_seconds: BigInt(Number(config.idle_timeout_seconds) || 3600),
            cpu_cores: BigInt(Number(config.cpu_cores) || 2),
            memory_mb: BigInt(Number(config.memory_mb) || 2048),
            disk_gb: BigInt(Number(config.disk_gb) || 10),
            tee_required: Boolean(config.tee_required),
            tee_type: Number(config.tee_type) || 0,
          },
          operators,
          String(values.distribution || 'round_robin'),
        ],
      );
    },
  },
  {
    // ABI: BatchTaskRequest { sidecar_urls[], prompt, session_id, max_turns, model, context_json, timeout_ms, parallel, aggregation }
    id: JOB_IDS.BATCH_TASK,
    name: 'batch_task',
    label: 'Batch Task',
    description: 'Run an autonomous task across multiple sandboxes in parallel.',
    category: 'batch',
    icon: 'i-ph:lightning',
    pricingMultiplier: 500,
    requiresSandbox: false,
    fields: [
      { name: 'sidecarUrls', label: 'Sidecar URLs', type: 'textarea', required: true, placeholder: 'http://sidecar-1:3000\nhttp://sidecar-2:3000', helperText: 'One URL per line', abiType: 'string[]', abiParam: 'sidecar_urls' },
      { name: 'prompt', label: 'Task Prompt', type: 'textarea', required: true, abiType: 'string' },
      { name: 'sessionId', label: 'Session ID', type: 'text', abiType: 'string', abiParam: 'session_id' },
      { name: 'maxTurns', label: 'Max Turns', type: 'number', defaultValue: 10, min: 1, abiType: 'uint64', abiParam: 'max_turns' },
      { name: 'model', label: 'Model', type: 'text', placeholder: 'default', abiType: 'string' },
      { name: 'contextJson', label: 'Context (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}', abiType: 'string', abiParam: 'context_json' },
      { name: 'timeoutMs', label: 'Timeout (ms)', type: 'number', defaultValue: 300000, min: 0, abiType: 'uint64', abiParam: 'timeout_ms' },
      { name: 'parallel', label: 'Parallel Execution', type: 'boolean', defaultValue: true, abiType: 'bool' },
      { name: 'aggregation', label: 'Aggregation Strategy', type: 'select', defaultValue: 'collect', abiType: 'string', options: [
        { label: 'Collect All', value: 'collect' },
        { label: 'First Success', value: 'first' },
      ] },
    ],
  },
  {
    // ABI: BatchExecRequest { sidecar_urls[], command, cwd, env_json, timeout_ms, parallel }
    id: JOB_IDS.BATCH_EXEC,
    name: 'batch_exec',
    label: 'Batch Exec',
    description: 'Execute a command across multiple sandboxes.',
    category: 'batch',
    icon: 'i-ph:terminal',
    pricingMultiplier: 50,
    requiresSandbox: false,
    fields: [
      { name: 'sidecarUrls', label: 'Sidecar URLs', type: 'textarea', required: true, placeholder: 'http://sidecar-1:3000\nhttp://sidecar-2:3000', helperText: 'One URL per line', abiType: 'string[]', abiParam: 'sidecar_urls' },
      { name: 'command', label: 'Command', type: 'text', required: true, abiType: 'string' },
      { name: 'cwd', label: 'Working Directory', type: 'text', placeholder: '/workspace', abiType: 'string' },
      { name: 'envJson', label: 'Environment (JSON)', type: 'json', placeholder: '{}', defaultValue: '{}', abiType: 'string', abiParam: 'env_json' },
      { name: 'timeoutMs', label: 'Timeout (ms)', type: 'number', defaultValue: 30000, min: 0, abiType: 'uint64', abiParam: 'timeout_ms' },
      { name: 'parallel', label: 'Parallel Execution', type: 'boolean', defaultValue: true, abiType: 'bool' },
    ],
  },
  {
    // ABI: BatchCollectRequest { batch_id }
    id: JOB_IDS.BATCH_COLLECT,
    name: 'batch_collect',
    label: 'Batch Collect',
    description: 'Collect results from a batch operation.',
    category: 'batch',
    icon: 'i-ph:receipt',
    pricingMultiplier: 1,
    requiresSandbox: false,
    fields: [
      { name: 'batchId', label: 'Batch ID', type: 'text', required: true, abiType: 'string', abiParam: 'batch_id' },
    ],
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

  // ── SSH ──
  {
    // ABI: SshProvisionRequest { sidecar_url, username, public_key }
    id: JOB_IDS.SSH_PROVISION,
    name: 'ssh_provision',
    label: 'Provision SSH',
    description: 'Add an SSH public key to a sandbox for remote access.',
    category: 'ssh',
    icon: 'i-ph:key',
    pricingMultiplier: 2,
    requiresSandbox: true,
    contextParams: [...SIDECAR_URL_CTX],
    fields: [
      { name: 'username', label: 'Username', type: 'text', required: true, placeholder: 'agent', abiType: 'string' },
      { name: 'publicKey', label: 'SSH Public Key', type: 'textarea', required: true, placeholder: 'ssh-ed25519 AAAA...', abiType: 'string', abiParam: 'public_key' },
    ],
  },
  {
    // ABI: SshRevokeRequest { sidecar_url, username, public_key }
    id: JOB_IDS.SSH_REVOKE,
    name: 'ssh_revoke',
    label: 'Revoke SSH',
    description: 'Remove an SSH public key from a sandbox.',
    category: 'ssh',
    icon: 'i-ph:key',
    pricingMultiplier: 1,
    requiresSandbox: true,
    contextParams: [...SIDECAR_URL_CTX],
    fields: [
      { name: 'username', label: 'Username', type: 'text', required: true, abiType: 'string' },
      { name: 'publicKey', label: 'SSH Public Key', type: 'textarea', required: true, abiType: 'string', abiParam: 'public_key' },
    ],
  },
];

// ── Blueprint Definition ──

export const SANDBOX_BLUEPRINT: BlueprintDefinition = {
  id: 'ai-agent-sandbox-blueprint',
  name: 'AI Agent Sandbox',
  version: '0.4.0',
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

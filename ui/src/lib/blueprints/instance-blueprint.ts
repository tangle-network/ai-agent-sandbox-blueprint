import { INSTANCE_JOB_IDS, INSTANCE_PRICING_TIERS } from '~/lib/types/instance';
import { type BlueprintDefinition, type JobDefinition, registerBlueprint } from '@tangle-network/blueprint-ui';
import type { Address } from 'viem';

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
    { key: 'workflow', label: 'Workflows', icon: 'i-ph:flow-arrow' },
  ],
};

export function initInstanceBlueprint(addressesByChain: Record<number, Address>) {
  INSTANCE_BLUEPRINT.contracts = addressesByChain;
  registerBlueprint(INSTANCE_BLUEPRINT);
}

registerBlueprint(INSTANCE_BLUEPRINT);

/**
 * Job IDs matching the unified AgentSandboxBlueprint contract (instance mode).
 * Read-only ops (exec, prompt, task, ssh, snapshot) are served by the
 * operator API instead of on-chain jobs.
 */
export const INSTANCE_JOB_IDS = {
  PROVISION: 0,
  WORKFLOW_CREATE: 2,
  WORKFLOW_TRIGGER: 3,
  WORKFLOW_CANCEL: 4,
} as const;

/** Pricing tiers for instance blueprint */
export const INSTANCE_PRICING_TIERS: Record<number, { label: string; multiplier: number }> = {
  [INSTANCE_JOB_IDS.WORKFLOW_CREATE]: { label: 'Create Workflow', multiplier: 2 },
  [INSTANCE_JOB_IDS.WORKFLOW_TRIGGER]: { label: 'Trigger Workflow', multiplier: 5 },
  [INSTANCE_JOB_IDS.WORKFLOW_CANCEL]: { label: 'Cancel Workflow', multiplier: 1 },
};

/** Pricing tiers for TEE instance */
export const TEE_INSTANCE_PRICING_TIERS: Record<number, { label: string; multiplier: number }> = {
  [INSTANCE_JOB_IDS.WORKFLOW_CREATE]: { label: 'Create Workflow', multiplier: 2 },
  [INSTANCE_JOB_IDS.WORKFLOW_TRIGGER]: { label: 'Trigger Workflow', multiplier: 5 },
  [INSTANCE_JOB_IDS.WORKFLOW_CANCEL]: { label: 'Cancel Workflow', multiplier: 1 },
};

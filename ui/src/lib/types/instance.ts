/** Job IDs matching the Rust instance blueprint router */
export const INSTANCE_JOB_IDS = {
  PROVISION: 0,
  EXEC: 1,
  PROMPT: 2,
  TASK: 3,
  SSH_PROVISION: 4,
  SSH_REVOKE: 5,
  SNAPSHOT: 6,
  DEPROVISION: 7,
} as const;

/** Pricing tiers for instance blueprint */
export const INSTANCE_PRICING_TIERS: Record<number, { label: string; multiplier: number }> = {
  [INSTANCE_JOB_IDS.PROVISION]: { label: 'Provision Instance', multiplier: 50 },
  [INSTANCE_JOB_IDS.EXEC]: { label: 'Exec Command', multiplier: 1 },
  [INSTANCE_JOB_IDS.PROMPT]: { label: 'Prompt', multiplier: 20 },
  [INSTANCE_JOB_IDS.TASK]: { label: 'Agent Task', multiplier: 250 },
  [INSTANCE_JOB_IDS.SSH_PROVISION]: { label: 'SSH Provision', multiplier: 2 },
  [INSTANCE_JOB_IDS.SSH_REVOKE]: { label: 'SSH Revoke', multiplier: 1 },
  [INSTANCE_JOB_IDS.SNAPSHOT]: { label: 'Snapshot', multiplier: 5 },
  [INSTANCE_JOB_IDS.DEPROVISION]: { label: 'Deprovision', multiplier: 1 },
};

/** Pricing tiers for TEE instance (higher multipliers for TEE overhead) */
export const TEE_INSTANCE_PRICING_TIERS: Record<number, { label: string; multiplier: number }> = {
  [INSTANCE_JOB_IDS.PROVISION]: { label: 'Provision TEE Instance', multiplier: 100 },
  [INSTANCE_JOB_IDS.EXEC]: { label: 'Exec Command', multiplier: 2 },
  [INSTANCE_JOB_IDS.PROMPT]: { label: 'Prompt', multiplier: 25 },
  [INSTANCE_JOB_IDS.TASK]: { label: 'Agent Task', multiplier: 300 },
  [INSTANCE_JOB_IDS.SSH_PROVISION]: { label: 'SSH Provision', multiplier: 2 },
  [INSTANCE_JOB_IDS.SSH_REVOKE]: { label: 'SSH Revoke', multiplier: 1 },
  [INSTANCE_JOB_IDS.SNAPSHOT]: { label: 'Snapshot', multiplier: 5 },
  [INSTANCE_JOB_IDS.DEPROVISION]: { label: 'Deprovision', multiplier: 2 },
};

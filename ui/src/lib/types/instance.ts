/**
 * Job IDs matching the unified AgentSandboxBlueprint contract (instance mode).
 * Read-only ops (exec, prompt, task, ssh, snapshot) are now served
 * by the operator API instead of on-chain jobs.
 */
export const INSTANCE_JOB_IDS = {
  PROVISION: 5,
  DEPROVISION: 6,
} as const;

/** Pricing tiers for instance blueprint */
export const INSTANCE_PRICING_TIERS: Record<number, { label: string; multiplier: number }> = {
  [INSTANCE_JOB_IDS.PROVISION]: { label: 'Provision Instance', multiplier: 50 },
  [INSTANCE_JOB_IDS.DEPROVISION]: { label: 'Deprovision', multiplier: 1 },
};

/** Pricing tiers for TEE instance (higher multipliers for TEE overhead) */
export const TEE_INSTANCE_PRICING_TIERS: Record<number, { label: string; multiplier: number }> = {
  [INSTANCE_JOB_IDS.PROVISION]: { label: 'Provision TEE Instance', multiplier: 100 },
  [INSTANCE_JOB_IDS.DEPROVISION]: { label: 'Deprovision', multiplier: 2 },
};

export type SandboxStatus = 'running' | 'stopped' | 'warm' | 'cold' | 'gone' | 'error';

export interface Sandbox {
  id: string;
  name: string;
  status: SandboxStatus;
  image: string;
  stack: string;
  sidecarUrl: string;
  sshPort?: number;
  cpuCores: number;
  memoryMb: number;
  diskGb: number;
  idleTimeoutSeconds: number;
  maxLifetimeSeconds: number;
  createdAt: number;
  lastActivityAt: number;
  sidecarToken: string;
  sshEnabled: boolean;
  webTerminalEnabled: boolean;
  metadata?: Record<string, string>;
}

export interface SandboxSnapshot {
  id: string;
  sandboxId: string;
  tier: 'hot' | 'warm' | 'cold';
  destination?: string;
  createdAt: number;
  sizeBytes?: number;
}

/**
 * Job IDs — must match sequential indices in the unified AgentSandboxBlueprint contract.
 * Read-only ops (exec, prompt, task, ssh, snapshot, stop, resume, batch) are now served
 * by the operator API instead of on-chain jobs.
 */
export const JOB_IDS = {
  SANDBOX_CREATE: 0,
  SANDBOX_DELETE: 1,
  WORKFLOW_CREATE: 2,
  WORKFLOW_TRIGGER: 3,
  WORKFLOW_CANCEL: 4,
} as const;

/** Pricing tiers (multipliers of base rate) */
export const PRICING_TIERS: Record<number, { label: string; multiplier: number }> = {
  [JOB_IDS.SANDBOX_CREATE]: { label: 'Create Sandbox', multiplier: 50 },
  [JOB_IDS.SANDBOX_DELETE]: { label: 'Delete', multiplier: 1 },
  [JOB_IDS.WORKFLOW_CREATE]: { label: 'Create Workflow', multiplier: 2 },
  [JOB_IDS.WORKFLOW_TRIGGER]: { label: 'Trigger Workflow', multiplier: 5 },
  [JOB_IDS.WORKFLOW_CANCEL]: { label: 'Cancel Workflow', multiplier: 1 },
};

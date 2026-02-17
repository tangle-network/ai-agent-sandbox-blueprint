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

export interface SandboxCreateParams {
  name: string;
  image: string;
  stack: string;
  agentIdentifier: string;
  envJson: string;
  metadataJson: string;
  sshEnabled: boolean;
  sshPublicKey: string;
  webTerminalEnabled: boolean;
  maxLifetimeSeconds: number;
  idleTimeoutSeconds: number;
  cpuCores: number;
  memoryMb: number;
  diskGb: number;
}

export interface SandboxSnapshot {
  id: string;
  sandboxId: string;
  tier: 'hot' | 'warm' | 'cold';
  destination?: string;
  createdAt: number;
  sizeBytes?: number;
}

/** Job IDs matching the Rust blueprint router */
export const JOB_IDS = {
  SANDBOX_CREATE: 0,
  SANDBOX_STOP: 1,
  SANDBOX_RESUME: 2,
  SANDBOX_DELETE: 3,
  SANDBOX_SNAPSHOT: 4,
  EXEC: 10,
  PROMPT: 11,
  TASK: 12,
  BATCH_CREATE: 20,
  BATCH_TASK: 21,
  BATCH_EXEC: 22,
  BATCH_COLLECT: 23,
  WORKFLOW_CREATE: 30,
  WORKFLOW_TRIGGER: 31,
  WORKFLOW_CANCEL: 32,
  SSH_PROVISION: 40,
  SSH_REVOKE: 41,
} as const;

/** Pricing tiers (multipliers of base rate) */
export const PRICING_TIERS: Record<number, { label: string; multiplier: number }> = {
  [JOB_IDS.SANDBOX_CREATE]: { label: 'Create Sandbox', multiplier: 50 },
  [JOB_IDS.SANDBOX_STOP]: { label: 'Stop', multiplier: 1 },
  [JOB_IDS.SANDBOX_RESUME]: { label: 'Resume', multiplier: 50 },
  [JOB_IDS.SANDBOX_DELETE]: { label: 'Delete', multiplier: 1 },
  [JOB_IDS.SANDBOX_SNAPSHOT]: { label: 'Snapshot', multiplier: 5 },
  [JOB_IDS.EXEC]: { label: 'Exec Command', multiplier: 1 },
  [JOB_IDS.PROMPT]: { label: 'Prompt', multiplier: 20 },
  [JOB_IDS.TASK]: { label: 'Agent Task', multiplier: 250 },
  [JOB_IDS.SSH_PROVISION]: { label: 'SSH Provision', multiplier: 2 },
  [JOB_IDS.SSH_REVOKE]: { label: 'SSH Revoke', multiplier: 1 },
};

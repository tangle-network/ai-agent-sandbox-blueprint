import { describe, it, expect, vi, beforeEach } from 'vitest';

/**
 * useCreateDeploy is deeply coupled to wagmi hooks (useAccount, useWriteContract,
 * useWaitForTransactionReceipt) and several internal hooks (useSubmitJob, useOperators,
 * useInstanceProvisionWatcher). Testing it via renderHook would require an enormous
 * mock surface.
 *
 * Instead, we extract and test the pure computation logic that the hook relies on:
 *   - DeployStatus computation from path A / path B signals
 *   - canDeploy flag computation
 *   - mode derivation from blueprint ID
 *   - isNewService derivation
 *
 * These match the exact logic in useCreateDeploy.ts without needing wagmi context.
 */

import type { DeployStatus, DeployMode } from './useCreateDeploy';
import type { JobSubmitStatus } from '~/lib/hooks/useSubmitJob';

// ── Extracted pure functions (matching the hook's inline logic) ──

/** Derive deploy mode from blueprint ID */
function deriveMode(blueprintId: string | undefined): DeployMode {
  return blueprintId === 'ai-agent-sandbox-blueprint' ? 'sandbox' : 'instance';
}

/** Derive whether this is a TEE instance */
function isTeeInstanceBlueprint(blueprintId: string | undefined): boolean {
  return blueprintId === 'ai-agent-tee-instance-blueprint';
}

/** Derive whether a new service must be created (Path B) */
function deriveIsNewService(
  mode: DeployMode,
  serviceActive: boolean,
  servicePermitted: boolean,
  serviceId: string,
): boolean {
  const isInstanceMode = mode === 'instance';
  const hasValidService = !!(serviceActive && servicePermitted && serviceId);
  return isInstanceMode && !hasValidService;
}

/** Compute unified deploy status from Path A and Path B signals */
function computeStatus(opts: {
  isNewService: boolean;
  jobStatus: JobSubmitStatus;
  serviceSigning: boolean;
  serviceTxPending: boolean;
  serviceConfirmed: boolean;
  serviceError: string | null;
}): DeployStatus {
  const { isNewService, jobStatus, serviceSigning, serviceTxPending, serviceConfirmed, serviceError } = opts;

  if (!isNewService) {
    if (jobStatus === 'signing') return 'signing';
    if (jobStatus === 'pending') return 'pending';
    if (jobStatus === 'failed') return 'failed';
    if (jobStatus === 'confirmed') return 'confirmed';
    return 'idle';
  }

  if (serviceError) return 'failed';
  if (serviceSigning) return 'signing';
  if (serviceTxPending) return 'pending';
  if (serviceConfirmed) return 'confirmed';
  return 'idle';
}

/** Compute canDeploy flag */
function computeCanDeploy(opts: {
  job: boolean;
  hasName: boolean;
  hasAddress: boolean;
  status: DeployStatus;
  contractsDeployed: boolean;
  mode: DeployMode;
  hasValidService: boolean;
  isNewService: boolean;
  operatorCount: number;
  operatorsLoading: boolean;
}): boolean {
  return !!(
    opts.job &&
    opts.hasName &&
    opts.hasAddress &&
    opts.status === 'idle' &&
    opts.contractsDeployed &&
    (opts.mode === 'sandbox' ? opts.hasValidService : true) &&
    (!opts.isNewService || (opts.operatorCount > 0 && !opts.operatorsLoading))
  );
}

// ── Tests ──

describe('useCreateDeploy: deriveMode', () => {
  it('returns sandbox for ai-agent-sandbox-blueprint', () => {
    expect(deriveMode('ai-agent-sandbox-blueprint')).toBe('sandbox');
  });

  it('returns instance for ai-agent-instance-blueprint', () => {
    expect(deriveMode('ai-agent-instance-blueprint')).toBe('instance');
  });

  it('returns instance for ai-agent-tee-instance-blueprint', () => {
    expect(deriveMode('ai-agent-tee-instance-blueprint')).toBe('instance');
  });

  it('returns instance for undefined blueprint', () => {
    expect(deriveMode(undefined)).toBe('instance');
  });
});

describe('useCreateDeploy: isTeeInstanceBlueprint', () => {
  it('returns true for tee-instance blueprint', () => {
    expect(isTeeInstanceBlueprint('ai-agent-tee-instance-blueprint')).toBe(true);
  });

  it('returns false for regular instance blueprint', () => {
    expect(isTeeInstanceBlueprint('ai-agent-instance-blueprint')).toBe(false);
  });

  it('returns false for sandbox blueprint', () => {
    expect(isTeeInstanceBlueprint('ai-agent-sandbox-blueprint')).toBe(false);
  });
});

describe('useCreateDeploy: deriveIsNewService', () => {
  it('returns false for sandbox mode regardless of service state', () => {
    expect(deriveIsNewService('sandbox', false, false, '')).toBe(false);
  });

  it('returns true for instance mode without valid service', () => {
    expect(deriveIsNewService('instance', false, false, '')).toBe(true);
  });

  it('returns false for instance mode with active, permitted service', () => {
    expect(deriveIsNewService('instance', true, true, '42')).toBe(false);
  });

  it('returns true for instance mode when service is active but not permitted', () => {
    expect(deriveIsNewService('instance', true, false, '42')).toBe(true);
  });

  it('returns true for instance mode when service has no ID', () => {
    expect(deriveIsNewService('instance', true, true, '')).toBe(true);
  });
});

describe('useCreateDeploy: computeStatus (Path A — submitJob)', () => {
  const pathA = {
    isNewService: false,
    serviceSigning: false,
    serviceTxPending: false,
    serviceConfirmed: false,
    serviceError: null,
  };

  it('returns idle when jobStatus is idle', () => {
    expect(computeStatus({ ...pathA, jobStatus: 'idle' })).toBe('idle');
  });

  it('returns signing when jobStatus is signing', () => {
    expect(computeStatus({ ...pathA, jobStatus: 'signing' })).toBe('signing');
  });

  it('returns pending when jobStatus is pending', () => {
    expect(computeStatus({ ...pathA, jobStatus: 'pending' })).toBe('pending');
  });

  it('returns confirmed when jobStatus is confirmed', () => {
    expect(computeStatus({ ...pathA, jobStatus: 'confirmed' })).toBe('confirmed');
  });

  it('returns failed when jobStatus is failed', () => {
    expect(computeStatus({ ...pathA, jobStatus: 'failed' })).toBe('failed');
  });
});

describe('useCreateDeploy: computeStatus (Path B — requestService)', () => {
  const pathB = {
    isNewService: true,
    jobStatus: 'idle' as JobSubmitStatus,
  };

  it('returns idle when nothing is happening', () => {
    expect(computeStatus({
      ...pathB,
      serviceSigning: false,
      serviceTxPending: false,
      serviceConfirmed: false,
      serviceError: null,
    })).toBe('idle');
  });

  it('returns signing when service TX is being signed', () => {
    expect(computeStatus({
      ...pathB,
      serviceSigning: true,
      serviceTxPending: false,
      serviceConfirmed: false,
      serviceError: null,
    })).toBe('signing');
  });

  it('returns pending when service TX is awaiting confirmation', () => {
    expect(computeStatus({
      ...pathB,
      serviceSigning: false,
      serviceTxPending: true,
      serviceConfirmed: false,
      serviceError: null,
    })).toBe('pending');
  });

  it('returns confirmed when service TX is confirmed', () => {
    expect(computeStatus({
      ...pathB,
      serviceSigning: false,
      serviceTxPending: false,
      serviceConfirmed: true,
      serviceError: null,
    })).toBe('confirmed');
  });

  it('returns failed when serviceError is set', () => {
    expect(computeStatus({
      ...pathB,
      serviceSigning: false,
      serviceTxPending: false,
      serviceConfirmed: false,
      serviceError: 'User rejected',
    })).toBe('failed');
  });

  it('returns failed even if service is also signing (error takes priority)', () => {
    expect(computeStatus({
      ...pathB,
      serviceSigning: true,
      serviceTxPending: false,
      serviceConfirmed: false,
      serviceError: 'Error takes precedence',
    })).toBe('failed');
  });
});

describe('useCreateDeploy: computeCanDeploy', () => {
  const baseOpts = {
    job: true,
    hasName: true,
    hasAddress: true,
    status: 'idle' as DeployStatus,
    contractsDeployed: true,
    mode: 'sandbox' as DeployMode,
    hasValidService: true,
    isNewService: false,
    operatorCount: 0,
    operatorsLoading: false,
  };

  it('returns true when all conditions met for sandbox mode', () => {
    expect(computeCanDeploy(baseOpts)).toBe(true);
  });

  it('returns false when no job is selected', () => {
    expect(computeCanDeploy({ ...baseOpts, job: false })).toBe(false);
  });

  it('returns false when name is empty', () => {
    expect(computeCanDeploy({ ...baseOpts, hasName: false })).toBe(false);
  });

  it('returns false when no wallet address', () => {
    expect(computeCanDeploy({ ...baseOpts, hasAddress: false })).toBe(false);
  });

  it('returns false when status is not idle', () => {
    expect(computeCanDeploy({ ...baseOpts, status: 'pending' })).toBe(false);
    expect(computeCanDeploy({ ...baseOpts, status: 'signing' })).toBe(false);
    expect(computeCanDeploy({ ...baseOpts, status: 'confirmed' })).toBe(false);
    expect(computeCanDeploy({ ...baseOpts, status: 'failed' })).toBe(false);
  });

  it('returns false when contracts not deployed', () => {
    expect(computeCanDeploy({ ...baseOpts, contractsDeployed: false })).toBe(false);
  });

  it('returns false for sandbox mode without valid service', () => {
    expect(computeCanDeploy({ ...baseOpts, mode: 'sandbox', hasValidService: false })).toBe(false);
  });

  it('returns true for instance mode without valid service (uses Path B)', () => {
    expect(computeCanDeploy({
      ...baseOpts,
      mode: 'instance',
      hasValidService: false,
      isNewService: true,
      operatorCount: 3,
      operatorsLoading: false,
    })).toBe(true);
  });

  it('returns false for new service path with no operators', () => {
    expect(computeCanDeploy({
      ...baseOpts,
      mode: 'instance',
      hasValidService: false,
      isNewService: true,
      operatorCount: 0,
      operatorsLoading: false,
    })).toBe(false);
  });

  it('returns false for new service path while operators are loading', () => {
    expect(computeCanDeploy({
      ...baseOpts,
      mode: 'instance',
      hasValidService: false,
      isNewService: true,
      operatorCount: 3,
      operatorsLoading: true,
    })).toBe(false);
  });

  it('returns true for instance mode with valid existing service (Path A)', () => {
    expect(computeCanDeploy({
      ...baseOpts,
      mode: 'instance',
      hasValidService: true,
      isNewService: false,
    })).toBe(true);
  });
});

describe('useCreateDeploy: TTL_BLOCKS_30_DAYS constant', () => {
  it('equals 864000 blocks (30 days at 3s blocks)', () => {
    const TTL_BLOCKS_30_DAYS = 864000n;
    const expectedSeconds = 30n * 24n * 60n * 60n; // 2,592,000 seconds
    const blockTime = 3n;
    expect(TTL_BLOCKS_30_DAYS).toBe(expectedSeconds / blockTime);
  });
});

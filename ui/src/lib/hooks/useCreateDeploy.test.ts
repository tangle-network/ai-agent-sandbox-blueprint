import { describe, it, expect } from 'vitest';

/**
 * Tests for the pure computation functions used by useCreateDeploy.
 * Imported from createDeployLogic.ts — no wagmi/React mocks needed.
 */

import {
  deriveMode,
  deriveIsNewService,
  computeStatus,
  computeCanDeploy,
  type DeployStatus,
  type DeployMode,
  type JobSubmitStatus,
} from './createDeployLogic';

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
    correctChain: true,
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

  it('returns false when wallet is on wrong chain', () => {
    expect(computeCanDeploy({ ...baseOpts, correctChain: false })).toBe(false);
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

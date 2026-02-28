/**
 * useCreateDeploy — unified deploy hook for the create wizard.
 *
 * Abstracts the two deployment paths behind a single state machine:
 *   Path A (sandbox / instance with existing service): submitJob → watch provision
 *   Path B (instance without service): requestService → watch ServiceActivated → watch OperatorProvisioned
 *
 * The hook owns all TX lifecycle state, store updates, and provision watching,
 * so the consuming component only needs to render based on `status` + `provision`.
 */

import { useState, useCallback, useEffect, useMemo, useRef } from 'react';
import { useAccount, useWriteContract, useWaitForTransactionReceipt } from 'wagmi';
import { encodeJobArgs } from '@tangle/blueprint-ui';
import { tangleServicesAbi } from '~/lib/contracts/abi';
import { getAddresses } from '@tangle/blueprint-ui';
import { useSubmitJob } from '@tangle/blueprint-ui';
import { useOperators, type DiscoveredOperator } from '@tangle/blueprint-ui';
import {
  deriveMode,
  deriveIsNewService,
  computeStatus,
  computeCanDeploy,
  type DeployMode,
  type DeployStatus,
  type JobSubmitStatus,
} from './createDeployLogic';
import { useInstanceProvisionWatcher } from '~/lib/hooks/useProvisionWatcher';
import { addSandbox, updateSandboxStatus } from '~/lib/stores/sandboxes';
import { addInstance, updateInstanceStatus } from '~/lib/stores/instances';
import type { BlueprintDefinition, JobDefinition } from '~/lib/blueprints';
import { isContractDeployed, type SandboxAddresses } from '~/lib/contracts/chains';
import type { InfraConfig } from '@tangle/blueprint-ui';
import type { Address } from 'viem';

// Re-export types from logic module for external consumers
export type { DeployMode, DeployStatus, JobSubmitStatus } from './createDeployLogic';

export interface ProvisionInfo {
  sandboxId: string;
  sidecarUrl: string;
}

export interface CreateDeployState {
  status: DeployStatus;
  txHash?: `0x${string}`;
  error?: string;
  callId?: number;
  provision?: ProvisionInfo;
  /** Discovered operators (only relevant for instance mode without existing service) */
  operators: DiscoveredOperator[];
  operatorsLoading: boolean;
  /** Whether we're creating a new service (instance Path B) vs submitting a job (Path A) */
  isNewService: boolean;
}

interface UseCreateDeployOpts {
  blueprint: BlueprintDefinition | undefined;
  job: JobDefinition | null;
  values: Record<string, unknown>;
  infra: InfraConfig;
  validate: () => boolean;
}

/** ~30 days at 3s blocks */
const TTL_BLOCKS_30_DAYS = 864000n;

// ── Hook ──

export function useCreateDeploy({ blueprint, job, values, infra, validate }: UseCreateDeployOpts) {
  const { address } = useAccount();

  const mode = deriveMode(blueprint?.id);
  const isTeeInstance = blueprint?.id === 'ai-agent-tee-instance-blueprint';
  const isInstanceMode = mode === 'instance';

  // Store latest values in refs to avoid stale closures in useEffect hooks.
  // These change on every keystroke, but the effects should only fire on their
  // specific trigger deps (callId, serviceConfirmed, instanceProvision).
  const valuesRef = useRef(values);
  valuesRef.current = values;

  const modeRef = useRef(mode);
  modeRef.current = mode;

  const infraRef = useRef(infra);
  infraRef.current = infra;

  const isTeeInstanceRef = useRef(isTeeInstance);
  isTeeInstanceRef.current = isTeeInstance;

  // Path A: submitJob (sandbox, or instance with existing service)
  const {
    submitJob, status: jobStatus, error: jobError, txHash: jobTxHash, callId, reset: resetJob,
  } = useSubmitJob();

  // Path B: requestService (instance without existing service)
  const {
    writeContractAsync: requestServiceWrite,
    data: serviceTxHash,
    isPending: serviceSigning,
    reset: resetServiceTx,
  } = useWriteContract();
  const {
    isSuccess: serviceConfirmed,
    isLoading: serviceTxPending,
  } = useWaitForTransactionReceipt({ hash: serviceTxHash });

  // Service error tracking for Path B
  const [serviceError, setServiceError] = useState<string | null>(null);

  // Operator discovery (instance mode)
  const { operators, isLoading: operatorsLoading } = useOperators(
    isInstanceMode ? BigInt(infra.blueprintId || '0') : 0n,
  );

  // Check if existing service is valid (cached in infra store)
  const hasValidService = !!(
    infra.serviceInfo?.active &&
    infra.serviceInfo?.permitted &&
    infra.serviceId
  );

  // Whether we're creating a new service (Path B) vs submitting a job (Path A)
  const isNewService = isInstanceMode && !hasValidService;

  // ── Unified status ──

  const status = useMemo<DeployStatus>(
    () => computeStatus({ isNewService, jobStatus, serviceSigning, serviceTxPending, serviceConfirmed, serviceError }),
    [isNewService, jobStatus, serviceSigning, serviceTxPending, serviceConfirmed, serviceError],
  );

  const txHash = isNewService ? serviceTxHash : jobTxHash;
  const error = isNewService ? serviceError : jobError;

  // ── Instance provision watching ──

  const instanceProvision = useInstanceProvisionWatcher(
    infra.serviceId ? BigInt(infra.serviceId) : null,
    isTeeInstance ? 'tee-instance' : 'instance',
    status === 'confirmed' && isInstanceMode,
  );

  // ── Store updates ──

  // Update store when submitJob callId is parsed from receipt
  useEffect(() => {
    if (callId != null) {
      const name = String(valuesRef.current.name || '');
      if (!name) return;
      if (modeRef.current === 'sandbox') {
        updateSandboxStatus(name, 'creating', { callId });
      } else {
        updateInstanceStatus(name, 'creating', { callId });
      }
    }
  }, [callId]);

  // Add instance to store when requestService confirms
  useEffect(() => {
    if (serviceConfirmed && isInstanceMode) {
      const v = valuesRef.current;
      const name = String(v.name || '');
      if (!name) return;
      addInstance({
        id: name,
        name,
        image: String(v.image || ''),
        cpuCores: Number(v.cpuCores) || 2,
        memoryMb: Number(v.memoryMb) || 2048,
        diskGb: Number(v.diskGb) || 10,
        createdAt: Date.now(),
        blueprintId: infraRef.current.blueprintId,
        serviceId: infraRef.current.serviceId || '',
        teeEnabled: isTeeInstanceRef.current,
        status: 'creating',
        txHash: serviceTxHash,
      });
    }
  }, [serviceConfirmed, isInstanceMode, serviceTxHash]);

  // Update store when instance provision event arrives
  useEffect(() => {
    if (instanceProvision) {
      const name = String(valuesRef.current.name || '');
      if (name) {
        updateInstanceStatus(name, 'running', {
          id: instanceProvision.sandboxId,
          sidecarUrl: instanceProvision.sidecarUrl,
        });
      }
    }
  }, [instanceProvision]);

  // ── Deploy action ──

  const deploy = useCallback(async () => {
    if (!job || !validate()) return;

    const args = encodeJobArgs(job, values);
    const name = String(values.name || '');

    // Path A: Submit job to existing service
    if (!isNewService) {
      const hash = await submitJob({
        serviceId: BigInt(infra.serviceId || '0'),
        jobId: job.id,
        args,
        label: `${job.label}: ${name}`,
        value: BigInt(job.pricingMultiplier ?? 50) * 1_000_000_000_000_000n,
      });

      if (hash) {
        const common = {
          id: name,
          name,
          image: String(values.image || ''),
          cpuCores: Number(values.cpuCores) || 2,
          memoryMb: Number(values.memoryMb) || 2048,
          diskGb: Number(values.diskGb) || 10,
          createdAt: Date.now(),
          blueprintId: infra.blueprintId,
          serviceId: infra.serviceId,
          status: 'creating' as const,
          txHash: hash,
        };
        if (mode === 'sandbox') {
          addSandbox({ ...common, teeEnabled: isTeeInstance || undefined });
        } else {
          addInstance({ ...common, teeEnabled: isTeeInstance });
        }
      }
      return;
    }

    // Path B: Create new service with config as requestInputs
    if (!address) return;
    const addrs = getAddresses<SandboxAddresses>();
    const ops = operators.map((o) => o.address);
    if (ops.length === 0) return;

    try {
      setServiceError(null);
      await requestServiceWrite({
        address: addrs.services,
        abi: tangleServicesAbi,
        functionName: 'requestService',
        args: [
          BigInt(infra.blueprintId),
          ops,
          args as `0x${string}`,
          [address as Address],
          TTL_BLOCKS_30_DAYS,
          '0x0000000000000000000000000000000000000000' as Address,
          0n,
        ],
      });
    } catch (err: unknown) {
      const message =
        err instanceof Error
          ? (err as Error & { shortMessage?: string }).shortMessage ?? err.message
          : String(err);
      setServiceError(message || 'Service creation failed');
    }
  }, [job, values, infra, validate, isNewService, submitJob, address, operators, requestServiceWrite, mode, isTeeInstance]);

  // ── Reset ──

  const reset = useCallback(() => {
    resetJob();
    resetServiceTx();
    setServiceError(null);
  }, [resetJob, resetServiceTx]);

  // ── Computed flags ──

  // Check if contracts are deployed on the current network
  const addrs = getAddresses<SandboxAddresses>();
  const contractsDeployed = isContractDeployed(addrs.jobs) && isContractDeployed(addrs.services);

  const canDeploy = computeCanDeploy({
    job: !!job,
    hasName: !!values.name,
    hasAddress: !!address,
    status,
    contractsDeployed,
    mode,
    hasValidService,
    isNewService,
    operatorCount: operators.length,
    operatorsLoading,
  });

  return {
    // State
    mode,
    status,
    txHash,
    error,
    callId: callId ?? undefined,
    provision: instanceProvision ?? undefined,
    operators,
    operatorsLoading,
    isNewService,
    isInstanceMode,
    isTeeInstance,
    hasValidService,
    contractsDeployed,
    canDeploy,
    // Actions
    deploy,
    reset,
  };
}
